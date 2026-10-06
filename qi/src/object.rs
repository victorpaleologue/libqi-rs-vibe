//! Objects: the units of interaction of the `qi` type system.
//!
//! An [`Object`] exposes methods that may be called, signals that may be subscribed to and
//! properties that may be read, written and subscribed to. Its members are described by a
//! [`MetaObject`], which identifies each of them by a name and an [`ActionId`].
//!
//! Objects are either *local*, implemented in this process, or *remote*, in which case they are
//! manipulated through an [`ObjectClient`] proxy that forwards each interaction through a
//! messaging session. Both implement the [`Object`] trait, and [`ObjectExt`] provides typed
//! conveniences over it.
//!
//! Objects are values of the type system: they may be passed as arguments to methods, returned
//! by them or carried by signals. When an object crosses a session, a *reference* to it
//! ([`value::object::Object`]) is transmitted and the receiving side binds it to a proxy. The
//! conversions between objects and values are implemented on `Arc<dyn Object>`.

pub(crate) mod generic;
pub(crate) mod params;

pub use crate::value::object::{
    ActionId, ActionNameOrId, Id, MetaMethod, MetaMethodParameter, MetaObject, MetaProperty,
    MetaSignal, Uid,
};
pub use generic::{is_special, SPECIAL_MEMBER_MAX_ID};

use crate::{
    error::ValueConversionError,
    messaging::message,
    property::Property,
    service,
    session::{Capabilities, Protocol, Session},
    signal::{Signal, Subscription, ValueStream},
    value::{self, Dynamic, FromValue, FromValueError, IntoValue, Reflect, RuntimeReflect, Value},
    Error, Result,
};
use async_trait::async_trait;
use sealed::sealed;
use std::sync::Arc;
use tracing::{debug, warn};

/// A reference to an object as transmitted in the type system.
pub type Reference = value::object::Object;

/// A shared, type-erased object.
///
/// `AnyObject` is the handle through which objects travel: services are published as
/// `AnyObject`s, remote services are returned as `AnyObject`s wrapping proxies, and objects are
/// passed to and returned from methods as `AnyObject`s. It is cheap to clone and implements
/// [`Object`] by delegation.
#[derive(Clone)]
pub struct AnyObject(Arc<dyn Object + Send + Sync>);

impl AnyObject {
    /// Erases the type of an object.
    pub fn new<O>(object: O) -> Self
    where
        O: Object + 'static,
    {
        Self(Arc::new(object))
    }

    /// Wraps a shared object.
    pub fn from_arc(object: Arc<dyn Object + Send + Sync>) -> Self {
        Self(object)
    }

    /// The shared object.
    pub fn as_arc(&self) -> &Arc<dyn Object + Send + Sync> {
        &self.0
    }

    /// Returns the proxy this object is, if it is a proxy to a remote object.
    pub fn as_client(&self) -> Option<&ObjectClient> {
        self.0.as_object_client()
    }
}

impl std::ops::Deref for AnyObject {
    type Target = dyn Object + Send + Sync;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl std::fmt::Debug for AnyObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnyObject")
            .field("uid", &self.0.uid())
            .field("description", &self.0.meta().description)
            .finish()
    }
}

impl<O> From<Arc<O>> for AnyObject
where
    O: Object + 'static,
{
    fn from(object: Arc<O>) -> Self {
        Self(object)
    }
}

#[async_trait]
impl Object for AnyObject {
    fn meta(&self) -> &MetaObject {
        self.0.meta()
    }

    async fn meta_call(&self, ident: ActionNameOrId, args: Value<'_>) -> Result<Value<'static>> {
        self.0.meta_call(ident, args).await
    }

    async fn meta_post(&self, ident: ActionNameOrId, args: Value<'_>) {
        self.0.meta_post(ident, args).await
    }

    async fn meta_emit(&self, ident: ActionNameOrId, params: Value<'_>) -> Result<()> {
        self.0.meta_emit(ident, params).await
    }

    async fn meta_property(&self, ident: ActionNameOrId) -> Result<Value<'static>> {
        self.0.meta_property(ident).await
    }

