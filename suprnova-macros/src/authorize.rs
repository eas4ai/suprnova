//! `#[authorize]`: declarative authorization on a `#[handler]`.
//!
//! The check itself is emitted by `#[handler]` (see `handler.rs`): the
//! handler macro collects every `#[authorize(...)]` on the function and runs
//! one gate check per attribute, in the order written, after the route
//! parameters are bound and before the request body is read.
//!
//! This attribute's own expansion runs only in the two cases `#[handler]`
//! never sees it. Attribute macros expand outermost first, and `#[handler]`
//! consumes every `#[authorize]` below it, so this one runs when it sits
//! above `#[handler]` or when there is no `#[handler]`. Above, it hands
//! itself back below `#[handler]`, so the order of the attributes does not
//! matter. Without `#[handler]` it is a compile error: there is no
//! extraction step to run the check in, and a check that silently did
//! nothing would leave the handler open.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Attribute, Ident, ItemFn, LitStr, Meta, Token, Type};

/// The shape the attribute accepts, quoted in every argument error.
const USAGE: &str = "expected `#[authorize(\"ability\", Type)]` or \
                     `#[authorize(\"ability\", param)]`";

/// The error for an `#[authorize]` that no `#[handler]` will apply.
const NO_HANDLER: &str = "#[authorize] works only on a `#[handler]` function: \
                          add `#[handler]` to it";

/// One parsed `#[authorize(ability, target)]`.
pub(crate) struct AuthorizeSpec {
    /// The gate ability, passed to the gate as written.
    pub(crate) ability: LitStr,
    /// What the ability is checked against.
    pub(crate) target: Target,
}

/// The second argument of `#[authorize]`.
///
/// A single identifier that starts with a lowercase letter or `_` names a
/// handler parameter, as Rust names values. Anything else is a type: an
/// identifier that starts with an uppercase letter, or a path such as
/// `post::Model` or `crate::models::Post`. Rust's naming conventions make
/// the split unambiguous in practice, and it needs no extra syntax.
pub(crate) enum Target {
    /// A handler parameter: the check runs against the bound value.
    Param(Ident),
    /// A type: the check runs against `<Type as Default>::default()`.
    Type(Box<Type>),
}

impl Parse for AuthorizeSpec {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ability: LitStr = input
            .parse()
            .map_err(|err| syn::Error::new(err.span(), USAGE))?;
        if ability.value().is_empty() {
            return Err(syn::Error::new_spanned(
                &ability,
                "#[authorize] needs a non-empty ability name",
            ));
        }
        input
            .parse::<Token![,]>()
            .map_err(|err| syn::Error::new(err.span(), USAGE))?;
        let target: Type = input
            .parse()
            .map_err(|err| syn::Error::new(err.span(), USAGE))?;
        input.parse::<Option<Token![,]>>()?;
        if !input.is_empty() {
            return Err(input.error(USAGE));
        }
        Ok(Self {
            ability,
            target: Target::classify(target),
        })
    }
}

impl Target {
    /// Split a parameter name from a type; see [`Target`].
    fn classify(target: Type) -> Self {
        if let Type::Path(path) = &target
            && path.qself.is_none()
            && let Some(ident) = path.path.get_ident()
            && ident
                .to_string()
                .starts_with(|c: char| c.is_lowercase() || c == '_')
        {
            return Self::Param(ident.clone());
        }
        Self::Type(Box::new(target))
    }
}

/// Whether `attr` is an `#[authorize]`, however its path is spelled.
pub(crate) fn is_authorize_attr(attr: &Attribute) -> bool {
    last_segment_is(attr, "authorize")
}

/// Whether `attr` is a `#[handler]`, however its path is spelled.
fn is_handler_attr(attr: &Attribute) -> bool {
    last_segment_is(attr, "handler")
}

fn last_segment_is(attr: &Attribute, name: &str) -> bool {
    attr.path()
        .segments
        .last()
        .is_some_and(|segment| segment.ident == name)
}

/// Parse one `#[authorize(...)]` attribute.
pub(crate) fn parse_spec(attr: &Attribute) -> syn::Result<AuthorizeSpec> {
    match &attr.meta {
        Meta::List(_) => attr.parse_args(),
        // `#[authorize]` and `#[authorize = ...]` carry no argument list.
        _ => Err(syn::Error::new_spanned(attr, USAGE)),
    }
}

/// Entry point for the attribute's own expansion; see the module docs.
pub fn authorize_impl(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    authorize_impl_inner(attr.into(), item.into()).into()
}

/// `proc_macro2` form of [`authorize_impl`], so unit tests can drive it.
pub(crate) fn authorize_impl_inner(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut item_fn: ItemFn = match syn::parse2(item.clone()) {
        Ok(item_fn) => item_fn,
        Err(_) => {
            let error = syn::Error::new(Span::call_site(), NO_HANDLER).to_compile_error();
            return quote! { #error #item };
        }
    };
    if let Err(err) = syn::parse2::<AuthorizeSpec>(attr.clone()) {
        let error = err.to_compile_error();
        return quote! { #error #item_fn };
    }
    let Some(index) = item_fn.attrs.iter().position(is_handler_attr) else {
        let error = syn::Error::new(Span::call_site(), NO_HANDLER).to_compile_error();
        return quote! { #error #item_fn };
    };
    // `#[handler]` goes first and this attribute right after it, ahead of
    // any `#[authorize]` still on the item, which keeps the written order.
    let handler = item_fn.attrs.remove(index);
    quote! {
        #handler
        #[::suprnova::authorize(#attr)]
        #item_fn
    }
}
