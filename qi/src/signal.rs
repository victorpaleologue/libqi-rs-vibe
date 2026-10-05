//! Signals: typed asynchronous notifications.
//!
//! A [`Signal`] is a source of values that subscribers receive asynchronously as a stream. Signals
//! are the `qi` equivalent of events: objects expose signals in their meta object, and remote
//! peers may subscribe to them through the messaging layer.
//!
//! A signal is either *local*, created with [`Signal::new`] by the implementer of an object, or a
//! *proxy* to the signal of a remote object. Both are used through the same API, so that traits of
//! objects may expose signals as `&Signal<T>` members implemented either by objects or by their
//! clients.
//!
//! Signals are lossy by design, like in `libqi`: a subscriber that does not consume values fast
//! enough misses the oldest ones, and values that cannot be converted to the subscriber type are
//! dropped with a warning.

use crate::{
    object::{params, ActionNameOrId, AnyObject, Object},
    value::{FromValue, IntoValue, Reflect, Type, Value},
    Result,
};
use futures::{stream::BoxStream, Stream, StreamExt};
use std::{
    marker::PhantomData,
    pin::Pin,
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll},
};
use tokio::sync::broadcast;
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream};
use tracing::warn;

/// The identifier of a link between a signal and one of its subscribers.
///
/// In the messaging protocol, links are chosen by the subscribing peer.
#[derive(
    Default,
    Copy,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    qi_macros::Valuable,
    derive_more::Display,
)]
#[qi(value(crate = "crate::value", transparent))]
pub struct Link(pub u64);

impl Link {
    /// The invalid link, that is never connected.
    pub const INVALID: Self = Self(u64::MAX);

    /// Creates a new link identifier, unique in this process.
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    pub fn is_valid(&self) -> bool {
        *self != Self::INVALID
    }
}

/// The default capacity of the buffer of values of local signals.
pub const DEFAULT_CAPACITY: usize = 64;

/// A stream of values received from a signal.
///
/// Subscriptions end when the signal is dropped (for local signals) or when the connection to the
/// remote object is lost (for remote signals). Dropping a subscription disconnects it from the
/// signal.
#[must_use = "subscriptions are disconnected when dropped"]
pub struct Subscription<T> {
    inner: SubscriptionInner<T>,
}

enum SubscriptionInner<T> {
    Local(BroadcastStream<T>),
    /// A stream of parameters tuples, converted to `T` on the fly.
    Erased {
        stream: ValueStream,
        ty: Option<Type>,
        phantom: PhantomData<fn() -> T>,
    },
}

impl<T> Subscription<T> {
    /// Creates a subscription from a stream of parameters tuples, converting each tuple to a
    /// value of the given static type.
    pub(crate) fn from_params(stream: ValueStream, ty: Option<Type>) -> Self {
        Self {
            inner: SubscriptionInner::Erased {
                stream,
                ty,
                phantom: PhantomData,
            },
        }
    }

    fn local(receiver: broadcast::Receiver<T>) -> Self
    where
        T: Clone + Send + 'static,
    {
        Self {
            inner: SubscriptionInner::Local(BroadcastStream::new(receiver)),
        }
    }
}

impl<T> Stream for Subscription<T>
where
    T: Clone + Send + 'static + FromValue<'static>,
{
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match &mut self.inner {
                SubscriptionInner::Local(stream) => match stream.poll_next_unpin(cx) {
                    Poll::Ready(Some(Ok(value))) => return Poll::Ready(Some(value)),
                    Poll::Ready(Some(Err(BroadcastStreamRecvError::Lagged(count)))) => {
                        warn!(count, "signal subscription lagged, values were dropped");
                        continue;
                    }
                    Poll::Ready(None) => return Poll::Ready(None),
                    Poll::Pending => return Poll::Pending,
                },
                SubscriptionInner::Erased { stream, ty, .. } => match stream.poll_next_unpin(cx) {
                    Poll::Ready(Some(value)) => {
                        let converted = params::from_params_of(ty.as_ref(), value)
                            .convert_to(ty.as_ref())
                            .and_then(T::from_value);
                        match converted {
                            Ok(value) => return Poll::Ready(Some(value)),
                            Err(error) => {
                                warn!(
                                error = &error as &dyn std::error::Error,
                                "signal value dropped: conversion to the subscriber type failed"
                            );
                                continue;
                            }
                        }
                    }
                    Poll::Ready(None) => return Poll::Ready(None),
                    Poll::Pending => return Poll::Pending,
                },
            }
        }
    }
}

