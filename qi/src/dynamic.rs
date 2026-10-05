//! Objects built dynamically from closures, signals and properties.
//!
//! [`ObjectBuilder`] declares the members of an object at runtime and produces a
//! [`DynamicObject`], which implements [`Object`]. It is the counterpart of the
//! `DynamicObjectBuilder` of the reference implementation, and is useful to expose services
//! whose interface is not known statically, such as simulated ones.
//!
//! ```
//! use qi::{dynamic::ObjectBuilder, ObjectExt, Signal};
//! # tokio_test::block_on(async {
//! let fired = Signal::<i32>::new();
//! let mut builder = ObjectBuilder::new();
//! builder.add_method("add", |(a, b): (i32, i32)| async move { Ok(a + b) });
//! builder.add_signal("fired", fired.clone());
//! let object = builder.build();
//! let sum: i32 = object.call("add", (1, 2)).await.unwrap();
//! assert_eq!(sum, 3);
//! # });
//! ```

use crate::{
    object::{
        self, params, ActionId, ActionNameOrId, MetaMethod, MetaObject, MetaProperty, MetaSignal,
    },
    property::Property,
    signal::{Signal, ValueStream},
    value::{FromValue, IntoValue, Reflect, Value},
    Error, Object, Result,
};
use async_trait::async_trait;
use futures::future::BoxFuture;
use std::{collections::HashMap, future::Future, sync::Arc};

type MethodFn =
    Arc<dyn Fn(Value<'static>) -> BoxFuture<'static, Result<Value<'static>>> + Send + Sync>;

/// A type-erased signal handle.
trait ErasedSignal: Send + Sync {
    fn erased_subscribe(&self) -> BoxFuture<'_, Result<ValueStream>>;
    fn erased_emit(&self, params: Value<'_>) -> Result<()>;
}

impl<T> ErasedSignal for Signal<T>
where
    T: Clone + Send + Sync + 'static + IntoValue<'static> + FromValue<'static> + Reflect,
{
    fn erased_subscribe(&self) -> BoxFuture<'_, Result<ValueStream>> {
        Box::pin(self.subscribe_erased())
    }

    fn erased_emit(&self, params: Value<'_>) -> Result<()> {
        self.emit_erased(params)
    }
}

/// A type-erased property handle.
trait ErasedProperty: Send + Sync {
    fn erased_get(&self) -> BoxFuture<'_, Result<Value<'static>>>;
    fn erased_set<'a>(&'a self, value: Value<'a>) -> BoxFuture<'a, Result<()>>;
    fn erased_subscribe(&self) -> BoxFuture<'_, Result<ValueStream>>;
}

impl<T> ErasedProperty for Property<T>
where
    T: Clone + Send + Sync + 'static + IntoValue<'static> + FromValue<'static> + Reflect,
{
    fn erased_get(&self) -> BoxFuture<'_, Result<Value<'static>>> {
        Box::pin(self.get_erased())
    }

    fn erased_set<'a>(&'a self, value: Value<'a>) -> BoxFuture<'a, Result<()>> {
        Box::pin(self.set_erased(value))
    }

    fn erased_subscribe(&self) -> BoxFuture<'_, Result<ValueStream>> {
        Box::pin(self.subscribe_erased())
    }
}

/// A builder of dynamic objects.
#[derive(Default)]
pub struct ObjectBuilder {
    meta: MetaObject,
    next_id: ActionId,
    methods: HashMap<ActionId, MethodFn>,
    signals: HashMap<ActionId, Box<dyn ErasedSignal>>,
    properties: HashMap<ActionId, Box<dyn ErasedProperty>>,
}

impl ObjectBuilder {
    /// Creates a builder of an object with no member.
    pub fn new() -> Self {
        Self {
            meta: MetaObject::default(),
            next_id: object::ACTION_START_ID,
            methods: HashMap::new(),
            signals: HashMap::new(),
            properties: HashMap::new(),
        }
    }

    fn next_id(&mut self) -> ActionId {
        self.next_id.wrapping_next()
    }

    /// Sets the description of the object.
    pub fn set_description<S: Into<String>>(&mut self, description: S) -> &mut Self {
        self.meta.description = description.into();
        self
    }

    /// Adds a method implemented by an asynchronous function of its arguments.
    ///
    /// The arguments type `Args` follows the parameters conventions of [`ObjectExt`](crate::ObjectExt): a tuple
    /// maps to several parameters, `()` to none and any other type to a single parameter.
    pub fn add_method<F, Fut, Args, R>(&mut self, name: impl Into<String>, f: F) -> ActionId
    where
        F: Fn(Args) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<R>> + Send + 'static,
        Args: FromValue<'static> + Reflect + Send,
        R: IntoValue<'static> + Reflect,
    {
        let id = self.next_id();
        self.add_method_with_id(id, name, f);
        id
    }

