//! Helpers over meta objects: resolution of members and human-readable signatures.

use qi::{
    object::{is_special, ActionId, MetaMethod, MetaObject, MetaProperty, MetaSignal},
    value::{ty::Tuple, Signature, Type},
};

/// Splits a `SERVICE.MEMBER` target at its last dot.
pub(crate) fn split_target(target: &str) -> Result<(&str, &str), TargetError> {
    match target.rsplit_once('.') {
        Some((service, member)) if !service.is_empty() && !member.is_empty() => {
            Ok((service, member))
        }
        _ => Err(TargetError(target.to_owned())),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("expected a SERVICE.MEMBER target, found \"{0}\"")]
pub(crate) struct TargetError(String);

/// The methods with the given name, sorted by identifier. Methods may be overloaded.
pub(crate) fn methods_named<'m>(meta: &'m MetaObject, name: &str) -> Vec<&'m MetaMethod> {
    let mut methods: Vec<_> = meta
        .methods
        .values()
        .filter(|method| method.name == name)
        .collect();
    methods.sort_by_key(|method| method.uid);
    methods
}

pub(crate) fn signal_named<'m>(meta: &'m MetaObject, name: &str) -> Option<&'m MetaSignal> {
    meta.signals.values().find(|signal| signal.name == name)
}

pub(crate) fn property_named<'m>(meta: &'m MetaObject, name: &str) -> Option<&'m MetaProperty> {
    meta.properties
        .values()
        .find(|property| property.name == name)
}

/// The types of the elements of a parameters signature, which is a tuple.
pub(crate) fn tuple_types(signature: &Signature) -> Vec<Option<Type>> {
    match signature.as_type() {
        Some(Type::Tuple(tuple)) => tuple.element_types(),
        other => vec![other.cloned()],
    }
}

/// Hidden members are conventionally named with a leading underscore.
pub(crate) fn is_hidden(name: &str) -> bool {
    name.starts_with('_')
}

/// The selection of the members of a meta object to show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MemberFilter {
    /// Show hidden members.
    pub(crate) hidden: bool,
    /// Show the special members of the messaging protocol.
    pub(crate) details: bool,
}

impl MemberFilter {
    pub(crate) fn shows(&self, uid: ActionId, name: &str) -> bool {
        (self.hidden || !is_hidden(name)) && (self.details || !is_special(uid))
    }

    /// The methods to show, sorted by identifier.
    pub(crate) fn methods<'m>(&self, meta: &'m MetaObject) -> Vec<&'m MetaMethod> {
        let mut methods: Vec<_> = meta
            .methods
            .values()
            .filter(|method| self.shows(method.uid, &method.name))
            .collect();
        methods.sort_by_key(|method| method.uid);
        methods
    }

    /// The signals to show, sorted by identifier. Properties are not repeated as signals.
    pub(crate) fn signals<'m>(&self, meta: &'m MetaObject) -> Vec<&'m MetaSignal> {
        let mut signals: Vec<_> = meta
            .signals
            .values()
            .filter(|signal| {
                self.shows(signal.uid, &signal.name) && !meta.properties.contains_key(&signal.uid)
            })
            .collect();
        signals.sort_by_key(|signal| signal.uid);
        signals
    }

    /// The properties to show, sorted by identifier.
    pub(crate) fn properties<'m>(&self, meta: &'m MetaObject) -> Vec<&'m MetaProperty> {
        let mut properties: Vec<_> = meta
            .properties
            .values()
            .filter(|property| self.shows(property.uid, &property.name))
            .collect();
        properties.sort_by_key(|property| property.uid);
        properties
    }
}

