//! Services: named objects published on a space.
//!
//! A service is the main object of a node, registered under a name to the service directory of a
//! space, and described by a service [`Info`].

use crate::{
    node, object,
    object::AnyObject,
    session,
    value::{self, os},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
pub use value::service::*;

/// The service identifier of services that are not registered yet.
const UNSPECIFIED_ID: Id = Id(0);

/// The information describing a service in a service directory.
///
/// The object UID field was added by `libqi` 2.9: the service directories of older robots
/// (NAOqi 2.1 to 2.8) write the six other fields only, which the conversion from values accepts.
#[derive(
    Default,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    qi_macros::Reflect,
    qi_macros::ToValue,
    qi_macros::IntoValue,
)]
#[qi(value(crate = "crate::value", case = "camelCase", name = "ServiceInfo"))]
pub struct Info {
    pub(super) name: String,
    #[qi(value(name = "serviceId"))]
    pub(super) id: Id,
    pub(super) machine_id: os::MachineId,
    pub(super) process_id: u32,
    pub(super) endpoints: Vec<session::Target>,
    #[qi(value(name = "sessionId"))]
    pub(super) node_uid: node::Uid,
    /// Object uid in service info are represented as strings containing pure binary data for
    /// compatibility reasons. They are therefore NOT UTF-8 valid strings or even contain printable
    /// characters. An empty string means the object UID is unknown.
    pub(super) object_uid: ObjectUidAsStr,
}

impl Info {
    pub(super) fn unregistered(
        name: String,
        endpoints: Vec<session::Target>,
        node_uid: node::Uid,
        object_uid: object::Uid,
    ) -> Self {
        Self::process_local(name, UNSPECIFIED_ID, endpoints, node_uid, Some(object_uid))
    }

    pub(super) fn process_local(
        name: String,
        id: Id,
        endpoints: Vec<session::Target>,
        node_uid: node::Uid,
        object_uid: Option<object::Uid>,
    ) -> Self {
        Self {
            name,
            id,
            machine_id: os::MachineId::local(),
            process_id: std::process::id(),
            endpoints,
            node_uid,
            object_uid: ObjectUidAsStr(object_uid),
        }
    }

    /// The name of the service.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The identifier of the service in its space.
    pub fn id(&self) -> Id {
        self.id
    }

    /// The identifier of the machine that hosts the service.
    pub fn machine_id(&self) -> os::MachineId {
        self.machine_id
    }

    /// The identifier of the process that hosts the service.
    pub fn process_id(&self) -> u32 {
        self.process_id
    }

    /// The endpoints the service can be reached at.
    pub fn endpoints(&self) -> &[session::Target] {
        &self.endpoints
    }

    /// The identifier of the node that hosts the service.
    pub fn node_uid(&self) -> node::Uid {
        self.node_uid.clone()
    }

    /// The UID of the main object of the service, if known.
    pub fn object_uid(&self) -> Option<object::Uid> {
        self.object_uid.0
    }

    /// The type of service infos without the object UID field, as peers older than `libqi` 2.9
    /// know them.
    pub(crate) fn ty_without_object_uid() -> value::Type {
        let Some(value::Type::Tuple(value::ty::Tuple::Struct { name, mut fields })) =
            <Self as value::Reflect>::ty()
        else {
            unreachable!("a service info is a structure");
        };
        fields.pop();
        value::Type::Tuple(value::ty::Tuple::Struct { name, fields })
    }

    /// Converts into a value without the object UID field, for peers older than `libqi` 2.9.
    pub(crate) fn into_value_without_object_uid(self) -> value::Value<'static> {
        let mut value = value::IntoValue::into_value(self);
        if let value::Value::Tuple(fields) = &mut value {
            fields.truncate(6);
        }
        value
    }
}

