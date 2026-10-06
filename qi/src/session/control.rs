//! The control protocol of sessions: authentication, capabilities negotiation and the detection
//! of the protocol variant of the peer.
//!
//! Control messages are addressed to the server object (service 0, object 0). In the standard
//! protocol, the only control call is the authentication call (action 8), which a connecting
//! peer must send first. Its arguments are a map of the capabilities of the peer and of its
//! credentials; the reply is a map of the capabilities of the accepting peer and of the
//! authentication state.
//!
//! Legacy peers (`libqi` up to 2.2, NAOqi 2.1) know no authentication: they advertise their
//! capabilities with a `Capabilities` message right after the connection and send their requests
//! directly. A legacy server answers the authentication call with an error. Both situations are
//! detected here, as the reference implementation does since `libqi` 2.3:
//!
//! - the connecting side sends the authentication call; an error reply after a `Capabilities`
//!   message of the peer means a legacy server, and the local capabilities are then advertised
//!   with a `Capabilities` message;
//! - the accepting side, when it enforces no authentication, accepts a first request that is not
//!   the authentication call as coming from a legacy client, and advertises its capabilities with
//!   a `Capabilities` message.
//!
//! The accepting side may also *emulate* a legacy server, for testing clients against the
//! handshake of NAOqi 2.1: it advertises its capabilities on connection and rejects the
//! authentication call like `libqi` 2.1 does.

use super::capabilities::{self, Capabilities, Protocol};
use super::WeakSession;
use crate::{
    auth::{self, Authenticator},
    error::{HandlerError, NoHandlerError},
    messaging::{self, handler, message, CancellationToken},
    object, service,
    value::{FormatInto, IntoFormat, KeyDynValueMap},
    Error,
};
use futures::{
    future::{err, ready},
    FutureExt, TryFutureExt,
};
use std::{future::Future, sync::Arc};
use tokio::sync::watch;
use tracing::{debug, info};

const SERVICE_ID: service::Id = service::Id(0);
const OBJECT_ID: object::Id = object::Id(0);
const AUTHENTICATE_ACTION_ID: object::ActionId = object::ActionId(8);

fn is_control_address(address: message::Address) -> bool {
    address.service() == SERVICE_ID && address.object() == OBJECT_ID
}

pub(crate) const AUTHENTICATE_ADDRESS: message::Address =
    message::Address(SERVICE_ID, OBJECT_ID, AUTHENTICATE_ACTION_ID);

/// The error a `libqi` 2.1 server answers to the authentication call with (there is no bound
/// object for service 0): emulated legacy servers answer the same.
pub(crate) const LEGACY_AUTHENTICATE_ERROR: &str = "can't find service, address: {0.0.8}";

#[derive(Clone)]
pub(super) struct Controller {
    authenticator: Option<Arc<dyn Authenticator + Send + Sync>>,
    /// The protocol this side speaks when it accepts a peer.
    server_protocol: Protocol,
    /// The capabilities the peer advertised, with a `Capabilities` message or in the
    /// authentication handshake.
    remote_map: watch::Sender<Option<KeyDynValueMap>>,
    capabilities: watch::Sender<Option<Capabilities>>,
    protocol: watch::Sender<Option<Protocol>>,
    remote_authorized: watch::Sender<bool>,
    session: WeakSession,
}

impl Controller {
    /// Handles an authentication request from the remote peer.
    fn authenticate(
        &self,
        request: KeyDynValueMap,
    ) -> Result<KeyDynValueMap, AuthenticateClientError> {
        if let Some(authenticator) = &self.authenticator {
            authenticator
                .authenticate(request.clone())
                .map_err(AuthenticateClientError::AuthenticationVerification)?;
        }
        self.remote_map.send_replace(Some(request));
        self.update_shared_capabilities();
        self.protocol.send_replace(Some(Protocol::Standard));
        self.remote_authorized.send_replace(true);
        // Like the reference implementation, the reply advertises the local capabilities, and the
        // remote computes the shared ones.
        Ok(auth::state_done_map(capabilities::local_map().clone()))
    }

    /// Records the capabilities the remote peer advertised with a `Capabilities` message.
    fn set_remote_capabilities(&self, map: KeyDynValueMap) {
        self.remote_map.send_replace(Some(map));
        self.update_shared_capabilities();
    }

