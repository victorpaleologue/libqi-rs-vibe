mod convert;
pub mod de;
mod impls;
mod ser;
mod string;

pub use self::string::String;
use crate::value::{map::Map, reflect::RuntimeReflect, ty, Object, Type};
use ordered_float::OrderedFloat;
use std::{borrow::Cow, string::String as StdString};

/// The [`Value`] structure represents any value of the `qi` type system.
/// TODO: Should this be just a trait ?
#[derive(Default, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Value<'a> {
    #[default]
    Unit,
    Bool(bool),
    Int8(i8),
    UInt8(u8),
    Int16(i16),
    UInt16(u16),
    Int32(i32),
    UInt32(u32),
    Int64(i64),
    UInt64(u64),
    Float32(OrderedFloat<f32>),
    Float64(OrderedFloat<f64>),
    String(String<'a>),
    Raw(Cow<'a, [u8]>),
    Option(Option<Box<Value<'a>>>),
    List(Vec<Value<'a>>),
    Map(Map<Value<'a>, Value<'a>>),
    Tuple(Vec<Value<'a>>),
    Object(Box<Object>),
    Dynamic(Box<Value<'a>>),
}

impl<'a> Value<'a> {
    pub fn new<V>(value: V) -> Self
    where
        V: IntoValue<'a>,
    {
        value.into_value()
    }

    pub fn cast_into<T>(self) -> Result<T, FromValueError>
    where
        T: FromValue<'a>,
    {
        T::from_value(self)
    }

    pub fn as_string(&self) -> Option<&String<'a>> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn into_string(self) -> Option<String<'a>> {
        match self {
            Self::String(str) => Some(str),
            _ => None,
        }
    }

    pub fn into_dynamic(self) -> Self {
        Self::Dynamic(Box::new(self))
    }

    /// Converts the value to the given type, `None` standing for the dynamic type.
    ///
    /// The rules follow the reference implementation:
    ///
    /// - a dynamic target wraps the value into a dynamic value, unless it is one already;
    /// - a dynamic value is unwrapped, and its content converted to the target;
    /// - numbers convert to any numeric type in which they are representable;
    /// - lists and tuples convert elementwise to lists, and to tuples of the same length; maps
    ///   convert elementwise; optionals convert elementwise, and a value converts to an optional
    ///   holding it;
    /// - any other value must already be of the target type.
    ///
    /// # Errors
    ///
    /// Returns a [`FromValueError::TypeMismatch`] when the value cannot be converted.
    pub fn convert_to(self, ty: Option<&Type>) -> Result<Value<'a>, FromValueError> {
        convert::convert_to(self, ty)
    }

    pub fn into_owned(self) -> Value<'static> {
        match self {
            Self::Unit => Value::Unit,
            Self::Bool(v) => Value::Bool(v),
            Self::Int8(v) => Value::Int8(v),
            Self::UInt8(v) => Value::UInt8(v),
            Self::Int16(v) => Value::Int16(v),
            Self::UInt16(v) => Value::UInt16(v),
            Self::Int32(v) => Value::Int32(v),
            Self::UInt32(v) => Value::UInt32(v),
            Self::Int64(v) => Value::Int64(v),
            Self::UInt64(v) => Value::UInt64(v),
            Self::Float32(v) => Value::Float32(v),
            Self::Float64(v) => Value::Float64(v),
            Self::String(v) => Value::String(v.into_owned()),
            Self::Raw(v) => Value::Raw(v.into_owned().into()),
            Self::Option(v) => Value::Option(v.map(|v| Box::new(v.into_owned()))),
            Self::List(v) => Value::List(v.into_iter().map(Value::into_owned).collect()),
            Self::Map(v) => Value::Map(
                v.into_iter()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect(),
            ),
            Self::Tuple(v) => Value::Tuple(v.into_iter().map(Value::into_owned).collect()),
            Self::Object(v) => Value::Object(v),
            Self::Dynamic(v) => Value::Dynamic(Box::new(v.into_owned())),
        }
    }
}

impl std::fmt::Display for Value<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Unit => f.write_str("()"),
            Value::Bool(v) => v.fmt(f),
            Value::Int8(v) => v.fmt(f),
            Value::UInt8(v) => v.fmt(f),
            Value::Int16(v) => v.fmt(f),
            Value::UInt16(v) => v.fmt(f),
            Value::Int32(v) => v.fmt(f),
            Value::UInt32(v) => v.fmt(f),
            Value::Int64(v) => v.fmt(f),
            Value::UInt64(v) => v.fmt(f),
            Value::Float32(v) => v.fmt(f),
            Value::Float64(v) => v.fmt(f),
            Value::String(v) => v.fmt(f),
            Value::Raw(v) => {
                write!(f, "raw[len={}]", v.len())
            }
            Value::Option(v) => {
                match v {
                    Some(v) => write!(f, "some({v})")?,
                    None => f.write_str("none")?,
                };
                f.write_str(")")
            }
            Value::List(l) => {
                write!(f, "[len={}/", l.len())?;
                let mut add_sep = false;
                for v in l {
                    if add_sep {
                        f.write_str(",")?;
                    }
                    v.fmt(f)?;
                    add_sep = true;
                }
                f.write_str("]")
            }
            Value::Map(m) => {
                write!(f, "{{len={}/", m.len())?;
                let mut add_sep = false;
                for (k, v) in m {
                    if add_sep {
                        f.write_str(",")?;
                    }
                    write!(f, "{k}:{v}")?;
                    add_sep = true;
                }
                f.write_str("}}")
            }
            Value::Tuple(elems) => {
                write!(f, "(")?;
                let mut add_sep = false;
                for v in elems {
                    if add_sep {
                        f.write_str(",")?;
                    }
                    v.fmt(f)?;
                    add_sep = true;
                }
                f.write_str(")")
            }
            Value::Object(object) => object.fmt(f),
            Value::Dynamic(v) => v.fmt(f),
        }
    }
}

