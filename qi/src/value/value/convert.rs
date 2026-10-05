//! Conversion of values to a target type.
//!
//! The reference implementation converts the arguments of a call to the parameter types of the
//! called method, and the values it receives to the types the caller expects. These conversions
//! let statically typed callers interact with dynamically typed members (and vice versa), and
//! tolerate differences of numeric width between peers.

use super::{FromValueError, Value};
use crate::value::{map::Map, Type};
use ordered_float::OrderedFloat;

/// Converts the value to the given type, `None` standing for the dynamic type. See
/// [`Value::convert_to`].
pub(super) fn convert_to<'a>(
    value: Value<'a>,
    ty: Option<&Type>,
) -> Result<Value<'a>, FromValueError> {
    let Some(ty) = ty else {
        return Ok(match value {
            dynamic @ Value::Dynamic(_) => dynamic,
            value => value.into_dynamic(),
        });
    };
    match (ty, value) {
        (_, Value::Dynamic(inner)) => inner.convert_to(Some(ty)),
        (Type::Unit, Value::Unit) => Ok(Value::Unit),
        (Type::Bool, value @ Value::Bool(_)) => Ok(value),
        (
            Type::Int8
            | Type::UInt8
            | Type::Int16
            | Type::UInt16
            | Type::Int32
            | Type::UInt32
            | Type::Int64
            | Type::UInt64
            | Type::Float32
            | Type::Float64,
            value,
        ) => convert_number(ty, value),
        (Type::String, value @ Value::String(_)) => Ok(value),
        (Type::Raw, value @ Value::Raw(_)) => Ok(value),
        (Type::Object, value @ Value::Object(_)) => Ok(value),
        (Type::Option(inner), Value::Option(option)) => Ok(Value::Option(match option {
            Some(value) => Some(Box::new(value.convert_to(inner.as_deref())?)),
            None => None,
        })),
        (Type::Option(inner), value) => Ok(Value::Option(Some(Box::new(
            value.convert_to(inner.as_deref())?,
        )))),
        (
            Type::List(element) | Type::VarArgs(element),
            Value::List(items) | Value::Tuple(items),
        ) => items
            .into_iter()
            .map(|item| item.convert_to(element.as_deref()))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::List),
        (Type::Map { key, value }, Value::Map(map)) => {
            let mut converted = Map::with_capacity(map.len());
            for (k, v) in map {
                converted.insert(
                    k.convert_to(key.as_deref())?,
                    v.convert_to(value.as_deref())?,
                );
            }
            Ok(Value::Map(converted))
        }
        (Type::Tuple(tuple), Value::Tuple(items) | Value::List(items)) => {
            let types = tuple.element_types();
            if types.len() != items.len() {
                return Err(mismatch(ty, &Value::Tuple(items)));
            }
            items
                .into_iter()
                .zip(types)
                .map(|(item, ty)| item.convert_to(ty.as_ref()))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Tuple)
        }
        (_, value) => Err(mismatch(ty, &value)),
    }
}

fn mismatch(expected: &Type, actual: &Value<'_>) -> FromValueError {
    FromValueError::TypeMismatch {
        expected: expected.to_string(),
        actual: actual.to_string(),
    }
}

enum Number {
    Int(i128),
    Float(f64),
}

