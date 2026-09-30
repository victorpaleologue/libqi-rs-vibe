use crate::value::{os, service, ty, FromValue, FromValueError, IntoValue, Signature, Type, Value};
use sha1_smol::Sha1;
use std::{any::Any, collections::BTreeMap, sync::Arc};

/// An opaque handle to the entity an [`Object`] reference points to.
///
/// The `qi` type system only knows about object *references* (the service and object
/// identifiers, the object UID and the meta object). The layer that implements the messaging of
/// objects binds a reference to something callable: either a local object that is (or is about to
/// be) hosted for a remote peer, or a proxy to a remote object. That binding is carried by this
/// handle and is never serialized.
pub type Handle = Arc<dyn Any + Send + Sync>;

/// A reference to an object in the `qi` type system, of signature `o`.
///
/// An object reference is made of the object [meta object](MetaObject) (the description of its
/// members), the address of the object (the identifier of the service and the identifier of the
/// object within that service) and the [UID](Uid) of the object.
///
/// A reference may be bound to a [`Handle`] by the messaging layer. Unbound references that have a
/// handle (i.e. local objects that were not hosted yet) cannot be serialized, as they have no
/// meaningful address on the wire.
#[derive(Clone, serde::Deserialize)]
pub struct Object {
    pub meta_object: MetaObject,
    pub service_id: service::Id,
    pub object_id: Id,
    pub object_uid: Uid,
    #[serde(skip)]
    handle: Option<Handle>,
    /// Whether the UID is written when the reference is serialized. Peers that do not support the
    /// `ObjectPtrUID` capability expect references without UID.
    #[serde(skip, default = "default_uid_on_wire")]
    uid_on_wire: bool,
}

fn default_uid_on_wire() -> bool {
    true
}

impl Default for Object {
    fn default() -> Self {
        Self {
            meta_object: Default::default(),
            service_id: Default::default(),
            object_id: Default::default(),
            object_uid: Default::default(),
            handle: None,
            uid_on_wire: true,
        }
    }
}

impl Object {
    /// The object identifier of null objects. No bound object has this identifier.
    pub const NULL_ID: Id = Id(0);

    /// Creates a new object reference with no handle.
    pub fn new(
        meta_object: MetaObject,
        service_id: service::Id,
        object_id: Id,
        object_uid: Uid,
    ) -> Self {
        Self {
            meta_object,
            service_id,
            object_id,
            object_uid,
            handle: None,
            uid_on_wire: true,
        }
    }

    /// Creates a reference to a local object that is not hosted (addressed) yet.
    pub fn unbound(meta_object: MetaObject, object_uid: Uid, handle: Handle) -> Self {
        Self {
            meta_object,
            service_id: service::Id(0),
            object_id: Self::NULL_ID,
            object_uid,
            handle: Some(handle),
            uid_on_wire: true,
        }
    }

    /// Sets whether the UID of the object is written when the reference is serialized.
    ///
    /// This must be set to `false` when serializing for a peer that does not support the
    /// `ObjectPtrUID` capability. Defaults to `true`.
    pub fn set_uid_on_wire(&mut self, on_wire: bool) {
        self.uid_on_wire = on_wire;
    }

    /// Whether the UID of the object is written when the reference is serialized.
    pub fn uid_on_wire(&self) -> bool {
        self.uid_on_wire
    }

    /// Returns true if the reference has an address on the wire, i.e. it points to a bound
    /// object.
    pub fn is_addressed(&self) -> bool {
        self.object_id != Self::NULL_ID
    }

    /// Sets the handle of this reference.
    pub fn with_handle(mut self, handle: Handle) -> Self {
        self.handle = Some(handle);
        self
    }

    /// Sets the handle of this reference.
    pub fn set_handle(&mut self, handle: Handle) -> Option<Handle> {
        self.handle.replace(handle)
    }

    /// The handle of this reference, if any.
    pub fn handle(&self) -> Option<&Handle> {
        self.handle.as_ref()
    }

    /// Takes the handle out of this reference, leaving none.
    pub fn take_handle(&mut self) -> Option<Handle> {
        self.handle.take()
    }

