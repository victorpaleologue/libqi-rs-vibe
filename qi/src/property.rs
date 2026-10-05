//! Properties: typed values whose changes are notified.
//!
//! A [`Property`] holds a value that may be read and written, and notifies its changes to
//! subscribers like a [`Signal`]. In the `qi` type system, a property is also a
//! signal of the same identifier and name.
//!
//! Like signals, properties are either *local* or *proxies* to the property of a remote object,
//! and share the same API.

use crate::{
    error::ValueConversionError,
    object::{ActionNameOrId, AnyObject, Object},
    signal::{Signal, Subscription, ValueStream},
    value::{self, FromValue, IntoValue, Reflect, Value},
    Result,
};
use std::sync::{Arc, RwLock};

/// A typed property, either local or a proxy to the property of a remote object.
///
/// Properties are cheap to clone: clones of a local property share the same value and signal.
///
/// # Local properties
///
/// ```
/// # tokio_test::block_on(async {
/// use futures::StreamExt;
/// let property = qi::Property::new(1);
/// let mut changes = property.subscribe().await.unwrap();
/// property.set(2).await.unwrap();
/// assert_eq!(property.get().await.unwrap(), 2);
/// assert_eq!(changes.next().await, Some(2));
/// # });
/// ```
pub struct Property<T> {
    inner: PropertyInner<T>,
}

enum PropertyInner<T> {
    Local {
        value: Arc<RwLock<T>>,
        signal: Signal<T>,
    },
    /// A proxy to the property of an object, identified by name or identifier.
    Remote {
        object: AnyObject,
        ident: ActionNameOrId,
        signal: Signal<T>,
    },
}

impl<T> Property<T>
where
    T: Clone + Send + Sync + 'static + IntoValue<'static> + FromValue<'static> + Reflect,
{
    /// Creates a local property with an initial value.
    pub fn new(value: T) -> Self {
        Self {
            inner: PropertyInner::Local {
                value: Arc::new(RwLock::new(value)),
                signal: Signal::new(),
            },
        }
    }

    /// Creates a proxy to a property of an object.
    ///
    /// The object may be local or remote: accesses go through its [`Object`] implementation.
    /// The existence of the property is checked when it is used.
    pub fn of_object(object: AnyObject, ident: ActionNameOrId) -> Self {
        let signal = Signal::of_object(object.clone(), ident.clone());
        Self {
            inner: PropertyInner::Remote {
                object,
                ident,
                signal,
            },
        }
    }

    /// Returns true if the property is a proxy to the property of an object.
    pub fn is_remote(&self) -> bool {
        matches!(self.inner, PropertyInner::Remote { .. })
    }

    /// Gets the current value of the property.
    pub async fn get(&self) -> Result<T> {
        match &self.inner {
            PropertyInner::Local { value, .. } => Ok(read(value).clone()),
            PropertyInner::Remote { object, ident, .. } => {
                let ty = <T as Reflect>::ty();
                object
                    .meta_property(ident.clone())
                    .await?
                    .convert_to(ty.as_ref())
                    .and_then(T::from_value)
                    .map_err(|err| ValueConversionError::MethodReturnValue(err).into())
            }
        }
    }

    /// Sets the value of the property, notifying subscribers of the change.
    pub async fn set(&self, new_value: T) -> Result<()> {
        match &self.inner {
            PropertyInner::Local { value, signal } => {
                *write(value) = new_value.clone();
                signal.emit(new_value);
                Ok(())
            }
            PropertyInner::Remote { object, ident, .. } => {
                object
                    .meta_set_property(ident.clone(), new_value.into_value())
                    .await
            }
        }
    }

    /// Subscribes to the changes of the property.
    pub async fn subscribe(&self) -> Result<Subscription<T>> {
        self.signal().subscribe().await
    }

    /// The signal of changes of the property.
    pub fn signal(&self) -> &Signal<T> {
        match &self.inner {
            PropertyInner::Local { signal, .. } | PropertyInner::Remote { signal, .. } => signal,
        }
    }

    /// Gets the value of the property as an untyped value.
    #[doc(hidden)]
    pub async fn get_erased(&self) -> Result<Value<'static>> {
        Ok(self.get().await?.into_value())
    }

    /// Sets the value of the property from an untyped value, converting it to the property type.
    #[doc(hidden)]
    pub async fn set_erased(&self, value: Value<'_>) -> Result<()> {
        let ty = <T as Reflect>::ty();
        let value = value
            .convert_to(ty.as_ref())
            .and_then(|value| T::from_value(value.into_owned()))
            .map_err(ValueConversionError::Arguments)?;
        self.set(value).await
    }

    /// The type of the values of the property, `None` if dynamic.
    pub fn value_type() -> Option<value::Type> {
        <T as Reflect>::ty()
    }

    /// Subscribes to the changes of the property as a stream of untyped parameters tuples.
    #[doc(hidden)]
    pub async fn subscribe_erased(&self) -> Result<ValueStream> {
        self.signal().subscribe_erased().await
    }
}

impl<T> Default for Property<T>
where
    T: Default + Clone + Send + Sync + 'static + IntoValue<'static> + FromValue<'static> + Reflect,
{
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> Clone for Property<T> {
    fn clone(&self) -> Self {
        Self {
            inner: match &self.inner {
                PropertyInner::Local { value, signal } => PropertyInner::Local {
                    value: Arc::clone(value),
                    signal: signal.clone(),
                },
                PropertyInner::Remote {
                    object,
                    ident,
                    signal,
                } => PropertyInner::Remote {
                    object: object.clone(),
                    ident: ident.clone(),
                    signal: signal.clone(),
                },
            },
        }
    }
}

impl<T> std::fmt::Debug for Property<T>
where
    T: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.inner {
            PropertyInner::Local { value, .. } => f
                .debug_struct("Property")
                .field("kind", &"local")
                .field("value", &*read(value))
                .finish(),
            PropertyInner::Remote { object, ident, .. } => f
                .debug_struct("Property")
                .field("kind", &"remote")
                .field("object", object)
                .field("ident", ident)
                .finish(),
        }
    }
}

fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|err| {
        lock.clear_poison();
        err.into_inner()
    })
}

fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(|err| {
        lock.clear_poison();
        err.into_inner()
    })
}