    async fn meta_set_property(&self, ident: ActionNameOrId, value: Value<'_>) -> Result<()> {
        self.0.meta_set_property(ident, value).await
    }

    async fn meta_subscribe(&self, ident: ActionNameOrId) -> Result<ValueStream> {
        self.0.meta_subscribe(ident).await
    }

    fn uid(&self) -> Uid {
        self.0.uid()
    }

    fn as_object_client(&self) -> Option<&ObjectClient> {
        self.0.as_object_client()
    }
}

/// The identifier of the main object of a service.
pub const MAIN_OBJECT_ID: Id = Id(1);

/// The identifier of the first user member of an object. Identifiers below it are reserved for
/// the special members of the messaging protocol.
pub const ACTION_START_ID: ActionId = SPECIAL_MEMBER_MAX_ID;

/// An object of the `qi` type system.
///
/// The methods of this trait are the *meta* interface of objects: they identify members by name
/// or identifier and exchange untyped [`Value`]s. Arguments of calls, posts and events are always
/// a tuple of the parameters of the member (see [`ObjectExt`] for the conventions of typed
/// conversions). Implementations are usually generated by the [`object`](macro@crate::object) macro or
/// built dynamically, and this trait is what the messaging layer relies upon to serve objects to
/// remote peers.
#[async_trait]
pub trait Object: Send + Sync {
    /// The description of the members of this object.
    fn meta(&self) -> &MetaObject;

    /// Calls a method with a tuple of arguments and returns its result.
    async fn meta_call(&self, ident: ActionNameOrId, args: Value<'_>) -> Result<Value<'static>>;

    /// Calls a method with a tuple of arguments, discarding its result and errors.
    ///
    /// The default implementation calls the method and logs its errors.
    async fn meta_post(&self, ident: ActionNameOrId, args: Value<'_>) {
        if let Err(error) = self.meta_call(ident.clone(), args).await {
            debug!(
                member = %ident,
                error = &error as &dyn std::error::Error,
                "post request error"
            );
        }
    }

    /// Emits a signal with a tuple of parameters.
    ///
    /// The default implementation fails with [`Error::SignalNotFound`].
    async fn meta_emit(&self, ident: ActionNameOrId, params: Value<'_>) -> Result<()> {
        let _ = params;
        Err(Error::SignalNotFound(ident))
    }

    /// Gets the value of a property.
    ///
    /// The default implementation fails with [`Error::PropertyNotFound`].
    async fn meta_property(&self, ident: ActionNameOrId) -> Result<Value<'static>> {
        Err(Error::PropertyNotFound(ident))
    }

    /// Sets the value of a property.
    ///
    /// The default implementation fails with [`Error::PropertyNotFound`].
    async fn meta_set_property(&self, ident: ActionNameOrId, value: Value<'_>) -> Result<()> {
        let _ = value;
        Err(Error::PropertyNotFound(ident))
    }

    /// Subscribes to a signal or to the changes of a property, as a stream of tuples of
    /// parameters.
    ///
    /// The default implementation fails with [`Error::SignalNotFound`].
    async fn meta_subscribe(&self, ident: ActionNameOrId) -> Result<ValueStream> {
        Err(Error::SignalNotFound(ident))
    }

    /// The unique identifier of the object.
    ///
    /// The default implementation derives it from the address of the object in memory, the
    /// process and the machine. Proxies return the identifier of the object they refer to.
    fn uid(&self) -> Uid {
        Uid::from_ptr(self)
    }

    /// Returns this object as a proxy to a remote object, if it is one.
    #[doc(hidden)]
    fn as_object_client(&self) -> Option<&ObjectClient> {
        None
    }
}

