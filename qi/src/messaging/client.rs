use crate::messaging::{
    handler::Reply,
    id::CreateId,
    message::{Address, Flags, Id, Response},
    Error, Message,
};
use bytes::Bytes;
use futures::{
    stream::{FusedStream, FuturesUnordered},
    Stream, StreamExt,
};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

/// A client of an endpoint, that sends requests to the peer of the endpoint.
///
/// Clients are cheap to clone. The endpoint messaging loop terminates when all clients associated
/// with it are dropped and all pending server calls are answered.
#[derive(Debug, Clone)]
pub struct Client {
    requests: mpsc::Sender<Request>,
}

impl Client {
    fn new(requests: mpsc::Sender<Request>) -> Self {
        Self { requests }
    }

    pub fn downgrade(&self) -> WeakClient {
        WeakClient {
            requests: self.requests.downgrade(),
        }
    }

    /// Sends a call request and waits for its reply.
    ///
    /// Dropping the returned future before its completion sends a cancellation request for the
    /// call to the peer.
    pub async fn call(
        &self,
        address: Address,
        payload: Bytes,
        flags: Flags,
    ) -> Result<Reply, Error> {
        let request_permit = self
            .requests
            .reserve()
            .await
            .map_err(client_dissociated_with_endpoint_error)?;
        let (response_sender, response_receiver) = oneshot::channel();
        let cancel_token = CancellationToken::new();
        let drop_guard = cancel_token.clone().drop_guard();
        request_permit.send(Request::Call {
            address,
            payload,
            flags,
            cancel_token,
            response_sender,
        });
        let response = response_receiver
            .await
            .map_err(client_dissociated_with_endpoint_error);
        drop_guard.disarm();
        response?
    }

    /// Sends an event notification.
    pub async fn send_event(
        &self,
        address: Address,
        payload: Bytes,
        flags: Flags,
    ) -> Result<(), Error> {
        self.requests
            .send(Request::Event {
                address,
                payload,
                flags,
            })
            .await
            .map_err(client_dissociated_with_endpoint_error)
    }

    /// Sends a post request.
    pub async fn post(&self, address: Address, payload: Bytes, flags: Flags) -> Result<(), Error> {
        self.requests
            .send(Request::Post {
                address,
                payload,
                flags,
            })
            .await
            .map_err(client_dissociated_with_endpoint_error)
    }

    /// Tries to send an event notification without waiting.
    ///
    /// Fails if the requests buffer is full or if the client is dissociated with its endpoint.
    pub fn try_send_event(
        &self,
        address: Address,
        payload: Bytes,
        flags: Flags,
    ) -> Result<(), Error> {
        self.requests
            .try_send(Request::Event {
                address,
                payload,
                flags,
            })
            .map_err(client_dissociated_with_endpoint_error)
    }

    /// Tries to send a post request without waiting.
    ///
    /// Fails if the requests buffer is full or if the client is dissociated with its endpoint.
    pub fn try_post(&self, address: Address, payload: Bytes, flags: Flags) -> Result<(), Error> {
        self.requests
            .try_send(Request::Post {
                address,
                payload,
                flags,
            })
            .map_err(client_dissociated_with_endpoint_error)
    }

    /// Returns true if the client is still associated with its endpoint.
    pub fn is_connected(&self) -> bool {
        !self.requests.is_closed()
    }
}

#[derive(Debug, Clone)]
pub struct WeakClient {
    requests: mpsc::WeakSender<Request>,
}

impl WeakClient {
    pub fn upgrade(&self) -> Option<Client> {
        self.requests.upgrade().map(Client::new)
    }
}

fn client_dissociated_with_endpoint_error<E>(_err: E) -> Error {
    Error::LinkLost("the client has been dissociated with the messaging loop".into())
}

