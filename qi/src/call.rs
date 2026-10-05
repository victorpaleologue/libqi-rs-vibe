//! Context of the execution of object method calls.
//!
//! When an object method is called through the messaging layer, its execution is scoped in a
//! [`Context`] that is accessible from anywhere in the asynchronous call stack of the method
//! through [`context`]. The context carries the cancellation token of the call, that is triggered
//! when the caller requests the cancellation of the call, and the token of the messaging link of
//! the caller, that is triggered when the link is closed.
//!
//! Cancellation is cooperative: a method that wants to be cancelable checks the token, for
//! instance by selecting on [`cancelled`] while awaiting a long operation, and returns
//! [`Error::CallCanceled`](crate::Error::CallCanceled). Methods that ignore the token run to
//! completion and their result is discarded by the caller.
//!
//! ```
//! use qi::call;
//!
//! async fn long_operation() -> qi::Result<()> {
//!     tokio::select! {
//!         () = tokio::time::sleep(std::time::Duration::from_secs(10)) => Ok(()),
//!         () = call::cancelled() => Err(qi::Error::CallCanceled),
//!     }
//! }
//! # let _ = long_operation;
//! ```

use crate::session::WeakSession;
use std::future::Future;
use tokio_util::sync::CancellationToken;

tokio::task_local! {
    static CONTEXT: Context;
}

/// The context of the execution of an object method call.
#[derive(Debug, Clone, Default)]
pub struct Context {
    cancel: CancellationToken,
    link_closed: Option<CancellationToken>,
    session: Option<WeakSession>,
}

impl Context {
    /// Creates a new context with the given cancellation token and, optionally, the token of the
    /// messaging link of the caller.
    pub fn new(cancel: CancellationToken, link_closed: Option<CancellationToken>) -> Self {
        Self {
            cancel,
            link_closed,
            session: None,
        }
    }

    /// Attaches the session of the caller to the context.
    pub(crate) fn with_session(mut self, session: WeakSession) -> Self {
        self.session = Some(session);
        self
    }

    /// The session of the caller, if the call comes from a remote peer.
    pub(crate) fn session(&self) -> Option<&WeakSession> {
        self.session.as_ref()
    }

    /// The cancellation token of the call.
    pub fn cancellation_token(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Returns true if the cancellation of the call has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// A token that is cancelled when the messaging link to the caller is closed, if the call
    /// comes from a remote peer.
    ///
    /// Services may use it to release what a peer registered when it disconnects.
    pub fn link_closed(&self) -> Option<&CancellationToken> {
        self.link_closed.as_ref()
    }

    /// Runs a future within this context.
    pub async fn scope<F>(self, future: F) -> F::Output
    where
        F: Future,
    {
        CONTEXT.scope(self, future).await
    }
}

/// The context of the current call, if the current task runs within one.
pub fn context() -> Option<Context> {
    CONTEXT.try_with(Clone::clone).ok()
}

/// Returns true if the current task runs within a call whose cancellation has been requested.
///
/// Returns false when not running within a call.
pub fn is_cancelled() -> bool {
    context().is_some_and(|ctx| ctx.is_cancelled())
}

/// Waits until the cancellation of the current call is requested.
///
/// When not running within a call, the future never completes.
pub async fn cancelled() {
    match context() {
        Some(ctx) => ctx.cancel.cancelled_owned().await,
        None => std::future::pending().await,
    }
}

/// Returns the token of the messaging link of the caller of the current call, if any.
pub fn link_closed() -> Option<CancellationToken> {
    context().and_then(|ctx| ctx.link_closed)
}
