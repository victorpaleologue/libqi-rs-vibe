use crate::messaging::{
    handler::{self, CallError},
    message::{Address, Id},
    Message,
};
use futures::{
    stream::{FusedStream, FuturesUnordered},
    Stream, StreamExt, TryFuture,
};
use pin_project_lite::pin_project;
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};
use tokio_util::sync::CancellationToken;

pub(super) struct CallFutures<F> {
    call_futures: FuturesUnordered<CallFuture<F>>,
    cancel_tokens: HashMap<Id, CancellationToken>,
}

impl<F> Default for CallFutures<F> {
    fn default() -> Self {
        Self {
            call_futures: Default::default(),
            cancel_tokens: Default::default(),
        }
    }
}

impl<F> std::fmt::Debug for CallFutures<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallFutures")
            .field("call_futures", &self.call_futures)
            .finish()
    }
}

impl<F> CallFutures<F> {
    pub(super) fn push(&mut self, id: Id, address: Address, cancel: CancellationToken, future: F) {
        self.cancel_tokens.insert(id, cancel.clone());
        self.call_futures
            .push(CallFuture::new(id, address, cancel, future));
    }

    /// Requests the cancellation of the call with the given identifier.
    ///
    /// Cancellation is cooperative: the future of the call is notified through its cancellation
    /// token and keeps running until it terminates.
    pub(super) fn cancel(&mut self, id: &Id) {
        if let Some(token) = self.cancel_tokens.get(id) {
            token.cancel();
        }
    }
}

impl<F> Stream for CallFutures<F>
where
    CallFuture<F>: Future<Output = (Message, DispatchFlow)>,
{
    type Item = (Message, DispatchFlow);

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let item = ready!(self.call_futures.poll_next_unpin(cx));
        if let Some((message, _)) = &item {
            self.cancel_tokens.remove(&message.id());
        }
        Poll::Ready(item)
    }
}

impl<F> FusedStream for CallFutures<F>
where
    CallFuture<F>: Future<Output = (Message, DispatchFlow)>,
{
    fn is_terminated(&self) -> bool {
        self.call_futures.is_terminated()
    }
}

pin_project! {
    struct CallFuture<F> {
        id: Id,
        address: Address,
        cancel: CancellationToken,
        #[pin]
        inner: F,
    }

    impl<F> PinnedDrop for CallFuture<F> {
        fn drop(this: Pin<&mut Self>) {
            // A call future dropped before completion (endpoint termination) is a canceled call.
            this.cancel.cancel();
        }
    }
}

impl<F> std::fmt::Debug for CallFuture<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallFuture")
            .field("id", &self.id)
            .field("address", &self.address)
            .field("canceled", &self.cancel.is_cancelled())
            .finish()
    }
}

impl<F> CallFuture<F> {
    fn new(id: Id, address: Address, cancel: CancellationToken, inner: F) -> Self {
        Self {
            id,
            address,
            cancel,
            inner,
        }
    }
}

impl<F> Future for CallFuture<F>
where
    F: TryFuture<Ok = handler::Reply>,
    F::Error: handler::CallError,
{
    type Output = (Message, DispatchFlow);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let call_result = ready!(this.inner.try_poll(cx));
        let (id, address) = (*this.id, *this.address);
        Poll::Ready(match call_result {
            Ok(handler::Reply { payload, flags }) => (
                Message::Reply {
                    id,
                    address,
                    payload,
                    flags,
                },
                DispatchFlow::Continue,
            ),
            Err(error) if error.is_canceled() => {
                (Message::Canceled { id, address }, DispatchFlow::Continue)
            }
            Err(error) => (
                Message::Error {
                    id,
                    address,
                    error: error.to_string(),
                },
                if error.is_fatal() {
                    DispatchFlow::Stop
                } else {
                    DispatchFlow::Continue
                },
            ),
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum DispatchFlow {
    Continue,
    Stop,
}
