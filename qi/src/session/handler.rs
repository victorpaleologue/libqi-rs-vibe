//! The messaging handler of sessions.
//!
//! The handler routes incoming requests to objects: the main objects of the services registered
//! on the node, and the objects hosted by the session for its peer. It implements the special
//! members of bound objects (events registration, properties access, meta object retrieval and
//! termination), and dispatches incoming events to the local subscriptions of remote signals.

use super::{SessionState, WeakSession};
use crate::{
    call,
    error::{HandlerError, NoHandlerError},
    messaging::{self, handler, message, CancellationToken},
    object::{
        self,
        generic::{self, is_special},
        ActionId, ActionNameOrId, AnyObject, Id,
    },
    service::{self, SharedServices},
    signal::Link,
    value::{self, Dynamic, FromValue, IntoValue, Reflect, Type, Value},
    Error, Result,
};
use futures::{future::ready, FutureExt, StreamExt};
use std::future::Future;
use tokio::task;
use tokio_util::task::AbortOnDropHandle;
use tracing::{debug, trace};

/// The error message of calls to addresses that no object handles, as in the reference
/// implementation.
const UNHANDLED_CALL_ERROR: &str = "The call request could not be handled.";

#[derive(Clone)]
pub(super) struct SessionHandler {
    session: WeakSession,
    services: SharedServices,
}

impl SessionHandler {
    pub(super) fn new(session: WeakSession, services: SharedServices) -> Self {
        Self { session, services }
    }

    /// Finds the object targeted by an address: the main object of a registered service, or an
    /// object hosted by the session.
    fn object(&self, state: &SessionState, address: message::Address) -> Result<AnyObject> {
        let object = if address.object() == object::MAIN_OBJECT_ID {
            self.services.object(address.service())
        } else {
            state
                .host
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .get(address.object())
                .cloned()
        };
        object.ok_or_else(|| Error::Other(UNHANDLED_CALL_ERROR.into()))
    }
}

impl messaging::CallHandler for SessionHandler {
    type Error = HandlerError;

    fn handle_call(
        &mut self,
        call: handler::Call,
        cancel: CancellationToken,
    ) -> impl Future<Output = std::result::Result<handler::Reply, Self::Error>> + Send + 'static
    {
        let Some(session) = self.session.upgrade() else {
            return ready(Err(HandlerError::fatal("the session is closed"))).left_future();
        };
        let object = match self.object(&session.0, call.address) {
            Ok(object) => object,
            Err(err) => return ready(Err(HandlerError::non_fatal(err))).left_future(),
        };
        let link_closed = session.0.closed.clone();
        let context =
            call::Context::new(cancel, Some(link_closed)).with_session(session.downgrade());
        let task =
            task::spawn(context.scope(async move { handle_call(&session.0, object, call).await }));
        AbortOnDropHandle::new(task)
            .map(|res| match res {
                Ok(res) => res.map_err(HandlerError::from),
                Err(err) => Err(HandlerError::from(err)),
            })
            .right_future()
    }
}

impl messaging::PostHandler for SessionHandler {
    fn handle_post(&mut self, post: handler::Post) {
        let Some(session) = self.session.upgrade() else {
            return;
        };
        let object = match self.object(&session.0, post.address) {
            Ok(object) => object,
            Err(_) => {
                debug!(address = %post.address, "post request discarded: no handler");
                return;
            }
        };
        task::spawn(async move {
            if let Err(error) = handle_post(&session.0, object, post).await {
                debug!(
                    error = &error as &dyn std::error::Error,
                    "post request discarded"
                );
            }
        });
    }
}

impl messaging::EventHandler for SessionHandler {
    fn handle_event(&mut self, event: handler::Event) {
        if let Some(session) = self.session.upgrade() {
            session
                .0
                .dispatch_event(event.address, event.payload, event.flags);
        }
    }
}

impl messaging::CapabilitiesHandler for SessionHandler {
    fn handle_capabilities(&mut self, _address: message::Address, _map: value::KeyDynValueMap) {
        // Handled by the control layer.
    }
}

/// Handles a call to an object.
async fn handle_call(
    state: &SessionState,
    object: AnyObject,
    call: handler::Call,
) -> Result<handler::Reply> {
    let message::Address(service_id, object_id, action) = call.address;
    if is_special(action) {
        return handle_special_call(state, service_id, object_id, &object, action, call).await;
    }

    let ident = ActionNameOrId::Id(action);
    let params_type = object
        .meta()
        .method(&ident)
        .ok_or_else(|| Error::MethodNotFound(ident.clone()))?
        .parameters_signature
        .clone()
        .into_type();
    let Arguments { args, return_type } = decode_arguments(state, &call, params_type.as_ref())?;
    trace!(%ident, ?args, "calling object method");
    let result = object.meta_call(ident, args).await?;
    let result = state.bind_outgoing(result, service_id)?;
    encode_reply(state, result, return_type)
}

/// The decoded arguments of a call, and the type the reply is expected with.
struct Arguments {
    args: Value<'static>,
    return_type: ReturnType,
}

