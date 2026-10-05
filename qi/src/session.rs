//! Sessions: authenticated messaging links to peers.
//!
//! A session wraps a messaging endpoint with the control protocol (authentication and
//! capabilities), the hosting of objects transmitted to the peer, the subscriptions to the
//! signals of the peer, and the conversions of values (with their objects) to and from the wire.
//!
//! Sessions are symmetrical once established: both peers may call, post and subscribe to the
//! objects of the other. The connecting side authenticates to the accepting side first.

pub(crate) mod capabilities;
mod control;
mod handler;
mod host;
mod store;
mod target;

pub use self::capabilities::Capabilities;
pub(crate) use self::store::Store;
pub use self::target::Target;
use self::{control::Control, handler::SessionHandler, host::ObjectHost};
use crate::messaging::Address;
use crate::{
    auth::Authenticator,
    error::Error,
    messaging::{self, message, Flags},
    object::{self, ObjectClient},
    service::{self, SharedServices},
    signal::{Link, ValueStream},
    value::{de, KeyDynValueMap, Signature, Type, Value},
    Result,
};
use bytes::Bytes;
use futures::{stream::FusedStream, Sink, StreamExt, TryStream};
use serde::de::DeserializeSeed;
use std::{
    collections::HashMap,
    net::SocketAddr,
    pin::pin,
    sync::{Arc, Mutex, Weak},
};
use tokio::{
    select,
    sync::{broadcast, watch},
    task, time,
};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};
use tracing::{debug, trace};

/// The state of a session, shared between the session handles, the proxies of objects of the
/// peer and the messaging handler.
pub(crate) struct SessionState {
    client: messaging::Client,
    capabilities: watch::Receiver<Option<Capabilities>>,
    /// Objects transmitted to the peer.
    host: Mutex<ObjectHost>,
    /// Subscriptions of the peer to signals of local objects.
    event_links: Mutex<HashMap<Link, AbortOnDropHandle<()>>>,
    /// Subscriptions to signals of remote objects.
    subscriptions: tokio::sync::Mutex<HashMap<message::Address, RemoteSubscription>>,
    /// Cancelled when the messaging link is closed.
    closed: CancellationToken,
    weak_self: Weak<SessionState>,
}

struct RemoteSubscription {
    sender: broadcast::Sender<Event>,
    link: Link,
}

#[derive(Clone)]
struct Event {
    payload: Bytes,
    flags: Flags,
}

impl SessionState {
    /// The capabilities shared with the peer. Before negotiation, no capability is assumed.
    pub(crate) fn capabilities(&self) -> Capabilities {
        self.capabilities.borrow().unwrap_or(Capabilities::NONE)
    }

    fn weak_session(&self) -> WeakSession {
        WeakSession(Weak::clone(&self.weak_self))
    }

    fn session(&self) -> Option<Session> {
        self.weak_self.upgrade().map(Session)
    }

