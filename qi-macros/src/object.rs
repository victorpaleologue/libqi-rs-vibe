//! Implementation of the `#[qi::object]` attribute macro.
//!
//! The macro turns a trait declaring the interface of an object into:
//!
//! - the trait itself, with `#[async_trait]` applied;
//! - `<Trait>Object<T>`, an adapter implementing `qi::Object` for any `T: Trait`, whose meta
//!   object is derived from the trait declaration;
//! - `<Trait>Client`, a typed proxy implementing `Trait` over any `qi::AnyObject`.

use convert_case::{Case, Casing};
use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{
    ext::IdentExt,
    parse::{Parse, ParseStream},
    parse_quote,
    punctuated::Punctuated,
    Attribute, Error, Expr, ExprLit, FnArg, GenericArgument, Ident, ItemTrait, Lit, LitStr, Meta,
    Pat, Path, PathArguments, Result, ReturnType, Token, TraitItem, TraitItemFn, Type,
};

/// The identifier of the first user member of an object, as in the reference implementation.
const FIRST_MEMBER_ID: u32 = 100;

/// The arguments of the attribute: `#[qi::object(crate = "...", case = "...")]`.
pub(super) struct Args {
    crate_path: Path,
    case: Option<Case<'static>>,
}

impl Parse for Args {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut crate_path: Path = parse_quote!(::qi);
        let mut case = None;
        if !input.is_empty() {
            for meta in Punctuated::<Meta, Token![,]>::parse_terminated(input)? {
                match meta {
                    Meta::NameValue(nv) if nv.path.is_ident("crate") => {
                        crate_path = lit_str(&nv.value)?.parse()?;
                    }
                    Meta::NameValue(nv) if nv.path.is_ident("case") => {
                        case = Some(super::value::parse_case(&lit_str(&nv.value)?)?);
                    }
                    other => {
                        return Err(Error::new_spanned(
                            other,
                            "unknown attribute, expected `crate = \"...\"` or `case = \"...\"`",
                        ))
                    }
                }
            }
        }
        Ok(Self { crate_path, case })
    }
}

fn lit_str(expr: &Expr) -> Result<LitStr> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) => Ok(s.clone()),
        other => Err(Error::new_spanned(other, "expected a string literal")),
    }
}

/// A member of the object interface.
struct Member {
    id: u32,
    /// The name of the member in the `qi` type system.
    name: String,
    /// The Rust identifier of the trait function.
    ident: Ident,
    description: String,
    kind: MemberKind,
}

enum MemberKind {
    Method(Method),
    Signal(Type),
    Property(Type),
}

struct Method {
    /// The parameters of the method, receiver excluded.
    params: Vec<(Ident, Type)>,
    /// The `Ok` type of the returned `Result`.
    ok_type: Type,
}

/// The kind of member a trait function declares, from its `#[qi::…]` tag.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tag {
    Method,
    Signal,
    Property,
}

/// The options of a member tag: `#[qi::method(name = "...")]`.
#[derive(Default)]
struct TagOptions {
    name: Option<String>,
}