/// Typed conveniences over the meta interface of [`Object`].
///
/// # Arguments and parameters
///
/// A method of the type system takes a tuple of parameters. When calling a method, the arguments
/// are given as a single Rust value: a Rust tuple `(a, b)` maps to two parameters, `()` to no
/// parameter, and any other value to a single parameter. The same convention applies to the values
/// of signals.
#[sealed]
#[async_trait]
pub trait ObjectExt: Object {
    /// Calls a method and converts its result.
    async fn call<'a, R, I, T>(&self, ident: I, args: T) -> Result<R>
    where
        I: Into<ActionNameOrId> + Send,
        T: IntoValue<'a> + Reflect + Send,
        R: FromValue<'static> + Reflect,
    {
        let args = params::to_params::<T>(args.into_value());
        let result = self
            .meta_call(ident.into(), args)
            .await?
            .convert_to(R::ty().as_ref())
            .map_err(ValueConversionError::MethodReturnValue)?;
        R::from_value(result).map_err(|err| ValueConversionError::MethodReturnValue(err).into())
    }

    /// Calls a method without waiting for its result.
    async fn post<'a, I, T>(&self, ident: I, args: T)
    where
        I: Into<ActionNameOrId> + Send,
        T: IntoValue<'a> + Reflect + Send,
    {
        let args = params::to_params::<T>(args.into_value());
        self.meta_post(ident.into(), args).await
    }

    /// Emits a signal of the object with a value.
    async fn emit<'a, I, T>(&self, ident: I, value: T) -> Result<()>
    where
        I: Into<ActionNameOrId> + Send,
        T: IntoValue<'a> + Reflect + Send,
    {
        let params = params::to_params::<T>(value.into_value());
        self.meta_emit(ident.into(), params).await
    }

    /// Gets the value of a property.
    async fn property<I, R>(&self, ident: I) -> Result<R>
    where
        I: Into<ActionNameOrId> + Send,
        R: FromValue<'static>,
    {
        let value = self.meta_property(ident.into()).await?;
        R::from_value(value).map_err(|err| ValueConversionError::MethodReturnValue(err).into())
    }

    /// Sets the value of a property.
    async fn set_property<'a, I, T>(&self, ident: I, value: T) -> Result<()>
    where
        I: Into<ActionNameOrId> + Send,
        T: IntoValue<'a> + Send,
    {
        self.meta_set_property(ident.into(), value.into_value())
            .await
    }

    /// The names of the properties of the object.
    fn properties(&self) -> Vec<String> {
        self.meta()
            .properties
            .values()
            .map(|prop| prop.name.clone())
            .collect()
    }

    /// Subscribes to a signal or to the changes of a property.
    async fn subscribe<I, T>(&self, ident: I) -> Result<Subscription<T>>
    where
        I: Into<ActionNameOrId> + Send,
        T: Reflect,
    {
        let stream = self.meta_subscribe(ident.into()).await?;
        Ok(Subscription::from_params(stream, T::ty()))
    }
}

#[sealed]
#[async_trait]
impl<O> ObjectExt for O where O: Object + ?Sized {}

/// A proxy to a remote object, that forwards interactions through a messaging session.
///
/// Proxies are cheap to clone and share the same underlying state. When the last clone of a proxy
/// to an object that was transmitted by value (as opposed to the main object of a service) is
/// dropped, the remote peer is notified that the object is no longer referenced.
#[derive(Clone)]
pub struct ObjectClient(Arc<ClientInner>);

struct ClientInner {
    service_id: service::Id,
    id: Id,
    uid: Uid,
    meta: MetaObject,
    session: Session,
    /// The object was transmitted by value and must be released on the remote peer when the last
    /// proxy is dropped.
    transient: bool,
}

impl Drop for ClientInner {
    fn drop(&mut self) {
        if self.transient {
            self.session.terminate_object(self.service_id, self.id);
        }
    }
}

impl ObjectClient {
    /// Creates a proxy to an object whose meta object is already known.
    pub(crate) fn new(
        service_id: service::Id,
        id: Id,
        uid: Uid,
        meta: MetaObject,
        session: Session,
        transient: bool,
    ) -> Self {
        Self(Arc::new(ClientInner {
            service_id,
            id,
            uid,
            meta,
            session,
            transient,
        }))
    }