impl std::fmt::Display for Info {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Info {
            name,
            id: service_id,
            machine_id,
            process_id,
            endpoints,
            node_uid,
            object_uid,
        } = self;
        write!(
            f,
            "{name}({service_id}, machine={machine_id}, \
                process={process_id}, \
                endpoints=["
        )?;
        for (index, endpoint) in endpoints.iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            endpoint.fmt(f)?;
        }
        write!(f, "], node={node_uid}, object={object_uid})")
    }
}

impl<'a> value::FromValue<'a> for Info {
    fn from_value(v: value::Value<'a>) -> Result<Self, value::FromValueError> {
        let mismatch = |v: &value::Value<'_>| value::FromValueError::TypeMismatch {
            expected: "ServiceInfo".to_owned(),
            actual: v.to_string(),
        };
        let value::Value::Tuple(mut fields) = v else {
            return Err(mismatch(&v));
        };
        match fields.len() {
            // Before libqi 2.9, there was no object UID.
            6 => fields.push(value::Value::String(String::new().into())),
            7 => {}
            _ => return Err(mismatch(&value::Value::Tuple(fields))),
        }
        let mut fields = fields.into_iter();
        let mut next = || fields.next().expect("seven fields");
        Ok(Self {
            name: next().cast_into()?,
            id: next().cast_into()?,
            machine_id: next().cast_into()?,
            process_id: next().cast_into()?,
            endpoints: next().cast_into()?,
            node_uid: next().cast_into()?,
            object_uid: next().cast_into()?,
        })
    }
}

/// An optional object UID represented as a string of raw bytes.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct ObjectUidAsStr(pub Option<object::Uid>);

impl std::fmt::Display for ObjectUidAsStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(uid) => uid.fmt(f),
            None => f.write_str("none"),
        }
    }
}

impl value::Reflect for ObjectUidAsStr {
    fn ty() -> Option<value::Type> {
        Some(value::Type::String)
    }
}

impl value::RuntimeReflect for ObjectUidAsStr {
    fn ty(&self) -> value::Type {
        value::Type::String
    }
}

impl value::ToValue for ObjectUidAsStr {
    fn to_value(&self) -> value::Value<'_> {
        match &self.0 {
            Some(uid) => value::String::from_maybe_utf8(uid.bytes()).into(),
            None => value::String::Borrowed("").into(),
        }
    }
}

impl<'a> value::IntoValue<'a> for ObjectUidAsStr {
    fn into_value(self) -> value::Value<'a> {
        match self.0 {
            Some(uid) => value::String::from_maybe_utf8_owned(uid.bytes().to_vec()).into(),
            None => value::String::Borrowed("").into(),
        }
    }
}

impl<'a> value::FromValue<'a> for ObjectUidAsStr {
    fn from_value(value: value::Value<'a>) -> std::result::Result<Self, value::FromValueError> {
        use value::RuntimeReflect;
        let value_type = value.ty();
        let value_str = value
            .into_string()
            .ok_or_else(|| value::FromValueError::TypeMismatch {
                expected: "an Object UID".to_owned(),
                actual: value_type.to_string(),
            })?;
        let bytes = value_str.as_bytes();
        if bytes.is_empty() {
            return Ok(Self(None));
        }
        let bytes =
            <[u8; 20]>::try_from(bytes).map_err(|err| value::FromValueError::Other(err.into()))?;
        Ok(Self(Some(bytes.into())))
    }
}

struct Service {
    info: Info,
    object: AnyObject,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service")
            .field("info", &self.info)
            .field("object_uid", &self.object.uid())
            .finish()
    }
}

/// The services registered on a node, indexed by identifier.
#[derive(Debug, Default)]
pub(super) struct Services(HashMap<Id, Service>);

impl Services {
    pub(super) fn info_mut(&mut self) -> impl Iterator<Item = &mut Info> {
        self.0.values_mut().map(|service| &mut service.info)
    }

    pub(super) fn infos(&self) -> impl Iterator<Item = &Info> {
        self.0.values().map(|service| &service.info)
    }
}

