//! Implementation of the `#[qi::module]` attribute macro.
//!
//! The macro turns an inline `mod` into a `qi` module:
//!
//! - the module itself, its exported functions untouched except for their `#[qi::…]` tags;
//! - `__QiModuleInterface`, a hidden `#[qi::object]` trait with one method per function and
//!   factory, and `__QiModuleImpl`, its implementation calling them: the module object;
//! - `__QI_MODULE`, the `qi::module::ModuleDef` of the module;
//! - the entry points of the library (`qi::__export_module!`).

use super::object::{convert_name, doc_string, lit_str, result_ok_type};
use convert_case::{Case, Casing};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{
    ext::IdentExt,
    parse::{Parse, ParseStream},
    parse_quote,
    punctuated::Punctuated,
    Attribute, Error, FnArg, Ident, Item, ItemFn, ItemMod, LitStr, Meta, Pat, Path, Result,
    ReturnType, Token, Type, Visibility,
};

/// The arguments of the attribute: `#[qi::module(name = "...", case = "...", crate = "...")]`.
pub(super) struct Args {
    crate_path: Path,
    name: Option<String>,
    case: Option<Case<'static>>,
}

impl Parse for Args {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut args = Self {
            crate_path: parse_quote!(::qi),
            name: None,
            case: None,
        };
        for meta in Punctuated::<Meta, Token![,]>::parse_terminated(input)? {
            match meta {
                Meta::NameValue(nv) if nv.path.is_ident("crate") => {
                    args.crate_path = lit_str(&nv.value)?.parse()?;
                }
                Meta::NameValue(nv) if nv.path.is_ident("name") => {
                    args.name = Some(lit_str(&nv.value)?.value());
                }
                Meta::NameValue(nv) if nv.path.is_ident("case") => {
                    args.case = Some(super::value::parse_case(&lit_str(&nv.value)?)?);
                }
                other => {
                    return Err(Error::new_spanned(
                        other,
                        "unknown attribute, expected `name = \"...\"`, `case = \"...\"` or \
                         `crate = \"...\"`",
                    ))
                }
            }
        }
        Ok(args)
    }
}

/// An exported function of the module.
struct Member {
    /// The name of the member in the module object.
    name: String,
    /// The Rust identifier of the function.
    ident: Ident,
    description: String,
    is_async: bool,
    /// The function takes a `Session` as first parameter.
    takes_session: bool,
    /// The parameters of the function, session excluded.
    params: Vec<(Ident, Type)>,
    output: Output,
    factory: Option<Factory>,
}

/// What a function returns.
enum Output {
    /// `Result<T>`: the `T`.
    Result(Type),
    /// A plain value, possibly `()`.
    Value(Type),
}

struct Factory {
    autoload: bool,
}

/// The tag of an exported function: `#[qi::function(...)]` or `#[qi::factory(...)]`.
enum Tag {
    Function {
        name: Option<String>,
    },
    Factory {
        name: Option<String>,
        autoload: bool,
    },
}

