//! `ALValue`: the dynamically typed values of the NAOqi APIs.
//!
//! NAOqi services take and return `ALValue`s, which travel on the wire as dynamic values (`m`).
//! [`AlValue`] wraps a [`Value`] so that it may be used as a method parameter, a method return
//! value or a signal value of a dynamic object, and normalizes the containers it holds so that
//! they serialize the way NAOqi's do (see [`AlValue::new`]).

use qi::value::{FromValue, FromValueError, IntoValue, Reflect, RuntimeReflect, Type, Value};

/// A dynamically typed value, of signature `m`.
///
/// Every container element of an `ALValue` is itself an `ALValue`: an array whose elements have
/// different types is a list of dynamic values on the wire. This type keeps that invariant while
/// keeping homogeneous lists statically typed, which is what the reference implementation does
/// when it serializes an `ALValue`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AlValue(pub Value<'static>);

impl AlValue {
    /// Wraps a value, normalizing its containers: dynamic wrappers are removed, and the elements
    /// of lists and maps whose types differ are wrapped in dynamic values.
    pub fn new<V: IntoValue<'static>>(value: V) -> Self {
        Self(normalize(value.into_value()))
    }

    /// An `ALValue` array of `ALValue`s.
    pub fn list<I>(items: I) -> Self
    where
        I: IntoIterator<Item = AlValue>,
    {
        Self::new(Value::List(items.into_iter().map(|item| item.0).collect()))
    }

    /// An `ALValue` array of floats.
    pub fn floats<I>(items: I) -> Self
    where
        I: IntoIterator<Item = f32>,
    {
        Self(Value::List(
            items.into_iter().map(IntoValue::into_value).collect(),
        ))
    }

    /// An `ALValue` array of strings.
    pub fn strings<I, S>(items: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self(Value::List(
            items
                .into_iter()
                .map(|item| item.into().into_value())
                .collect(),
        ))
    }

    /// The wrapped value.
    pub fn into_inner(self) -> Value<'static> {
        self.0
    }

    /// The wrapped value.
    pub fn value(&self) -> &Value<'static> {
        &self.0
    }

    /// Converts the value to a 32-bit float, if it is a number.
    pub fn as_f32(&self) -> Option<f32> {
        self.convert::<f32>(Type::Float32)
    }

    /// Converts the value to a 64-bit float, if it is a number.
    pub fn as_f64(&self) -> Option<f64> {
        self.convert::<f64>(Type::Float64)
    }

    /// Converts the value to a 32-bit integer, if it is an integral number.
    pub fn as_i32(&self) -> Option<i32> {
        self.convert::<i32>(Type::Int32)
    }

    /// The value as a boolean. Numbers are true when non-zero.
    pub fn as_bool(&self) -> Option<bool> {
        match &self.0 {
            Value::Bool(b) => Some(*b),
            _ => self.as_f64().map(|f| f != 0.0),
        }
    }

    /// The value as a string, if it is one.
    pub fn as_str(&self) -> Option<&str> {
        self.0.as_string().and_then(|s| s.as_str())
    }

    /// The value as a list of values, if it is a list or a tuple.
    pub fn as_list(&self) -> Option<Vec<AlValue>> {
        match &self.0 {
            Value::List(items) | Value::Tuple(items) => Some(
                items
                    .iter()
                    .map(|item| AlValue(strip_dynamic(item.clone())))
                    .collect(),
            ),
            _ => None,
        }
    }

    /// The value as a list of strings, if it is a list of strings.
    pub fn as_strings(&self) -> Option<Vec<String>> {
        self.as_list()?
            .iter()
            .map(|item| item.as_str().map(str::to_owned))
            .collect()
    }

    /// The value as a list of floats, if it is a list of numbers.
    pub fn as_floats(&self) -> Option<Vec<f32>> {
        self.as_list()?.iter().map(AlValue::as_f32).collect()
    }

    /// Interprets the value as one or several names: a string, or a list of strings.
    ///
    /// This is the convention of the `names` parameters of `ALMotion`.
    pub fn names(&self) -> Option<Vec<String>> {
        match self.as_str() {
            Some(name) => Some(vec![name.to_owned()]),
            None => self.as_strings(),
        }
    }

    /// Interprets the value as one or several numbers: a number, or a list of numbers.
    ///
    /// This is the convention of the `angles` and `stiffnesses` parameters of `ALMotion`.
    pub fn numbers(&self) -> Option<Vec<f32>> {
        match self.as_f32() {
            Some(number) => Some(vec![number]),
            None => self.as_floats(),
        }
    }

    /// Converts a JSON value to an `ALValue`: numbers become 32-bit integers when integral and
    /// 32-bit floats otherwise, objects become maps of strings to values.
    pub fn from_json(json: &serde_json::Value) -> Self {
        Self::new(json_to_value(json))
    }

    /// Converts the value to JSON, for display and scripting. Raw buffers become their length.
    pub fn to_json(&self) -> serde_json::Value {
        value_to_json(&self.0)
    }

    fn convert<T: FromValue<'static>>(&self, ty: Type) -> Option<T> {
        self.0
            .clone()
            .convert_to(Some(&ty))
            .ok()
            .and_then(|value| T::from_value(value).ok())
    }
}

