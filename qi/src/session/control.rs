//! The control protocol of sessions: authentication and capabilities negotiation.
//!
//! Control messages are addressed to the server object (service 0, object 0). The only control
//! call is the authentication call (action 8), which a connecting peer must send first. Its
//! arguments are a map of the capabilities of the peer and of its credentials; the reply is a map
//! of the capabilities of the accepting peer and of the authentication state.

use super::capabilities::{self, Capabilities};
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

const SERVICE_ID: service::Id = service::Id(0);
const OBJECT_ID: object::Id = object::Id(0);
const AUTHENTICATE_ACTION_ID: object::ActionId = object::ActionId(8);

fn is_control_address(address: message::Address) -> bool {
    address.service() == SERVICE_ID && address.object() == OBJECT_ID
}

pub(crate) const AUTHENTICATE_ADDRESS: message::Address =
    message::Address(SERVICE_ID, OBJECT_ID, AUTHENTICATE_ACTION_ID);

#[derive(Clone)]
pub(super) struct Controller {
    authenticator: Option<Arc<dyn Authenticator + Send + Sync>>,
    capabilities: watch::Sender<Option<Capabilities>>,
    remote_authorized: watch::Sender<bool>,
}

impl Controller {
    /// Handles an authentication request from the remote peer.
    fn authenticate(
        &self,
        request: KeyDynValueMap,
    ) -> Result<KeyDynValueMap, AuthenticateClientError> {
        let shared = Capabilities::shared_with_remote_map(&request);
        if let Some(authenticator) = &self.authenticator {
            authenticator
                .authenticate(request)
                .map_err(AuthenticateClientError::AuthenticationVerification)?;
        }
        self.capabilities.send_replace(Some(shared));
        self.remote_authorized.send_replace(true);
        // Like the reference implementation, the reply advertises the local capabilities, and the
        // remote computes the shared ones.
        Ok(auth::state_done_map(capabilities::local_map().clone()))
    }

    /// Records the capabilities of a remote peer that advertised them without authenticating
    /// (compatibility path).
    fn set_remote_capabilities(&self, map: &KeyDynValueMap) {
        if self.capabilities.borrow().is_none() {
            self.capabilities
                .send_replace(Some(Capabilities::shared_with_remote_map(map)));
        }
    }

    /// Authenticates to the remote peer.
    pub(super) async fn authenticate_to_server(
        &self,
        client: &messaging::Client,
        parameters: KeyDynValueMap,
    ) -> Result<Capabilities, Error> {
        // Reset the current capabilities
        self.capabilities.send_replace(None);
        let mut request = capabilities::local_map().clone();
        request.extend(parameters);
        let reply = client
            .call(
                AUTHENTICATE_ADDRESS,
                request.into_format_args()?,
                message::Flags::NONE,
            )
            .await?;
        let mut remote: KeyDynValueMap = reply.payload.to_reflect_args()?;
        auth::extract_state_result(&mut remote).map_err(AuthenticateToServerError::ResultState)?;
        let shared = Capabilities::shared_with_remote_map(&remote);
        self.capabilities.send_replace(Some(shared));
        Ok(shared)
    }
}

impl std::fmt::Debug for Controller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Control")
            .field("capabilities", &self.capabilities)
            .field("remote_authorized", &self.remote_authorized)
            .finish()
    }
}

pub(super) struct Control<H> {
    pub(super) controller: Controller,
    pub(super) capabilities: watch::Receiver<Option<Capabilities>>,
    pub(super) handler: ControlledHandler<H>,
}

/// Creates the control of a session over a handler.
///
/// The `authenticator` verifies the credentials of remote peers that authenticate to this side;
/// it is meaningless for the connecting side. `remote_authorized` tells whether the remote peer is
/// authorized to send requests right away: the connecting side authorizes the accepting side
/// immediately, while the accepting side authorizes the connecting side only once authenticated.
pub(super) fn create<Handler>(
    handler: Handler,
    authenticator: Option<Arc<dyn Authenticator + Send + Sync>>,
    remote_authorized: bool,
) -> Control<Handler> {
    let (capabilities_sender, capabilities_receiver) = watch::channel(Default::default());
    let (remote_authorized_sender, _remote_authorized_receiver) = watch::channel(remote_authorized);
    let controller = Controller {
        authenticator,
        capabilities: capabilities_sender,
        remote_authorized: remote_authorized_sender,
    };
    let controlled_handler = ControlledHandler {
        inner: handler,
        controller: controller.clone(),
    };
    Control {
        controller,
        capabilities: capabilities_receiver,
        handler: controlled_handler,
    }
}

/// A messaging handler that handles the control protocol and filters requests until the remote
/// is authorized.
pub(super) struct ControlledHandler<H> {
    inner: H,
    controller: Controller,
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
        } else if *self.controller.remote_authorized.borrow() {
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
        if !is_control_address(event.address) && *self.controller.remote_authorized.borrow() {
            self.inner.handle_event(event)
        }
    }
}

impl<Handler> messaging::PostHandler for ControlledHandler<Handler>
where
    Handler: messaging::PostHandler + Sync,
{
    fn handle_post(&mut self, post: handler::Post) {
        if !is_control_address(post.address) && *self.controller.remote_authorized.borrow() {
            self.inner.handle_post(post)
        }
    }
}

impl<Handler> messaging::CapabilitiesHandler for ControlledHandler<Handler> {
    fn handle_capabilities(&mut self, _address: message::Address, map: KeyDynValueMap) {
        self.controller.set_remote_capabilities(&map);
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
}

impl From<AuthenticateToServerError> for Error {
    fn from(err: AuthenticateToServerError) -> Self {
        Error::Other(err.into())
    }
}