/// The shared, thread-safe registry of the services of a node.
#[derive(Default, Clone, Debug)]
pub(crate) struct SharedServices {
    services: Arc<Mutex<Services>>,
}

impl SharedServices {
    fn lock(&self) -> std::sync::MutexGuard<'_, Services> {
        self.services.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// Adds a service, making its main object reachable under its identifier.
    pub(crate) fn add(&self, info: Info, object: AnyObject) {
        self.lock().0.insert(info.id(), Service { info, object });
    }

    /// Removes a service, returning its info if it was registered.
    pub(crate) fn remove(&self, id: Id) -> Option<Info> {
        self.lock().0.remove(&id).map(|service| service.info)
    }

    /// The main object of a service.
    pub(crate) fn object(&self, id: Id) -> Option<AnyObject> {
        self.lock().0.get(&id).map(|service| service.object.clone())
    }

    /// Finds a service by name.
    pub(crate) fn find(&self, name: &str) -> Option<(Info, AnyObject)> {
        self.lock()
            .0
            .values()
            .find(|service| service.info.name == name)
            .map(|service| (service.info.clone(), service.object.clone()))
    }

    /// Updates the endpoints of every service, returning the updated infos.
    pub(crate) fn set_endpoints(&self, endpoints: &[session::Target]) -> Vec<Info> {
        let mut services = self.lock();
        for info in services.info_mut() {
            info.endpoints = endpoints.to_vec();
        }
        services.infos().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{messaging::Address, value::FormatInto, value::IntoFormat};
    use std::net::{Ipv4Addr, SocketAddr};

    #[test]
    fn service_info_from_format_value() {
        let value_in = &[
            0x0a, 0x00, 0x00, 0x00, 0x43, 0x61, 0x6c, 0x63, 0x75, 0x6c, 0x61, 0x74, 0x6f, 0x72,
            0x02, 0x00, 0x00, 0x00, 0x24, 0x00, 0x00, 0x00, 0x39, 0x61, 0x36, 0x35, 0x62, 0x35,
            0x36, 0x65, 0x2d, 0x63, 0x33, 0x64, 0x33, 0x2d, 0x34, 0x34, 0x38, 0x35, 0x2d, 0x38,
            0x39, 0x32, 0x34, 0x2d, 0x36, 0x36, 0x31, 0x62, 0x30, 0x33, 0x36, 0x32, 0x30, 0x32,
            0x62, 0x33, 0x46, 0x31, 0x34, 0x00, 0x02, 0x00, 0x00, 0x00, 0x0d, 0x00, 0x00, 0x00,
            0x71, 0x69, 0x3a, 0x43, 0x61, 0x6c, 0x63, 0x75, 0x6c, 0x61, 0x74, 0x6f, 0x72, 0x15,
            0x00, 0x00, 0x00, 0x74, 0x63, 0x70, 0x3a, 0x2f, 0x2f, 0x31, 0x32, 0x37, 0x2e, 0x30,
            0x2e, 0x30, 0x2e, 0x31, 0x3a, 0x34, 0x31, 0x36, 0x38, 0x31, 0x24, 0x00, 0x00, 0x00,
            0x33, 0x36, 0x31, 0x65, 0x63, 0x65, 0x63, 0x34, 0x2d, 0x30, 0x30, 0x66, 0x37, 0x2d,
            0x34, 0x63, 0x39, 0x34, 0x2d, 0x61, 0x36, 0x65, 0x32, 0x2d, 0x64, 0x39, 0x31, 0x65,
            0x32, 0x38, 0x63, 0x35, 0x61, 0x30, 0x36, 0x63, 0x14, 0x00, 0x00, 0x00, 0xfd, 0xeb,
            0xc1, 0x2e, 0xcb, 0xea, 0x6b, 0x58, 0xcc, 0x42, 0x20, 0xb7, 0x33, 0x3d, 0xc4, 0xe1,
            0x0d, 0x8a, 0xd6, 0x16,
        ][..];
        let service_info: Info = value_in.to_reflect_value().unwrap();
        assert_eq!(
            service_info,
            Info {
                name: "Calculator".to_owned(),
                id: Id(2),
                machine_id: "9a65b56e-c3d3-4485-8924-661b036202b3".parse().unwrap(),
                process_id: 3420486,
                endpoints: vec![
                    session::Target::service("Calculator"),
                    session::Target::from(Address::Tcp {
                        address: SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 41681),
                        ssl: None
                    })
                ],
                node_uid: node::Uid::from_string("361ecec4-00f7-4c94-a6e2-d91e28c5a06c".to_owned()),
                object_uid: ObjectUidAsStr(Some(object::Uid::from([
                    0xfd, 0xeb, 0xc1, 0x2e, 0xcb, 0xea, 0x6b, 0x58, 0xcc, 0x42, 0x20, 0xb7, 0x33,
                    0x3d, 0xc4, 0xe1, 0x0d, 0x8a, 0xd6, 0x16
                ])))
            }
        );
        // Round trip.
        assert_eq!(service_info.into_format().unwrap(), value_in);
    }

