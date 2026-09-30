// TODO: #![deny(missing_docs)]
#![doc = include_str!("value/README.md")]

mod as_raw;
pub mod dynamic;
mod kv_map;
pub mod map;
pub mod object;
pub mod os;
mod reflect;
pub mod service;
pub mod signature;
pub mod ty;
// The `Value` type: `qi::value::value` was already a public path before the crates merged.
#[allow(clippy::module_inception)]
pub mod value;
mod wire;

#[doc(inline)]
pub use crate::value::{
    as_raw::AsRaw,
    dynamic::{AsDynamic, AsDynamicOwned, Dynamic},
    kv_map::KeyDynValueMap,
    map::Map,
    object::Object,
    reflect::{Reflect, RuntimeReflect},
    signature::Signature,
    ty::Type,
    value::{de, FromValue, FromValueError, IntoValue, String, ToValue, Value},
};

pub use wire::{FormatInto, IntoFormat};
