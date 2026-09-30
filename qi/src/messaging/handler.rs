//! Handlers of incoming requests.
//!
//! An endpoint dispatches each incoming request message to a handler. Calls are handled
//! asynchronously: the handler returns a future of the reply, which the endpoint drives and
//! answers to the peer. Posts, events and capabilities are notifications that are handled
//! synchronously and never answered.

use crate::messaging::{
    message::{Address, Flags},
    value::KeyDynValueMap,
};
use bytes::Bytes;
use std::{convert::Infallible, future::Future};
pub use tokio_util::sync::CancellationToken;

/// An incoming call request.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Call {
    pub address: Address,
    pub payload: Bytes,
    pub flags: Flags,
}

/// The reply to a call request.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Reply {
    pub payload: Bytes,
    pub flags: Flags,
}

impl Reply {
    /// Constructs a reply from a payload of the expected type, with no flags.
    pub fn new(payload: Bytes) -> Self {
        Self {
            payload,
            flags: Flags::NONE,
        }
    }

    /// Constructs a reply from a payload that is a dynamic value.
    pub fn dynamic(payload: Bytes) -> Self {
        Self {
            payload,
            flags: Flags::DYNAMIC_PAYLOAD,
        }
    }
}

impl From<Bytes> for Reply {
    fn from(payload: Bytes) -> Self {
        Self::new(payload)
    }
}

/// An incoming post request.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Post {
    pub address: Address,
    pub payload: Bytes,
    pub flags: Flags,
}

/// An incoming event notification.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Event {
    pub address: Address,
    pub payload: Bytes,
    pub flags: Flags,
}

/// A handler of call requests.
pub trait CallHandler {
    type Error: CallError;

    /// Handles a call request.
    ///
    /// The returned future is driven by the endpoint. The cancellation token is triggered when the
    /// peer requests the cancellation of the call. Cancellation is cooperative: the future keeps
    /// being polled after the token is triggered, and its result is sent back to the peer. If the
    /// result is an error for which [`CallError::is_canceled`] is true, the peer is notified
    /// that the call was canceled instead.
    fn handle_call(
        &mut self,
        call: Call,
        cancel: CancellationToken,
    ) -> impl Future<Output = Result<Reply, Self::Error>> + Send + 'static;
}

/// A handler of event notifications.
pub trait EventHandler {
    fn handle_event(&mut self, event: Event);
}

/// A handler of post requests.
pub trait PostHandler {
    fn handle_post(&mut self, post: Post);
}

/// A handler of capabilities notifications.
pub trait CapabilitiesHandler {
    fn handle_capabilities(&mut self, address: Address, map: KeyDynValueMap);
}

pub trait Handler: CallHandler + EventHandler + PostHandler + CapabilitiesHandler {}

impl<T> Handler for T where T: CallHandler + EventHandler + PostHandler + CapabilitiesHandler {}

/// An call handler error that is able to signify handling conditions to the messaging loop.
pub trait CallError: std::fmt::Display {
    /// The error is a consequence of a request cancellation. The messaging loop must notify the
    /// client that the request has been canceled.
    fn is_canceled(&self) -> bool;

    /// The error is fatal to the messaging loop. The loop must send the error back to the client
    /// and then terminate.
    fn is_fatal(&self) -> bool;
}

impl CallError for Infallible {
    fn is_canceled(&self) -> bool {
        false
    }

    fn is_fatal(&self) -> bool {
        false
    }
}