impl ToValue for Value<'_> {
    fn to_value(&self) -> Value<'_> {
        self.clone()
    }
}

impl<'long: 'short, 'short> IntoValue<'short> for Value<'long> {
    fn into_value(self) -> Value<'short> {
        self
    }
}

impl<'long: 'short, 'short> FromValue<'long> for Value<'short> {
    fn from_value(value: Value<'long>) -> Result<Self, FromValueError> {
        Ok(value)
    }
}

impl RuntimeReflect for Value<'_> {
    fn ty(&self) -> Type {
        match self {
            Self::Unit => Type::Unit,
            Self::Bool(_) => Type::Bool,
            Self::Int8(_) => Type::Int8,
            Self::UInt8(_) => Type::UInt8,
            Self::Int16(_) => Type::Int16,
            Self::UInt16(_) => Type::UInt16,
            Self::Int32(_) => Type::Int32,
            Self::UInt32(_) => Type::UInt32,
            Self::Int64(_) => Type::Int64,
            Self::UInt64(_) => Type::UInt64,
            Self::Float32(_) => Type::Float32,
            Self::Float64(_) => Type::Float64,
            Self::String(_) => Type::String,
            Self::Raw(_) => Type::Raw,
            Self::Option(v) => Type::Option(v.as_deref().and_then(element_type).map(Box::new)),
            Self::List(v) => Type::List(reduce_element_types(v.iter()).map(Box::new)),
            Self::Map(v) => Type::Map {
                key: reduce_element_types(v.keys()).map(Box::new),
                value: reduce_element_types(v.values()).map(Box::new),
            },
            Self::Tuple(v) => Type::Tuple(ty::Tuple::Tuple(v.iter().map(element_type).collect())),
            Self::Object(_) => Type::Object,
            // A dynamic value stands for its content: this is the type written in its signature
            // when it is serialized on its own.
            Self::Dynamic(v) => v.ty(),
        }
    }
}

/// The type of a value as an element of a container: dynamic values are dynamic elements
/// (`None`), since they are serialized with their own signature.
fn element_type(value: &Value<'_>) -> Option<Type> {
    match value {
        Value::Dynamic(_) => None,
        value => Some(value.ty()),
    }
}

/// The common type of the elements of a container, `None` (dynamic) if any element is dynamic
/// or if their types differ.
pub(crate) fn reduce_element_types<'a, 'v: 'a>(
    values: impl Iterator<Item = &'a Value<'v>>,
) -> Option<Type> {
    let mut types = Vec::new();
    for value in values {
        types.push(element_type(value)?);
    }
    ty::reduce_type(types)
}

pub trait IntoValue<'a>: Sized {
    fn into_value(self) -> Value<'a>;
}

// TODO: remove this trait
pub trait ToValue {
    fn to_value(&self) -> Value<'_>;
}

pub trait FromValue<'a>: Sized {
    fn from_value(value: Value<'a>) -> Result<Self, FromValueError>;
}

#[derive(thiserror::Error, Debug)]
pub enum FromValueError {
    #[error("value type mismatch: expected \"{expected}\", found \"{actual}\"")]
    TypeMismatch {
        expected: StdString,
        actual: StdString,
    },

    #[error("bad string null character")]
    BadNulChar(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

impl From<std::convert::Infallible> for FromValueError {
    fn from(value: std::convert::Infallible) -> Self {
        match value {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_elements_make_containers_dynamic() {
        use crate::value::{reflect::RuntimeReflect, Signature};
        let sig = |value: &Value<'_>| Signature::from(value.ty()).to_string();
        let dynamic = |value: Value<'static>| Value::Dynamic(Box::new(value));
        assert_eq!(
            sig(&Value::List(vec![Value::Int32(1), Value::Int32(2)])),
            "[i]"
        );
        assert_eq!(
            sig(&Value::List(vec![
                dynamic(Value::Int32(1)),
                dynamic(Value::Int32(2))
            ])),
            "[m]"
        );
        assert_eq!(
            sig(&Value::List(vec![Value::Int32(1), dynamic(Value::Unit)])),
            "[m]"
        );
        assert_eq!(
            sig(&Value::Tuple(vec![
                Value::Int32(1),
                dynamic(Value::Bool(true))
            ])),
            "(im)"
        );
        assert_eq!(
            sig(&Value::Option(Some(Box::new(dynamic(Value::Int32(1)))))),
            "+m"
        );
        let mut map = crate::value::Map::new();
        map.insert(Value::String("k".into()), dynamic(Value::Int32(1)));
        assert_eq!(sig(&Value::Map(map)), "{sm}");
        // A dynamic value on its own is typed as its content.
        assert_eq!(sig(&dynamic(Value::Int32(1))), "i");
    }
}
