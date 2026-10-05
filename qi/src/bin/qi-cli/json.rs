//! Conversions between JSON and values of the `qi` type system.
//!
//! JSON is the format of the arguments given on the command line and of the results printed by
//! the tool. Since the `qi` type system is richer than JSON (it distinguishes numeric widths,
//! tuples from lists, raw data from strings...), converting a JSON document to a value is guided
//! by the *type* of the value, as advertised by the meta object of the target member: the same
//! JSON number `1` becomes an `int32` for one method and a `float32` for another.
//!
//! # Representation of values in JSON
//!
//! | `qi` type              | JSON                                                              |
//! |------------------------|-------------------------------------------------------------------|
//! | unit                   | `null`                                                            |
//! | bool                   | `true`, `false`                                                   |
//! | integers, floats       | numbers (non-finite floats are the strings `"NaN"`, `"inf"`, `"-inf"`) |
//! | string                 | string                                                            |
//! | raw                    | base64 string (also accepted as input: an array of bytes)         |
//! | option                 | `null` or the value                                               |
//! | list                   | array                                                             |
//! | map                    | object when keys are strings, numbers or booleans, otherwise an array of `[key, value]` pairs |
//! | tuple                  | array (a structure with named fields is an object)                |
//! | object                 | a summary object (objects cannot be given as input)               |
//! | dynamic                | the value itself, its type being inferred on input                |
//!
//! When the type of a value is dynamic, its type is inferred from the JSON: integers become
//! `int32` when they fit (`int64` or `uint64` otherwise), other numbers `float64`, arrays lists
//! and objects maps of strings. Heterogeneous arrays and objects hold dynamic elements.

use base64::Engine;
use qi::value::{
    object::Object, ty::Tuple, IntoValue, Map, RuntimeReflect, String as QiString, Type, Value,
};
use serde_json::Value as Json;
use std::{borrow::Cow, fmt};

const BASE64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Converts a JSON document to a value of the given type.
///
/// The absence of type is the dynamic type: the type of the value is inferred from the document,
/// and the result is a [`Value::Dynamic`].
pub(crate) fn to_value(json: &Json, ty: Option<&Type>) -> Result<Value<'static>, Error> {
    Converter::default().convert(json, ty)
}

/// Parses a command-line argument as a value of the given type.
///
/// The argument is a JSON document, with a convenience for strings: when the type is a string
/// (or dynamic), an argument that is not a JSON string literal is taken as the text itself, so
/// that `hello` and `"hello"` are the same string.
pub(crate) fn parse_arg(text: &str, ty: Option<&Type>) -> Result<Value<'static>, Error> {
    let json = serde_json::from_str::<Json>(text);
    match (json, ty) {
        (Ok(json @ Json::Null), Some(Type::Option(_))) => to_value(&json, ty),
        (Ok(json @ Json::String(_)), _) => to_value(&json, ty),
        (_, ty) if is_string(ty) => to_value(&Json::String(text.to_owned()), ty),
        (Ok(json), _) => to_value(&json, ty),
        (Err(_), None) => to_value(&Json::String(text.to_owned()), None),
        (Err(err), Some(_)) => Err(Error {
            path: String::new(),
            kind: ErrorKind::Syntax(err),
        }),
    }
}

/// Converts a value to a JSON document.
///
/// The type of the value, when known, names the fields of structures. It is not required: the
/// conversion never fails.
pub(crate) fn from_value(value: &Value<'_>, ty: Option<&Type>) -> Json {
    match value {
        Value::Unit => Json::Null,
        Value::Bool(v) => Json::Bool(*v),
        Value::Int8(v) => Json::from(*v),
        Value::UInt8(v) => Json::from(*v),
        Value::Int16(v) => Json::from(*v),
        Value::UInt16(v) => Json::from(*v),
        Value::Int32(v) => Json::from(*v),
        Value::UInt32(v) => Json::from(*v),
        Value::Int64(v) => Json::from(*v),
        Value::UInt64(v) => Json::from(*v),
        Value::Float32(v) => float32_json(v.0),
        Value::Float64(v) => float64_json(v.0),
        Value::String(v) => Json::String(string_lossy(v)),
        Value::Raw(v) => Json::String(base64(v)),
        Value::Option(v) => match v {
            None => Json::Null,
            Some(inner) => from_value(inner, option_element(ty)),
        },
        Value::List(items) => {
            let element = list_element(ty);
            Json::Array(items.iter().map(|item| from_value(item, element)).collect())
        }
        Value::Map(map) => map_json(map, ty),
        Value::Tuple(items) => tuple_json(items, ty),
        Value::Object(object) => object_json(object),
        Value::Dynamic(inner) => from_value(inner, None),
    }
}

/// An error of conversion of a JSON document to a value.
#[derive(Debug, thiserror::Error)]
pub(crate) struct Error {
    /// The path of the erroneous element in the document, empty for the document itself.
    path: String,
    kind: ErrorKind,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.path.is_empty() {
            write!(f, "at {}: ", self.path)?;
        }
        self.kind.fmt(f)
    }
}

#[derive(Debug, thiserror::Error)]
enum ErrorKind {
    #[error("expected {expected}, found {found}")]
    Type { expected: String, found: String },

    #[error("the number {number} is not representable as {expected}")]
    Number { expected: String, number: String },

    #[error("expected a tuple of {expected} element(s), found {found}")]
    Length { expected: usize, found: usize },

    #[error("missing field \"{0}\"")]
    MissingField(String),

    #[error("unknown field \"{0}\"")]
    UnknownField(String),

    #[error("invalid base64 raw data")]
    Base64(#[source] base64::DecodeError),

    #[error("invalid map key \"{key}\": expected {expected}")]
    Key { key: String, expected: String },

    #[error("objects cannot be given as JSON")]
    Object,

    #[error("invalid JSON")]
    Syntax(#[source] serde_json::Error),
}

/// A segment of the path of an element in a JSON document.
enum Segment {
    Index(usize),
    Field(String),
    Key(String),
}

impl fmt::Display for Segment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Index(index) => write!(f, "[{index}]"),
            Self::Field(name) => write!(f, ".{name}"),
            Self::Key(key) => write!(f, "[{}]", Json::String(key.clone())),
        }
    }
}