    /// The map of local capabilities advertised to the peer.
    fn local_map(&self) -> &'static KeyDynValueMap {
        match self.server_protocol {
            Protocol::Standard => capabilities::local_map(),
            Protocol::Legacy => capabilities::legacy_local_map(),
        }
    }

    /// The capabilities shared with the peer, from what both sides advertise.
    fn shared_capabilities(&self) -> Capabilities {
        let local = Capabilities::from_map(self.local_map());
        match self.remote_map.borrow().as_ref() {
            Some(remote) => local.shared_with(Capabilities::from_map(remote)),
            None => Capabilities::NONE,
        }
    }

    fn update_shared_capabilities(&self) {
        let shared = self.shared_capabilities();
        self.capabilities.send_replace(Some(shared));
    }

    /// Whether the peer may send requests before authenticating: when no authentication is
    /// enforced, a first request that is not the authentication call comes from a legacy peer.
    fn authorizes_legacy_peer(&self) -> bool {
        self.authenticator.is_none()
    }

    /// Accepts the remote peer as a legacy client: it is authorized, and the local capabilities
    /// are advertised to it with a `Capabilities` message, as `libqi` 2.3+ servers do.
    fn accept_legacy_peer(&self) {
        if self.remote_authorized.send_replace(true) {
            return;
        }
        info!("accepting a peer that speaks the legacy protocol (NAOqi 2.1)");
        self.protocol.send_replace(Some(Protocol::Legacy));
        self.update_shared_capabilities();
        if self.server_protocol == Protocol::Standard {
            self.send_capabilities();
        }
    }

    /// Sends the local capabilities to the peer with a `Capabilities` message.
    fn send_capabilities(&self) {
        let Some(session) = self.session.upgrade() else {
            return;
        };
        let map = self.local_map().clone();
        if session.0.client.try_send_capabilities(map.clone()).is_err() {
            // The requests buffer is full: send asynchronously.
            let client = session.0.client.clone();
            tokio::task::spawn(async move {
                if let Err(error) = client.send_capabilities(map).await {
                    debug!(
                        error = &error as &dyn std::error::Error,
                        "failure to advertise the capabilities"
                    );
                }
            });
        }
    }

    /// Advertises the local capabilities to a legacy server, as `libqi` 2.3+ clients do.
    pub(super) async fn advertise_capabilities(&self, client: &messaging::Client) {
        if let Err(error) = client.send_capabilities(self.local_map().clone()).await {
            debug!(
                error = &error as &dyn std::error::Error,
                "failure to advertise the capabilities"
            );
        }
    }

    /// Authenticates to the remote peer, detecting a legacy server.
    pub(super) async fn authenticate_to_server(
        &self,
        client: &messaging::Client,
        parameters: KeyDynValueMap,
    ) -> Result<Capabilities, Error> {
        let has_credentials = !parameters.as_map().is_empty();
        let mut request = capabilities::local_map().clone();
        request.extend(parameters);
        let reply = client
            .call(
                AUTHENTICATE_ADDRESS,
                request.into_format_args()?,
                message::Flags::NONE,
            )
            .await;
        match reply {
            Ok(reply) => {
                let mut remote: KeyDynValueMap = reply.payload.to_reflect_args()?;
                auth::extract_state_result(&mut remote)
                    .map_err(AuthenticateToServerError::ResultState)?;
                self.remote_map.send_replace(Some(remote));
                self.protocol.send_replace(Some(Protocol::Standard));
                self.update_shared_capabilities();
                Ok(self.shared_capabilities())
            }
            Err(messaging::Error::CallError(error)) => {
                // A legacy server advertises its capabilities as soon as it accepts the
                // connection, so they have been received before its error reply. A standard
                // server that refuses the authentication sends nothing before its error.
                let advertised = self.remote_map.borrow().is_some();
                if !advertised {
                    return Err(AuthenticateToServerError::Refused(error).into());
                }
                if has_credentials {
                    return Err(AuthenticateToServerError::LegacyWithCredentials(error).into());
                }
                info!(
                    peer_error = %error,
                    "the peer speaks the legacy protocol (NAOqi 2.1): no authentication"
                );
                self.protocol.send_replace(Some(Protocol::Legacy));
                self.update_shared_capabilities();
                self.advertise_capabilities(client).await;
                Ok(self.shared_capabilities())
            }
            Err(error) => Err(error.into()),
        }
    }
}

impl std::fmt::Debug for Controller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Control")
            .field("server_protocol", &self.server_protocol)
            .field("capabilities", &self.capabilities)
            .field("protocol", &self.protocol)
            .field("remote_authorized", &self.remote_authorized)
            .finish()
    }
}

pub(super) struct Control<H> {
    pub(super) controller: Controller,
    pub(super) capabilities: watch::Receiver<Option<Capabilities>>,
    pub(super) protocol: watch::Receiver<Option<Protocol>>,
    pub(super) handler: ControlledHandler<H>,
}