    /// Decodes a value of the given type from a payload, according to the capabilities of the
    /// session.
    pub(crate) fn decode(&self, payload: &[u8], ty: Option<&Type>) -> Result<Value<'static>> {
        let value = de::ValueType::new(ty)
            .with_object_uid_on_wire(self.capabilities().object_ptr_uid)
            .deserialize(crate::format::SliceDeserializer::new(payload))
            .map_err(crate::FormatError::ArgumentsDeserialization)?;
        Ok(value.into_owned())
    }

    /// Encodes a value to a payload.
    pub(crate) fn encode(&self, value: &Value<'_>) -> Result<Bytes> {
        crate::format::to_bytes(value)
            .map_err(crate::FormatError::ArgumentsSerialization)
            .map_err(Into::into)
    }

    /// Binds the objects of an outgoing value: local objects are hosted by the session for the
    /// peer, and their references are addressed under the given service.
    pub(crate) fn bind_outgoing<'a>(
        &self,
        value: Value<'a>,
        service_id: service::Id,
    ) -> Result<Value<'a>> {
        let uid_on_wire = self.capabilities().object_ptr_uid;
        let mut host = self.host.lock().unwrap_or_else(|err| err.into_inner());
        Ok(map_objects(value, &mut |mut reference| {
            if let Some(object) = object::bound_object(&reference) {
                let meta = object::generic::merge(object.meta());
                let id = host.add(service_id, object);
                reference.meta_object = meta;
                reference.service_id = service_id;
                reference.object_id = id;
                reference.take_handle();
            }
            reference.set_uid_on_wire(uid_on_wire);
            reference
        }))
    }

    /// Binds the objects of an incoming value: references to remote objects are bound to
    /// proxies over this session.
    pub(crate) fn bind_incoming<'a>(&self, value: Value<'a>) -> Value<'a> {
        let Some(session) = self.session() else {
            return value;
        };
        map_objects(value, &mut |mut reference| {
            if reference.handle().is_none() && reference.is_addressed() {
                let client = ObjectClient::new(
                    reference.service_id,
                    reference.object_id,
                    reference.object_uid,
                    reference.meta_object.clone(),
                    session.clone(),
                    true,
                );
                reference.set_handle(Arc::new(client));
            }
            reference
        })
    }

    fn register_event_link(&self, link: Link, task: AbortOnDropHandle<()>) {
        self.event_links
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(link, task);
    }

    fn unregister_event(&self, link: Link) -> Result<()> {
        let removed = self
            .event_links
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(&link);
        match removed {
            Some(_) => Ok(()),
            None if !link.is_valid() => Ok(()),
            None => Err(Error::Other(
                format!("Unregister request failed for {link}").into(),
            )),
        }
    }

    fn terminate_hosted(&self, object_id: object::Id) {
        if object_id == object::MAIN_OBJECT_ID {
            debug!("terminate() received on the main object of a service, ignored");
            return;
        }
        let removed = self
            .host
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(object_id);
        if removed.is_none() {
            debug!(%object_id, "terminate() received for an unknown hosted object");
        }
    }

    /// Dispatches an event received from the peer to the local subscriptions.
    fn dispatch_event(&self, address: message::Address, payload: Bytes, flags: Flags) {
        let subscriptions = self.subscriptions.try_lock();
        let sender = match &subscriptions {
            Ok(subscriptions) => subscriptions
                .get(&address)
                .map(|subscription| subscription.sender.clone()),
            Err(_) => {
                // The subscriptions are being modified: dispatch asynchronously.
                let Some(session) = self.session() else {
                    return;
                };
                task::spawn(async move {
                    let subscriptions = session.0.subscriptions.lock().await;
                    if let Some(subscription) = subscriptions.get(&address) {
                        let _res = subscription.sender.send(Event { payload, flags });
                    }
                });
                return;
            }
        };
        match sender {
            Some(sender) => {
                let _res = sender.send(Event { payload, flags });
            }
            None => trace!(%address, "event discarded: no subscription"),
        }
    }
}

impl std::fmt::Debug for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionState")
            .field("capabilities", &self.capabilities())
            .field("closed", &self.closed.is_cancelled())
            .finish_non_exhaustive()
    }
}

/// Applies a function to every object reference of a value, recursively.
fn map_objects<'a, F>(value: Value<'a>, f: &mut F) -> Value<'a>
where
    F: FnMut(object::Reference) -> object::Reference,
{
    match value {
        Value::Object(reference) => Value::Object(Box::new(f(*reference))),
        Value::Option(Some(inner)) => Value::Option(Some(Box::new(map_objects(*inner, f)))),
        Value::List(list) => Value::List(list.into_iter().map(|v| map_objects(v, f)).collect()),
        Value::Tuple(tuple) => Value::Tuple(tuple.into_iter().map(|v| map_objects(v, f)).collect()),
        Value::Map(map) => Value::Map(
            map.into_iter()
                .map(|(k, v)| (map_objects(k, f), map_objects(v, f)))
                .collect(),
        ),
        Value::Dynamic(inner) => Value::Dynamic(Box::new(map_objects(*inner, f))),
        value => value,
    }
}