    /// Service directories of NAOqi 2.1 to 2.8 write no object UID.
    #[test]
    fn service_info_without_object_uid_converts() {
        let value = value::Value::Tuple(vec![
            value::Value::String("ALMemory".into()),
            value::Value::UInt32(3),
            value::Value::String("9a65b56e-c3d3-4485-8924-661b036202b3".into()),
            value::Value::UInt32(1234),
            value::Value::List(vec![value::Value::String("tcp://127.0.0.1:9559".into())]),
            value::Value::String("361ecec4-00f7-4c94-a6e2-d91e28c5a06c".into()),
        ]);
        let info = <Info as value::FromValue>::from_value(value).unwrap();
        assert_eq!(info.name(), "ALMemory");
        assert_eq!(info.id(), Id(3));
        assert_eq!(info.process_id(), 1234);
        assert_eq!(info.endpoints().len(), 1);
        assert_eq!(info.object_uid(), None);
        // Five fields are not a service info.
        let value = value::Value::Tuple(vec![value::Value::Unit; 5]);
        assert!(<Info as value::FromValue>::from_value(value).is_err());
        // The legacy type and value have six fields and round trip.
        assert_eq!(
            value::Signature::from(Info::ty_without_object_uid()).to_string(),
            "(sIsI[s]s)<ServiceInfo,name,serviceId,machineId,processId,endpoints,sessionId>"
        );
        let legacy_value = info.clone().into_value_without_object_uid();
        assert!(matches!(&legacy_value, value::Value::Tuple(fields) if fields.len() == 6));
        assert_eq!(
            <Info as value::FromValue>::from_value(legacy_value).unwrap(),
            info
        );
    }

    #[test]
    fn object_uid_from_to_format() {
        let value_in = &[
            0x14, 0x00, 0x00, 0x00, 0xfd, 0xeb, 0xc1, 0x2e, 0xcb, 0xea, 0x6b, 0x58, 0xcc, 0x42,
            0x20, 0xb7, 0x33, 0x3d, 0xc4, 0xe1, 0x0d, 0x8a, 0xd6, 0x16,
        ][..];
        let object_uid: ObjectUidAsStr = value_in.to_reflect_value().unwrap();
        assert_eq!(
            object_uid.0.unwrap(),
            [
                0xfd, 0xeb, 0xc1, 0x2e, 0xcb, 0xea, 0x6b, 0x58, 0xcc, 0x42, 0x20, 0xb7, 0x33, 0x3d,
                0xc4, 0xe1, 0x0d, 0x8a, 0xd6, 0x16
            ]
        );
        let value_out = object_uid.into_format().unwrap();
        assert_eq!(value_out, value_in);

        // An absent UID is an empty string.
        let none: ObjectUidAsStr = (&[0u8, 0, 0, 0][..]).to_reflect_value().unwrap();
        assert_eq!(none.0, None);
    }
}