/// The conversion of a JSON document to a value, tracking the path of the element being converted
/// for error reporting.
#[derive(Default)]
struct Converter {
    path: Vec<Segment>,
}

impl Converter {
    fn nested<T>(
        &mut self,
        segment: Segment,
        f: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        self.path.push(segment);
        let result = f(self);
        self.path.pop();
        result
    }

    fn error(&self, kind: ErrorKind) -> Error {
        Error {
            path: self.path.iter().map(ToString::to_string).collect(),
            kind,
        }
    }

    fn mismatch(&self, expected: impl Into<String>, found: &Json) -> Error {
        self.error(ErrorKind::Type {
            expected: expected.into(),
            found: describe(found),
        })
    }

    fn convert(&mut self, json: &Json, ty: Option<&Type>) -> Result<Value<'static>, Error> {
        let Some(ty) = ty else {
            return Ok(Value::Dynamic(Box::new(infer(json))));
        };
        match ty {
            Type::Unit => match json {
                Json::Null => Ok(Value::Unit),
                other => Err(self.mismatch("null (unit)", other)),
            },
            Type::Bool => match json {
                Json::Bool(v) => Ok(Value::Bool(*v)),
                other => Err(self.mismatch("a boolean", other)),
            },
            Type::Int8
            | Type::UInt8
            | Type::Int16
            | Type::UInt16
            | Type::Int32
            | Type::UInt32
            | Type::Int64
            | Type::UInt64 => self.integer(json, ty),
            Type::Float32 | Type::Float64 => self.float(json, ty),
            Type::String => match json {
                Json::String(v) => Ok(Value::String(v.clone().into())),
                other => Err(self.mismatch("a string", other)),
            },
            Type::Raw => self.raw(json),
            Type::Object => Err(self.error(ErrorKind::Object)),
            Type::Option(element) => match json {
                Json::Null => Ok(Value::Option(None)),
                other => Ok(Value::Option(Some(Box::new(
                    self.convert(other, element.as_deref())?,
                )))),
            },
            Type::List(element) | Type::VarArgs(element) => match json {
                Json::Array(items) => self.list(items, element.as_deref()),
                other => Err(self.mismatch(
                    format!("a list of {}", type_name(element.as_deref())),
                    other,
                )),
            },
            Type::Map { key, value } => self.map(json, key.as_deref(), value.as_deref()),
            Type::Tuple(tuple) => self.tuple(json, tuple),
        }
    }

    fn integer(&self, json: &Json, ty: &Type) -> Result<Value<'static>, Error> {
        let Json::Number(number) = json else {
            return Err(self.mismatch(format!("an integer ({ty})"), json));
        };
        let wide = if let Some(v) = number.as_i64() {
            Some(i128::from(v))
        } else if let Some(v) = number.as_u64() {
            Some(i128::from(v))
        } else {
            // Floats convert to integers only when they are integral.
            number
                .as_f64()
                .filter(|v| v.is_finite() && v.fract() == 0.0)
                .map(|v| v as i128)
        };
        wide.and_then(|v| integer_value(ty, v)).ok_or_else(|| {
            self.error(ErrorKind::Number {
                expected: ty.to_string(),
                number: number.to_string(),
            })
        })
    }

    fn float(&self, json: &Json, ty: &Type) -> Result<Value<'static>, Error> {
        let v = match json {
            Json::Number(number) => number.as_f64().ok_or_else(|| {
                self.error(ErrorKind::Number {
                    expected: ty.to_string(),
                    number: number.to_string(),
                })
            })?,
            Json::String(text) => match text.as_str() {
                "NaN" | "nan" => f64::NAN,
                "inf" | "+inf" | "Infinity" => f64::INFINITY,
                "-inf" | "-Infinity" => f64::NEG_INFINITY,
                _ => return Err(self.mismatch(format!("a number ({ty})"), json)),
            },
            other => return Err(self.mismatch(format!("a number ({ty})"), other)),
        };
        Ok(match ty {
            Type::Float32 => (v as f32).into_value(),
            _ => v.into_value(),
        })
    }

    fn raw(&mut self, json: &Json) -> Result<Value<'static>, Error> {
        match json {
            Json::String(text) => BASE64
                .decode(text)
                .map(|bytes| Value::Raw(Cow::Owned(bytes)))
                .map_err(|err| self.error(ErrorKind::Base64(err))),
            Json::Array(items) => items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    self.nested(Segment::Index(index), |c| {
                        item.as_u64()
                            .and_then(|byte| u8::try_from(byte).ok())
                            .ok_or_else(|| c.mismatch("a byte (0-255)", item))
                    })
                })
                .collect::<Result<Vec<u8>, _>>()
                .map(|bytes| Value::Raw(Cow::Owned(bytes))),
            other => Err(self.mismatch("raw data, as a base64 string or an array of bytes", other)),
        }
    }

    fn list(&mut self, items: &[Json], element: Option<&Type>) -> Result<Value<'static>, Error> {
        items
            .iter()
            .enumerate()
            .map(|(index, item)| self.nested(Segment::Index(index), |c| c.convert(item, element)))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::List)
    }

    fn map(
        &mut self,
        json: &Json,
        key_type: Option<&Type>,
        value_type: Option<&Type>,
    ) -> Result<Value<'static>, Error> {
        let mut map = Map::new();
        match json {
            Json::Object(entries) => {
                for (key, value) in entries {
                    let k = self.nested(Segment::Key(key.clone()), |c| c.map_key(key, key_type))?;
                    let v =
                        self.nested(Segment::Key(key.clone()), |c| c.convert(value, value_type))?;
                    map.insert(k, v);
                }
            }
            Json::Array(pairs) => {
                for (index, pair) in pairs.iter().enumerate() {
                    let (k, v) = self.nested(Segment::Index(index), |c| match pair {
                        Json::Array(elements) => match elements.as_slice() {
                            [key, value] => Ok((
                                c.nested(Segment::Index(0), |c| c.convert(key, key_type))?,
                                c.nested(Segment::Index(1), |c| c.convert(value, value_type))?,
                            )),
                            _ => Err(c.mismatch("a [key, value] pair", pair)),
                        },
                        other => Err(c.mismatch("a [key, value] pair", other)),
                    })?;
                    map.insert(k, v);
                }
            }
            other => {
                return Err(self.mismatch(
                    format!(
                        "a map of {} to {}",
                        type_name(key_type),
                        type_name(value_type)
                    ),
                    other,
                ))
            }
        }
        Ok(Value::Map(map))
    }

    /// Converts the key of a JSON object to a map key of the given type.
    fn map_key(&self, key: &str, ty: Option<&Type>) -> Result<Value<'static>, Error> {
        let Some(ty) = ty else {
            return Ok(Value::Dynamic(Box::new(Value::String(
                key.to_owned().into(),
            ))));
        };
        let value = match ty {
            Type::String => Some(Value::String(key.to_owned().into())),
            Type::Bool => key.parse::<bool>().ok().map(Value::Bool),
            Type::Int8
            | Type::UInt8
            | Type::Int16
            | Type::UInt16
            | Type::Int32
            | Type::UInt32
            | Type::Int64
            | Type::UInt64 => key.parse::<i128>().ok().and_then(|v| integer_value(ty, v)),
            Type::Float32 => key.parse::<f32>().ok().map(IntoValue::into_value),
            Type::Float64 => key.parse::<f64>().ok().map(IntoValue::into_value),
            _ => None,
        };
        value.ok_or_else(|| {
            self.error(ErrorKind::Key {
                key: key.to_owned(),
                expected: ty.to_string(),
            })
        })
    }

    fn tuple(&mut self, json: &Json, tuple: &Tuple) -> Result<Value<'static>, Error> {
        let types = tuple.element_types();
        let names = tuple.field_names();
        match json {
            Json::Array(items) => {
                if items.len() != types.len() {
                    return Err(self.error(ErrorKind::Length {
                        expected: types.len(),
                        found: items.len(),
                    }));
                }
                items
                    .iter()
                    .zip(&types)
                    .enumerate()
                    .map(|(index, (item, ty))| {
                        let segment = names
                            .as_ref()
                            .and_then(|names| names.get(index))
                            .map_or(Segment::Index(index), |name| Segment::Field(name.clone()));
                        self.nested(segment, |c| c.convert(item, ty.as_ref()))
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(Value::Tuple)
            }
            Json::Object(entries) => {
                let Some(names) = names else {
                    return Err(self.mismatch(
                        format!("a tuple {} as a JSON array", tuple_name(&types)),
                        json,
                    ));
                };
                if let Some(unknown) = entries.keys().find(|key| !names.contains(key)) {
                    return Err(self.error(ErrorKind::UnknownField(unknown.clone())));
                }
                names
                    .iter()
                    .zip(&types)
                    .map(|(name, ty)| match entries.get(name) {
                        Some(value) => self.nested(Segment::Field(name.clone()), |c| {
                            c.convert(value, ty.as_ref())
                        }),
                        // Absent optional fields are none.
                        None if matches!(ty, Some(Type::Option(_))) => Ok(Value::Option(None)),
                        None => Err(self.error(ErrorKind::MissingField(name.clone()))),
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(Value::Tuple)
            }
            other => Err(self.mismatch(format!("a tuple {}", tuple_name(&types)), other)),
        }
    }
}

/// Infers a value from a JSON document, for dynamic types.
fn infer(json: &Json) -> Value<'static> {
    match json {
        Json::Null => Value::Unit,
        Json::Bool(v) => Value::Bool(*v),
        Json::Number(number) => {
            if let Some(v) = number.as_i64() {
                i32::try_from(v).map_or(Value::Int64(v), Value::Int32)
            } else if let Some(v) = number.as_u64() {
                Value::UInt64(v)
            } else {
                number
                    .as_f64()
                    .map(IntoValue::into_value)
                    .unwrap_or_default()
            }
        }
        Json::String(v) => Value::String(v.clone().into()),
        Json::Array(items) => Value::List(homogenize(items.iter().map(infer).collect())),
        Json::Object(entries) => {
            let values = homogenize(entries.values().map(infer).collect());
            Value::Map(
                entries
                    .keys()
                    .map(|key| Value::String(key.clone().into()))
                    .zip(values)
                    .collect(),
            )
        }
    }
}

/// Makes the elements of a container suitable for serialization: elements of different types must
/// be dynamic values, so that each one carries its own signature.
fn homogenize(values: Vec<Value<'static>>) -> Vec<Value<'static>> {
    let mut types = values.iter().map(RuntimeReflect::ty);
    let first = types.next();
    if first.is_none_or(|first| types.all(|ty| ty == first)) {
        values
    } else {
        values
            .into_iter()
            .map(|value| Value::Dynamic(Box::new(value)))
            .collect()
    }
}

/// Makes an integer value of the given type, if the number is representable in it.
fn integer_value(ty: &Type, v: i128) -> Option<Value<'static>> {
    Some(match ty {
        Type::Int8 => Value::Int8(i8::try_from(v).ok()?),
        Type::UInt8 => Value::UInt8(u8::try_from(v).ok()?),
        Type::Int16 => Value::Int16(i16::try_from(v).ok()?),
        Type::UInt16 => Value::UInt16(u16::try_from(v).ok()?),
        Type::Int32 => Value::Int32(i32::try_from(v).ok()?),
        Type::UInt32 => Value::UInt32(u32::try_from(v).ok()?),
        Type::Int64 => Value::Int64(i64::try_from(v).ok()?),
        Type::UInt64 => Value::UInt64(u64::try_from(v).ok()?),
        _ => return None,
    })
}

/// Describes a JSON document for error messages.
fn describe(json: &Json) -> String {
    match json {
        Json::Null => "null".to_owned(),
        Json::Bool(v) => format!("boolean {v}"),
        Json::Number(v) => format!("number {v}"),
        Json::String(_) => format!("string {json}"),
        Json::Array(items) => format!("an array of {} element(s)", items.len()),
        Json::Object(entries) => format!("an object with {} field(s)", entries.len()),
    }
}

fn type_name(ty: Option<&Type>) -> String {
    qi::value::ty::DisplayOption(&ty).to_string()
}

fn tuple_name(types: &[Option<Type>]) -> String {
    let names: Vec<_> = types.iter().map(|ty| type_name(ty.as_ref())).collect();
    format!("({})", names.join(","))
}

fn is_string(ty: Option<&Type>) -> bool {
    match ty {
        Some(Type::String) => true,
        Some(Type::Option(element)) => is_string(element.as_deref()),
        _ => false,
    }
}

fn option_element(ty: Option<&Type>) -> Option<&Type> {
    match ty {
        Some(Type::Option(element)) => element.as_deref(),
        _ => None,
    }
}

fn list_element(ty: Option<&Type>) -> Option<&Type> {
    match ty {
        Some(Type::List(element) | Type::VarArgs(element)) => element.as_deref(),
        _ => None,
    }
}

fn float64_json(v: f64) -> Json {
    serde_json::Number::from_f64(v).map_or_else(|| Json::String(non_finite(v)), Json::Number)
}

/// Single precision floats are printed with their own shortest representation (`0.1`, not the
/// `0.10000000149011612` of the equivalent double).
fn float32_json(v: f32) -> Json {
    if !v.is_finite() {
        return Json::String(non_finite(f64::from(v)));
    }
    v.to_string()
        .parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
        .map_or(Json::Null, Json::Number)
}

fn non_finite(v: f64) -> String {
    if v.is_nan() {
        "NaN".to_owned()
    } else if v.is_sign_negative() {
        "-inf".to_owned()
    } else {
        "inf".to_owned()
    }
}

/// Encodes raw data as base64, the representation of raw values in JSON.
pub(crate) fn base64(bytes: &[u8]) -> String {
    BASE64.encode(bytes)
}

/// Strings of the type system may hold non UTF-8 data.
pub(crate) fn string_lossy(v: &QiString<'_>) -> String {
    v.as_str().map_or_else(
        || String::from_utf8_lossy(v.as_bytes()).into_owned(),
        ToOwned::to_owned,
    )
}

/// The JSON object key of a map key, if the key has a natural string representation.
fn key_string(key: &Value<'_>) -> Option<String> {
    match key {
        Value::String(v) => Some(string_lossy(v)),
        Value::Bool(v) => Some(v.to_string()),
        Value::Int8(v) => Some(v.to_string()),
        Value::UInt8(v) => Some(v.to_string()),
        Value::Int16(v) => Some(v.to_string()),
        Value::UInt16(v) => Some(v.to_string()),
        Value::Int32(v) => Some(v.to_string()),
        Value::UInt32(v) => Some(v.to_string()),
        Value::Int64(v) => Some(v.to_string()),
        Value::UInt64(v) => Some(v.to_string()),
        Value::Float32(v) => Some(v.to_string()),
        Value::Float64(v) => Some(v.to_string()),
        Value::Dynamic(inner) => key_string(inner),
        _ => None,
    }
}

fn map_json(map: &Map<Value<'_>, Value<'_>>, ty: Option<&Type>) -> Json {
    let (key_type, value_type) = match ty {
        Some(Type::Map { key, value }) => (key.as_deref(), value.as_deref()),
        _ => (None, None),
    };
    let keys: Option<Vec<String>> = map.keys().map(key_string).collect();
    match keys {
        Some(keys) => Json::Object(
            keys.into_iter()
                .zip(map.values())
                .map(|(key, value)| (key, from_value(value, value_type)))
                .collect(),
        ),
        None => Json::Array(
            map.iter()
                .map(|(key, value)| {
                    Json::Array(vec![
                        from_value(key, key_type),
                        from_value(value, value_type),
                    ])
                })
                .collect(),
        ),
    }
}

fn tuple_json(items: &[Value<'_>], ty: Option<&Type>) -> Json {
    match ty {
        Some(Type::Tuple(Tuple::Struct { fields, .. })) if fields.len() == items.len() => {
            Json::Object(
                fields
                    .iter()
                    .zip(items)
                    .map(|(field, item)| (field.name.clone(), from_value(item, field.ty.as_ref())))
                    .collect(),
            )
        }
        Some(Type::Tuple(tuple)) if tuple.len() == items.len() => Json::Array(
            items
                .iter()
                .zip(tuple.element_types())
                .map(|(item, ty)| from_value(item, ty.as_ref()))
                .collect(),
        ),
        _ => Json::Array(items.iter().map(|item| from_value(item, None)).collect()),
    }
}

/// Objects are summarized: their address, identity and the names of their members.
fn object_json(object: &Object) -> Json {
    let meta = &object.meta_object;
    let names =
        |names: Vec<&String>| Json::Array(names.into_iter().cloned().map(Json::String).collect());
    serde_json::json!({
        "object": {
            "serviceId": u32::from(object.service_id),
            "objectId": u32::from(object.object_id),
            "objectUid": object.object_uid.to_string(),
            "description": meta.description,
            "methods": names(meta.methods.values().map(|m| &m.name).collect()),
            "signals": names(meta.signals.values().map(|s| &s.name).collect()),
            "properties": names(meta.properties.values().map(|p| &p.name).collect()),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qi::value::Signature;

    fn json(text: &str) -> Json {
        serde_json::from_str(text).unwrap()
    }

    fn convert(text: &str, ty: Type) -> Result<Value<'static>, Error> {
        to_value(&json(text), Some(&ty))
    }

    fn dynamic(text: &str) -> Value<'static> {
        match to_value(&json(text), None).unwrap() {
            Value::Dynamic(inner) => *inner,
            other => other,
        }
    }

    fn string(text: &str) -> Value<'static> {
        Value::String(text.to_owned().into())
    }

    fn point_type() -> Type {
        Type::struct_of(
            "Point",
            [
                ("x", Some(Type::Int32)),
                ("y", Some(Type::Int32)),
                ("label", Some(Type::option_of(Type::String))),
            ],
        )
    }

    #[test]
    fn integers_convert_when_representable() {
        assert_eq!(convert("1", Type::Int32).unwrap(), Value::Int32(1));
        assert_eq!(convert("-1", Type::Int8).unwrap(), Value::Int8(-1));
        assert_eq!(convert("255", Type::UInt8).unwrap(), Value::UInt8(255));
        assert_eq!(
            convert("65535", Type::UInt16).unwrap(),
            Value::UInt16(u16::MAX)
        );
        assert_eq!(
            convert("-32768", Type::Int16).unwrap(),
            Value::Int16(i16::MIN)
        );
        assert_eq!(
            convert("4294967295", Type::UInt32).unwrap(),
            Value::UInt32(u32::MAX)
        );
        assert_eq!(
            convert("-9223372036854775808", Type::Int64).unwrap(),
            Value::Int64(i64::MIN)
        );
        assert_eq!(
            convert("18446744073709551615", Type::UInt64).unwrap(),
            Value::UInt64(u64::MAX)
        );
        // Integral floats are accepted.
        assert_eq!(convert("2.0", Type::Int32).unwrap(), Value::Int32(2));

        let err = convert("-1", Type::UInt8).unwrap_err().to_string();
        assert_eq!(err, "the number -1 is not representable as uint8");
        assert!(convert("300", Type::Int8).is_err());
        assert!(convert("2.5", Type::Int32).is_err());
        assert!(convert("18446744073709551616", Type::UInt64).is_err());
        let err = convert("\"1\"", Type::Int32).unwrap_err().to_string();
        assert_eq!(err, "expected an integer (int32), found string \"1\"");
        assert!(convert("true", Type::Int64).is_err());
    }

    #[test]
    fn floats_convert_from_any_number() {
        assert_eq!(convert("1", Type::Float32).unwrap(), 1.0f32.into_value());
        assert_eq!(convert("1.5", Type::Float64).unwrap(), 1.5f64.into_value());
        assert_eq!(
            convert("-2", Type::Float64).unwrap(),
            (-2.0f64).into_value()
        );
        assert!(
            matches!(convert("\"NaN\"", Type::Float64).unwrap(), Value::Float64(v) if v.is_nan())
        );
        assert_eq!(
            convert("\"inf\"", Type::Float32).unwrap(),
            f32::INFINITY.into_value()
        );
        assert_eq!(
            convert("\"-inf\"", Type::Float64).unwrap(),
            f64::NEG_INFINITY.into_value()
        );
        assert!(convert("true", Type::Float32).is_err());
        assert!(convert("\"abc\"", Type::Float32).is_err());
    }

    #[test]
    fn scalars_convert_strictly() {
        assert_eq!(convert("true", Type::Bool).unwrap(), Value::Bool(true));
        assert!(convert("1", Type::Bool).is_err());
        assert_eq!(convert("\"abc\"", Type::String).unwrap(), string("abc"));
        let err = convert("1", Type::String).unwrap_err().to_string();
        assert_eq!(err, "expected a string, found number 1");
        assert_eq!(convert("null", Type::Unit).unwrap(), Value::Unit);
        assert!(convert("0", Type::Unit).is_err());
        let err = convert("{}", Type::Object).unwrap_err().to_string();
        assert_eq!(err, "objects cannot be given as JSON");
    }

    #[test]
    fn raw_converts_from_base64_or_bytes() {
        assert_eq!(
            convert("\"AAEC\"", Type::Raw).unwrap(),
            Value::Raw(vec![0, 1, 2].into())
        );
        assert_eq!(
            convert("\"\"", Type::Raw).unwrap(),
            Value::Raw(Vec::new().into())
        );
        assert_eq!(
            convert("[0, 1, 255]", Type::Raw).unwrap(),
            Value::Raw(vec![0, 1, 255].into())
        );
        let err = convert("[0, 256]", Type::Raw).unwrap_err().to_string();
        assert_eq!(err, "at [1]: expected a byte (0-255), found number 256");
        let err = convert("\"!!!\"", Type::Raw).unwrap_err().to_string();
        assert_eq!(err, "invalid base64 raw data");
        assert!(convert("1", Type::Raw).is_err());
    }

    #[test]
    fn options_convert_from_null_or_value() {
        let ty = Type::option_of(Type::Int32);
        assert_eq!(convert("null", ty.clone()).unwrap(), Value::Option(None));
        assert_eq!(
            convert("3", ty.clone()).unwrap(),
            Value::Option(Some(Box::new(Value::Int32(3))))
        );
        assert!(convert("\"x\"", ty).is_err());
        // Dynamic options hold dynamic values.
        assert_eq!(
            convert("3", Type::option_of(None)).unwrap(),
            Value::Option(Some(Box::new(Value::Dynamic(Box::new(Value::Int32(3))))))
        );
    }

    #[test]
    fn lists_convert_elementwise() {
        assert_eq!(
            convert("[1, 2]", Type::list_of(Type::Int16)).unwrap(),
            Value::List(vec![Value::Int16(1), Value::Int16(2)])
        );
        assert_eq!(
            convert("[]", Type::list_of(Type::Int16)).unwrap(),
            Value::List(vec![])
        );
        assert_eq!(
            convert("[[1], []]", Type::list_of(Type::list_of(Type::UInt8))).unwrap(),
            Value::List(vec![
                Value::List(vec![Value::UInt8(1)]),
                Value::List(vec![])
            ])
        );
        assert_eq!(
            convert("[1, 2]", Type::varargs_of(Type::Int64)).unwrap(),
            Value::List(vec![Value::Int64(1), Value::Int64(2)])
        );
        // Elements of a list of dynamics are dynamic values.
        assert_eq!(
            convert("[1, \"a\"]", Type::list_of(None)).unwrap(),
            Value::List(vec![
                Value::Dynamic(Box::new(Value::Int32(1))),
                Value::Dynamic(Box::new(string("a"))),
            ])
        );
        let err = convert("[1, \"x\"]", Type::list_of(Type::Int16))
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "at [1]: expected an integer (int16), found string \"x\""
        );
        let err = convert("{}", Type::list_of(Type::Int16))
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "expected a list of int16, found an object with 0 field(s)"
        );
    }

    #[test]
    fn maps_convert_from_objects_or_pairs() {
        let mut expected = Map::new();
        expected.insert(string("a"), Value::Int32(1));
        expected.insert(string("b"), Value::Int32(2));
        assert_eq!(
            convert(
                "{\"a\": 1, \"b\": 2}",
                Type::map_of(Type::String, Type::Int32)
            )
            .unwrap(),
            Value::Map(expected)
        );

        // Integer keys are parsed from the object keys.
        let mut expected = Map::new();
        // JSON object keys are sorted: the map follows that order.
        expected.insert(Value::Int32(-2), string("y"));
        expected.insert(Value::Int32(1), string("x"));
        assert_eq!(
            convert(
                "{\"1\": \"x\", \"-2\": \"y\"}",
                Type::map_of(Type::Int32, Type::String)
            )
            .unwrap(),
            Value::Map(expected)
        );
        let err = convert("{\"a\": \"x\"}", Type::map_of(Type::Int32, Type::String))
            .unwrap_err()
            .to_string();
        assert_eq!(err, "at [\"a\"]: invalid map key \"a\": expected int32");
        let err = convert("{\"1\": 2}", Type::map_of(Type::Int32, Type::String))
            .unwrap_err()
            .to_string();
        assert_eq!(err, "at [\"1\"]: expected a string, found number 2");

        // Boolean and float keys.
        let mut expected = Map::new();
        expected.insert(Value::Bool(true), Value::Unit);
        assert_eq!(
            convert("{\"true\": null}", Type::map_of(Type::Bool, Type::Unit)).unwrap(),
            Value::Map(expected)
        );
        let mut expected = Map::new();
        expected.insert(1.5f64.into_value(), Value::Int8(1));
        assert_eq!(
            convert("{\"1.5\": 1}", Type::map_of(Type::Float64, Type::Int8)).unwrap(),
            Value::Map(expected)
        );

        // Any key type as an array of pairs.
        let key_type = Type::tuple_of([Type::Int32, Type::Int32]);
        let mut expected = Map::new();
        expected.insert(
            Value::Tuple(vec![Value::Int32(1), Value::Int32(2)]),
            string("x"),
        );
        assert_eq!(
            convert(
                "[[[1, 2], \"x\"]]",
                Type::map_of(key_type.clone(), Type::String)
            )
            .unwrap(),
            Value::Map(expected)
        );
        let err = convert("[[1, 2, 3]]", Type::map_of(key_type, Type::String))
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "at [0]: expected a [key, value] pair, found an array of 3 element(s)"
        );

        // Dynamic keys and values.
        let mut expected = Map::new();
        expected.insert(
            Value::Dynamic(Box::new(string("k"))),
            Value::Dynamic(Box::new(Value::Bool(true))),
        );
        assert_eq!(
            convert("{\"k\": true}", Type::map_of(None, None)).unwrap(),
            Value::Map(expected)
        );
        let err = convert("1", Type::map_of(Type::String, None))
            .unwrap_err()
            .to_string();
        assert_eq!(err, "expected a map of string to dynamic, found number 1");
    }

    #[test]
    fn tuples_convert_from_arrays() {
        let ty = Type::tuple_of([Some(Type::Int32), Some(Type::String), None]);
        assert_eq!(
            convert("[1, \"a\", true]", ty.clone()).unwrap(),
            Value::Tuple(vec![
                Value::Int32(1),
                string("a"),
                Value::Dynamic(Box::new(Value::Bool(true)))
            ])
        );
        let err = convert("[1, \"a\"]", ty.clone()).unwrap_err().to_string();
        assert_eq!(err, "expected a tuple of 3 element(s), found 2");
        let err = convert("[1, 2, 3]", ty.clone()).unwrap_err().to_string();
        assert_eq!(err, "at [1]: expected a string, found number 2");
        let err = convert("{\"a\": 1}", ty).unwrap_err().to_string();
        assert_eq!(
            err,
            "expected a tuple (int32,string,dynamic) as a JSON array, found an object with 1 field(s)"
        );
        assert_eq!(
            convert("[]", Type::unit_tuple()).unwrap(),
            Value::Tuple(vec![])
        );
        // Tuple structures are positional too.
        let ty = Type::tuple_struct_of("Pair", [Type::Int8, Type::Int8]);
        assert_eq!(
            convert("[1, 2]", ty.clone()).unwrap(),
            Value::Tuple(vec![Value::Int8(1), Value::Int8(2)])
        );
        assert!(convert("{\"0\": 1, \"1\": 2}", ty).is_err());
    }

    #[test]
    fn structures_convert_from_arrays_or_objects() {
        let expected = Value::Tuple(vec![
            Value::Int32(1),
            Value::Int32(2),
            Value::Option(Some(Box::new(string("origin")))),
        ]);
        assert_eq!(
            convert("[1, 2, \"origin\"]", point_type()).unwrap(),
            expected
        );
        assert_eq!(
            convert("{\"y\": 2, \"x\": 1, \"label\": \"origin\"}", point_type()).unwrap(),
            expected
        );
        // Absent optional fields are none.
        assert_eq!(
            convert("{\"x\": 1, \"y\": 2}", point_type()).unwrap(),
            Value::Tuple(vec![Value::Int32(1), Value::Int32(2), Value::Option(None)])
        );
        let err = convert("{\"x\": 1}", point_type()).unwrap_err().to_string();
        assert_eq!(err, "missing field \"y\"");
        let err = convert("{\"x\": 1, \"y\": 2, \"z\": 3}", point_type())
            .unwrap_err()
            .to_string();
        assert_eq!(err, "unknown field \"z\"");
        let err = convert("{\"x\": 1, \"y\": \"2\"}", point_type())
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "at .y: expected an integer (int32), found string \"2\""
        );
        // Positional errors name the fields too.
        let err = convert("[1, \"2\", null]", point_type())
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "at .y: expected an integer (int32), found string \"2\""
        );
        // Nested paths.
        let err = convert("[{\"x\": 1, \"y\": [2]}]", Type::list_of(point_type()))
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "at [0].y: expected an integer (int32), found an array of 1 element(s)"
        );
    }

    #[test]
    fn dynamic_values_are_inferred() {
        assert_eq!(dynamic("null"), Value::Unit);
        assert_eq!(dynamic("true"), Value::Bool(true));
        assert_eq!(dynamic("1"), Value::Int32(1));
        assert_eq!(dynamic("-2147483648"), Value::Int32(i32::MIN));
        assert_eq!(dynamic("2147483648"), Value::Int64(i64::from(i32::MAX) + 1));
        assert_eq!(dynamic("18446744073709551615"), Value::UInt64(u64::MAX));
        assert_eq!(dynamic("1.5"), 1.5f64.into_value());
        assert_eq!(dynamic("1.0"), 1.0f64.into_value());
        assert_eq!(dynamic("\"a\""), string("a"));
        // The result is wrapped as a dynamic value.
        assert_eq!(
            to_value(&json("1"), None).unwrap(),
            Value::Dynamic(Box::new(Value::Int32(1)))
        );
    }

    #[test]
    fn inferred_containers_have_consistent_signatures() {
        let signature = |value: &Value<'_>| Signature::from(value.ty()).to_string();

        let list = dynamic("[1, 2]");
        assert_eq!(list, Value::List(vec![Value::Int32(1), Value::Int32(2)]));
        assert_eq!(signature(&list), "[i]");

        // Heterogeneous elements are dynamic, so that each carries its signature.
        let list = dynamic("[1, \"a\"]");
        assert_eq!(
            list,
            Value::List(vec![
                Value::Dynamic(Box::new(Value::Int32(1))),
                Value::Dynamic(Box::new(string("a"))),
            ])
        );
        assert_eq!(signature(&list), "[m]");

        let nested = dynamic("[[1, \"a\"], [2, \"b\"]]");
        assert_eq!(signature(&nested), "[[m]]");
        let nested = dynamic("[[1, 2], [3, \"b\"]]");
        assert_eq!(signature(&nested), "[m]");
        assert!(
            matches!(&nested, Value::List(items) if items.iter().all(|item| matches!(item, Value::Dynamic(_))))
        );
        assert_eq!(signature(&dynamic("[]")), "[m]");

        let mut expected = Map::new();
        expected.insert(string("a"), Value::Int32(1));
        let map = dynamic("{\"a\": 1}");
        assert_eq!(map, Value::Map(expected));
        assert_eq!(signature(&map), "{si}");

        let mut expected = Map::new();
        expected.insert(string("a"), Value::Dynamic(Box::new(Value::Int32(1))));
        expected.insert(string("b"), Value::Dynamic(Box::new(Value::Bool(true))));
        let map = dynamic("{\"a\": 1, \"b\": true}");
        assert_eq!(map, Value::Map(expected));
        assert_eq!(signature(&map), "{sm}");
    }

    #[test]
    fn arguments_are_json_or_plain_strings() {
        let arg = |text: &str, ty: Option<Type>| parse_arg(text, ty.as_ref());
        assert_eq!(arg("hello", Some(Type::String)).unwrap(), string("hello"));
        assert_eq!(
            arg("\"hello\"", Some(Type::String)).unwrap(),
            string("hello")
        );
        assert_eq!(arg("123", Some(Type::String)).unwrap(), string("123"));
        assert_eq!(arg("[1]", Some(Type::String)).unwrap(), string("[1]"));
        assert_eq!(arg("null", Some(Type::String)).unwrap(), string("null"));
        assert_eq!(arg("123", Some(Type::Int32)).unwrap(), Value::Int32(123));
        assert_eq!(
            arg("123", None).unwrap(),
            Value::Dynamic(Box::new(Value::Int32(123)))
        );
        assert_eq!(
            arg("\"123\"", None).unwrap(),
            Value::Dynamic(Box::new(string("123")))
        );
        assert_eq!(
            arg("hello", None).unwrap(),
            Value::Dynamic(Box::new(string("hello")))
        );
        let err = arg("nope", Some(Type::Int32)).unwrap_err().to_string();
        assert_eq!(err, "invalid JSON");
        let err = arg("\"x\"", Some(Type::Int32)).unwrap_err().to_string();
        assert_eq!(err, "expected an integer (int32), found string \"x\"");
        // Optional strings.
        let ty = Type::option_of(Type::String);
        assert_eq!(arg("null", Some(ty.clone())).unwrap(), Value::Option(None));
        assert_eq!(
            arg("x", Some(ty)).unwrap(),
            Value::Option(Some(Box::new(string("x"))))
        );
        // Raw data is given as base64 (a JSON string).
        assert_eq!(
            arg("AAEC", Some(Type::Raw)).unwrap_err().to_string(),
            "invalid JSON"
        );
        assert_eq!(
            arg("\"AAEC\"", Some(Type::Raw)).unwrap(),
            Value::Raw(vec![0, 1, 2].into())
        );
    }

    #[test]
    fn values_convert_to_json() {
        let to_json = |value: Value<'static>| from_value(&value, None);
        assert_eq!(to_json(Value::Unit), Json::Null);
        assert_eq!(to_json(Value::Bool(true)), json("true"));
        assert_eq!(to_json(Value::Int8(-1)), json("-1"));
        assert_eq!(
            to_json(Value::UInt64(u64::MAX)),
            json("18446744073709551615")
        );
        assert_eq!(to_json(0.1f32.into_value()).to_string(), "0.1");
        assert_eq!(to_json(3.4e38f32.into_value()).to_string(), "3.4e+38");
        assert_eq!(to_json(0.1f64.into_value()).to_string(), "0.1");
        assert_eq!(to_json(f64::NAN.into_value()), json("\"NaN\""));
        assert_eq!(to_json(f32::INFINITY.into_value()), json("\"inf\""));
        assert_eq!(to_json(f64::NEG_INFINITY.into_value()), json("\"-inf\""));
        assert_eq!(to_json(string("a")), json("\"a\""));
        assert_eq!(
            to_json(Value::String(vec![0xff, b'a'].into())),
            json("\"\u{fffd}a\"")
        );
        assert_eq!(to_json(Value::Raw(vec![0, 1, 2].into())), json("\"AAEC\""));
        assert_eq!(to_json(Value::Option(None)), Json::Null);
        assert_eq!(
            to_json(Value::Option(Some(Box::new(Value::Int32(1))))),
            json("1")
        );
        assert_eq!(
            to_json(Value::List(vec![Value::Int32(1), Value::Int32(2)])),
            json("[1, 2]")
        );
        assert_eq!(
            to_json(Value::Tuple(vec![Value::Int32(1), string("a")])),
            json("[1, \"a\"]")
        );
        assert_eq!(
            to_json(Value::Dynamic(Box::new(Value::Int32(1)))),
            json("1")
        );
    }

    #[test]
    fn maps_convert_to_objects_or_pairs() {
        let mut map = Map::new();
        map.insert(string("a"), Value::Int32(1));
        assert_eq!(from_value(&Value::Map(map), None), json("{\"a\": 1}"));

        let mut map = Map::new();
        map.insert(Value::Int32(1), string("x"));
        map.insert(Value::Dynamic(Box::new(Value::Int32(2))), string("y"));
        assert_eq!(
            from_value(&Value::Map(map), None),
            json("{\"1\": \"x\", \"2\": \"y\"}")
        );

        let mut map = Map::new();
        map.insert(
            Value::Tuple(vec![Value::Int32(1), Value::Int32(2)]),
            string("x"),
        );
        assert_eq!(
            from_value(&Value::Map(map), None),
            json("[[[1, 2], \"x\"]]")
        );
    }

    #[test]
    fn structures_convert_to_objects_when_typed() {
        let point = Value::Tuple(vec![Value::Int32(1), Value::Int32(2), Value::Option(None)]);
        assert_eq!(
            from_value(&point, Some(&point_type())),
            json("{\"x\": 1, \"y\": 2, \"label\": null}")
        );
        assert_eq!(from_value(&point, None), json("[1, 2, null]"));
        // Nested in lists and options.
        assert_eq!(
            from_value(
                &Value::List(vec![point.clone()]),
                Some(&Type::list_of(point_type()))
            ),
            json("[{\"x\": 1, \"y\": 2, \"label\": null}]")
        );
        assert_eq!(
            from_value(
                &Value::Option(Some(Box::new(point.clone()))),
                Some(&Type::option_of(point_type()))
            ),
            json("{\"x\": 1, \"y\": 2, \"label\": null}")
        );
        // A type of the wrong length is ignored.
        assert_eq!(
            from_value(&point, Some(&Type::tuple_of([Type::Int32]))),
            json("[1, 2, null]")
        );
    }

    #[test]
    fn objects_convert_to_summaries() {
        let mut builder = qi::object::MetaObject::builder();
        builder.add_method({
            let mut method = qi::object::MetaMethod::builder(qi::object::ActionId(100));
            method.set_name("add");
            method.build()
        });
        let mut meta = builder.build();
        meta.description = "An object".to_owned();
        let object = Object::new(
            meta,
            qi::service::Id(2),
            qi::object::Id(3),
            qi::object::Uid::from_bytes([0; 20]),
        );
        let json = from_value(&Value::Object(Box::new(object)), Some(&Type::Object));
        assert_eq!(json["object"]["serviceId"], Json::from(2));
        assert_eq!(json["object"]["objectId"], Json::from(3));
        assert_eq!(json["object"]["description"], Json::from("An object"));
        assert_eq!(
            json["object"]["methods"],
            Json::Array(vec![Json::from("add")])
        );
    }

    #[test]
    fn typed_conversions_round_trip() {
        let ty = Type::struct_of(
            "Record",
            [
                ("id", Some(Type::UInt32)),
                ("ratio", Some(Type::Float32)),
                ("tags", Some(Type::list_of(Type::String))),
                ("scores", Some(Type::map_of(Type::Int16, Type::Float64))),
                ("data", Some(Type::Raw)),
                ("point", Some(point_type())),
                ("extra", None),
            ],
        );
        let document = json(
            r#"{
                "id": 7,
                "ratio": 0.5,
                "tags": ["a", "b"],
                "scores": {"1": 1.5, "2": 2.5},
                "data": "AAEC",
                "point": {"x": 1, "y": 2, "label": "p"},
                "extra": [1, "mixed"]
            }"#,
        );
        let value = to_value(&document, Some(&ty)).unwrap();
        assert_eq!(from_value(&value, Some(&ty)), document);
    }
}