    /// Downcasts the handle to a concrete type.
    pub fn handle_as<T>(&self) -> Option<&T>
    where
        T: Any,
    {
        self.handle
            .as_deref()
            .and_then(|handle| handle.downcast_ref())
    }
}

impl<'a> IntoValue<'a> for Object {
    fn into_value(self) -> Value<'a> {
        Value::Object(Box::new(self))
    }
}

impl FromValue<'_> for Object {
    fn from_value(value: Value<'_>) -> Result<Self, FromValueError> {
        match value {
            Value::Object(object) => Ok(*object),
            _ => Err(FromValueError::TypeMismatch {
                expected: "an Object".to_owned(),
                actual: value.to_string(),
            }),
        }
    }
}

impl std::fmt::Display for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "object(service={}, id={}, uid={})",
            self.service_id, self.object_id, self.object_uid
        )
    }
}

/// A `serde` deserialization seed of object references, that reads the UID of the object only
/// when the peer that wrote the reference supports the `ObjectPtrUID` capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjectSeed {
    pub uid_on_wire: bool,
}

impl<'de> serde::de::DeserializeSeed<'de> for ObjectSeed {
    type Value = Object;

    fn deserialize<D>(self, deserializer: D) -> Result<Object, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor {
            uid_on_wire: bool,
        }
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Object;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an object reference")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                use serde::de::Error;
                let meta_object = seq
                    .next_element()?
                    .ok_or_else(|| A::Error::missing_field("metaObject"))?;
                let service_id = seq
                    .next_element()?
                    .ok_or_else(|| A::Error::missing_field("serviceId"))?;
                let object_id = seq
                    .next_element()?
                    .ok_or_else(|| A::Error::missing_field("objectId"))?;
                let object_uid = if self.uid_on_wire {
                    seq.next_element()?
                        .ok_or_else(|| A::Error::missing_field("objectUid"))?
                } else {
                    Uid::default()
                };
                Ok(Object {
                    meta_object,
                    service_id,
                    object_id,
                    object_uid,
                    handle: None,
                    uid_on_wire: self.uid_on_wire,
                })
            }
        }
        let fields = if self.uid_on_wire { 4 } else { 3 };
        deserializer.deserialize_tuple(
            fields,
            Visitor {
                uid_on_wire: self.uid_on_wire,
            },
        )
    }
}

impl PartialEq for Object {
    fn eq(&self, other: &Self) -> bool {
        self.meta_object == other.meta_object
            && self.service_id == other.service_id
            && self.object_id == other.object_id
            && self.object_uid == other.object_uid
    }
}

impl Eq for Object {}

impl PartialOrd for Object {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Object {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            &self.service_id,
            &self.object_id,
            &self.object_uid,
            &self.meta_object,
        )
            .cmp(&(
                &other.service_id,
                &other.object_id,
                &other.object_uid,
                &other.meta_object,
            ))
    }
}

impl std::hash::Hash for Object {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.meta_object.hash(state);
        self.service_id.hash(state);
        self.object_id.hash(state);
        self.object_uid.hash(state);
    }
}

impl std::fmt::Debug for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Object")
            .field("meta_object", &self.meta_object)
            .field("service_id", &self.service_id)
            .field("object_id", &self.object_id)
            .field("object_uid", &self.object_uid)
            .field("handle", &self.handle.as_ref().map(|_| "..."))
            .finish()
    }
}

impl serde::Serialize for Object {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        if self.handle.is_some() && !self.is_addressed() {
            return Err(serde::ser::Error::custom(
                "the object reference is bound to a local object that is not hosted, \
                 it cannot be serialized",
            ));
        }
        let fields = if self.uid_on_wire { 4 } else { 3 };
        let mut object = serializer.serialize_struct("Object", fields)?;
        object.serialize_field("metaObject", &self.meta_object)?;
        object.serialize_field("serviceId", &self.service_id)?;
        object.serialize_field("objectId", &self.object_id)?;
        if self.uid_on_wire {
            object.serialize_field("objectUid", &self.object_uid)?;
        }
        object.end()
    }
}

