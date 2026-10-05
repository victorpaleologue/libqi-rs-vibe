//! The members shared by every bound object of the messaging protocol.
//!
//! Every object exposed through the messaging layer answers a set of *special* methods, with
//! identifiers below [`SPECIAL_MEMBER_MAX_ID`], that implement the protocol of events, properties
//! and meta object retrieval. They are part of the meta object seen by remote peers, in addition
//! to the members of the object itself.

use crate::value::{
    object::{ActionId, MetaMethod, MetaObject},
    Reflect, Type,
};
use once_cell::sync::Lazy;

/// Identifiers of special members are strictly below this value.
pub const SPECIAL_MEMBER_MAX_ID: ActionId = ActionId(100);

pub(crate) const REGISTER_EVENT: ActionId = ActionId(0);
pub(crate) const UNREGISTER_EVENT: ActionId = ActionId(1);
pub(crate) const META_OBJECT: ActionId = ActionId(2);
pub(crate) const TERMINATE: ActionId = ActionId(3);
// There is no special member 4: it is the identifier of the connection function of the messaging
// server object.
pub(crate) const PROPERTY: ActionId = ActionId(5);
pub(crate) const SET_PROPERTY: ActionId = ActionId(6);
pub(crate) const PROPERTIES: ActionId = ActionId(7);
pub(crate) const REGISTER_EVENT_WITH_SIGNATURE: ActionId = ActionId(8);

/// Returns true if the identifier is one of a special member.
pub fn is_special(id: ActionId) -> bool {
    id < SPECIAL_MEMBER_MAX_ID
}

/// The meta object of the special members of bound objects.
pub(crate) fn meta_object() -> &'static MetaObject {
    static META: Lazy<MetaObject> = Lazy::new(|| {
        let mut builder = MetaObject::builder();
        builder.add_method({
            let mut m = MetaMethod::builder(REGISTER_EVENT);
            m.set_name("registerEvent");
            m.parameter(0).set_type(Type::UInt32);
            m.parameter(1).set_type(Type::UInt32);
            m.parameter(2).set_type(Type::UInt64);
            m.return_value().set_type(Type::UInt64);
            m.build()
        });
        builder.add_method({
            let mut m = MetaMethod::builder(UNREGISTER_EVENT);
            m.set_name("unregisterEvent");
            m.parameter(0).set_type(Type::UInt32);
            m.parameter(1).set_type(Type::UInt32);
            m.parameter(2).set_type(Type::UInt64);
            m.build()
        });
        builder.add_method({
            let mut m = MetaMethod::builder(META_OBJECT);
            m.set_name("metaObject");
            m.parameter(0).set_type(Type::UInt32);
            m.return_value().set_type(MetaObject::ty());
            m.build()
        });
        builder.add_method({
            let mut m = MetaMethod::builder(TERMINATE);
            m.set_name("terminate");
            m.parameter(0).set_type(Type::UInt32);
            m.build()
        });
        builder.add_method({
            let mut m = MetaMethod::builder(PROPERTY);
            m.set_name("property");
            m.parameter(0).set_type(None);
            m.return_value().set_type(None);
            m.build()
        });
        builder.add_method({
            let mut m = MetaMethod::builder(SET_PROPERTY);
            m.set_name("setProperty");
            m.parameter(0).set_type(None);
            m.parameter(1).set_type(None);
            m.build()
        });
        builder.add_method({
            let mut m = MetaMethod::builder(PROPERTIES);
            m.set_name("properties");
            m.return_value().set_type(Type::list_of(Type::String));
            m.build()
        });
        builder.add_method({
            let mut m = MetaMethod::builder(REGISTER_EVENT_WITH_SIGNATURE);
            m.set_name("registerEventWithSignature");
            m.parameter(0).set_type(Type::UInt32);
            m.parameter(1).set_type(Type::UInt32);
            m.parameter(2).set_type(Type::UInt64);
            m.parameter(3).set_type(Type::String);
            m.return_value().set_type(Type::UInt64);
            m.build()
        });
        builder.build()
    });
    &META
}

/// Merges the meta object of an object with the special members, producing the meta object seen
/// by remote peers.
pub(crate) fn merge(meta: &MetaObject) -> MetaObject {
    let generic = meta_object();
    let mut merged = meta.clone();
    for (id, method) in generic.methods.iter() {
        if !merged.methods.contains_key(id) {
            merged.methods.insert(*id, method.clone());
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_members_signatures() {
        let meta = meta_object();
        let sig = |id: ActionId| {
            let m = &meta.methods[&id];
            (
                m.name.clone(),
                m.parameters_signature.to_string(),
                m.return_signature.to_string(),
            )
        };
        assert_eq!(
            sig(REGISTER_EVENT),
            ("registerEvent".into(), "(IIL)".into(), "L".into())
        );
        assert_eq!(
            sig(UNREGISTER_EVENT),
            ("unregisterEvent".into(), "(IIL)".into(), "v".into())
        );
        assert_eq!(sig(META_OBJECT).0, "metaObject");
        assert_eq!(sig(META_OBJECT).1, "(I)");
        assert!(sig(META_OBJECT).2.starts_with("({I(Issss[(ss)<MetaMethodParameter,name,description>]s)<MetaMethod,uid,returnSignature,name,parametersSignature,description,parameters,returnDescription>}{I(Iss)<MetaSignal,uid,name,signature>}{I(Iss)<MetaProperty,uid,name,signature>}s)<MetaObject,methods,signals,properties,description>"), "{}", sig(META_OBJECT).2);
        assert_eq!(
            sig(TERMINATE),
            ("terminate".into(), "(I)".into(), "v".into())
        );
        assert_eq!(sig(PROPERTY), ("property".into(), "(m)".into(), "m".into()));
        assert_eq!(
            sig(SET_PROPERTY),
            ("setProperty".into(), "(mm)".into(), "v".into())
        );
        assert_eq!(
            sig(PROPERTIES),
            ("properties".into(), "()".into(), "[s]".into())
        );
        assert_eq!(
            sig(REGISTER_EVENT_WITH_SIGNATURE),
            (
                "registerEventWithSignature".into(),
                "(IILs)".into(),
                "L".into()
            )
        );
    }
}