/// Creates the control of a session over a handler.
///
/// The `authenticator` verifies the credentials of remote peers that authenticate to this side;
/// it is meaningless for the connecting side. `server_protocol` is the protocol this side speaks
/// when it accepts peers. `remote_authorized` tells whether the remote peer is authorized to send
/// requests right away: the connecting side authorizes the accepting side immediately, while the
/// accepting side authorizes the connecting side only once authenticated (or recognized as a
/// legacy peer). The session is used to send capabilities messages to the peer.
pub(super) fn create<Handler>(
    handler: Handler,
    authenticator: Option<Arc<dyn Authenticator + Send + Sync>>,
    server_protocol: Protocol,
    remote_authorized: bool,
    session: WeakSession,
) -> Control<Handler> {
    let (capabilities_sender, capabilities_receiver) = watch::channel(Default::default());
    let (protocol_sender, protocol_receiver) = watch::channel(Default::default());
    let (remote_map_sender, _remote_map_receiver) = watch::channel(Default::default());
    let (remote_authorized_sender, _remote_authorized_receiver) = watch::channel(remote_authorized);
    let controller = Controller {
        authenticator,
        server_protocol,
        remote_map: remote_map_sender,
        capabilities: capabilities_sender,
        protocol: protocol_sender,
        remote_authorized: remote_authorized_sender,
        session,
    };
    let controlled_handler = ControlledHandler {
        inner: handler,
        controller: controller.clone(),
    };
    Control {
        controller,
        capabilities: capabilities_receiver,
        protocol: protocol_receiver,
        handler: controlled_handler,
    }
}

/// A messaging handler that handles the control protocol and filters requests until the remote
/// is authorized.
pub(super) struct ControlledHandler<H> {
    inner: H,
    controller: Controller,
}

impl<H> ControlledHandler<H> {
    /// Whether the peer may send the request: it is authorized, or it is recognized as a legacy
    /// peer by this first request and authorized from now on.
    fn authorize_request(&self) -> bool {
        if *self.controller.remote_authorized.borrow() {
            return true;
        }
        if self.controller.authorizes_legacy_peer() {
            self.controller.accept_legacy_peer();
            return true;
        }
        false
    }
}

impl<Handler> messaging::CallHandler for ControlledHandler<Handler>
where
    Handler: messaging::CallHandler + Sync,
    Handler::Error: Into<HandlerError> + 'static,
{
    type Error = HandlerError;

    fn handle_call(
        &mut self,
        call: handler::Call,
        cancel: CancellationToken,
    ) -> impl Future<Output = Result<handler::Reply, Self::Error>> + Send + 'static {
        if is_control_address(call.address) {
            if call.address.action() != AUTHENTICATE_ACTION_ID {
                return err(HandlerError::non_fatal(NoHandlerError(
                    message::Type::Call,
                    call.address,
                )))
                .left_future();
            }
            if self.controller.server_protocol == Protocol::Legacy {
                // A legacy server has no server object: the call fails, the link stays open and
                // the client proceeds without authentication.
                return err(HandlerError::non_fatal(LEGACY_AUTHENTICATE_ERROR)).left_future();
            }
            let authenticate = || {
                self.controller
                    .authenticate(call.payload.to_reflect_args()?)
                    // All authentication errors are fatal
                    .map_err(HandlerError::fatal)?
                    .into_format_return_value()
                    .map(handler::Reply::new)
                    .map_err(Into::into)
            };
            ready(authenticate()).left_future()
        } else if self.authorize_request() {
            self.inner
                .handle_call(call, cancel)
                .map_err(Into::into)
                .right_future()
        } else {
            err(HandlerError::non_fatal(NoHandlerError(
                message::Type::Call,
                call.address,
            )))
            .left_future()
        }
    }
}

impl<Handler> messaging::EventHandler for ControlledHandler<Handler>
where
    Handler: messaging::EventHandler + Sync,
{
    fn handle_event(&mut self, event: handler::Event) {
        if !is_control_address(event.address) && self.authorize_request() {
            self.inner.handle_event(event)
        }
    }
}

impl<Handler> messaging::PostHandler for ControlledHandler<Handler>
where
    Handler: messaging::PostHandler + Sync,
{
    fn handle_post(&mut self, post: handler::Post) {
        if !is_control_address(post.address) && self.authorize_request() {
            self.inner.handle_post(post)
        }
    }
}

impl<Handler> messaging::CapabilitiesHandler for ControlledHandler<Handler> {
    fn handle_capabilities(&mut self, _address: message::Address, map: KeyDynValueMap) {
        self.controller.set_remote_capabilities(map);
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum AuthenticateClientError {
    #[error("failure to verify authentication request")]
    AuthenticationVerification(#[source] auth::Error),
}

impl From<AuthenticateClientError> for Error {
    fn from(err: AuthenticateClientError) -> Self {
        Error::Other(err.into())
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum AuthenticateToServerError {
    #[error("the authentication state sent back by the server is invalid")]
    ResultState(#[from] auth::StateError),

    #[error("the server refused the authentication: {0}")]
    Refused(String),

    #[error(
        "the server speaks the legacy protocol (NAOqi 2.1) and supports no authentication, \
         but credentials were given; its answer to the authentication was: {0}"
    )]
    LegacyWithCredentials(String),
}

impl From<AuthenticateToServerError> for Error {
    fn from(err: AuthenticateToServerError) -> Self {
        Error::Other(err.into())
    }
}