#[derive(
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    derive_more::From,
    derive_more::Into,
    derive_more::IntoIterator,
)]
#[into_iterator(owned, ref)]
#[serde(transparent)]
pub struct Uid(
    // SHA-1 digest as bytes of Big Endian encoded sequence of 5 DWORD.
    [u8; 20],
);

impl Uid {
    pub const fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    pub const fn bytes(&self) -> &[u8; 20] {
        &self.0
    }

    pub fn from_ptr<T: ?Sized>(ptr: *const T) -> Self {
        let machine_id = os::MachineId::default();
        let process_uuid = os::process_uuid();
        let ptr_addr = ptr.cast::<()>() as usize;
        let mut hasher = Sha1::new();
        hasher.update(machine_id.as_bytes());
        hasher.update(process_uuid.as_bytes());
        hasher.update(&ptr_addr.to_ne_bytes());
        Self(hasher.digest().bytes())
    }
}

impl std::fmt::Display for Uid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, bytes) in self.0.as_chunks::<4>().0.iter().enumerate() {
            if i > 0 {
                write!(f, "-")?;
            }
            let dword = u32::from_be_bytes(*bytes);
            write!(f, "{dword:x}")?;
        }
        Ok(())
    }
}

impl PartialEq<[u8; 20]> for Uid {
    fn eq(&self, other: &[u8; 20]) -> bool {
        &self.0 == other
    }
}

impl PartialEq<Uid> for [u8; 20] {
    fn eq(&self, other: &Uid) -> bool {
        self == &other.0
    }
}

#[derive(
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    qi_macros::Valuable,
    serde::Serialize,
    serde::Deserialize,
    derive_more::Display,
    derive_more::From,
    derive_more::Into,
)]
#[serde(transparent)]
#[qi(value(crate = "crate::value", transparent))]
pub struct Id(pub u32);

#[derive(
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    qi_macros::Valuable,
    serde::Serialize,
    serde::Deserialize,
    derive_more::Display,
    derive_more::From,
    derive_more::Into,
)]
#[serde(transparent)]
#[qi(value(crate = "crate::value", transparent))]
pub struct ActionId(pub u32);

impl ActionId {
    pub fn wrapping_next(&mut self) -> Self {
        let old_id = self.0;
        self.0 = self.0.wrapping_add(1);
        Self(old_id)
    }
}

impl Iterator for ActionId {
    type Item = Self;

    fn next(&mut self) -> Option<Self::Item> {
        Some(self.wrapping_next())
    }
}

#[derive(
    Clone,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    qi_macros::Valuable,
    serde::Serialize,
    serde::Deserialize,
)]
#[qi(value(crate = "crate::value"))]
pub struct MetaObject {
    // Members are kept sorted by identifier: this is the order the reference implementation
    // (`std::map`) serializes them in.
    pub methods: BTreeMap<ActionId, MetaMethod>,
    pub signals: BTreeMap<ActionId, MetaSignal>,
    pub properties: BTreeMap<ActionId, MetaProperty>,
    pub description: String,
}

impl MetaObject {
    pub fn builder() -> MetaObjectBuilder {
        MetaObjectBuilder::new()
    }

    pub fn signal(&self, name_or_id: &ActionNameOrId) -> Option<&MetaSignal> {
        match name_or_id {
            ActionNameOrId::Id(id) => self.signals.get(id),
            ActionNameOrId::Name(name) => self.signals.values().find(|sig| &sig.name == name),
        }
    }

    pub fn property(&self, name_or_id: &ActionNameOrId) -> Option<&MetaProperty> {
        match name_or_id {
            ActionNameOrId::Id(id) => self.properties.get(id),
            ActionNameOrId::Name(name) => self.properties.values().find(|prop| &prop.name == name),
        }
    }

    pub fn method(&self, name_or_id: &ActionNameOrId) -> Option<&MetaMethod> {
        match name_or_id {
            ActionNameOrId::Id(id) => self.methods.get(id),
            ActionNameOrId::Name(name) => self.methods.values().find(|method| &method.name == name),
        }
    }
}

#[derive(Default, Debug)]
pub struct MetaObjectBuilder {
    meta_object: MetaObject,
}

impl MetaObjectBuilder {
    pub fn new() -> Self {
        Self {
            meta_object: Default::default(),
        }
    }

