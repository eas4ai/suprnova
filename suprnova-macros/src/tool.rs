//! `#[tool]`: an agent tool on an ordinary `#[handler]` (AGENT-001).
//!
//! A tool is a routed handler an agent may call. The attribute sits on the
//! handler next to `#[handler]`, in either order, and emits the handler
//! unchanged plus one `ToolRecord` submitted through `inventory`, keyed by
//! the handler function's type the way `#[handler]` keys its record. The
//! framework finds the handler's route from that key, so a tool call runs
//! the route's middleware, bindings, validation and `#[authorize]` checks.
//!
//! An agent reads a tool's name, description and schemas as instructions, so
//! all of them are fixed when the crate builds:
//!
//! - `name` and `description` are string literals. A name is 1 to 128
//!   characters of ASCII letters, digits, `_`, `-` and `.`; a description is
//!   not empty. Anything else fails the build.
//! - Two tools with one name in a crate fail the build: each tool also
//!   emits a `#[macro_export]` macro named after its name, and a crate cannot
//!   export two macros of one name. Tools of two crates with one name are
//!   refused when the application boots.
//! - `scopes = [...]` takes paths to the constants `tool_scopes!` declares,
//!   so a scope nobody declared is an unresolved name and fails the build.
//! - The schemas come from the `input` and `output` types through
//!   `schemars`, never from data.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::parse::{ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Attribute, Expr, ExprLit, Ident, ItemFn, Lit, LitBool, LitStr, Path, Token, Type};

use crate::authorize::{is_handler_attr, last_segment_is};
use crate::handler::{parse_handler_args, stable_hash};

/// The error for a `#[tool]` that no `#[handler]` will route.
const NO_HANDLER: &str = "#[tool] works only on a `#[handler]` function: add `#[handler]` to it";

/// The longest name a tool may have, as MCP limits it.
const MAX_NAME_LEN: usize = 128;

/// Whether `attr` is a `#[tool]`, however its path is spelled.
pub(crate) fn is_tool_attr(attr: &Attribute) -> bool {
    last_segment_is(attr, "tool")
}

/// The caller kinds a tool may accept, as `callers = [...]` spells them.
const CALLER_KINDS: [&str; 3] = ["public", "user", "agent"];

/// The hints `annotations(...)` may set.
#[derive(Default)]
struct Annotations {
    read_only: Option<bool>,
    destructive: Option<bool>,
    idempotent: Option<bool>,
    open_world: Option<bool>,
    untrusted_content: Option<bool>,
}

/// One parsed `#[tool(...)]`.
struct ToolSpec {
    name: LitStr,
    description: LitStr,
    title: Option<LitStr>,
    callers: Vec<Ident>,
    scopes: Vec<Path>,
    input: Option<Type>,
    output: Option<Type>,
    annotations: Annotations,
    route: Option<LitStr>,
    guard: Option<LitStr>,
}

/// Entry point for the attribute; see the module docs.
pub fn tool_impl(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    tool_impl_inner(attr.into(), item.into()).into()
}