#[derive(Debug)]
enum Request {
    Call {
        address: Address,
        payload: Bytes,
        flags: Flags,
        cancel_token: CancellationToken,
        response_sender: oneshot::Sender<Result<Reply, Error>>,
    },
    Post {
        address: Address,
        payload: Bytes,
        flags: Flags,
    },
    Event {
        address: Address,
        payload: Bytes,
        flags: Flags,
    },
}

/// Creates a client and a stream of its requests.
pub(crate) fn new_with_requests(requests_buffer_capacity: usize) -> (Client, Requests) {
    let (sender, receiver) = mpsc::channel(requests_buffer_capacity);
    (Client::new(sender), Requests::new(receiver))
}

pub(crate) struct Requests {
    id: CreateId,
    receiver: Option<mpsc::Receiver<Request>>,
    running_calls: HashMap<Id, CallState>,
    /// Completes with the identifier of a running call when its cancellation is requested.
    cancellations: FuturesUnordered<Cancellation>,
}

impl std::fmt::Debug for Requests {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Requests")
            .field("running_calls", &self.running_calls)
            .finish_non_exhaustive()
    }
}

pin_project_lite::pin_project! {
    struct Cancellation {
        id: Id,
        #[pin]
        cancelled: WaitForCancellationFutureOwned,
    }
}

impl Future for Cancellation {
    type Output = Id;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        ready!(this.cancelled.poll(cx));
        Poll::Ready(*this.id)
    }
}

impl Requests {
    fn new(receiver: mpsc::Receiver<Request>) -> Self {
        Self {
            id: CreateId::default(),
            receiver: Some(receiver),
            running_calls: HashMap::new(),
            cancellations: FuturesUnordered::new(),
        }
    }

    pub(super) fn dispatch_response(&mut self, id: Id, response: Response) {
        if let Some(CallState {
            response_sender, ..
        }) = self.running_calls.remove(&id)
        {
            let _res = response_sender.send(match response {
                Response::Reply(payload, flags) => Ok(Reply { payload, flags }),
                Response::Error(error) => Err(Error::CallError(error)),
                Response::Canceled => Err(Error::CallCanceled),
            });
        }
    }
}

impl Stream for Requests {
    type Item = Message;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Check if any running call has been canceled.
        while let Poll::Ready(Some(id)) = self.cancellations.poll_next_unpin(cx) {
            // The call may have been answered since.
            if let Some(CallState { address, .. }) = self.running_calls.remove(&id) {
                // Cancel messages are addressed to the object, without action.
                return Poll::Ready(Some(Message::Cancel {
                    id: self.id.create(),
                    address: address.with_action(Default::default()),
                    call_id: id,
                }));
            }
        }

        match self.receiver {
            Some(ref mut receiver) => match ready!(receiver.poll_recv(cx)) {
                Some(request) => {
                    let id = self.id.create();
                    let message = match request {
                        Request::Call {
                            address,
                            payload,
                            flags,
                            cancel_token,
                            response_sender,
                        } => {
                            self.cancellations.push(Cancellation {
                                id,
                                cancelled: cancel_token.clone().cancelled_owned(),
                            });
                            self.running_calls.insert(
                                id,
                                CallState {
                                    address,
                                    response_sender,
                                },
                            );
                            Message::Call {
                                id,
                                address,
                                payload,
                                flags,
                            }
                        }
                        Request::Post {
                            address,
                            payload,
                            flags,
                        } => Message::Post {
                            id,
                            address,
                            payload,
                            flags,
                        },
                        Request::Event {
                            address,
                            payload,
                            flags,
                        } => Message::Event {
                            id,
                            address,
                            payload,
                            flags,
                        },
                    };
                    Poll::Ready(Some(message))
                }
                None => {
                    self.receiver = None;
                    self.running_calls.clear();
                    Poll::Ready(None)
                }
            },
            None => Poll::Ready(None),
        }
    }
}

impl FusedStream for Requests {
    fn is_terminated(&self) -> bool {
        self.receiver.is_none()
    }
}

#[derive(Debug)]
struct CallState {
    address: Address,
    response_sender: oneshot::Sender<Result<Reply, Error>>,
}