    /// Creates a proxy to an object, fetching its meta object from the remote peer.
    pub(crate) async fn connect(
        service_id: service::Id,
        id: Id,
        uid: Uid,
        session: Session,
    ) -> Result<Self> {
        let meta = Self::fetch_meta_object(&session, service_id, id).await?;
        Ok(Self::new(service_id, id, uid, meta, session, false))
    }

    async fn fetch_meta_object(
        session: &Session,
        service_id: service::Id,
        id: Id,
    ) -> Result<MetaObject> {
        let value = session
            .call(
                message::Address(service_id, id, generic::META_OBJECT),
                Value::Tuple(vec![u32::from(id).into_value()]),
                <MetaObject as Reflect>::ty().as_ref(),
            )
            .await?;
        MetaObject::from_value(value)
            .map_err(|err| ValueConversionError::MethodReturnValue(err).into())
    }

    /// The identifier of the service the object belongs to.
    pub fn service_id(&self) -> service::Id {
        self.0.service_id
    }

    /// The protocol variant the peer hosting the object speaks, detected when the session to it
    /// was established.
    pub fn protocol(&self) -> Option<Protocol> {
        self.0.session.protocol()
    }

    /// The capabilities shared with the peer hosting the object.
    pub fn capabilities(&self) -> Capabilities {
        self.0.session.capabilities()
    }

    /// The identifier of the object within its service.
    pub fn id(&self) -> Id {
        self.0.id
    }

    /// The address of a member of the object.
    fn address(&self, action: ActionId) -> message::Address {
        message::Address(self.0.service_id, self.0.id, action)
    }

    fn method(&self, ident: &ActionNameOrId) -> Result<&MetaMethod> {
        self.0
            .meta
            .method(ident)
            .ok_or_else(|| Error::MethodNotFound(ident.clone()))
    }

    fn signal_id(&self, ident: &ActionNameOrId) -> Result<ActionId> {
        self.0
            .meta
            .signal(ident)
            .map(|signal| signal.uid)
            .or_else(|| self.0.meta.property(ident).map(|prop| prop.uid))
            .ok_or_else(|| Error::SignalNotFound(ident.clone()))
    }

    fn property_id(&self, ident: &ActionNameOrId) -> Result<ActionId> {
        self.0
            .meta
            .property(ident)
            .map(|prop| prop.uid)
            .ok_or_else(|| Error::PropertyNotFound(ident.clone()))
    }

