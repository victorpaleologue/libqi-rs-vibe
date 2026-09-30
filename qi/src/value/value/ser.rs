use serde::Serialize;

use super::{reduce_element_types, Value};
use crate::value::dynamic;

/// An element of a container, written as a dynamic value (with its signature) when the
/// container is dynamically typed and the element is not already a dynamic value.
struct Element<'e, 'v> {
    value: &'e Value<'v>,
    dynamic: bool,
}

impl serde::Serialize for Element<'_, '_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self.value {
            Value::Dynamic(_) => self.value.serialize(serializer),
            value if self.dynamic => dynamic::serialize(value, serializer),
            value => value.serialize(serializer),
        }
    }
}

impl serde::Serialize for Value<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Value::Unit => ().serialize(serializer),
            Value::Bool(v) => v.serialize(serializer),
            Value::Int8(v) => v.serialize(serializer),
            Value::UInt8(v) => v.serialize(serializer),
            Value::Int16(v) => v.serialize(serializer),
            Value::UInt16(v) => v.serialize(serializer),
            Value::Int32(v) => v.serialize(serializer),
            Value::UInt32(v) => v.serialize(serializer),
            Value::Int64(v) => v.serialize(serializer),
            Value::UInt64(v) => v.serialize(serializer),
            Value::Float32(v) => v.serialize(serializer),
            Value::Float64(v) => v.serialize(serializer),
            Value::String(v) => v.serialize(serializer),
            Value::Raw(v) => serializer.serialize_bytes(v),
            Value::Option(opt) => opt.serialize(serializer),
            Value::List(list) => {
                use serde::ser::SerializeSeq;
                // Elements of a dynamically typed list are each written with their signature.
                let dynamic = reduce_element_types(list.iter()).is_none();
                let mut seq = serializer.serialize_seq(Some(list.len()))?;
                for element in list {
                    seq.serialize_element(&Element {
                        value: element,
                        dynamic,
                    })?;
                }
                seq.end()
            }
            Value::Map(map) => {
                use serde::ser::SerializeMap;
                let dynamic_keys = reduce_element_types(map.keys()).is_none();
                let dynamic_values = reduce_element_types(map.values()).is_none();
                let mut entries = serializer.serialize_map(Some(map.len()))?;
                for (key, value) in map {
                    entries.serialize_entry(
                        &Element {
                            value: key,
                            dynamic: dynamic_keys,
                        },
                        &Element {
                            value,
                            dynamic: dynamic_values,
                        },
                    )?;
                }
                entries.end()
            }
            Value::Tuple(elements) => {
                use serde::ser::SerializeTuple;
                let mut serializer = serializer.serialize_tuple(elements.len())?;
                for element in elements {
                    serializer.serialize_element(&element)?;
                }
                serializer.end()
            }
            Value::Object(obj) => obj.serialize(serializer),
            Value::Dynamic(val) => dynamic::serialize(val, serializer),
        }
    }
}

impl serde_with::SerializeAs<Value<'static>> for Value<'_> {
    fn serialize_as<S>(source: &Value<'static>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        source.serialize(serializer)
    }
}