pub(super) fn expand(args: Args, mut item: ItemTrait) -> Result<TokenStream> {
    if !item.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &item.generics,
            "`#[qi::object]` does not support generic traits",
        ));
    }
    let description = doc_string(&item.attrs);
    let mut members = Vec::new();
    let mut next_id = FIRST_MEMBER_ID;
    for trait_item in &mut item.items {
        if let TraitItem::Fn(func) = trait_item {
            let member = parse_member(func, next_id, args.case)?;
            next_id += 1;
            members.push(member);
        }
    }
    let has_async_trait = item.attrs.iter().any(|attr| {
        attr.path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "async_trait")
    });
    let qi = &args.crate_path;
    let async_trait_attr = (!has_async_trait).then(|| quote!(#[#qi::async_trait]));

    let vis = &item.vis;
    let trait_ident = &item.ident;
    let object_ident = format_ident!("{trait_ident}Object");
    let client_ident = format_ident!("{trait_ident}Client");
    let meta_fn_ident = format_ident!(
        "__{}_meta_object",
        trait_ident.unraw().to_string().to_case(Case::Snake)
    );

    let meta_object = expand_meta_object(qi, &meta_fn_ident, vis, &description, &members);
    let object = expand_object(
        qi,
        &object_ident,
        trait_ident,
        &meta_fn_ident,
        vis,
        &members,
    );
    let client = expand_client(
        qi,
        &client_ident,
        trait_ident,
        &meta_fn_ident,
        vis,
        &members,
    );

    Ok(quote! {
        #async_trait_attr
        #item

        #meta_object
        #object
        #client
    })
}

/// Parses a trait function into a member, stripping the `#[qi::…]` tag from its attributes.
fn parse_member(func: &mut TraitItemFn, id: u32, case: Option<Case<'static>>) -> Result<Member> {
    let (tag, options) = extract_tag(&mut func.attrs)?;
    let description = doc_string(&func.attrs);
    let ident = func.sig.ident.clone();
    let name = options
        .name
        .unwrap_or_else(|| convert_name(&ident.unraw().to_string(), case));

    let receiver_is_ref_self = matches!(
        func.sig.inputs.first(),
        Some(FnArg::Receiver(receiver)) if receiver.reference.is_some() && receiver.mutability.is_none()
    );
    if !receiver_is_ref_self {
        return Err(Error::new_spanned(
            &func.sig,
            "object members must take `&self`",
        ));
    }
    if !func.sig.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &func.sig.generics,
            "object members cannot be generic",
        ));
    }

    let kind = match tag {
        Tag::Method => MemberKind::Method(parse_method(func)?),
        Tag::Signal => MemberKind::Signal(parse_accessor(func, "Signal")?),
        Tag::Property => MemberKind::Property(parse_accessor(func, "Property")?),
    };
    Ok(Member {
        id,
        name,
        ident,
        description,
        kind,
    })
}

fn parse_method(func: &TraitItemFn) -> Result<Method> {
    if func.sig.asyncness.is_none() {
        return Err(Error::new_spanned(
            &func.sig,
            "object methods must be `async fn`; signals and properties are declared with \
             `#[qi::signal]` and `#[qi::property]`",
        ));
    }
    let params = func
        .sig
        .inputs
        .iter()
        .skip(1)
        .map(|input| match input {
            FnArg::Typed(pat_type) => match &*pat_type.pat {
                Pat::Ident(pat) => Ok((pat.ident.clone(), (*pat_type.ty).clone())),
                other => Err(Error::new_spanned(
                    other,
                    "object method parameters must be plain identifiers",
                )),
            },
            FnArg::Receiver(receiver) => Err(Error::new_spanned(receiver, "unexpected receiver")),
        })
        .collect::<Result<Vec<_>>>()?;
    let ok_type = match &func.sig.output {
        ReturnType::Type(_, ty) => result_ok_type(ty)?,
        ReturnType::Default => {
            return Err(Error::new_spanned(
                &func.sig,
                "object methods must return a `Result`",
            ))
        }
    };
    Ok(Method { params, ok_type })
}

/// Extracts `T` from a return type `Result<T>` or `Result<T, E>`.
fn result_ok_type(ty: &Type) -> Result<Type> {
    let error = || Error::new_spanned(ty, "object methods must return a `Result<T>`");
    let Type::Path(path) = ty else {
        return Err(error());
    };
    let segment = path.path.segments.last().ok_or_else(error)?;
    if segment.ident != "Result" {
        return Err(error());
    }
    match &segment.arguments {
        PathArguments::AngleBracketed(args) => match args.args.first() {
            Some(GenericArgument::Type(ok)) => Ok(ok.clone()),
            _ => Err(error()),
        },
        _ => Err(error()),
    }
}