/// A handle to a session.
///
/// Sessions are cheap to clone. The messaging link is closed when the last handle is dropped.
#[derive(Clone)]
pub(crate) struct Session(pub(crate) Arc<SessionState>);

impl Session {
    /// Establishes a session with a peer as the connecting side.
    pub(crate) async fn connect<MsgStream, MsgSink>(
        messages_in: MsgStream,
        messages_out: MsgSink,
        credentials: KeyDynValueMap,
        services: SharedServices,
    ) -> Result<Self>
    where
        MsgStream: TryStream<Ok = messaging::Message> + Send + 'static,
        MsgStream::Error: Send,
        MsgSink: Sink<messaging::Message> + Send + 'static,
        MsgSink::Error: Send,
    {
        let (session, controller) = Self::start(
            messages_in,
            messages_out,
            None,
            true,
            services,
            host::CLIENT_FIRST_ID,
        );
        controller
            .authenticate_to_server(&session.0.client, credentials)
            .await?;
        Ok(session)
    }

    /// Starts the messaging of a session, without authentication.
    fn start<MsgStream, MsgSink>(
        messages_in: MsgStream,
        messages_out: MsgSink,
        authenticator: Option<Arc<dyn Authenticator + Send + Sync>>,
        remote_authorized: bool,
        services: SharedServices,
        first_hosted_id: u32,
    ) -> (Self, control::Controller)
    where
        MsgStream: TryStream<Ok = messaging::Message> + Send + 'static,
        MsgStream::Error: Send,
        MsgSink: Sink<messaging::Message> + Send + 'static,
        MsgSink::Error: Send,
    {
        let closed = CancellationToken::new();
        let mut controller = None;
        let state = Arc::new_cyclic(|weak: &Weak<SessionState>| {
            let handler = SessionHandler::new(WeakSession(Weak::clone(weak)), services);
            let Control {
                controller: control,
                capabilities,
                handler,
            } = control::create(handler, authenticator, remote_authorized);
            let (client, connection) =
                messaging::endpoint::start(messages_in, messages_out, handler);
            task::spawn({
                let closed = closed.clone();
                async move {
                    let _res = connection.await;
                    closed.cancel();
                }
            });
            controller = Some(control);
            SessionState {
                client,
                capabilities,
                host: Mutex::new(ObjectHost::new(first_hosted_id)),
                event_links: Default::default(),
                subscriptions: Default::default(),
                closed,
                weak_self: Weak::clone(weak),
            }
        });
        let controller = controller.expect("controller is set by the state constructor");
        (Self(state), controller)
    }

    /// The capabilities shared with the peer.
    pub(crate) fn capabilities(&self) -> Capabilities {
        self.0.capabilities()
    }