    /// Calls a method by identifier with a tuple of arguments.
    pub(crate) async fn call_action(
        &self,
        action: ActionId,
        args: Value<'_>,
        return_type: Option<&value::Type>,
    ) -> Result<Value<'static>> {
        self.0
            .session
            .call(self.address(action), args, return_type)
            .await
    }

    /// Posts a tuple of arguments to a method or a signal by identifier, without waiting.
    pub(crate) fn post_action(&self, action: ActionId, args: Value<'_>) {
        if let Err(error) = self.0.session.post(self.address(action), args) {
            debug!(
                error = &error as &dyn std::error::Error,
                "post request error: failure to send"
            );
        }
    }

    /// Subscribes to a signal by identifier.
    pub(crate) async fn subscribe_action(&self, action: ActionId) -> Result<ValueStream> {
        let signature = self
            .0
            .meta
            .signal(&ActionNameOrId::Id(action))
            .map(|signal| signal.signature.clone())
            .or_else(|| {
                self.0
                    .meta
                    .property(&ActionNameOrId::Id(action))
                    .map(|prop| {
                        value::Signature::from(params::params_type_of(
                            prop.signature.clone().into_type(),
                        ))
                    })
            })
            .ok_or(Error::SignalNotFound(ActionNameOrId::Id(action)))?;
        self.0
            .session
            .subscribe(self.address(action), signature)
            .await
    }

    /// Gets a property by identifier.
    pub(crate) async fn property_action<R>(&self, action: ActionId) -> Result<R>
    where
        R: FromValue<'static>,
    {
        let value = self
            .call_action(
                generic::PROPERTY,
                Value::Tuple(vec![Dynamic(u32::from(action)).into_value()]),
                None,
            )
            .await?;
        R::from_value(unwrap_dynamic(value))
            .map_err(|err| ValueConversionError::MethodReturnValue(err).into())
    }

    /// Sets a property by identifier.
    pub(crate) async fn set_property_action(
        &self,
        action: ActionId,
        value: Value<'_>,
    ) -> Result<()> {
        let () = self
            .call_action(
                generic::SET_PROPERTY,
                Value::Tuple(vec![
                    Dynamic(u32::from(action)).into_value(),
                    Value::Dynamic(Box::new(value)),
                ]),
                Some(&value::Type::Unit),
            )
            .await?
            .cast_into()
            .map_err(ValueConversionError::MethodReturnValue)?;
        Ok(())
    }

    /// Emits a signal of the remote object by posting its parameters, without waiting.
    pub(crate) fn emit_now(&self, ident: &ActionNameOrId, params: Value<'_>) -> Result<()> {
        let id = self.signal_id(ident)?;
        self.post_action(id, params);
        Ok(())
    }

    /// Returns a proxy to a signal of the object.
    pub fn signal<T, I>(&self, ident: I) -> Result<Signal<T>>
    where
        I: Into<ActionNameOrId>,
        T: Clone + Send + 'static,
    {
        let id = self.signal_id(&ident.into())?;
        Ok(Signal::of_object(AnyObject::new(self.clone()), id.into()))
    }

    /// Returns a proxy to a property of the object.
    pub fn property_proxy<T, I>(&self, ident: I) -> Result<Property<T>>
    where
        I: Into<ActionNameOrId>,
        T: Clone + Send + Sync + 'static + IntoValue<'static> + FromValue<'static> + Reflect,
    {
        let id = self.property_id(&ident.into())?;
        Ok(Property::of_object(AnyObject::new(self.clone()), id.into()))
    }
}

fn unwrap_dynamic(value: Value<'_>) -> Value<'_> {
    match value {
        Value::Dynamic(inner) => *inner,
        value => value,
    }
}

impl std::fmt::Debug for ObjectClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectClient")
            .field("service_id", &self.0.service_id)
            .field("id", &self.0.id)
            .field("uid", &self.0.uid)
            .field("transient", &self.0.transient)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Object for ObjectClient {
    fn meta(&self) -> &MetaObject {
        &self.0.meta
    }

    async fn meta_call(&self, ident: ActionNameOrId, args: Value<'_>) -> Result<Value<'static>> {
        let method = self.method(&ident)?;
        let (uid, return_type) = (method.uid, method.return_signature.as_type());
        // Arguments are converted to the parameters of the remote method, as the remote peer
        // decodes them according to its own signature.
        let args = args
            .convert_to(method.parameters_signature.as_type())
            .map_err(ValueConversionError::Arguments)?;
        self.call_action(uid, args, return_type).await
    }

    async fn meta_post(&self, ident: ActionNameOrId, args: Value<'_>) {
        let action = self
            .0
            .meta
            .method(&ident)
            .map(|method| (method.uid, method.parameters_signature.as_type()))
            .or_else(|| {
                self.0
                    .meta
                    .signal(&ident)
                    .map(|signal| (signal.uid, signal.signature.as_type()))
            });
        let Some((action, params_type)) = action else {
            warn!(member = %ident, "post request error: member not found");
            return;
        };
        match args.convert_to(params_type) {
            Ok(args) => self.post_action(action, args),
            Err(error) => warn!(
                member = %ident,
                error = &error as &dyn std::error::Error,
                "post request error: arguments conversion failed"
            ),
        }
    }

    async fn meta_emit(&self, ident: ActionNameOrId, params: Value<'_>) -> Result<()> {
        // Emitting a remote signal is posting its parameters to the signal member: the remote
        // object triggers it and bounces the event to subscribers.
        self.emit_now(&ident, params)
    }

    async fn meta_property(&self, ident: ActionNameOrId) -> Result<Value<'static>> {
        let id = self.property_id(&ident)?;
        self.property_action(id).await
    }

    async fn meta_set_property(&self, ident: ActionNameOrId, value: Value<'_>) -> Result<()> {
        let id = self.property_id(&ident)?;
        self.set_property_action(id, value).await
    }

    async fn meta_subscribe(&self, ident: ActionNameOrId) -> Result<ValueStream> {
        let id = self.signal_id(&ident)?;
        self.subscribe_action(id).await
    }

    fn uid(&self) -> Uid {
        self.0.uid
    }

    fn as_object_client(&self) -> Option<&ObjectClient> {
        Some(self)
    }
}

// Conversions between objects and values.

impl Reflect for AnyObject {
    fn ty() -> Option<value::Type> {
        Some(value::Type::Object)
    }
}

impl RuntimeReflect for AnyObject {
    fn ty(&self) -> value::Type {
        value::Type::Object
    }
}

impl<'a> IntoValue<'a> for AnyObject {
    fn into_value(self) -> Value<'a> {
        let reference = Reference::unbound(self.meta().clone(), self.uid(), Arc::new(self));
        Value::Object(Box::new(reference))
    }
}