impl<T> std::fmt::Debug for Subscription<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.inner {
            SubscriptionInner::Local(_) => "local",
            SubscriptionInner::Erased { .. } => "remote",
        };
        f.debug_struct("Subscription").field("kind", &kind).finish()
    }
}

/// A stream of untyped signal parameters tuples, as handled by the messaging layer.
pub type ValueStream = BoxStream<'static, Value<'static>>;

/// A typed signal, either local or a proxy to the signal of a remote object.
///
/// # Local signals
///
/// ```
/// # tokio_test::block_on(async {
/// use futures::StreamExt;
/// let signal = qi::Signal::<i32>::new();
/// let mut subscription = signal.subscribe().await.unwrap();
/// signal.emit(42);
/// assert_eq!(subscription.next().await, Some(42));
/// # });
/// ```
pub struct Signal<T> {
    inner: SignalInner<T>,
}

enum SignalInner<T> {
    Local(broadcast::Sender<T>),
    /// A proxy to the signal of an object, identified by name or identifier.
    Remote {
        object: AnyObject,
        ident: ActionNameOrId,
        phantom: PhantomData<fn() -> T>,
    },
}

impl<T> Signal<T>
where
    T: Clone + Send + 'static,
{
    /// Creates a local signal with the default buffer capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// Creates a local signal with the given buffer capacity.
    ///
    /// The capacity is the number of values a slow subscriber may lag behind before it misses
    /// values.
    pub fn with_capacity(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            inner: SignalInner::Local(sender),
        }
    }

    /// Creates a proxy to a signal of an object.
    ///
    /// The object may be local or remote: emissions and subscriptions go through its
    /// [`Object`] implementation. The existence of the signal is checked when it is used.
    pub fn of_object(object: AnyObject, ident: ActionNameOrId) -> Self {
        Self {
            inner: SignalInner::Remote {
                object,
                ident,
                phantom: PhantomData,
            },
        }
    }

    /// Returns true if the signal is a proxy to the signal of an object.
    pub fn is_remote(&self) -> bool {
        matches!(self.inner, SignalInner::Remote { .. })
    }

    /// The number of local subscribers of a local signal. Always 0 for remote signals.
    pub fn subscriber_count(&self) -> usize {
        match &self.inner {
            SignalInner::Local(sender) => sender.receiver_count(),
            SignalInner::Remote { .. } => 0,
        }
    }
}