/// `proc_macro2` form of [`tool_impl`], so unit tests can drive it.
pub(crate) fn tool_impl_inner(attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_fn: ItemFn = match syn::parse2(item.clone()) {
        Ok(item_fn) => item_fn,
        Err(_) => {
            let error = syn::Error::new(Span::call_site(), NO_HANDLER).to_compile_error();
            return quote! { #error #item };
        }
    };
    let Some(handler_attr) = item_fn.attrs.iter().find(|attr| is_handler_attr(attr)) else {
        let error = syn::Error::new(Span::call_site(), NO_HANDLER).to_compile_error();
        return quote! { #error #item_fn };
    };
    let self_ty = match handler_self(handler_attr) {
        Ok(self_ty) => self_ty,
        Err(error) => {
            let error = error.to_compile_error();
            return quote! { #error #item_fn };
        }
    };
    let spec = match parse_spec(attr) {
        Ok(spec) => spec,
        Err(error) => {
            let error = error.to_compile_error();
            return quote! { #error #item_fn };
        }
    };
    if item_fn.sig.asyncness.is_none() {
        let error = syn::Error::new_spanned(
            &item_fn.sig.fn_token,
            "#[tool] needs an `async fn` handler: a tool call awaits it",
        )
        .to_compile_error();
        return quote! { #error #item_fn };
    }
    if item_fn
        .sig
        .generics
        .params
        .iter()
        .any(|param| !matches!(param, syn::GenericParam::Lifetime(_)))
    {
        let error = syn::Error::new_spanned(
            &item_fn.sig.generics,
            "#[tool] cannot mark a generic handler: a tool is one route, and a generic \
             handler has no single type to find it by",
        )
        .to_compile_error();
        return quote! { #error #item_fn };
    }
    let record = record_items(&item_fn, self_ty.as_ref(), &spec);
    match self_ty {
        // Inside an `impl` block the items go into an associated const, the
        // one place an impl item can hold them.
        Some(_) => {
            let site = format_ident!("__SUPRNOVA_TOOL_{:016X}", stable_hash(&spec.name.value()));
            quote! {
                #item_fn

                #[doc(hidden)]
                #[allow(dead_code)]
                const #site: () = {
                    #record
                };
            }
        }
        None => quote! {
            #item_fn

            #record
        },
    }
}

/// The `Self = Type` of the `#[handler]` on the item, for a handler inside
/// an `impl` block.
fn handler_self(attr: &Attribute) -> syn::Result<Option<Type>> {
    match &attr.meta {
        syn::Meta::List(list) => parse_handler_args(list.tokens.clone()),
        _ => Ok(None),
    }
}

/// The marker macro, the schema and handler functions, and the record.
fn record_items(item_fn: &ItemFn, self_ty: Option<&Type>, spec: &ToolSpec) -> TokenStream {
    let name = &spec.name;
    let description = &spec.description;
    let marker = format_ident!(
        "__suprnova_tool_{}",
        escape(&name.value()),
        span = name.span()
    );
    let mut fn_ref = item_fn.sig.ident.clone();
    fn_ref.set_span(Span::call_site());
    let handler = match self_ty {
        Some(self_ty) => quote! { <#self_ty>::#fn_ref },
        None => quote! { #fn_ref },
    };
    let display = match self_ty {
        Some(self_ty) => format!(
            "{}::{}",
            quote!(#self_ty).to_string().replace(' ', ""),
            fn_ref
        ),
        None => fn_ref.to_string(),
    };
    let callers = spec.callers.iter().map(|caller| {
        let variant = match caller.to_string().as_str() {
            "public" => quote! { Public },
            "user" => quote! { User },
            _ => quote! { Agent },
        };
        quote! { ::suprnova::agents::CallerKind::#variant }
    });
    let scopes = &spec.scopes;
    let input_schema = match &spec.input {
        Some(ty) => quote! { ::suprnova::agents::__input_schema::<#ty> },
        None => quote! { ::suprnova::agents::__empty_input_schema },
    };
    let output_schema = match &spec.output {
        Some(ty) => quote! {
            ::core::option::Option::Some(::suprnova::agents::__output_schema::<#ty>)
        },
        None => quote! { ::core::option::Option::None },
    };
    let optional = |value: &Option<LitStr>| match value {
        Some(value) => quote! { ::core::option::Option::Some(#value) },
        None => quote! { ::core::option::Option::None },
    };
    let title = optional(&spec.title);
    let route = optional(&spec.route);
    let guard = optional(&spec.guard);
    let hint = |value: Option<bool>| match value {
        Some(value) => quote! { ::core::option::Option::Some(#value) },
        None => quote! { ::core::option::Option::None },
    };
    let read_only = hint(spec.annotations.read_only);
    let destructive = hint(spec.annotations.destructive);
    let idempotent = hint(spec.annotations.idempotent);
    let open_world = hint(spec.annotations.open_world);
    let untrusted_content = hint(spec.annotations.untrusted_content);
    quote! {
        // A second tool of this name in the crate exports a second macro of
        // this name, which the compiler refuses (AGENT-001).
        #[doc(hidden)]
        #[macro_export]
        macro_rules! #marker {
            () => {};
        }

        const _: () = {
            fn __suprnova_tool_handler() -> ::std::any::TypeId {
                ::suprnova::routing::__type_id_of(&#handler)
            }
            ::suprnova::inventory::submit! {
                ::suprnova::agents::ToolRecord::__declare(::suprnova::agents::__ToolDeclaration {
                    name: #name,
                    description: #description,
                    title: #title,
                    callers: &[#(#callers),*],
                    scopes: &[#(#scopes),*],
                    annotations: ::suprnova::agents::ToolAnnotations::__declare(
                        #read_only,
                        #destructive,
                        #idempotent,
                        #open_world,
                        #untrusted_content,
                    ),
                    input_schema: #input_schema,
                    output_schema: #output_schema,
                    handler: __suprnova_tool_handler,
                    handler_name: #display,
                    module: ::core::module_path!(),
                    route: #route,
                    guard: #guard,
                })
            }
        };
    }
}

/// A tool name as an identifier fragment that no other name shares: letters
/// and digits stay, and `_`, `.` and `-` become `_5f`, `_2e` and `_2d`, so
/// `posts.create` and `posts_create` stay apart.
fn escape(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.push_str(&format!("_{:02x}", u32::from(c)));
        }
    }
    out
}

/// Parse the attribute's arguments and check every rule the build enforces.
fn parse_spec(attr: TokenStream) -> syn::Result<ToolSpec> {
    let span = attr.span();
    let mut name: Option<LitStr> = None;
    let mut description: Option<LitStr> = None;
    let mut title: Option<LitStr> = None;
    let mut callers: Option<Vec<Ident>> = None;
    let mut scopes: Vec<Path> = Vec::new();
    let mut input: Option<Type> = None;
    let mut output: Option<Type> = None;
    let mut annotations = Annotations::default();
    let mut route: Option<LitStr> = None;
    let mut guard: Option<LitStr> = None;

    let parser = syn::meta::parser(|meta| {
        let key = meta
            .path
            .get_ident()
            .map(ToString::to_string)
            .unwrap_or_default();
        match key.as_str() {
            "name" => {
                name = Some(literal(meta.value()?, "name")?);
                Ok(())
            }
            "description" => {
                description = Some(literal(meta.value()?, "description")?);
                Ok(())
            }
            "title" => {
                title = Some(literal(meta.value()?, "title")?);
                Ok(())
            }
            "route" => {
                route = Some(literal(meta.value()?, "route")?);
                Ok(())
            }
            "guard" => {
                guard = Some(literal(meta.value()?, "guard")?);
                Ok(())
            }
            "callers" => {
                let stream = meta.value()?;
                let content;
                syn::bracketed!(content in stream);
                let list = Punctuated::<Ident, Token![,]>::parse_terminated(&content)?;
                callers = Some(list.into_iter().collect());
                Ok(())
            }
            "scopes" => {
                let stream = meta.value()?;
                let content;
                syn::bracketed!(content in stream);
                let list = Punctuated::<Path, Token![,]>::parse_terminated(&content)?;
                scopes = list.into_iter().collect();
                Ok(())
            }
            "input" => {
                input = Some(meta.value()?.parse()?);
                Ok(())
            }
            "output" => {
                output = Some(meta.value()?.parse()?);
                Ok(())
            }
            "annotations" => meta.parse_nested_meta(|hint| {
                let key = hint
                    .path
                    .get_ident()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                let value = if hint.input.peek(Token![=]) {
                    hint.value()?.parse::<LitBool>()?.value
                } else {
                    true
                };
                let slot = match key.as_str() {
                    "read_only" => &mut annotations.read_only,
                    "destructive" => &mut annotations.destructive,
                    "idempotent" => &mut annotations.idempotent,
                    "open_world" => &mut annotations.open_world,
                    "untrusted_content" => &mut annotations.untrusted_content,
                    _ => {
                        return Err(hint.error(
                            "unknown tool annotation; expected `read_only`, `destructive`, \
                             `idempotent`, `open_world` or `untrusted_content`",
                        ));
                    }
                };
                *slot = Some(value);
                Ok(())
            }),
            _ => Err(meta.error(
                "unknown #[tool] argument; expected `name`, `description`, `title`, \
                 `callers`, `scopes`, `input`, `output`, `annotations`, `route` or `guard`",
            )),
        }
    });
    parser.parse2(attr)?;

    let name = name.ok_or_else(|| {
        syn::Error::new(
            span,
            "#[tool] needs `name = \"...\"`: the name agents call it by",
        )
    })?;
    check_name(&name)?;
    let description = description.ok_or_else(|| {
        syn::Error::new(
            span,
            "#[tool] needs `description = \"...\"`: an agent reads it to decide when to call \
             the tool",
        )
    })?;
    if description.value().trim().is_empty() {
        return Err(syn::Error::new(
            description.span(),
            "#[tool] needs a description an agent can read: this one is empty",
        ));
    }
    let callers = callers.ok_or_else(|| {
        syn::Error::new(
            span,
            "#[tool] needs `callers = [...]`: the callers it accepts, of `public`, `user` and \
             `agent`",
        )
    })?;
    if callers.is_empty() {
        return Err(syn::Error::new(
            span,
            "#[tool] needs at least one caller in `callers = [...]`",
        ));
    }
    let mut seen = Vec::new();
    for caller in &callers {
        let text = caller.to_string();
        if !CALLER_KINDS.contains(&text.as_str()) {
            return Err(syn::Error::new_spanned(
                caller,
                format!("unknown caller `{text}`; expected `public`, `user` or `agent`"),
            ));
        }
        if seen.contains(&text) {
            return Err(syn::Error::new_spanned(
                caller,
                format!("caller `{text}` is listed twice"),
            ));
        }
        seen.push(text);
    }
    Ok(ToolSpec {
        name,
        description,
        title,
        callers,
        scopes,
        input,
        output,
        annotations,
        route,
        guard,
    })
}

/// A string literal argument, or an error that says why it must be one.
fn literal(input: ParseStream, key: &str) -> syn::Result<LitStr> {
    let expr: Expr = input.parse()?;
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(lit), ..
        }) => Ok(lit),
        other => Err(syn::Error::new_spanned(
            other,
            format!(
                "#[tool] takes `{key}` as a string literal: an agent reads it as an \
                 instruction, so it is fixed when the crate builds and never taken from data"
            ),
        )),
    }
}

/// Check a tool name: 1 to 128 characters of ASCII letters, digits, `_`,
/// `-` and `.`.
fn check_name(name: &LitStr) -> syn::Result<()> {
    let value = name.value();
    if value.is_empty() || value.len() > MAX_NAME_LEN {
        return Err(syn::Error::new(
            name.span(),
            format!(
                "a tool name has 1 to {MAX_NAME_LEN} characters; `{value}` has {}",
                value.chars().count()
            ),
        ));
    }
    if let Some(bad) = value
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
    {
        return Err(syn::Error::new(
            name.span(),
            format!(
                "a tool name uses ASCII letters, digits, `_`, `-` and `.` only; `{value}` \
                 contains {bad:?}"
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{escape, tool_impl_inner};
    use quote::quote;

    fn expand(attr: proc_macro2::TokenStream) -> String {
        tool_impl_inner(
            attr,
            quote! {
                #[handler]
                pub async fn store() -> Response { body() }
            },
        )
        .to_string()
    }

    #[test]
    fn agent_001_names_escape_without_collisions() {
        assert_eq!(escape("posts.create"), "posts_2ecreate");
        assert_eq!(escape("posts_create"), "posts_5fcreate");
        assert_eq!(escape("posts-create"), "posts_2dcreate");
        assert_ne!(escape("a.b"), escape("a_b"));
    }

    #[test]
    fn agent_001_a_valid_tool_emits_the_marker_and_the_record() {
        let out = expand(quote! {
            name = "posts.create", description = "Create a post.", callers = [user]
        });
        assert!(out.contains("macro_export"), "{out}");
        assert!(out.contains("__suprnova_tool_posts_2ecreate"), "{out}");
        assert!(out.contains("ToolRecord :: __declare"), "{out}");
    }

    #[test]
    fn agent_001_rejects_bad_names_and_descriptions() {
        for (attr, message) in [
            (
                quote! { name = "", description = "d", callers = [user] },
                "1 to 128",
            ),
            (
                quote! { name = "has space", description = "d", callers = [user] },
                "contains",
            ),
            (
                quote! { name = "x", description = "  ", callers = [user] },
                "empty",
            ),
            (
                quote! { name = "x", callers = [user] },
                "needs `description",
            ),
            (
                quote! { description = "d", callers = [user] },
                "needs `name",
            ),
            (
                quote! { name = "x", description = DESCRIPTION, callers = [user] },
                "string literal",
            ),
            (quote! { name = "x", description = "d" }, "needs `callers"),
            (
                quote! { name = "x", description = "d", callers = [robot] },
                "unknown caller",
            ),
        ] {
            let out = expand(attr);
            assert!(out.contains("compile_error"), "{out}");
            assert!(out.contains(message), "expected {message:?} in {out}");
        }
        let long = "a".repeat(129);
        let out = expand(quote! { name = #long, description = "d", callers = [user] });
        assert!(out.contains("1 to 128"), "{out}");
    }

    #[test]
    fn agent_001_rejects_a_tool_without_handler() {
        let out = tool_impl_inner(
            quote! { name = "x", description = "d", callers = [user] },
            quote! { pub async fn store() -> Response { body() } },
        )
        .to_string();
        assert!(out.contains("works only on a `#[handler]`"), "{out}");
    }
}