    /// Calls a member of an object of the peer with a tuple of arguments, and returns its
    /// result decoded with the given type.
    pub(crate) async fn call(
        &self,
        address: message::Address,
        args: Value<'_>,
        return_type: Option<&Type>,
    ) -> Result<Value<'static>> {
        let args = self.0.bind_outgoing(args, address.service())?;
        let payload = self.0.encode(&args)?;
        let reply = self.0.client.call(address, payload, Flags::NONE).await?;
        let value = if reply.flags.is_dynamic_payload() {
            match self.0.decode(&reply.payload, None)? {
                Value::Dynamic(inner) => *inner,
                value => value,
            }
        } else {
            self.0
                .decode(&reply.payload, return_type)
                .map_err(|err| match err {
                    Error::Other(err) => crate::FormatError::MethodReturnValueDeserialization(
                        crate::format::Error::Custom(err.to_string()),
                    )
                    .into(),
                    err => err,
                })?
        };
        Ok(self.0.bind_incoming(value))
    }

    /// Posts a tuple of arguments to a member of an object of the peer, without waiting.
    pub(crate) fn post(&self, address: message::Address, args: Value<'_>) -> Result<()> {
        let args = self.0.bind_outgoing(args, address.service())?;
        let payload = self.0.encode(&args)?;
        match self
            .0
            .client
            .try_post(address, payload.clone(), Flags::NONE)
        {
            Ok(()) => Ok(()),
            Err(_) => {
                // The requests buffer is full, or the link is closed: retry asynchronously.
                let client = self.0.client.clone();
                task::spawn(async move {
                    if let Err(error) = client.post(address, payload, Flags::NONE).await {
                        debug!(
                            error = &error as &dyn std::error::Error,
                            "post request error: failure to send"
                        );
                    }
                });
                Ok(())
            }
        }
    }

    /// Subscribes to a signal of an object of the peer, returning the stream of its parameters
    /// tuples decoded with the given signature.
    pub(crate) async fn subscribe(
        &self,
        address: message::Address,
        signature: Signature,
    ) -> Result<ValueStream> {
        let (receiver, guard) = {
            let mut subscriptions = self.0.subscriptions.lock().await;
            match subscriptions.get(&address) {
                Some(subscription) => (
                    subscription.sender.subscribe(),
                    SubscriptionGuard {
                        session: self.0.weak_session(),
                        address,
                    },
                ),
                None => {
                    let link = Link::next();
                    let (sender, receiver) = broadcast::channel(crate::signal::DEFAULT_CAPACITY);
                    // Register the event with the peer, holding the lock so that concurrent
                    // subscriptions to the same signal wait for the registration.
                    let () = self
                        .call(
                            address.with_action(object::generic::REGISTER_EVENT),
                            Value::Tuple(vec![
                                Value::UInt32(address.service().into()),
                                Value::UInt32(address.action().into()),
                                Value::UInt64(link.0),
                            ]),
                            Some(&Type::UInt64),
                        )
                        .await
                        .map(|_| ())?;
                    subscriptions.insert(address, RemoteSubscription { sender, link });
                    (
                        receiver,
                        SubscriptionGuard {
                            session: self.0.weak_session(),
                            address,
                        },
                    )
                }
            }
        };
        let session = self.clone();
        let params_type = signature.into_type();
        let stream =
            tokio_stream::wrappers::BroadcastStream::new(receiver).filter_map(move |event| {
                let session = session.clone();
                let params_type = params_type.clone();
                async move {
                    let event = match event {
                        Ok(event) => event,
                        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(
                            n,
                        )) => {
                            debug!(count = n, "signal events dropped: subscriber lagged");
                            return None;
                        }
                    };
                    let value = if event.flags.is_dynamic_payload() {
                        session
                            .0
                            .decode(&event.payload, None)
                            .map(|value| match value {
                                Value::Dynamic(inner) => *inner,
                                value => value,
                            })
                    } else {
                        session.0.decode(&event.payload, params_type.as_ref())
                    };
                    match value {
                        Ok(value) => Some(session.0.bind_incoming(value)),
                        Err(error) => {
                            debug!(
                                error = &error as &dyn std::error::Error,
                                "signal event dropped: failure to decode parameters"
                            );
                            None
                        }
                    }
                }
            });
        Ok(SubscriptionStream {
            stream,
            _guard: guard,
        }
        .boxed())
    }

    /// Notifies the peer that a hosted object is no longer referenced. Fire and forget.
    pub(crate) fn terminate_object(&self, service_id: service::Id, id: object::Id) {
        let address = message::Address(service_id, id, object::generic::TERMINATE);
        let args = Value::Tuple(vec![Value::UInt32(id.into())]);
        if let Err(error) = self.post(address, args) {
            debug!(
                error = &error as &dyn std::error::Error,
                "failed to notify the peer of the termination of a hosted object"
            );
        }
    }

    pub(crate) fn downgrade(&self) -> WeakSession {
        WeakSession(Arc::downgrade(&self.0))
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Session").field(&self.0).finish()
    }
}

pin_project_lite::pin_project! {
    struct SubscriptionStream<S> {
        #[pin]
        stream: S,
        _guard: SubscriptionGuard,
    }
}