enum ReturnType {
    /// The reply is encoded with the declared return type of the method.
    Declared,
    /// The caller requested the reply as a dynamic value converted to the given type.
    Forced(value::Signature),
}

fn decode_arguments(
    state: &SessionState,
    call: &handler::Call,
    params_type: Option<&Type>,
) -> Result<Arguments> {
    let flags = call.flags;
    if flags.is_dynamic_payload() {
        // The payload is a dynamic value holding the tuple of arguments.
        let mut value = state.decode(&call.payload, None)?;
        if let Value::Dynamic(inner) = value {
            value = *inner;
        }
        let (args, return_type) = if flags.has_return_type() {
            split_forced_return_type(value)?
        } else {
            (value, None)
        };
        return Ok(Arguments {
            args: state.bind_incoming(args),
            return_type: return_type
                .map(ReturnType::Forced)
                .unwrap_or(ReturnType::Declared),
        });
    }
    if flags.has_return_type() {
        // The payload is a tuple of the parameters tuple and of a signature string.
        let ty = Type::tuple_of([params_type.cloned(), Some(Type::String)]);
        let value = state.decode(&call.payload, Some(&ty))?;
        let (args, return_type) = split_forced_return_type(value)?;
        return Ok(Arguments {
            args: state.bind_incoming(args),
            return_type: return_type
                .map(ReturnType::Forced)
                .unwrap_or(ReturnType::Declared),
        });
    }
    let args = state.decode(&call.payload, params_type)?;
    Ok(Arguments {
        args: state.bind_incoming(args),
        return_type: ReturnType::Declared,
    })
}

fn split_forced_return_type(
    value: Value<'static>,
) -> Result<(Value<'static>, Option<value::Signature>)> {
    match value {
        Value::Tuple(mut elements) if elements.len() == 2 => {
            let signature = elements.pop().map(value::Signature::from_value).transpose();
            let signature = signature.map_err(crate::error::ValueConversionError::Arguments)?;
            let args = elements.pop().unwrap_or_default();
            Ok((args, signature))
        }
        value => Err(Error::Other(
            format!("expected a tuple of arguments and return signature, got {value}").into(),
        )),
    }
}

fn encode_reply(
    state: &SessionState,
    result: Value<'static>,
    return_type: ReturnType,
) -> Result<handler::Reply> {
    match return_type {
        ReturnType::Declared => Ok(handler::Reply::new(state.encode(&result)?)),
        ReturnType::Forced(_signature) => {
            // Conversion to the requested signature is not attempted: the value is sent as a
            // dynamic and the caller converts it.
            let payload = state.encode(&Value::Dynamic(Box::new(result)))?;
            Ok(handler::Reply::dynamic(payload))
        }
    }
}

/// Handles a post to an object: a call of a method without reply, or the emission of a signal.
async fn handle_post(state: &SessionState, object: AnyObject, post: handler::Post) -> Result<()> {
    let message::Address(_service_id, object_id, action) = post.address;
    if is_special(action) {
        // The only special member that may be posted is the termination of a hosted object.
        if action == generic::TERMINATE {
            state.terminate_hosted(object_id);
        }
        return Ok(());
    }
    let ident = ActionNameOrId::Id(action);
    let meta = object.meta();
    if let Some(method) = meta.method(&ident) {
        let params_type = method.parameters_signature.clone().into_type();
        let args = decode_post_arguments(state, &post, params_type.as_ref())?;
        object.meta_post(ident, args).await;
        Ok(())
    } else if let Some(signal) = meta.signal(&ident) {
        let params_type = signal.signature.clone().into_type();
        let params = decode_post_arguments(state, &post, params_type.as_ref())?;
        object.meta_emit(ident, params).await
    } else {
        Err(Error::MethodNotFound(ident))
    }
}

fn decode_post_arguments(
    state: &SessionState,
    post: &handler::Post,
    params_type: Option<&Type>,
) -> Result<Value<'static>> {
    let args = if post.flags.is_dynamic_payload() {
        match state.decode(&post.payload, None)? {
            Value::Dynamic(inner) => *inner,
            value => value,
        }
    } else {
        state.decode(&post.payload, params_type)?
    };
    Ok(state.bind_incoming(args))
}

