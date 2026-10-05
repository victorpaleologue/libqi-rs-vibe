use crate::value::{AsDynamicOwned, FromValue, FromValueError, IntoValue, Reflect, Type, Value};
use serde_with::serde_as;
use std::collections::BTreeMap;

/// A map of string keys to dynamic values, of signature `{sm}`.
///
/// This is the type of capability maps and authentication parameters of the protocol.
///
/// # Order
///
/// Entries are kept sorted by key. This is the order the reference implementation (a
/// `std::map<std::string, AnyValue>`) uses on the wire, so that a map serialized by this
/// implementation is byte-identical to the one the reference implementation would produce.
#[serde_as]
#[derive(
    Default,
    Debug,
    Clone,
    PartialEq,
    Eq,
    derive_more::Into,
    derive_more::From,
    derive_more::IntoIterator,
    serde::Serialize,
    serde::Deserialize,
)]
#[into_iterator(owned, ref, ref_mut)]
pub struct KeyDynValueMap(
    #[serde_as(as = "BTreeMap<_, AsDynamicOwned>")] BTreeMap<String, Value<'static>>,
);

impl KeyDynValueMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn as_map(&self) -> &BTreeMap<String, Value<'static>> {
        &self.0
    }

    pub fn as_map_mut(&mut self) -> &mut BTreeMap<String, Value<'static>> {
        &mut self.0
    }

    pub fn set<K, V>(&mut self, key: K, value: V)
    where
        K: Into<String>,
        V: IntoValue<'static>,
    {
        self.0.insert(key.into(), value.into_value());
    }

    pub fn remove<K>(&mut self, key: &K) -> Option<Value<'static>>
    where
        String: std::borrow::Borrow<K>,
        K: Ord + ?Sized,
    {
        self.0.remove(key)
    }

    pub fn get<K>(&self, key: &K) -> Option<&Value<'static>>
    where
        String: std::borrow::Borrow<K>,
        K: Ord + ?Sized,
    {
        self.0.get(key)
    }

    pub fn get_as<K, T>(&self, key: &K) -> Option<T>
    where
        String: std::borrow::Borrow<K>,
        K: Ord + ?Sized,
        T: FromValue<'static>,
    {
        self.0
            .get(key)
            .map(|value| value.clone().cast_into())
            .transpose()
            .unwrap_or_default()
    }
}

impl FromIterator<(String, Value<'static>)> for KeyDynValueMap {
    fn from_iter<T: IntoIterator<Item = (String, Value<'static>)>>(iter: T) -> Self {
        Self(FromIterator::from_iter(iter))
    }
}

impl Extend<(String, Value<'static>)> for KeyDynValueMap {
    fn extend<T: IntoIterator<Item = (String, Value<'static>)>>(&mut self, iter: T) {
        self.0.extend(iter)
    }
}

// TODO: derive Valuable
impl Reflect for KeyDynValueMap {
    fn ty() -> Option<crate::value::Type> {
        Some(Type::map_of(Type::String, None))
    }
}

impl<'a> IntoValue<'a> for KeyDynValueMap {
    fn into_value(self) -> Value<'a> {
        Value::Map(
            self.into_iter()
                .map(|(k, v)| (k.into_value(), v.into_dynamic()))
                .collect(),
        )
    }
}

impl<'a> FromValue<'a> for KeyDynValueMap {
    fn from_value(value: Value<'a>) -> Result<Self, crate::value::FromValueError> {
        match value {
            Value::Map(map) => map
                .into_iter()
                .map(|(k, v)| {
                    let v = match v {
                        Value::Dynamic(v) => *v,
                        v => v,
                    };
                    Ok((k.cast_into()?, v.into_owned()))
                })
                .collect(),
            _ => Err(FromValueError::TypeMismatch {
                expected: "a KeyDynValueMap".to_owned(),
                actual: value.to_string(),
            }),
        }
    }
}