    /// Adds a method with an explicit identifier.
    pub fn add_method_with_id<F, Fut, Args, R>(
        &mut self,
        id: ActionId,
        name: impl Into<String>,
        f: F,
    ) -> &mut Self
    where
        F: Fn(Args) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<R>> + Send + 'static,
        Args: FromValue<'static> + Reflect + Send,
        R: IntoValue<'static> + Reflect,
    {
        let mut builder = MetaMethod::builder(id);
        builder.set_name(name);
        let params_type = params::params_type::<Args>();
        if let crate::value::Type::Tuple(tuple) = &params_type {
            for (index, ty) in tuple.element_types().into_iter().enumerate() {
                builder.parameter(index).set_type(ty);
            }
        }
        builder.return_value().set_type(<R as Reflect>::ty());
        self.meta.methods.insert(id, builder.build());
        let f = Arc::new(f);
        let params_type = Arc::new(params_type);
        self.methods.insert(
            id,
            Arc::new(move |args: Value<'static>| {
                let f = Arc::clone(&f);
                let params_type = Arc::clone(&params_type);
                Box::pin(async move {
                    // Local callers may pass arguments of other (compatible) types than the
                    // declared parameters, e.g. static values for dynamic parameters.
                    let args = args
                        .convert_to(Some(&params_type))
                        .map_err(crate::error::ValueConversionError::Arguments)?;
                    let args = params::from_params::<Args>(args);
                    let args = Args::from_value(args)
                        .map_err(crate::error::ValueConversionError::Arguments)?;
                    let result = f(args).await?;
                    Ok(result.into_value())
                })
            }),
        );
        self
    }

    /// Adds a signal.
    pub fn add_signal<T>(&mut self, name: impl Into<String>, signal: Signal<T>) -> ActionId
    where
        T: Clone + Send + Sync + 'static + IntoValue<'static> + FromValue<'static> + Reflect,
    {
        let id = self.next_id();
        self.add_signal_with_id(id, name, signal);
        id
    }

    /// Adds a signal with an explicit identifier.
    pub fn add_signal_with_id<T>(
        &mut self,
        id: ActionId,
        name: impl Into<String>,
        signal: Signal<T>,
    ) -> &mut Self
    where
        T: Clone + Send + Sync + 'static + IntoValue<'static> + FromValue<'static> + Reflect,
    {
        self.meta.signals.insert(
            id,
            MetaSignal {
                uid: id,
                name: name.into(),
                signature: Signal::<T>::params_type().into(),
            },
        );
        self.signals.insert(id, Box::new(signal));
        self
    }

    /// Adds a property.
    pub fn add_property<T>(&mut self, name: impl Into<String>, property: Property<T>) -> ActionId
    where
        T: Clone + Send + Sync + 'static + IntoValue<'static> + FromValue<'static> + Reflect,
    {
        let id = self.next_id();
        self.add_property_with_id(id, name, property);
        id
    }

    /// Adds a property with an explicit identifier.
    pub fn add_property_with_id<T>(
        &mut self,
        id: ActionId,
        name: impl Into<String>,
        property: Property<T>,
    ) -> &mut Self
    where
        T: Clone + Send + Sync + 'static + IntoValue<'static> + FromValue<'static> + Reflect,
    {
        let name = name.into();
        // Properties are also signals of their changes.
        self.meta.signals.insert(
            id,
            MetaSignal {
                uid: id,
                name: name.clone(),
                signature: Signal::<T>::params_type().into(),
            },
        );
        self.meta.properties.insert(
            id,
            MetaProperty {
                uid: id,
                name,
                signature: <T as Reflect>::ty().into(),
            },
        );
        self.properties.insert(id, Box::new(property));
        self
    }

    /// The meta object built so far.
    pub fn meta(&self) -> &MetaObject {
        &self.meta
    }

    /// Builds the object.
    pub fn build(self) -> DynamicObject {
        DynamicObject {
            meta: self.meta,
            methods: self.methods,
            signals: self.signals,
            properties: self.properties,
        }
    }
}

impl std::fmt::Debug for ObjectBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectBuilder")
            .field("meta", &self.meta)
            .finish_non_exhaustive()
    }
}

/// An object built dynamically. See [`ObjectBuilder`].
pub struct DynamicObject {
    meta: MetaObject,
    methods: HashMap<ActionId, MethodFn>,
    signals: HashMap<ActionId, Box<dyn ErasedSignal>>,
    properties: HashMap<ActionId, Box<dyn ErasedProperty>>,
}

impl DynamicObject {
    fn signal(&self, ident: &ActionNameOrId) -> Result<&dyn ErasedSignal> {
        self.meta
            .signal(ident)
            .and_then(|signal| self.signals.get(&signal.uid))
            .map(|signal| &**signal)
            .ok_or_else(|| Error::SignalNotFound(ident.clone()))
    }

    fn property(&self, ident: &ActionNameOrId) -> Result<&dyn ErasedProperty> {
        self.meta
            .property(ident)
            .and_then(|property| self.properties.get(&property.uid))
            .map(|property| &**property)
            .ok_or_else(|| Error::PropertyNotFound(ident.clone()))
    }
}

impl std::fmt::Debug for DynamicObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DynamicObject")
            .field("meta", &self.meta)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Object for DynamicObject {
    fn meta(&self) -> &MetaObject {
        &self.meta
    }

    async fn meta_call(&self, ident: ActionNameOrId, args: Value<'_>) -> Result<Value<'static>> {
        let method = self
            .meta
            .method(&ident)
            .and_then(|method| self.methods.get(&method.uid))
            .ok_or_else(|| Error::MethodNotFound(ident.clone()))?;
        method(args.into_owned()).await
    }

    async fn meta_emit(&self, ident: ActionNameOrId, params: Value<'_>) -> Result<()> {
        self.signal(&ident)?.erased_emit(params)
    }

    async fn meta_property(&self, ident: ActionNameOrId) -> Result<Value<'static>> {
        self.property(&ident)?.erased_get().await
    }

    async fn meta_set_property(&self, ident: ActionNameOrId, value: Value<'_>) -> Result<()> {
        self.property(&ident)?.erased_set(value).await
    }

    async fn meta_subscribe(&self, ident: ActionNameOrId) -> Result<ValueStream> {
        match self.signal(&ident) {
            Ok(signal) => signal.erased_subscribe().await,
            Err(err) => match self.property(&ident) {
                Ok(property) => property.erased_subscribe().await,
                Err(_) => Err(err),
            },
        }
    }
}
