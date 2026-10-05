//! Conventions of parameters tuples.
//!
//! In the `qi` type system, the arguments of a method call and the values carried by a signal
//! are *tuples of parameters*. Rust code manipulates them as a single value: a Rust tuple maps to
//! multiple parameters, and any other type maps to a single parameter. These functions convert
//! between the two representations, using the static type of the Rust value to decide.

use crate::value::{ty::Tuple, Reflect, Type, Value};

/// The type of the parameters tuple of a value of the given static type.
pub(crate) fn params_type_of(ty: Option<Type>) -> Type {
    match ty {
        Some(Type::Unit) => Type::unit_tuple(),
        Some(ty @ Type::Tuple(Tuple::Tuple(_))) => ty,
        ty => Type::Tuple(Tuple::Tuple(vec![ty])),
    }
}

/// The type of the parameters tuple of values of type `T`.
pub(crate) fn params_type<T: Reflect + ?Sized>() -> Type {
    params_type_of(T::ty())
}

/// Wraps a value of type `T` into a parameters tuple.
pub(crate) fn to_params<T: Reflect + ?Sized>(value: Value<'_>) -> Value<'_> {
    to_params_of(T::ty().as_ref(), value)
}

/// Wraps a value of the given static type into a parameters tuple.
pub(crate) fn to_params_of<'a>(ty: Option<&Type>, value: Value<'a>) -> Value<'a> {
    match (ty, value) {
        (Some(Type::Unit), Value::Unit) => Value::Tuple(vec![]),
        (Some(Type::Tuple(Tuple::Tuple(_))), value @ Value::Tuple(_)) => value,
        (_, value) => Value::Tuple(vec![value]),
    }
}

/// Unwraps a value of type `T` from a parameters tuple.
pub(crate) fn from_params<T: Reflect + ?Sized>(value: Value<'_>) -> Value<'_> {
    from_params_of(T::ty().as_ref(), value)
}

/// Unwraps a value of the given static type from a parameters tuple.
pub(crate) fn from_params_of<'a>(ty: Option<&Type>, value: Value<'a>) -> Value<'a> {
    match (ty, value) {
        (Some(Type::Unit), Value::Tuple(elements)) if elements.is_empty() => Value::Unit,
        (Some(Type::Tuple(Tuple::Tuple(_))), value @ Value::Tuple(_)) => value,
        (_, Value::Tuple(mut elements)) if elements.len() == 1 => elements.remove(0),
        (_, value) => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{IntoValue, Signature};

    fn sig(ty: Type) -> String {
        Signature::from(ty).to_string()
    }

    #[test]
    fn params_of_scalar_and_tuples() {
        assert_eq!(sig(params_type::<i32>()), "(i)");
        assert_eq!(sig(params_type::<(i32, String)>()), "(is)");
        assert_eq!(sig(params_type::<()>()), "()");
        assert_eq!(
            to_params::<i32>(3.into_value()),
            Value::Tuple(vec![Value::Int32(3)])
        );
        assert_eq!(to_params::<()>(().into_value()), Value::Tuple(vec![]));
        assert_eq!(
            to_params::<(i32, bool)>((3, true).into_value()),
            Value::Tuple(vec![Value::Int32(3), Value::Bool(true)])
        );
        assert_eq!(
            from_params::<i32>(Value::Tuple(vec![Value::Int32(3)])),
            Value::Int32(3)
        );
        assert_eq!(from_params::<()>(Value::Tuple(vec![])), Value::Unit);
    }
}