impl FromValue<'_> for AnyObject {
    fn from_value(value: Value<'_>) -> std::result::Result<Self, FromValueError> {
        match value {
            Value::Object(reference) => from_reference(&reference),
            _ => Err(FromValueError::TypeMismatch {
                expected: "an object".to_owned(),
                actual: value.to_string(),
            }),
        }
    }
}

impl Reflect for ObjectClient {
    fn ty() -> Option<value::Type> {
        Some(value::Type::Object)
    }
}

impl RuntimeReflect for ObjectClient {
    fn ty(&self) -> value::Type {
        value::Type::Object
    }
}

impl<'a> IntoValue<'a> for ObjectClient {
    fn into_value(self) -> Value<'a> {
        AnyObject::new(self).into_value()
    }
}

impl FromValue<'_> for ObjectClient {
    fn from_value(value: Value<'_>) -> std::result::Result<Self, FromValueError> {
        match value {
            Value::Object(reference) => match reference.handle_as::<ObjectClient>() {
                Some(client) => Ok(client.clone()),
                None => Err(FromValueError::Other(
                    "the object reference is not bound to a remote object".into(),
                )),
            },
            _ => Err(FromValueError::TypeMismatch {
                expected: "an object".to_owned(),
                actual: value.to_string(),
            }),
        }
    }
}

/// Extracts the object an object reference is bound to.
pub(crate) fn from_reference(
    reference: &Reference,
) -> std::result::Result<AnyObject, FromValueError> {
    if let Some(object) = reference.handle_as::<AnyObject>() {
        return Ok(object.clone());
    }
    if let Some(client) = reference.handle_as::<ObjectClient>() {
        return Ok(AnyObject::new(client.clone()));
    }
    Err(FromValueError::Other(
        "the object reference is not bound to any object: it was not received through a session"
            .into(),
    ))
}