impl<T> Signal<T>
where
    T: Clone + Send + Sync + 'static + IntoValue<'static> + FromValue<'static> + Reflect,
{
    /// Emits a value to all subscribers.
    ///
    /// For local signals, the value is delivered immediately to the buffers of subscribers. For
    /// remote signals, the value is posted to the remote object, which triggers the signal on its
    /// side. In both cases, the emission is fire and forget.
    pub fn emit(&self, value: T) {
        match &self.inner {
            SignalInner::Local(sender) => {
                // An error means there is no subscriber, which is not an error for a signal.
                let _res = sender.send(value);
            }
            SignalInner::Remote { object, ident, .. } => {
                let params =
                    params::to_params_of(<T as Reflect>::ty().as_ref(), value.into_value());
                let result = match object.as_client() {
                    // Remote objects post the parameters synchronously.
                    Some(client) => client.emit_now(ident, params),
                    // Other objects are emitted to asynchronously, in a task.
                    None => match tokio::runtime::Handle::try_current() {
                        Ok(handle) => {
                            let (object, ident) = (object.clone(), ident.clone());
                            let params = params.into_owned();
                            handle.spawn(async move {
                                if let Err(error) = object.meta_emit(ident.clone(), params).await {
                                    warn!(
                                        signal = %ident,
                                        error = &error as &dyn std::error::Error,
                                        "signal emission failed"
                                    );
                                }
                            });
                            Ok(())
                        }
                        Err(_) => Err(crate::Error::Other(
                            "cannot emit a signal of an object outside of an asynchronous runtime"
                                .into(),
                        )),
                    },
                };
                if let Err(error) = result {
                    warn!(
                        signal = %ident,
                        error = &error as &dyn std::error::Error,
                        "signal emission failed"
                    );
                }
            }
        }
    }

    /// Subscribes to the signal.
    ///
    /// For remote signals, this registers the subscription to the remote object.
    pub async fn subscribe(&self) -> Result<Subscription<T>> {
        match &self.inner {
            SignalInner::Local(sender) => Ok(Subscription::local(sender.subscribe())),
            SignalInner::Remote { object, ident, .. } => Ok(Subscription::from_params(
                object.meta_subscribe(ident.clone()).await?,
                <T as Reflect>::ty(),
            )),
        }
    }

    /// Subscribes to the signal as a stream of untyped parameters tuples.
    #[doc(hidden)]
    pub async fn subscribe_erased(&self) -> Result<ValueStream> {
        match &self.inner {
            SignalInner::Local(sender) => Ok(erase_stream::<_, T>(Subscription::<T>::local(
                sender.subscribe(),
            ))),
            SignalInner::Remote { object, ident, .. } => object.meta_subscribe(ident.clone()).await,
        }
    }

    /// Emits an untyped parameters tuple, converting it to the signal type first.
    #[doc(hidden)]
    pub fn emit_erased(&self, params: Value<'_>) -> Result<()> {
        let ty = <T as Reflect>::ty();
        let value = params::from_params_of(ty.as_ref(), params)
            .convert_to(ty.as_ref())
            .and_then(|value| T::from_value(value.into_owned()))
            .map_err(crate::error::ValueConversionError::Arguments)?;
        self.emit(value);
        Ok(())
    }

    /// The type of the parameters tuple of the signal, as advertised in meta objects.
    pub fn params_type() -> Type {
        params::params_type::<T>()
    }
}

impl<T> Signal<T>
where
    T: Reflect,
{
    /// The type of the values of the signal, `None` if dynamic.
    pub fn value_type() -> Option<Type> {
        <T as Reflect>::ty()
    }
}

/// Erases a stream of typed values into a stream of parameters tuples.
pub(crate) fn erase_stream<S, T>(stream: S) -> ValueStream
where
    S: Stream<Item = T> + Send + 'static,
    T: IntoValue<'static> + Reflect,
{
    stream
        .map(|value| params::to_params::<T>(value.into_value()))
        .boxed()
}

impl<T> Default for Signal<T>
where
    T: Clone + Send + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        Self {
            inner: match &self.inner {
                SignalInner::Local(sender) => SignalInner::Local(sender.clone()),
                SignalInner::Remote { object, ident, .. } => SignalInner::Remote {
                    object: object.clone(),
                    ident: ident.clone(),
                    phantom: PhantomData,
                },
            },
        }
    }
}

impl<T> std::fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.inner {
            SignalInner::Local(sender) => f
                .debug_struct("Signal")
                .field("kind", &"local")
                .field("subscribers", &sender.receiver_count())
                .finish(),
            SignalInner::Remote { object, ident, .. } => f
                .debug_struct("Signal")
                .field("kind", &"remote")
                .field("object", object)
                .field("ident", ident)
                .finish(),
        }
    }
}