/// The name of a type, in the style of the pretty signatures of the reference implementation.
pub(crate) fn pretty_type(ty: Option<&Type>) -> String {
    let Some(ty) = ty else {
        return "Value".to_owned();
    };
    match ty {
        Type::Unit => "Void".to_owned(),
        Type::Bool => "Bool".to_owned(),
        Type::Int8 => "Int8".to_owned(),
        Type::UInt8 => "UInt8".to_owned(),
        Type::Int16 => "Int16".to_owned(),
        Type::UInt16 => "UInt16".to_owned(),
        Type::Int32 => "Int32".to_owned(),
        Type::UInt32 => "UInt32".to_owned(),
        Type::Int64 => "Int64".to_owned(),
        Type::UInt64 => "UInt64".to_owned(),
        Type::Float32 => "Float".to_owned(),
        Type::Float64 => "Double".to_owned(),
        Type::String => "String".to_owned(),
        Type::Raw => "Raw".to_owned(),
        Type::Object => "Object".to_owned(),
        Type::Option(element) => format!("Optional<{}>", pretty_type(element.as_deref())),
        Type::List(element) => format!("List<{}>", pretty_type(element.as_deref())),
        Type::VarArgs(element) => format!("VarArgs<{}>", pretty_type(element.as_deref())),
        Type::Map { key, value } => format!(
            "Map<{},{}>",
            pretty_type(key.as_deref()),
            pretty_type(value.as_deref())
        ),
        Type::Tuple(Tuple::Tuple(elements)) => pretty_tuple(elements),
        Type::Tuple(tuple @ (Tuple::TupleStruct { name, .. } | Tuple::Struct { name, .. })) => {
            if name.is_empty() {
                pretty_tuple(&tuple.element_types())
            } else {
                name.clone()
            }
        }
    }
}

/// The names of the types of a tuple, e.g. `(Int32,String)`.
pub(crate) fn pretty_tuple(types: &[Option<Type>]) -> String {
    let names: Vec<_> = types.iter().map(|ty| pretty_type(ty.as_ref())).collect();
    format!("({})", names.join(","))
}

/// The signature of a method, e.g. `add::(Int32,Int32)->Int32`.
pub(crate) fn method_signature(method: &MetaMethod) -> String {
    format!(
        "{}::{}->{}",
        method.name,
        pretty_tuple(&tuple_types(&method.parameters_signature)),
        pretty_type(method.return_signature.as_type())
    )
}

/// The raw signature of a method, e.g. `(ii)->i`.
pub(crate) fn method_raw_signature(method: &MetaMethod) -> String {
    format!(
        "{}->{}",
        method.parameters_signature, method.return_signature
    )
}

/// The signature of a signal, e.g. `fired::(Int32)`.
pub(crate) fn signal_signature(signal: &MetaSignal) -> String {
    format!(
        "{}::{}",
        signal.name,
        pretty_tuple(&tuple_types(&signal.signature))
    )
}