impl std::fmt::Display for AlValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Reflect for AlValue {
    fn ty() -> Option<Type> {
        None
    }
}

impl RuntimeReflect for AlValue {
    fn ty(&self) -> Type {
        self.0.ty()
    }
}

impl IntoValue<'static> for AlValue {
    fn into_value(self) -> Value<'static> {
        Value::Dynamic(Box::new(self.0))
    }
}

impl FromValue<'static> for AlValue {
    fn from_value(value: Value<'static>) -> Result<Self, FromValueError> {
        Ok(Self::new(value))
    }
}

impl From<Value<'static>> for AlValue {
    fn from(value: Value<'static>) -> Self {
        Self::new(value)
    }
}

impl From<f32> for AlValue {
    fn from(value: f32) -> Self {
        Self(value.into_value())
    }
}

impl From<i32> for AlValue {
    fn from(value: i32) -> Self {
        Self(value.into_value())
    }
}

impl From<bool> for AlValue {
    fn from(value: bool) -> Self {
        Self(value.into_value())
    }
}

impl From<&str> for AlValue {
    fn from(value: &str) -> Self {
        Self(value.to_owned().into_value())
    }
}

impl From<String> for AlValue {
    fn from(value: String) -> Self {
        Self(value.into_value())
    }
}

/// Removes the dynamic wrappers of a value.
fn strip_dynamic(value: Value<'static>) -> Value<'static> {
    match value {
        Value::Dynamic(inner) => strip_dynamic(*inner),
        value => value,
    }
}

/// Normalizes the containers of a value so that it serializes correctly as a dynamic value.
///
/// The elements of a list are serialized with the type computed for the whole list: when the
/// element types differ, the list is a list of dynamics and each element must be a dynamic value.
fn normalize(value: Value<'static>) -> Value<'static> {
    match value {
        Value::Dynamic(inner) => normalize(*inner),
        Value::List(items) => Value::List(normalize_elements(items)),
        Value::Tuple(items) => Value::Tuple(items.into_iter().map(normalize).collect()),
        Value::Option(inner) => Value::Option(inner.map(|inner| Box::new(normalize(*inner)))),
        Value::Map(map) => {
            let (keys, values): (Vec<_>, Vec<_>) = map.into_iter().unzip();
            let keys = normalize_elements(keys);
            let values = normalize_elements(values);
            Value::Map(keys.into_iter().zip(values).collect())
        }
        value => value,
    }
}

fn normalize_elements(items: Vec<Value<'static>>) -> Vec<Value<'static>> {
    let items: Vec<_> = items.into_iter().map(normalize).collect();
    let homogeneous = items.windows(2).all(|pair| pair[0].ty() == pair[1].ty());
    if homogeneous {
        items
    } else {
        items
            .into_iter()
            .map(|item| Value::Dynamic(Box::new(item)))
            .collect()
    }
}

fn json_to_value(json: &serde_json::Value) -> Value<'static> {
    match json {
        serde_json::Value::Null => Value::Unit,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(number) => match number.as_i64() {
            Some(int) => match i32::try_from(int) {
                Ok(int) => Value::Int32(int),
                Err(_) => Value::Int64(int),
            },
            #[allow(clippy::cast_possible_truncation)]
            None => (number.as_f64().unwrap_or_default() as f32).into_value(),
        },
        serde_json::Value::String(s) => s.clone().into_value(),
        serde_json::Value::Array(items) => Value::List(items.iter().map(json_to_value).collect()),
        serde_json::Value::Object(fields) => Value::Map(
            fields
                .iter()
                .map(|(key, value)| (key.clone().into_value(), json_to_value(value)))
                .collect(),
        ),
    }
}

fn value_to_json(value: &Value<'static>) -> serde_json::Value {
    use serde_json::Value as Json;
    match value {
        Value::Unit => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Int8(v) => Json::from(*v),
        Value::UInt8(v) => Json::from(*v),
        Value::Int16(v) => Json::from(*v),
        Value::UInt16(v) => Json::from(*v),
        Value::Int32(v) => Json::from(*v),
        Value::UInt32(v) => Json::from(*v),
        Value::Int64(v) => Json::from(*v),
        Value::UInt64(v) => Json::from(*v),
        Value::Float32(v) => Json::from(f64::from(v.0)),
        Value::Float64(v) => Json::from(v.0),
        Value::String(s) => Json::String(s.to_string()),
        Value::Raw(bytes) => Json::from(bytes.len()),
        Value::Option(inner) => inner.as_deref().map(value_to_json).unwrap_or(Json::Null),
        Value::List(items) | Value::Tuple(items) => {
            Json::Array(items.iter().map(value_to_json).collect())
        }
        Value::Map(map) => Json::Object(
            map.iter()
                .map(|(key, value)| (key.to_string(), value_to_json(value)))
                .collect(),
        ),
        Value::Object(object) => Json::String(object.to_string()),
        Value::Dynamic(inner) => value_to_json(inner),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heterogeneous_lists_hold_dynamic_elements() {
        let value = AlValue::new(Value::List(vec![
            Value::Int32(1),
            "a".to_owned().into_value(),
        ]));
        assert_eq!(
            value.0,
            Value::List(vec![
                Value::Dynamic(Box::new(Value::Int32(1))),
                Value::Dynamic(Box::new("a".to_owned().into_value())),
            ])
        );
        // The list type is a list of dynamics, and serializes as such.
        assert_eq!(value.0.ty(), Type::List(None));
    }

    #[test]
    fn homogeneous_lists_stay_static() {
        let value = AlValue::new(Value::List(vec![
            Value::Dynamic(Box::new(Value::Int32(1))),
            Value::Int32(2),
        ]));
        assert_eq!(value.0, Value::List(vec![Value::Int32(1), Value::Int32(2)]));
        assert_eq!(value.as_floats(), Some(vec![1.0, 2.0]));
    }

    #[test]
    fn names_and_numbers_accept_scalars_and_lists() {
        assert_eq!(
            AlValue::from("HeadYaw").names(),
            Some(vec!["HeadYaw".to_owned()])
        );
        assert_eq!(
            AlValue::strings(["a", "b"]).names(),
            Some(vec!["a".to_owned(), "b".to_owned()])
        );
        assert_eq!(AlValue::from(1).numbers(), Some(vec![1.0]));
        assert_eq!(AlValue::floats([0.5, 1.5]).numbers(), Some(vec![0.5, 1.5]));
        assert_eq!(AlValue::from("x").numbers(), None);
    }

    #[test]
    fn json_round_trip() {
        let json: serde_json::Value = serde_json::from_str(r#"[1, 2.5, "x", true, null]"#).unwrap();
        let value = AlValue::from_json(&json);
        assert_eq!(value.to_json(), json);
    }
}