/// Extracts `T` from the return type `&Signal<T>` or `&Property<T>` of an accessor.
fn parse_accessor(func: &TraitItemFn, wrapper: &str) -> Result<Type> {
    let error = |span: &dyn ToTokens| {
        Error::new_spanned(
            span,
            format!(
                "`#[qi::{}]` members must be `fn name(&self) -> &{wrapper}<T>`",
                wrapper.to_lowercase()
            ),
        )
    };
    if func.sig.asyncness.is_some() || func.sig.inputs.len() != 1 {
        return Err(error(&func.sig));
    }
    let ReturnType::Type(_, ty) = &func.sig.output else {
        return Err(error(&func.sig));
    };
    let Type::Reference(reference) = &**ty else {
        return Err(error(ty));
    };
    let Type::Path(path) = &*reference.elem else {
        return Err(error(ty));
    };
    let segment = path.path.segments.last().ok_or_else(|| error(ty))?;
    if segment.ident != wrapper {
        return Err(error(ty));
    }
    match &segment.arguments {
        PathArguments::AngleBracketed(args) => match args.args.first() {
            Some(GenericArgument::Type(inner)) => Ok(inner.clone()),
            _ => Err(error(ty)),
        },
        _ => Err(error(ty)),
    }
}

/// Finds and removes the `#[qi::method]`, `#[qi::signal]` or `#[qi::property]` tag of a
/// function. Untagged functions are methods.
fn extract_tag(attrs: &mut Vec<Attribute>) -> Result<(Tag, TagOptions)> {
    let mut found = None;
    let mut index = 0;
    while index < attrs.len() {
        let tag = tag_of(&attrs[index]);
        match tag {
            Some(tag) => {
                let attr = attrs.remove(index);
                if found.is_some() {
                    return Err(Error::new_spanned(attr, "duplicate member tag"));
                }
                found = Some((tag, tag_options(&attr)?));
            }
            None => index += 1,
        }
    }
    Ok(found.unwrap_or((Tag::Method, TagOptions::default())))
}

fn tag_of(attr: &Attribute) -> Option<Tag> {
    let segments: Vec<_> = attr
        .path()
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    let [first, second] = segments.as_slice() else {
        return None;
    };
    if first != "qi" {
        return None;
    }
    match second.as_str() {
        "method" => Some(Tag::Method),
        "signal" => Some(Tag::Signal),
        "property" => Some(Tag::Property),
        _ => None,
    }
}

fn tag_options(attr: &Attribute) -> Result<TagOptions> {
    let mut options = TagOptions::default();
    if let Meta::List(_) = &attr.meta {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                options.name = Some(meta.value()?.parse::<LitStr>()?.value());
                Ok(())
            } else {
                Err(meta.error("unknown attribute, expected `name = \"...\"`"))
            }
        })?;
    }
    Ok(options)
}

/// Converts a Rust identifier to a member name, preserving leading underscores (hidden members).
fn convert_name(ident: &str, case: Option<Case<'static>>) -> String {
    let Some(case) = case else {
        return ident.to_owned();
    };
    let stripped = ident.trim_start_matches('_');
    let underscores = &ident[..ident.len() - stripped.len()];
    format!("{underscores}{}", stripped.to_case(case))
}

/// The text of the documentation attributes, lines trimmed and joined.
fn doc_string(attrs: &[Attribute]) -> String {
    let lines: Vec<String> = attrs
        .iter()
        .filter_map(|attr| match &attr.meta {
            Meta::NameValue(nv) if nv.path.is_ident("doc") => lit_str(&nv.value).ok(),
            _ => None,
        })
        .map(|lit| lit.value().trim().to_owned())
        .collect();
    lines.join("\n").trim().to_owned()
}