impl<S> futures::Stream for SubscriptionStream<S>
where
    S: futures::Stream,
{
    type Item = S::Item;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.project().stream.poll_next(cx)
    }
}

/// Unregisters the event from the peer when the last local subscriber is dropped.
struct SubscriptionGuard {
    session: WeakSession,
    address: message::Address,
}

impl Drop for SubscriptionGuard {
    fn drop(&mut self) {
        let Some(session) = self.session.upgrade() else {
            return;
        };
        let address = self.address;
        task::spawn(async move {
            let link = {
                let mut subscriptions = session.0.subscriptions.lock().await;
                match subscriptions.get(&address) {
                    Some(subscription) if subscription.sender.receiver_count() == 0 => {
                        subscriptions.remove(&address).map(|sub| sub.link)
                    }
                    _ => None,
                }
            };
            if let Some(link) = link {
                let result = session
                    .call(
                        address.with_action(object::generic::UNREGISTER_EVENT),
                        Value::Tuple(vec![
                            Value::UInt32(address.service().into()),
                            Value::UInt32(address.action().into()),
                            Value::UInt64(link.0),
                        ]),
                        Some(&Type::Unit),
                    )
                    .await;
                if let Err(error) = result {
                    debug!(
                        error = &error as &dyn std::error::Error,
                        "failed to unregister an event from the peer"
                    );
                }
            }
        });
    }
}

#[derive(Clone)]
pub(crate) struct WeakSession(Weak<SessionState>);

impl WeakSession {
    pub(crate) fn upgrade(&self) -> Option<Session> {
        self.0.upgrade().map(Session)
    }
}

impl std::fmt::Debug for WeakSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WeakSession")
    }
}

/// Binds a server of sessions to an address.
///
/// Spawn a server task that:
///   1) establishes a session as the accepting side, with the given authenticator, each time a
///      client connects to the server.
///   2) updates a list of endpoints for this server. The list of endpoints changes if the address
///      targets multiple interfaces and interfaces availability changes on the system.
///
/// The future terminates when the server is bound and clients can connect. The return value is a
/// watch receiver of a pair of:
///   - a local address that the server is bound to.
///   - a list of endpoints that clients can connect to.
///
/// The receiver is severed from its sender when the server is stopped.
pub(crate) async fn server(
    address: messaging::Address,
    authenticator: Option<Arc<dyn Authenticator + Send + Sync>>,
    services: SharedServices,
) -> std::result::Result<(Server, ServerEndpointsWatcher), std::io::Error> {
    let (clients, local_address) = messaging::channel::serve(address).await?;
    let (mut endpoints_sender, endpoints_receiver) =
        watch::channel((local_address, address_endpoints(local_address)));
    let task = task::spawn(async move {
        let mut clients = pin!(clients.fuse());
        let mut update_endpoints = pin!(update_address_endpoints(
            local_address,
            &mut endpoints_sender
        ));
        // Use a join set so that when this task is dropped, all spawned client session tasks are aborted.
        let mut client_tasks = task::JoinSet::new();
        loop {
            select! {
                Some((messages_stream, messages_sink, _address)) = clients.next(), if !clients.is_terminated() => {
                    client_tasks.spawn(serve_client(
                        messages_stream,
                        messages_sink,
                        authenticator.clone(),
                        services.clone(),
                    ));
                }
                () = &mut update_endpoints => {
                    // nothing, if this future terminates it means that the address was not an
                    // "ANY" IP address. The endpoints sender must not be dropped.
                }
                else => {
                    break;
                }
            }
        }
    });
    Ok((Server(AbortOnDropHandle::new(task)), endpoints_receiver))
}