pub(super) fn expand(args: Args, mut item: ItemMod) -> Result<TokenStream> {
    let Some((_, items)) = &mut item.content else {
        return Err(Error::new_spanned(
            &item,
            "`#[qi::module]` applies to inline modules: `mod name { ... }`",
        ));
    };
    let name = args
        .name
        .clone()
        .unwrap_or_else(|| item.ident.unraw().to_string());
    check_module_name(&name, &item.ident)?;

    let mut members = Vec::new();
    for item in items.iter_mut() {
        if let Item::Fn(func) = item {
            if let Some(member) = parse_member(func, args.case)? {
                members.push(member);
            }
        }
    }
    let mut names = std::collections::HashSet::new();
    for member in &members {
        if !names.insert(member.name.as_str()) {
            return Err(Error::new_spanned(
                &member.ident,
                format!("duplicate module member name `{}`", member.name),
            ));
        }
    }

    let qi = &args.crate_path;
    let description = doc_string(&item.attrs);
    let generated = expand_module(qi, &name, &description, &members);
    items.extend(
        syn::parse2::<syn::File>(generated)
            .expect("the generated items parse")
            .items,
    );
    Ok(quote!(#item))
}

fn check_module_name(name: &str, span: &Ident) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    {
        return Err(Error::new_spanned(
            span,
            format!("invalid module name `{name}`: use only characters from [_.a-zA-Z0-9]"),
        ));
    }
    Ok(())
}

/// Parses an exported function: public functions and tagged ones. Strips the tag.
fn parse_member(func: &mut ItemFn, case: Option<Case<'static>>) -> Result<Option<Member>> {
    let tag = extract_tag(&mut func.attrs)?;
    let is_public = matches!(func.vis, Visibility::Public(_));
    let tag = match tag {
        Some(tag) => tag,
        None if is_public => Tag::Function { name: None },
        None => return Ok(None),
    };
    let sig = &func.sig;
    if !sig.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &sig.generics,
            "module functions cannot be generic",
        ));
    }
    let ident = sig.ident.clone();
    let rust_name = ident.unraw().to_string();
    let (name, factory) = match tag {
        Tag::Function { name } => (name.unwrap_or_else(|| convert_name(&rust_name, case)), None),
        Tag::Factory { name, autoload } => (
            name.unwrap_or_else(|| rust_name.to_case(Case::Pascal)),
            Some(Factory { autoload }),
        ),
    };

    let mut takes_session = false;
    let mut params = Vec::new();
    for (index, input) in sig.inputs.iter().enumerate() {
        let FnArg::Typed(pat_type) = input else {
            return Err(Error::new_spanned(
                input,
                "module functions are free functions, without receiver",
            ));
        };
        let Pat::Ident(pat) = &*pat_type.pat else {
            return Err(Error::new_spanned(
                &pat_type.pat,
                "module function parameters must be plain identifiers",
            ));
        };
        if is_session(&pat_type.ty) {
            if index != 0 {
                return Err(Error::new_spanned(
                    &pat_type.ty,
                    "the session must be the first parameter of a module function",
                ));
            }
            takes_session = true;
            continue;
        }
        params.push((pat.ident.clone(), (*pat_type.ty).clone()));
    }

    let output = match &sig.output {
        ReturnType::Default => Output::Value(parse_quote!(())),
        ReturnType::Type(_, ty) if is_result(ty) => Output::Result(result_ok_type(ty)?),
        ReturnType::Type(_, ty) => Output::Value((**ty).clone()),
    };
    Ok(Some(Member {
        name,
        ident,
        description: doc_string(&func.attrs),
        is_async: sig.asyncness.is_some(),
        takes_session,
        params,
        output,
        factory,
    }))
}

/// Whether a type is the session of modules: a path ending with `Session`.
fn is_session(ty: &Type) -> bool {
    matches!(ty, Type::Path(path) if path.qself.is_none()
        && path.path.segments.last().is_some_and(|segment| segment.ident == "Session" && segment.arguments.is_empty()))
}

fn is_result(ty: &Type) -> bool {
    matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "Result"))
}

/// Finds and removes the `#[qi::function]` or `#[qi::factory]` tag of a function.
fn extract_tag(attrs: &mut Vec<Attribute>) -> Result<Option<Tag>> {
    let mut found = None;
    let mut index = 0;
    while index < attrs.len() {
        let Some(kind) = tag_kind(&attrs[index]) else {
            index += 1;
            continue;
        };
        let attr = attrs.remove(index);
        if found.is_some() {
            return Err(Error::new_spanned(attr, "duplicate module member tag"));
        }
        let mut name = None;
        let mut autoload = false;
        if let Meta::List(_) = &attr.meta {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("name") {
                    name = Some(meta.value()?.parse::<LitStr>()?.value());
                    Ok(())
                } else if meta.path.is_ident("autoload") && kind == "factory" {
                    autoload = true;
                    Ok(())
                } else if kind == "factory" {
                    Err(meta.error("unknown attribute, expected `name = \"...\"` or `autoload`"))
                } else {
                    Err(meta.error("unknown attribute, expected `name = \"...\"`"))
                }
            })?;
        }
        found = Some(match kind {
            "factory" => Tag::Factory { name, autoload },
            _ => Tag::Function { name },
        });
    }
    Ok(found)
}