fn expand_meta_object(
    qi: &Path,
    meta_fn_ident: &Ident,
    vis: &syn::Visibility,
    description: &str,
    members: &[Member],
) -> TokenStream {
    let entries = members.iter().map(|member| {
        let (id, name, member_description) = (member.id, &member.name, &member.description);
        match &member.kind {
            MemberKind::Method(method) => {
                let params = method
                    .params
                    .iter()
                    .enumerate()
                    .map(|(index, (ident, ty))| {
                        let param_name = ident.unraw().to_string();
                        quote! {
                            method
                                .parameter(#index)
                                .set_name(#param_name)
                                .set_type(<#ty as #qi::value::Reflect>::ty());
                        }
                    });
                let ok_type = &method.ok_type;
                quote! {
                    let mut method = #qi::object::MetaMethod::builder(#qi::object::ActionId(#id));
                    method.set_name(#name).set_description(#member_description);
                    #(#params)*
                    method
                        .return_value()
                        .set_type(<#ok_type as #qi::value::Reflect>::ty());
                    builder.add_method(method.build());
                }
            }
            MemberKind::Signal(ty) => quote! {
                builder.add_signal(#qi::object::MetaSignal {
                    uid: #qi::object::ActionId(#id),
                    name: ::std::string::ToString::to_string(#name),
                    signature: #qi::value::Signature::from(
                        #qi::__private::params_type_of(<#ty as #qi::value::Reflect>::ty()),
                    ),
                });
            },
            MemberKind::Property(ty) => quote! {
                builder.add_signal(#qi::object::MetaSignal {
                    uid: #qi::object::ActionId(#id),
                    name: ::std::string::ToString::to_string(#name),
                    signature: #qi::value::Signature::from(
                        #qi::__private::params_type_of(<#ty as #qi::value::Reflect>::ty()),
                    ),
                });
                builder.add_property(#qi::object::MetaProperty {
                    uid: #qi::object::ActionId(#id),
                    name: ::std::string::ToString::to_string(#name),
                    signature: #qi::value::Signature::from(<#ty as #qi::value::Reflect>::ty()),
                });
            },
        }
    });
    quote! {
        #[doc(hidden)]
        #vis fn #meta_fn_ident() -> &'static #qi::object::MetaObject {
            static META_OBJECT: ::std::sync::LazyLock<#qi::object::MetaObject> =
                ::std::sync::LazyLock::new(|| {
                    let mut builder = #qi::object::MetaObject::builder();
                    #(#entries)*
                    let mut meta_object = builder.build();
                    meta_object.description = ::std::string::ToString::to_string(#description);
                    meta_object
                });
            &META_OBJECT
        }
    }
}

fn expand_object(
    qi: &Path,
    object_ident: &Ident,
    trait_ident: &Ident,
    meta_fn_ident: &Ident,
    vis: &syn::Visibility,
    members: &[Member],
) -> TokenStream {
    let call_arms = members.iter().filter_map(|member| {
        let MemberKind::Method(method) = &member.kind else {
            return None;
        };
        let (id, ident) = (member.id, &member.ident);
        let names = method.params.iter().map(|(name, _)| name);
        let types = method.params.iter().map(|(_, ty)| ty);
        let decode = match method.params.len() {
            0 => quote!(let () = #qi::__private::decode_args::<()>(&method.parameters_signature, args)?;),
            1 => {
                let (name, ty) = &method.params[0];
                quote!(let #name: #ty = #qi::__private::decode_args::<#ty>(&method.parameters_signature, args)?;)
            }
            _ => quote! {
                let (#(#names),*): (#(#types),*) =
                    #qi::__private::decode_args(&method.parameters_signature, args)?;
            },
        };
        let names = method.params.iter().map(|(name, _)| name);
        Some(quote! {
            #qi::object::ActionId(#id) => {
                #decode
                let result = <T as #trait_ident>::#ident(&self.0, #(#names),*)
                    .await
                    .map_err(::std::convert::Into::<#qi::Error>::into)?;
                ::std::result::Result::Ok(#qi::__private::encode_return(result))
            }
        })
    });
    let signal_arms = |f: &dyn Fn(&Ident, bool) -> TokenStream| {
        members
            .iter()
            .filter_map(|member| match &member.kind {
                MemberKind::Signal(_) => Some((member.id, f(&member.ident, false))),
                MemberKind::Property(_) => Some((member.id, f(&member.ident, true))),
                MemberKind::Method(_) => None,
            })
            .map(|(id, body)| quote!(#qi::object::ActionId(#id) => #body,))
            .collect::<TokenStream>()
    };
    let emit_arms = signal_arms(&|ident, is_property| {
        if is_property {
            quote!(<T as #trait_ident>::#ident(&self.0).signal().emit_erased(params))
        } else {
            quote!(<T as #trait_ident>::#ident(&self.0).emit_erased(params))
        }
    });
    let subscribe_arms = signal_arms(
        &|ident, _| quote!(<T as #trait_ident>::#ident(&self.0).subscribe_erased().await),
    );
    let property_arms = |body: &dyn Fn(&Ident) -> TokenStream| {
        members
            .iter()
            .filter_map(|member| match &member.kind {
                MemberKind::Property(_) => Some((member.id, body(&member.ident))),
                _ => None,
            })
            .map(|(id, body)| quote!(#qi::object::ActionId(#id) => #body,))
            .collect::<TokenStream>()
    };
    let get_arms =
        property_arms(&|ident| quote!(<T as #trait_ident>::#ident(&self.0).get_erased().await));
    let set_arms = property_arms(
        &|ident| quote!(<T as #trait_ident>::#ident(&self.0).set_erased(value).await),
    );
    let doc = format!(
        "Implementations of [`{trait_ident}`] as objects: wraps a `T: {trait_ident}` into an \
         implementation of the `Object` trait of `qi`."
    );
    quote! {
        #[doc = #doc]
        #[derive(Debug, Clone)]
        #vis struct #object_ident<T>(T);

        impl<T> #object_ident<T> {
            /// Wraps an implementation of the interface.
            #vis fn new(inner: T) -> Self {
                Self(inner)
            }

            /// Unwraps the implementation.
            #vis fn into_inner(self) -> T {
                self.0
            }

            /// The meta object of the interface.
            #vis fn meta_object() -> &'static #qi::object::MetaObject {
                #meta_fn_ident()
            }
        }

        impl<T> #object_ident<T>
        where
            T: #trait_ident + ::std::marker::Send + ::std::marker::Sync + 'static,
        {
            /// Wraps the implementation into a shared, type-erased object.
            #vis fn into_any(self) -> #qi::AnyObject {
                #qi::AnyObject::new(self)
            }
        }

        impl<T> ::std::ops::Deref for #object_ident<T> {
            type Target = T;

            fn deref(&self) -> &T {
                &self.0
            }
        }

        impl<T> ::std::ops::DerefMut for #object_ident<T> {
            fn deref_mut(&mut self) -> &mut T {
                &mut self.0
            }
        }

        #[#qi::async_trait]
        impl<T> #qi::Object for #object_ident<T>
        where
            T: #trait_ident + ::std::marker::Send + ::std::marker::Sync + 'static,
        {
            fn meta(&self) -> &#qi::object::MetaObject {
                #meta_fn_ident()
            }

            async fn meta_call(
                &self,
                ident: #qi::object::ActionNameOrId,
                args: #qi::value::Value<'_>,
            ) -> #qi::Result<#qi::value::Value<'static>> {
                let method = self
                    .meta()
                    .method(&ident)
                    .ok_or_else(|| #qi::Error::MethodNotFound(ident.clone()))?;
                match method.uid {
                    #(#call_arms)*
                    _ => ::std::result::Result::Err(#qi::Error::MethodNotFound(ident)),
                }
            }

            async fn meta_emit(
                &self,
                ident: #qi::object::ActionNameOrId,
                params: #qi::value::Value<'_>,
            ) -> #qi::Result<()> {
                let signal = self
                    .meta()
                    .signal(&ident)
                    .ok_or_else(|| #qi::Error::SignalNotFound(ident.clone()))?;
                match signal.uid {
                    #emit_arms
                    _ => ::std::result::Result::Err(#qi::Error::SignalNotFound(ident)),
                }
            }

            async fn meta_subscribe(
                &self,
                ident: #qi::object::ActionNameOrId,
            ) -> #qi::Result<#qi::signal::ValueStream> {
                let signal = self
                    .meta()
                    .signal(&ident)
                    .ok_or_else(|| #qi::Error::SignalNotFound(ident.clone()))?;
                match signal.uid {
                    #subscribe_arms
                    _ => ::std::result::Result::Err(#qi::Error::SignalNotFound(ident)),
                }
            }

            async fn meta_property(
                &self,
                ident: #qi::object::ActionNameOrId,
            ) -> #qi::Result<#qi::value::Value<'static>> {
                let property = self
                    .meta()
                    .property(&ident)
                    .ok_or_else(|| #qi::Error::PropertyNotFound(ident.clone()))?;
                match property.uid {
                    #get_arms
                    _ => ::std::result::Result::Err(#qi::Error::PropertyNotFound(ident)),
                }
            }

            async fn meta_set_property(
                &self,
                ident: #qi::object::ActionNameOrId,
                value: #qi::value::Value<'_>,
            ) -> #qi::Result<()> {
                let property = self
                    .meta()
                    .property(&ident)
                    .ok_or_else(|| #qi::Error::PropertyNotFound(ident.clone()))?;
                match property.uid {
                    #set_arms
                    _ => ::std::result::Result::Err(#qi::Error::PropertyNotFound(ident)),
                }
            }
        }

        impl<T> #qi::value::Reflect for #object_ident<T> {
            fn ty() -> ::std::option::Option<#qi::value::Type> {
                ::std::option::Option::Some(#qi::value::Type::Object)
            }
        }

        impl<T> #qi::value::RuntimeReflect for #object_ident<T> {
            fn ty(&self) -> #qi::value::Type {
                #qi::value::Type::Object
            }
        }

        impl<'a, T> #qi::value::IntoValue<'a> for #object_ident<T>
        where
            T: #trait_ident + ::std::marker::Send + ::std::marker::Sync + 'static,
        {
            fn into_value(self) -> #qi::value::Value<'a> {
                #qi::value::IntoValue::into_value(self.into_any())
            }
        }
    }
}

fn expand_client(
    qi: &Path,
    client_ident: &Ident,
    trait_ident: &Ident,
    meta_fn_ident: &Ident,
    vis: &syn::Visibility,
    members: &[Member],
) -> TokenStream {
    let fields = members.iter().filter_map(|member| {
        let ident = &member.ident;
        match &member.kind {
            MemberKind::Signal(ty) => Some(quote!(#ident: #qi::Signal<#ty>,)),
            MemberKind::Property(ty) => Some(quote!(#ident: #qi::Property<#ty>,)),
            MemberKind::Method(_) => None,
        }
    });
    let field_inits = members.iter().filter_map(|member| {
        let (ident, name) = (&member.ident, &member.name);
        match &member.kind {
            MemberKind::Signal(_) => Some(quote! {
                #ident: #qi::Signal::of_object(
                    ::std::clone::Clone::clone(&object),
                    #qi::object::ActionNameOrId::from(#name),
                ),
            }),
            MemberKind::Property(_) => Some(quote! {
                #ident: #qi::Property::of_object(
                    ::std::clone::Clone::clone(&object),
                    #qi::object::ActionNameOrId::from(#name),
                ),
            }),
            MemberKind::Method(_) => None,
        }
    });
    let impls = members.iter().map(|member| {
        let (ident, name) = (&member.ident, &member.name);
        match &member.kind {
            MemberKind::Method(method) => {
                let params = method.params.iter().map(|(name, ty)| quote!(#name: #ty));
                let names = method.params.iter().map(|(name, _)| name);
                let args = match method.params.len() {
                    0 => quote!(()),
                    1 => {
                        let name = &method.params[0].0;
                        quote!(#name)
                    }
                    _ => quote!((#(#names),*)),
                };
                let ok_type = &method.ok_type;
                quote! {
                    async fn #ident(&self, #(#params),*) -> #qi::Result<#ok_type> {
                        #qi::ObjectExt::call::<#ok_type, _, _>(&self.__object, #name, #args).await
                    }
                }
            }
            MemberKind::Signal(ty) => quote! {
                fn #ident(&self) -> &#qi::Signal<#ty> {
                    &self.#ident
                }
            },
            MemberKind::Property(ty) => quote! {
                fn #ident(&self) -> &#qi::Property<#ty> {
                    &self.#ident
                }
            },
        }
    });
    let doc = format!(
        "A typed proxy implementing [`{trait_ident}`] over any object exposing its members, local \
         or remote."
    );
    quote! {
        #[doc = #doc]
        #[derive(Clone)]
        #vis struct #client_ident {
            __object: #qi::AnyObject,
            #(#fields)*
        }

        impl #client_ident {
            /// Wraps an object, checking that its meta object exposes every member of the
            /// interface.
            #vis fn new(object: #qi::AnyObject) -> #qi::Result<Self> {
                #qi::__private::check_interface(#qi::Object::meta(&object), Self::meta_object())?;
                ::std::result::Result::Ok(Self::new_unchecked(object))
            }

            /// Wraps an object without checking its meta object: members missing from the object
            /// fail when used.
            #vis fn new_unchecked(object: #qi::AnyObject) -> Self {
                Self {
                    #(#field_inits)*
                    __object: object,
                }
            }

            /// The meta object of the interface.
            #vis fn meta_object() -> &'static #qi::object::MetaObject {
                #meta_fn_ident()
            }

            /// The wrapped object.
            #vis fn as_object(&self) -> &#qi::AnyObject {
                &self.__object
            }

            /// Unwraps the object.
            #vis fn into_object(self) -> #qi::AnyObject {
                self.__object
            }
        }

        #[#qi::async_trait]
        impl #trait_ident for #client_ident {
            #(#impls)*
        }

        impl ::std::ops::Deref for #client_ident {
            type Target = #qi::AnyObject;

            fn deref(&self) -> &#qi::AnyObject {
                &self.__object
            }
        }

        impl ::std::fmt::Debug for #client_ident {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.debug_tuple(stringify!(#client_ident)).field(&self.__object).finish()
            }
        }

        impl ::std::convert::From<#client_ident> for #qi::AnyObject {
            fn from(client: #client_ident) -> Self {
                client.__object
            }
        }

        impl #qi::value::Reflect for #client_ident {
            fn ty() -> ::std::option::Option<#qi::value::Type> {
                ::std::option::Option::Some(#qi::value::Type::Object)
            }
        }

        impl #qi::value::RuntimeReflect for #client_ident {
            fn ty(&self) -> #qi::value::Type {
                #qi::value::Type::Object
            }
        }

        impl<'a> #qi::value::IntoValue<'a> for #client_ident {
            fn into_value(self) -> #qi::value::Value<'a> {
                #qi::value::IntoValue::into_value(self.__object)
            }
        }

        impl #qi::value::FromValue<'_> for #client_ident {
            fn from_value(
                value: #qi::value::Value<'_>,
            ) -> ::std::result::Result<Self, #qi::value::FromValueError> {
                let object = <#qi::AnyObject as #qi::value::FromValue>::from_value(value)?;
                Self::new(object).map_err(|err| {
                    #qi::value::FromValueError::Other(::std::convert::Into::into(err))
                })
            }
        }
    }
}