    pub fn add_method(&mut self, method: MetaMethod) -> &mut Self {
        self.meta_object.methods.insert(method.uid, method);
        self
    }

    pub fn add_signal(&mut self, signal: MetaSignal) -> &mut Self {
        self.meta_object.signals.insert(signal.uid, signal);
        self
    }

    pub fn add_property(&mut self, property: MetaProperty) -> &mut Self {
        let uid = property.uid;
        self.meta_object.properties.insert(uid, property.clone());
        // Properties are also signals of their changes. The signature of a signal is the tuple
        // of its parameters: a property of type `T` has the signal signature `(T)`.
        let signal_signature =
            Signature::new(Some(Type::tuple_of([property.signature.into_type()])));
        self.meta_object.signals.insert(
            uid,
            MetaSignal {
                uid,
                name: property.name,
                signature: signal_signature,
            },
        );
        self
    }

    pub fn build(self) -> MetaObject {
        self.meta_object
    }
}

#[derive(
    Clone,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    qi_macros::Valuable,
    serde::Serialize,
    serde::Deserialize,
)]
#[qi(value(crate = "crate::value", case = "camelCase"))]
pub struct MetaMethod {
    pub uid: ActionId,
    pub return_signature: Signature,
    pub name: String,
    pub parameters_signature: Signature,
    pub description: String,
    pub parameters: Vec<MetaMethodParameter>,
    pub return_description: String,
}

impl MetaMethod {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn builder<T: Into<ActionId>>(uid: T) -> MetaMethodBuilder {
        MetaMethodBuilder {
            uid: uid.into(),
            name: Default::default(),
            description: Default::default(),
            return_value: Default::default(),
            parameters: Default::default(),
        }
    }
}

#[derive(Debug)]
pub struct MetaMethodBuilder {
    uid: ActionId,
    name: String,
    description: String,
    return_value: MetaMethodBuilderReturnValue,
    parameters: Vec<MetaMethodBuilderParameter>,
}

impl MetaMethodBuilder {
    pub fn uid(&self) -> ActionId {
        self.uid
    }

    pub fn set_name<T: Into<String>>(&mut self, name: T) -> &mut Self {
        self.name = name.into();
        self
    }

    pub fn set_description<T: Into<String>>(&mut self, description: T) -> &mut Self {
        self.description = description.into();
        self
    }

    pub fn return_value(&mut self) -> &mut MetaMethodBuilderReturnValue {
        &mut self.return_value
    }

    pub fn parameter(&mut self, index: usize) -> &mut MetaMethodBuilderParameter {
        if self.parameters.len() <= index {
            self.parameters.resize_with(index + 1, Default::default);
        }
        &mut self.parameters[index]
    }

    pub fn build(self) -> MetaMethod {
        let (parameters, parameter_types): (Vec<MetaMethodParameter>, Vec<Option<Type>>) = self
            .parameters
            .into_iter()
            .map(|parameter| (parameter.parameter, parameter.ty))
            .unzip();
        // The reference implementation only lists the parameters that were documented (given a
        // name or a description). Parameters that were only typed are described by the parameters
        // signature alone.
        let parameters = if parameters
            .iter()
            .all(|parameter| parameter.name.is_empty() && parameter.description.is_empty())
        {
            Vec::new()
        } else {
            parameters
        };
        let parameters_tuple = ty::Type::Tuple(ty::Tuple::Tuple(parameter_types));
        let parameters_signature = Signature::new(Some(parameters_tuple));
        MetaMethod {
            uid: self.uid,
            return_signature: self.return_value.signature,
            name: self.name,
            parameters_signature,
            description: self.description,
            parameters,
            return_description: self.return_value.description,
        }
    }
}

#[derive(Debug)]
pub struct MetaMethodBuilderReturnValue {
    signature: Signature,
    description: String,
}

impl MetaMethodBuilderReturnValue {
    pub fn set_description<T: Into<String>>(&mut self, description: T) -> &mut Self {
        self.description = description.into();
        self
    }

    pub fn set_type<T: Into<Option<Type>>>(&mut self, ty: T) -> &mut Self {
        self.set_signature(Signature::new(ty.into()))
    }