/// Extracts the object an object reference is bound to, if any.
pub(crate) fn bound_object(reference: &Reference) -> Option<AnyObject> {
    from_reference(reference).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{object::MetaMethod, Type};
    use assert_matches::assert_matches;
    use once_cell::sync::Lazy;
    use tokio::sync::Mutex;

    #[derive(Debug)]
    struct Calculator {
        a: i32,
    }

    impl Calculator {
        fn new(a: i32) -> Self {
            Self { a }
        }

        fn add(&mut self, b: i32) -> i32 {
            self.a += b;
            self.a
        }

        fn div(&mut self, b: i32) -> std::result::Result<i32, DivisionByZeroError> {
            if b == 0 {
                Err(DivisionByZeroError)
            } else {
                self.a /= b;
                Ok(self.a)
            }
        }

        fn clamp(&mut self, min: i32, max: i32) -> i32 {
            self.a = self.a.clamp(min, max);
            self.a
        }

        fn ans(&self) -> i32 {
            self.a
        }
    }

    #[derive(Debug, thiserror::Error)]
    #[error("division by zero")]
    struct DivisionByZeroError;

    static META: Lazy<MetaObject> = Lazy::new(|| {
        let mut builder = MetaObject::builder();
        let mut id = ACTION_START_ID;
        builder.add_method({
            let mut m = MetaMethod::builder(id.next().unwrap());
            m.set_name("add");
            m.parameter(0).set_type(Type::Int32);
            m.return_value().set_type(Type::Int32);
            m.build()
        });
        builder.add_method({
            let mut m = MetaMethod::builder(id.next().unwrap());
            m.set_name("div");
            m.parameter(0).set_type(Type::Int32);
            m.return_value().set_type(Type::Int32);
            m.build()
        });
        builder.add_method({
            let mut m = MetaMethod::builder(id.next().unwrap());
            m.set_name("clamp");
            m.parameter(0).set_type(Type::Int32);
            m.parameter(1).set_type(Type::Int32);
            m.return_value().set_type(Type::Int32);
            m.build()
        });
        builder.add_method({
            let mut m = MetaMethod::builder(id.next().unwrap());
            m.set_name("ans");
            m.return_value().set_type(Type::Int32);
            m.build()
        });
        builder.build()
    });

    #[async_trait]
    impl Object for Mutex<Calculator> {
        fn meta(&self) -> &MetaObject {
            &META
        }

        async fn meta_call(
            &self,
            ident: ActionNameOrId,
            args: Value<'_>,
        ) -> Result<Value<'static>> {
            let method = META
                .method(&ident)
                .ok_or_else(|| Error::MethodNotFound(ident.clone()))?;
            let mut calc = self.lock().await;
            let result = match method.name.as_str() {
                "add" => {
                    let (b,): (i32,) = args.cast_into().map_err(ValueConversionError::Arguments)?;
                    calc.add(b)
                }
                "div" => {
                    let (b,): (i32,) = args.cast_into().map_err(ValueConversionError::Arguments)?;
                    calc.div(b).map_err(|err| Error::Other(err.into()))?
                }
                "clamp" => {
                    let (min, max): (i32, i32) =
                        args.cast_into().map_err(ValueConversionError::Arguments)?;
                    calc.clamp(min, max)
                }
                "ans" => calc.ans(),
                _ => return Err(Error::MethodNotFound(ident)),
            };
            Ok(result.into_value())
        }
    }

    #[tokio::test]
    async fn calculator_object_typed_calls() {
        let calc = Mutex::new(Calculator::new(42));
        let res: i32 = calc.call("add", 100).await.unwrap();
        assert_eq!(res, 142);
        let res: i32 = calc.call("div", 2).await.unwrap();
        assert_eq!(res, 71);
        let res: i32 = calc.call("clamp", (0, 50)).await.unwrap();
        assert_eq!(res, 50);
        let res: Result<i32> = calc.call("div", 0).await;
        assert_matches!(res, Err(Error::Other(err)) => {
            assert!(err.downcast::<DivisionByZeroError>().is_ok())
        });
        let res: Result<i32> = calc.call("log", 1).await;
        assert_matches!(
            res,
            Err(Error::MethodNotFound(ident)) => assert_eq!(ident, "log")
        );
        let res: i32 = calc.call("ans", ()).await.unwrap();
        assert_eq!(res, 50);
    }

    #[tokio::test]
    async fn object_to_value_round_trip_keeps_identity() {
        let calc = AnyObject::new(Mutex::new(Calculator::new(1)));
        let uid = calc.uid();
        let value = calc.clone().into_value();
        assert_matches!(&value, Value::Object(reference) => {
            assert!(!reference.is_addressed());
            assert_eq!(reference.object_uid, uid);
        });
        let back = AnyObject::from_value(value).unwrap();
        assert_eq!(back.uid(), uid);
        let res: i32 = back.call("ans", ()).await.unwrap();
        assert_eq!(res, 1);
    }
}