/// Converts a numeric value to a numeric type, failing if the value is not a number or is not
/// representable in the target type.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    clippy::float_cmp
)]
fn convert_number<'a>(ty: &Type, value: Value<'a>) -> Result<Value<'a>, FromValueError> {
    let number = match &value {
        Value::Int8(v) => Number::Int(i128::from(*v)),
        Value::UInt8(v) => Number::Int(i128::from(*v)),
        Value::Int16(v) => Number::Int(i128::from(*v)),
        Value::UInt16(v) => Number::Int(i128::from(*v)),
        Value::Int32(v) => Number::Int(i128::from(*v)),
        Value::UInt32(v) => Number::Int(i128::from(*v)),
        Value::Int64(v) => Number::Int(i128::from(*v)),
        Value::UInt64(v) => Number::Int(i128::from(*v)),
        Value::Float32(v) => Number::Float(f64::from(v.0)),
        Value::Float64(v) => Number::Float(v.0),
        _ => return Err(mismatch(ty, &value)),
    };
    macro_rules! to_int {
        ($variant:ident, $int:ty) => {
            match number {
                Number::Int(i) => <$int>::try_from(i).ok().map(Value::$variant),
                // Floats convert to integers only when they are integral and in range.
                Number::Float(f) => {
                    let is_integral = f.fract() == 0.0;
                    let in_range = f >= <$int>::MIN as f64 && f <= <$int>::MAX as f64;
                    (is_integral && in_range).then(|| Value::$variant(f as $int))
                }
            }
        };
    }
    let converted = match ty {
        Type::Int8 => to_int!(Int8, i8),
        Type::UInt8 => to_int!(UInt8, u8),
        Type::Int16 => to_int!(Int16, i16),
        Type::UInt16 => to_int!(UInt16, u16),
        Type::Int32 => to_int!(Int32, i32),
        Type::UInt32 => to_int!(UInt32, u32),
        Type::Int64 => to_int!(Int64, i64),
        Type::UInt64 => to_int!(UInt64, u64),
        Type::Float32 => Some(Value::Float32(OrderedFloat(match number {
            Number::Int(i) => i as f32,
            Number::Float(f) => f as f32,
        }))),
        Type::Float64 => Some(Value::Float64(OrderedFloat(match number {
            Number::Int(i) => i as f64,
            Number::Float(f) => f,
        }))),
        _ => None,
    };
    converted.ok_or_else(|| mismatch(ty, &value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{ty::Tuple, IntoValue};

    #[test]
    fn dynamic_target_wraps_and_dynamic_value_unwraps() {
        let value = Value::Int32(3).convert_to(None).unwrap();
        assert_eq!(value, Value::Dynamic(Box::new(Value::Int32(3))));
        // Wrapping is idempotent.
        assert_eq!(value.clone().convert_to(None).unwrap(), value);
        // Unwrapping converts the content.
        assert_eq!(
            value.convert_to(Some(&Type::Int64)).unwrap(),
            Value::Int64(3)
        );
    }

    #[test]
    fn numbers_convert_when_representable() {
        assert_eq!(
            Value::Int32(-5).convert_to(Some(&Type::Int64)).unwrap(),
            Value::Int64(-5)
        );
        assert_eq!(
            Value::UInt64(300).convert_to(Some(&Type::Int16)).unwrap(),
            Value::Int16(300)
        );
        assert_eq!(
            Value::Int32(2).convert_to(Some(&Type::Float64)).unwrap(),
            2.0f64.into_value()
        );
        assert_eq!(
            Value::Float64(OrderedFloat(4.0))
                .convert_to(Some(&Type::UInt8))
                .unwrap(),
            Value::UInt8(4)
        );
        assert!(Value::Int32(-1).convert_to(Some(&Type::UInt8)).is_err());
        assert!(Value::Int32(300).convert_to(Some(&Type::Int8)).is_err());
        assert!(Value::Float64(OrderedFloat(1.5))
            .convert_to(Some(&Type::Int32))
            .is_err());
        assert!(Value::Bool(true).convert_to(Some(&Type::Int32)).is_err());
    }

    #[test]
    fn containers_convert_elementwise() {
        let tuple_ty = Type::Tuple(Tuple::Tuple(vec![Some(Type::Int32), None]));
        let converted = Value::Tuple(vec![Value::Int8(1), Value::String("a".into())])
            .convert_to(Some(&tuple_ty))
            .unwrap();
        assert_eq!(
            converted,
            Value::Tuple(vec![
                Value::Int32(1),
                Value::Dynamic(Box::new(Value::String("a".into())))
            ])
        );
        // Lists convert to tuples of the same length, and tuples to lists.
        assert!(Value::List(vec![Value::Int32(1)])
            .convert_to(Some(&tuple_ty))
            .is_err());
        assert_eq!(
            Value::Tuple(vec![Value::Int32(1), Value::Int32(2)])
                .convert_to(Some(&Type::List(Some(Box::new(Type::Int64)))))
                .unwrap(),
            Value::List(vec![Value::Int64(1), Value::Int64(2)])
        );
        // Optionals.
        let opt_ty = Type::Option(Some(Box::new(Type::Int32)));
        assert_eq!(
            Value::Int8(7).convert_to(Some(&opt_ty)).unwrap(),
            Value::Option(Some(Box::new(Value::Int32(7))))
        );
        assert_eq!(
            Value::Option(None).convert_to(Some(&opt_ty)).unwrap(),
            Value::Option(None)
        );
        // Maps.
        let map_ty = Type::Map {
            key: Some(Box::new(Type::String)),
            value: None,
        };
        let mut map = Map::new();
        map.insert(Value::String("k".into()), Value::Int32(1));
        let converted = Value::Map(map).convert_to(Some(&map_ty)).unwrap();
        let mut expected = Map::new();
        expected.insert(
            Value::String("k".into()),
            Value::Dynamic(Box::new(Value::Int32(1))),
        );
        assert_eq!(converted, Value::Map(expected));
    }

    #[test]
    fn mismatches_are_errors() {
        let err = Value::String("a".into())
            .convert_to(Some(&Type::Bool))
            .unwrap_err();
        assert!(matches!(err, FromValueError::TypeMismatch { .. }), "{err}");
        assert!(Value::Unit.convert_to(Some(&Type::String)).is_err());
        assert_eq!(
            Value::Unit.convert_to(Some(&Type::Unit)).unwrap(),
            Value::Unit
        );
    }
}