    pub fn set_signature<T: Into<Signature>>(&mut self, signature: T) -> &mut Self {
        self.signature = signature.into();
        self
    }
}

impl Default for MetaMethodBuilderReturnValue {
    fn default() -> Self {
        Self {
            signature: Signature(Some(Type::Unit)),
            description: String::new(),
        }
    }
}

#[derive(Debug)]
pub struct MetaMethodBuilderParameter {
    parameter: MetaMethodParameter,
    ty: Option<Type>,
}

impl MetaMethodBuilderParameter {
    pub fn set_name<T: Into<String>>(&mut self, name: T) -> &mut Self {
        self.parameter.name = name.into();
        self
    }

    pub fn set_description<T: Into<String>>(&mut self, description: T) -> &mut Self {
        self.parameter.description = description.into();
        self
    }

    pub fn set_type<T: Into<Option<Type>>>(&mut self, ty: T) -> &mut Self {
        self.ty = ty.into();
        self
    }
}

impl Default for MetaMethodBuilderParameter {
    fn default() -> Self {
        Self {
            parameter: MetaMethodParameter::default(),
            ty: Some(Type::Unit),
        }
    }
}

#[derive(
    Clone,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    qi_macros::Valuable,
    serde::Serialize,
    serde::Deserialize,
)]
#[qi(value(crate = "crate::value"))]
pub struct MetaMethodParameter {
    pub name: String,
    pub description: String,
}

#[derive(
    Clone,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    qi_macros::Valuable,
    serde::Serialize,
    serde::Deserialize,
)]
#[qi(value(crate = "crate::value"))]
pub struct MetaSignal {
    pub uid: ActionId,
    pub name: String,
    pub signature: Signature,
}

#[derive(
    Clone,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    qi_macros::Valuable,
    serde::Serialize,
    serde::Deserialize,
)]
#[qi(value(crate = "crate::value"))]
pub struct MetaProperty {
    pub uid: ActionId,
    pub name: String,
    pub signature: Signature,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum ActionNameOrId {
    Name(String),
    Id(ActionId),
}

impl From<ActionId> for ActionNameOrId {
    fn from(value: ActionId) -> Self {
        Self::Id(value)
    }
}

impl From<String> for ActionNameOrId {
    fn from(value: String) -> Self {
        Self::Name(value)
    }
}

impl From<&str> for ActionNameOrId {
    fn from(value: &str) -> Self {
        Self::Name(value.to_owned())
    }
}

impl PartialEq<&str> for ActionNameOrId {
    fn eq(&self, other: &&str) -> bool {
        match self {
            Self::Name(name) => name == other,
            _ => false,
        }
    }
}

impl PartialEq<String> for ActionNameOrId {
    fn eq(&self, other: &String) -> bool {
        match self {
            Self::Name(name) => name == other,
            _ => false,
        }
    }
}

impl PartialEq<ActionId> for ActionNameOrId {
    fn eq(&self, other: &ActionId) -> bool {
        match self {
            Self::Id(id) => id == other,
            _ => false,
        }
    }
}

impl std::fmt::Display for ActionNameOrId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActionNameOrId::Id(id) => id.fmt(f),
            ActionNameOrId::Name(name) => name.fmt(f),
        }
    }
}

impl<'a> IntoValue<'a> for ActionNameOrId {
    fn into_value(self) -> Value<'a> {
        match self {
            ActionNameOrId::Id(id) => id.into_value(),
            ActionNameOrId::Name(name) => name.into_value(),
        }
    }
}

impl<'a> FromValue<'a> for ActionNameOrId {
    fn from_value(value: Value<'a>) -> Result<Self, FromValueError> {
        // TODO: IMPROVE: not ideal to clone the value here.
        if let Ok(id) = ActionId::from_value(value.clone()) {
            Ok(Self::Id(id))
        } else if let Ok(name) = String::from_value(value.clone()) {
            Ok(Self::Name(name))
        } else {
            Err(FromValueError::TypeMismatch {
                expected: "an object member identifier".to_owned(),
                actual: value.to_string(),
            })
        }
    }
}
