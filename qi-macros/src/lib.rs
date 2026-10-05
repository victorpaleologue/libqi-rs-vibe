#![allow(clippy::wrong_self_convention)]
mod object;
mod value;

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput, Error};

#[proc_macro_derive(Valuable, attributes(qi))]
pub fn proc_macro_derive_valuable(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    value::derive_impl(value::Trait::Valuable, input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

#[proc_macro_derive(Reflect, attributes(qi))]
pub fn proc_macro_derive_reflect(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    value::derive_impl(value::Trait::Reflect, input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

#[proc_macro_derive(ToValue, attributes(qi))]
pub fn proc_macro_derive_to_value(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    value::derive_impl(value::Trait::ToValue, input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

#[proc_macro_derive(IntoValue, attributes(qi))]
pub fn proc_macro_derive_into_value(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    value::derive_impl(value::Trait::IntoValue, input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

#[proc_macro_derive(FromValue, attributes(qi))]
pub fn proc_macro_derive_from_value(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    value::derive_impl(value::Trait::FromValue, input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// Declares the interface of an object type.
///
/// Applied to a trait, this attribute keeps the trait (adding `#[async_trait]` to it) and
/// generates:
///
/// - `<Trait>Object<T>`: an adapter that implements `qi::Object` for any `T: Trait`, so that
///   implementations of the trait can be published as services or passed as objects. Its meta
///   object is derived from the trait declaration.
/// - `<Trait>Client`: a typed proxy that implements the trait over any `qi::AnyObject`, local or
///   remote, and checks at construction that the object exposes the members of the interface.
///
/// # Members
///
/// - Methods are `async fn name(&self, ...) -> Result<T>`: their arguments and return value are
///   values of the `qi` type system (`qi::value::Reflect + FromValue + IntoValue`). A method may
///   be tagged `#[qi::method(name = "...")]` to set its name in the meta object.
/// - Signals are `#[qi::signal] fn name(&self) -> &qi::Signal<T>`.
/// - Properties are `#[qi::property] fn name(&self) -> &qi::Property<T>`.
///
/// Members get increasing identifiers from 100 in declaration order. Documentation comments
/// become the descriptions of the object and of its methods.
///
/// # Attribute arguments
///
/// - `case = "camelCase"` (or any casing of the value macros) converts the Rust identifiers of
///   the members to that case for their names in the meta object, keeping leading underscores
///   (hidden members).
/// - `crate = "path"` sets the path of the `qi` crate (default `::qi`).
///
/// # Example
///
/// ```ignore
/// use qi::{Property, Signal};
///
/// /// A counter.
/// #[qi::object(case = "camelCase")]
/// pub trait Counter {
///     /// Increments the counter and returns its new value.
///     async fn increment(&self, step: i32) -> qi::Result<i32>;
///     #[qi::signal]
///     fn changed(&self) -> &Signal<i32>;
///     #[qi::property]
///     fn value(&self) -> &Property<i32>;
/// }
///
/// struct MyCounter { changed: Signal<i32>, value: Property<i32> }
///
/// #[qi::async_trait]
/// impl Counter for MyCounter {
///     async fn increment(&self, step: i32) -> qi::Result<i32> {
///         let value = self.value.get().await? + step;
///         self.value.set(value).await?;
///         self.changed.emit(value);
///         Ok(value)
///     }
///     fn changed(&self) -> &Signal<i32> { &self.changed }
///     fn value(&self) -> &Property<i32> { &self.value }
/// }
///
/// // Publishing: `node.register_service("Counter", CounterObject::new(my_counter))`.
/// // Using: `let counter = CounterClient::new(node.service("Counter").await?)?;`
/// // then `counter.increment(2).await?` and `counter.changed().subscribe().await?`.
/// ```
#[proc_macro_attribute]
pub fn object(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as object::Args);
    let item = parse_macro_input!(item as syn::ItemTrait);
    object::expand(args, item)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}