/// Handles a call to a special member of a bound object.
async fn handle_special_call(
    state: &SessionState,
    service_id: service::Id,
    object_id: Id,
    object: &AnyObject,
    action: ActionId,
    call: handler::Call,
) -> Result<handler::Reply> {
    let params_type = generic::meta_object()
        .method(&ActionNameOrId::Id(action))
        .map(|method| method.parameters_signature.clone().into_type())
        .ok_or_else(|| Error::MethodNotFound(ActionNameOrId::Id(action)))?;
    let args = if call.flags.is_dynamic_payload() {
        match state.decode(&call.payload, None)? {
            Value::Dynamic(inner) => *inner,
            value => value,
        }
    } else {
        state.decode(&call.payload, params_type.as_ref())?
    };
    let result = match action {
        generic::REGISTER_EVENT => {
            let (_service, event, link): (u32, u32, u64) = cast_args(args)?;
            register_event(
                state,
                service_id,
                object_id,
                object,
                ActionId(event),
                Link(link),
            )
            .await?
            .into_value()
        }
        generic::REGISTER_EVENT_WITH_SIGNATURE => {
            let (_service, event, link, _signature): (u32, u32, u64, String) = cast_args(args)?;
            register_event(
                state,
                service_id,
                object_id,
                object,
                ActionId(event),
                Link(link),
            )
            .await?
            .into_value()
        }
        generic::UNREGISTER_EVENT => {
            let (_service, _event, link): (u32, u32, u64) = cast_args(args)?;
            state.unregister_event(Link(link))?;
            Value::Unit
        }
        generic::META_OBJECT => {
            let (_object,): (u32,) = cast_args(args)?;
            generic::merge(object.meta()).into_value()
        }
        generic::TERMINATE => {
            let (_object,): (u32,) = cast_args(args)?;
            state.terminate_hosted(object_id);
            Value::Unit
        }
        generic::PROPERTY => {
            let (ident,): (Dynamic<Value<'static>>,) = cast_args(args)?;
            let ident = property_ident(ident.0)?;
            let value = object.meta_property(ident).await?;
            let value = state.bind_outgoing(value, service_id)?;
            Value::Dynamic(Box::new(value))
        }
        generic::SET_PROPERTY => {
            let (ident, value): (Dynamic<Value<'static>>, Dynamic<Value<'static>>) =
                cast_args(args)?;
            let ident = property_ident(ident.0)?;
            let value = state.bind_incoming(value.0);
            object.meta_set_property(ident, value).await?;
            Value::Unit
        }
        generic::PROPERTIES => {
            let (): () = cast_args(args)?;
            object
                .meta()
                .properties
                .values()
                .map(|prop| prop.name.clone())
                .collect::<Vec<_>>()
                .into_value()
        }
        action => return Err(Error::MethodNotFound(ActionNameOrId::Id(action))),
    };
    Ok(handler::Reply::new(state.encode(&result)?))
}

fn cast_args<T>(args: Value<'static>) -> Result<T>
where
    T: FromValue<'static> + Reflect,
{
    let args = object::params::from_params::<T>(args);
    T::from_value(args).map_err(|err| crate::error::ValueConversionError::Arguments(err).into())
}

/// Converts the dynamic identifier of a property, either its name or its identifier.
fn property_ident(value: Value<'_>) -> Result<ActionNameOrId> {
    match value {
        Value::UInt32(id) => Ok(ActionNameOrId::Id(ActionId(id))),
        Value::Int32(id) => Ok(ActionNameOrId::Id(ActionId(id as u32))),
        Value::String(name) => Ok(ActionNameOrId::Name(name.to_string())),
        value => Err(Error::Other(
            format!("expected int or string for property index, got {value}").into(),
        )),
    }
}

/// Registers a subscription of the remote peer to a signal of an object: events of the signal
/// are forwarded to the peer until the link is unregistered or the session ends.
async fn register_event(
    state: &SessionState,
    service_id: service::Id,
    object_id: Id,
    object: &AnyObject,
    event: ActionId,
    link: Link,
) -> Result<Link> {
    let ident = ActionNameOrId::Id(event);
    let signature = object
        .meta()
        .signal(&ident)
        .map(|signal| signal.signature.clone())
        .or_else(|| {
            object.meta().property(&ident).map(|prop| {
                object::params::params_type_of(prop.signature.clone().into_type()).into()
            })
        })
        .ok_or_else(|| Error::SignalNotFound(ident.clone()))?;
    let mut stream = object.meta_subscribe(ident).await?;
    let session = state.weak_session();
    let address = message::Address(service_id, object_id, event);
    let forward = task::spawn(async move {
        while let Some(params) = stream.next().await {
            let Some(session) = session.upgrade() else {
                break;
            };
            let state = &session.0;
            // Parameters are sent with the declared signature of the signal, whatever the
            // types of the values emitted locally.
            let result = params
                .convert_to(signature.as_type())
                .map_err(|err| Error::from(crate::error::ValueConversionError::Arguments(err)))
                .and_then(|params| state.bind_outgoing(params, service_id))
                .and_then(|params| state.encode(&params));
            let payload = match result {
                Ok(payload) => payload,
                Err(error) => {
                    debug!(
                        error = &error as &dyn std::error::Error,
                        %address,
                        "signal event dropped: failure to encode parameters"
                    );
                    continue;
                }
            };
            if state
                .client
                .send_event(address, payload, message::Flags::NONE)
                .await
                .is_err()
            {
                break;
            }
        }
    });
    state.register_event_link(link, AbortOnDropHandle::new(forward));
    Ok(link)
}

impl From<NoHandlerError> for HandlerError {
    fn from(err: NoHandlerError) -> Self {
        HandlerError::non_fatal(err)
    }
}