/// Serves a client of a session server: establishes the session as the accepting side and keeps
/// it alive until the link is closed.
pub(crate) async fn serve_client<MsgStream, MsgSink>(
    messages_stream: MsgStream,
    messages_sink: MsgSink,
    authenticator: Option<Arc<dyn Authenticator + Send + Sync>>,
    services: SharedServices,
) where
    MsgStream: TryStream<Ok = messaging::Message> + Send + 'static,
    MsgStream::Error: Send,
    MsgSink: Sink<messaging::Message> + Send + 'static,
    MsgSink::Error: Send,
{
    let (session, _controller) = Session::start(
        messages_stream,
        messages_sink,
        authenticator,
        false,
        services,
        host::SERVER_FIRST_ID,
    );
    session.0.closed.cancelled().await;
}

#[derive(Debug)]
pub(crate) struct Server(#[allow(dead_code)] AbortOnDropHandle<()>);

pub(crate) type ServerEndpointsWatcher = watch::Receiver<(Address, Vec<Address>)>;

const NETWORK_INTERFACES_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// Lists the endpoints clients can connect to for a local address.
///
/// An "ANY" (unspecified) IP address is bound to all the network interfaces of the host: each of
/// their addresses is an endpoint. Any other address is its own endpoint.
fn address_endpoints(local_address: Address) -> Vec<Address> {
    match local_address {
        Address::Tcp {
            address: local_socket_address,
            ssl,
        } if local_socket_address.ip().is_unspecified() => {
            let networks = sysinfo::Networks::new_with_refreshed_list();
            let mut endpoints: Vec<_> = networks
                .values()
                .flat_map(|net| net.ip_networks())
                .filter(|ip_net| ip_net.addr.is_ipv4() == local_socket_address.is_ipv4())
                .map(|ip_net| Address::Tcp {
                    address: SocketAddr::new(ip_net.addr, local_socket_address.port()),
                    ssl,
                })
                .collect();
            endpoints.sort();
            endpoints.dedup();
            endpoints
        }
        _ => vec![local_address],
    }
}

/// Returns a future that will update endpoints associated to a local address into the sender.
///
/// A local address can be bound to an "ANY" IP address, meaning that it is bound to all network
/// interfaces of the host system. This means that when the set of interfaces changes, so do local
/// endpoints. This future checks if the address is an "ANY" IP address and then continuously tracks
/// changes to the network interfaces to update the list of endpoints.
///
/// If the local address is not an "ANY" IP address, then the endpoints are updated immediately with
/// the local address and only that address and the future terminates.
///
/// In the endpoints tuple value, only the list of endpoints (the second element) is updated. The
/// first value (the local address) is never set by this function.
async fn update_address_endpoints(
    local_address: Address,
    endpoints_sender: &mut watch::Sender<(Address, Vec<Address>)>,
) {
    match local_address {
        // An "ANY" address, aka "unspecified".
        Address::Tcp {
            address: local_socket_address,
            ssl,
        } if local_socket_address.ip().is_unspecified() => {
            // Watch network interfaces changes to list all IP addresses of the host.
            let _ = (local_socket_address, ssl);
            loop {
                time::sleep(NETWORK_INTERFACES_REFRESH_INTERVAL).await;
                let new_endpoints = address_endpoints(local_address);
                endpoints_sender.send_if_modified(move |(_, endpoints)| {
                    if endpoints != &new_endpoints {
                        *endpoints = new_endpoints;
                        true
                    } else {
                        false
                    }
                });
            }
        }
        // Not an any address: the endpoints never change.
        _ => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth,
        messaging::Message,
        value::{FormatInto, IntoFormat},
    };
    use assert_matches::assert_matches;
    use futures::{channel::mpsc, SinkExt, StreamExt};
    use std::convert::Infallible;
    use tokio::spawn;

    fn caps_map() -> KeyDynValueMap {
        capabilities::local_map().clone()
    }

    /// The server expects authentication parameters, the client sends correct ones.
    ///
    /// It is expected that:
    ///   1. the authentication succeeds.
    #[tokio::test]
    async fn client_sends_good_auth_parameters() {
        let auth = auth::UserTokenAuthenticator::new("myuser".to_owned(), "mytoken".to_owned());

        // 0.1: start the server session
        let (mut send_to_server, server_recv) = mpsc::unbounded();
        let (server_send, mut recv_from_server) = mpsc::unbounded();
        spawn(serve_client(
            server_recv.map(Ok::<_, Infallible>),
            server_send.sink_map_err(crate::messaging::Error::link_lost),
            Some(Arc::new(auth)),
            SharedServices::default(),
        ));

        // 0.2: start the request
        send_to_server
            .send(Message::Call {
                id: message::Id(0),
                address: control::AUTHENTICATE_ADDRESS,
                payload: {
                    let mut map = caps_map();
                    map.set(auth::USER_KEY, "myuser");
                    map.set(auth::TOKEN_KEY, "mytoken");
                    map
                }
                .into_format()
                .unwrap(),
                flags: Flags::NONE,
            })
            .await
            .unwrap();

        // 1.
        let response = recv_from_server.next().await.unwrap();
        let body = assert_matches!(
            response,
            Message::Reply {
                address: control::AUTHENTICATE_ADDRESS,
                id: message::Id(0),
                payload: body,
                ..
            } => body
        );

        let mut map: KeyDynValueMap = body.to_reflect_value().unwrap();
        let state: u32 = map
            .remove(auth::STATE_KEY)
            .unwrap_or_else(|| panic!("missing state key in map {map:?}"))
            .cast_into()
            .expect("state value is not a u32");
        assert_eq!(state, auth::STATE_DONE);
        // The reply advertises the local capabilities.
        assert_eq!(Capabilities::from_map(&map), Capabilities::LOCAL);
    }

    /// The client sends bad authentication parameters.
    ///
    /// It is expected that:
    ///   1. the server replies with an error.
    ///   2. the connection is closed.
    #[tokio::test]
    async fn client_send_bad_auth_parameters() {
        let auth = auth::UserTokenAuthenticator::new("myuser".to_owned(), "mytoken".to_owned());

        // 0.1: start the server session
        let (mut send_to_server, server_recv) = mpsc::unbounded();
        let (server_send, mut recv_from_server) = mpsc::unbounded();
        let task = spawn(serve_client(
            server_recv.map(Ok::<_, Infallible>),
            server_send.sink_map_err(crate::messaging::Error::link_lost),
            Some(Arc::new(auth)),
            SharedServices::default(),
        ));

        // 0.2: start the request
        send_to_server
            .send(Message::Call {
                id: message::Id(0),
                address: control::AUTHENTICATE_ADDRESS,
                payload: {
                    let mut map = caps_map();
                    map.set(auth::USER_KEY, "myuser");
                    map.set(auth::TOKEN_KEY, "badtoken"); // token is not correct
                    map
                }
                .into_format()
                .unwrap(),
                flags: Flags::NONE,
            })
            .await
            .unwrap();

        // 1.
        let response = recv_from_server.next().await.unwrap();
        let error = assert_matches!(
            response,
            Message::Error {
                address: control::AUTHENTICATE_ADDRESS,
                error,
                ..
            } => error
        );
        assert!(
            error.contains("failure to verify authentication request"),
            "error is not an authentication failure: {error}"
        );

        // 2.
        let () = task.await.unwrap();
    }

    /// A client authenticates to a server that does not enforce authentication, and the shared
    /// capabilities reflect what the peer advertised.
    #[tokio::test]
    async fn client_and_server_negotiate_capabilities() {
        let (client_to_server_tx, client_to_server_rx) = mpsc::unbounded();
        let (server_to_client_tx, server_to_client_rx) = mpsc::unbounded();
        spawn(serve_client(
            client_to_server_rx.map(Ok::<_, Infallible>),
            server_to_client_tx.sink_map_err(crate::messaging::Error::link_lost),
            None,
            SharedServices::default(),
        ));
        let session = Session::connect(
            server_to_client_rx.map(Ok::<_, Infallible>),
            client_to_server_tx.sink_map_err(crate::messaging::Error::link_lost),
            Default::default(),
            SharedServices::default(),
        )
        .await
        .unwrap();
        assert_eq!(session.capabilities(), Capabilities::LOCAL);
    }
}