fn tag_kind(attr: &Attribute) -> Option<&'static str> {
    let segments: Vec<_> = attr
        .path()
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    match segments.as_slice() {
        [qi, tag] if qi == "qi" && tag == "function" => Some("function"),
        [qi, tag] if qi == "qi" && tag == "factory" => Some("factory"),
        _ => None,
    }
}

fn expand_module(qi: &Path, name: &str, description: &str, members: &[Member]) -> TokenStream {
    let interface = Ident::new("__QiModuleInterface", Span::call_site());
    let object = Ident::new("__QiModuleInterfaceObject", Span::call_site());
    let qi_str = quote!(#qi).to_string().replace(' ', "");

    let declarations = members.iter().map(|member| {
        let Member {
            name,
            ident,
            description,
            params,
            ..
        } = member;
        let params = params.iter().map(|(ident, ty)| quote!(#ident: #ty));
        let ok = match (&member.factory, &member.output) {
            (Some(_), _) => quote!(#qi::AnyObject),
            (None, Output::Result(ty) | Output::Value(ty)) => quote!(#ty),
        };
        quote! {
            #[doc = #description]
            #[qi::method(name = #name)]
            async fn #ident(&self, #(#params),*) -> #qi::Result<#ok>;
        }
    });

    let implementations = members.iter().map(|member| {
        let ident = &member.ident;
        let param_decls = member.params.iter().map(|(ident, ty)| quote!(#ident: #ty));
        let session = member
            .takes_session
            .then(|| quote!(::std::clone::Clone::clone(&self.0),));
        let args = member.params.iter().map(|(ident, _)| ident);
        let call = quote!(self::#ident(#session #(#args),*));
        let call = if member.is_async {
            quote!(#call.await)
        } else {
            call
        };
        let result = match &member.output {
            Output::Result(_) => {
                quote!(#call.map_err(::std::convert::Into::<#qi::Error>::into))
            }
            Output::Value(_) => quote!(::std::result::Result::<_, #qi::Error>::Ok(#call)),
        };
        let result = match &member.factory {
            Some(_) => quote!(#result.map(#qi::module::IntoAnyObject::into_any_object)),
            None => result,
        };
        let ok = match (&member.factory, &member.output) {
            (Some(_), _) => quote!(#qi::AnyObject),
            (None, Output::Result(ty) | Output::Value(ty)) => quote!(#ty),
        };
        quote! {
            async fn #ident(&self, #(#param_decls),*) -> #qi::Result<#ok> {
                #result
            }
        }
    });

    let factory_defs = members.iter().filter_map(|member| {
        let factory = member.factory.as_ref()?;
        let (name, autoload) = (&member.name, factory.autoload);
        Some(quote!(#qi::module::FactoryDef { name: #name, autoload: #autoload }))
    });
    let factory_exports = members.iter().filter_map(|member| {
        let factory = member.factory.as_ref()?;
        let name = &member.name;
        let flags = u32::from(factory.autoload);
        Some(quote!((#name, #flags)))
    });

    quote! {
        #[doc = #description]
        #[doc(hidden)]
        #[#qi::object(crate = #qi_str)]
        pub trait #interface {
            #(#declarations)*
        }

        /// The implementation of the module object.
        #[doc(hidden)]
        pub struct __QiModuleImpl(#qi::module::Session);

        #[#qi::async_trait]
        impl #interface for __QiModuleImpl {
            #(#implementations)*
        }

        #[doc(hidden)]
        fn __qi_module_object(session: #qi::module::Session) -> #qi::AnyObject {
            #qi::AnyObject::new(#object::new(__QiModuleImpl(session)))
        }

        /// The definition of the module.
        #[doc(hidden)]
        pub static __QI_MODULE: #qi::module::ModuleDef = #qi::module::ModuleDef {
            name: #name,
            description: #description,
            factories: &[#(#factory_defs),*],
            object: __qi_module_object,
            state: #qi::module::State::new(),
        };

        #qi::__export_module! {
            def: self::__QI_MODULE,
            name: #name,
            factories: [#(#factory_exports),*],
        }
    }
}