/// The signature of a property, e.g. `value::Int32`.
pub(crate) fn property_signature(property: &MetaProperty) -> String {
    format!(
        "{}::{}",
        property.name,
        pretty_type(property.signature.as_type())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use qi::object::SPECIAL_MEMBER_MAX_ID;

    fn calculator() -> MetaObject {
        let mut builder = MetaObject::builder();
        let mut id = SPECIAL_MEMBER_MAX_ID;
        builder.add_method({
            let mut method = MetaMethod::builder(id.next().unwrap());
            method.set_name("add");
            method.parameter(0).set_type(Type::Int32);
            method.parameter(1).set_type(Type::Int32);
            method.return_value().set_type(Type::Int32);
            method.build()
        });
        // An overload.
        builder.add_method({
            let mut method = MetaMethod::builder(id.next().unwrap());
            method.set_name("add");
            method.parameter(0).set_type(Type::Float64);
            method.return_value().set_type(Type::Float64);
            method.build()
        });
        builder.add_method({
            let mut method = MetaMethod::builder(id.next().unwrap());
            method.set_name("_reset");
            method.build()
        });
        builder.add_method({
            let mut method = MetaMethod::builder(ActionId(2));
            method.set_name("metaObject");
            method.parameter(0).set_type(Type::UInt32);
            method.build()
        });
        builder.add_signal(MetaSignal {
            uid: id.next().unwrap(),
            name: "fired".to_owned(),
            signature: Type::tuple_of([Type::Int32]).into(),
        });
        builder.add_property(MetaProperty {
            uid: id.next().unwrap(),
            name: "value".to_owned(),
            signature: Type::Int32.into(),
        });
        builder.build()
    }

    #[test]
    fn targets_split_at_the_last_dot() {
        assert_eq!(
            split_target("Calculator.add").unwrap(),
            ("Calculator", "add")
        );
        assert_eq!(
            split_target("com.example.Svc.add").unwrap(),
            ("com.example.Svc", "add")
        );
        assert!(split_target("Calculator").is_err());
        assert!(split_target("Calculator.").is_err());
        assert!(split_target(".add").is_err());
        assert_eq!(
            split_target("nope").unwrap_err().to_string(),
            "expected a SERVICE.MEMBER target, found \"nope\""
        );
    }

    #[test]
    fn members_are_found_by_name() {
        let meta = calculator();
        let adds = methods_named(&meta, "add");
        assert_eq!(adds.len(), 2);
        assert_eq!(adds[0].uid, ActionId(100));
        assert_eq!(adds[1].uid, ActionId(101));
        assert!(methods_named(&meta, "nope").is_empty());
        assert_eq!(signal_named(&meta, "fired").unwrap().uid, ActionId(103));
        // Properties are signals too.
        assert_eq!(signal_named(&meta, "value").unwrap().uid, ActionId(104));
        assert_eq!(property_named(&meta, "value").unwrap().uid, ActionId(104));
        assert!(property_named(&meta, "fired").is_none());
    }

    #[test]
    fn filters_select_members() {
        let meta = calculator();
        let names = |methods: Vec<&MetaMethod>| -> Vec<String> {
            methods.into_iter().map(|m| m.name.clone()).collect()
        };
        let filter = MemberFilter::default();
        assert_eq!(names(filter.methods(&meta)), ["add", "add"]);
        assert_eq!(filter.signals(&meta).len(), 1);
        assert_eq!(filter.properties(&meta).len(), 1);
        let filter = MemberFilter {
            hidden: true,
            details: false,
        };
        assert_eq!(names(filter.methods(&meta)), ["add", "add", "_reset"]);
        let filter = MemberFilter {
            hidden: true,
            details: true,
        };
        assert_eq!(
            names(filter.methods(&meta)),
            ["metaObject", "add", "add", "_reset"]
        );
        assert!(is_hidden("_x"));
        assert!(!is_hidden("x"));
    }

    #[test]
    fn signatures_are_pretty() {
        let meta = calculator();
        let adds = methods_named(&meta, "add");
        assert_eq!(method_signature(adds[0]), "add::(Int32,Int32)->Int32");
        assert_eq!(method_raw_signature(adds[0]), "(ii)->i");
        assert_eq!(method_signature(adds[1]), "add::(Double)->Double");
        assert_eq!(
            method_signature(methods_named(&meta, "_reset")[0]),
            "_reset::()->Void"
        );
        assert_eq!(
            signal_signature(signal_named(&meta, "fired").unwrap()),
            "fired::(Int32)"
        );
        assert_eq!(
            property_signature(property_named(&meta, "value").unwrap()),
            "value::Int32"
        );
    }

    #[test]
    fn types_are_pretty() {
        assert_eq!(pretty_type(None), "Value");
        assert_eq!(pretty_type(Some(&Type::Float32)), "Float");
        assert_eq!(pretty_type(Some(&Type::Float64)), "Double");
        assert_eq!(
            pretty_type(Some(&Type::list_of(Type::option_of(Type::String)))),
            "List<Optional<String>>"
        );
        assert_eq!(
            pretty_type(Some(&Type::map_of(Type::String, None))),
            "Map<String,Value>"
        );
        assert_eq!(
            pretty_type(Some(&Type::varargs_of(Type::Raw))),
            "VarArgs<Raw>"
        );
        assert_eq!(
            pretty_type(Some(&Type::tuple_of([Some(Type::Int32), None]))),
            "(Int32,Value)"
        );
        assert_eq!(
            pretty_type(Some(&Type::struct_of("Point", [("x", Type::Int32)]))),
            "Point"
        );
        assert_eq!(
            pretty_type(Some(&Type::tuple_struct_of(
                "Pair",
                [Type::Int32, Type::Int32]
            ))),
            "Pair"
        );
        assert_eq!(
            pretty_type(Some(&Type::struct_of("", [("x", Type::Int32)]))),
            "(Int32)"
        );
        assert_eq!(
            tuple_types(&Signature::from(Type::Int32)),
            [Some(Type::Int32)]
        );
        assert_eq!(
            tuple_types(&Signature::from(Type::tuple_of([Type::Int32, Type::Bool]))),
            [Some(Type::Int32), Some(Type::Bool)]
        );
    }
}
