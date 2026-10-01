use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use std::path::{Path, PathBuf};
use syn::{DeriveInput, Expr, LitStr, Token, parse::Parse, parse::ParseStream, parse_macro_input};

/// Props can be either a typed struct expression or JSON-like syntax.
pub enum PropsKind {
    /// Typed struct: `HomeProps { title: "Welcome".into(), user }`
    Typed(Expr),
    /// JSON-like syntax: `{ "title": "Welcome" }`
    Json(proc_macro2::TokenStream),
}

/// Parsed `inertia_response!` invocation:
///
/// ```ignore
/// inertia_response!(&req, "Component", PropsExpr [, ConfigExpr])
/// ```
///
/// The leading request argument was introduced when we removed the
/// `thread_local!` `InertiaContext` - see `docs/parity/inertia.md` Tier 0.
pub struct InertiaResponseInput {
    pub request: Expr,
    pub component: LitStr,
    pub props: PropsKind,
    pub config: Option<Expr>,
}

impl Parse for InertiaResponseInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let request: Expr = input.parse()?;
        let _: Token![,] = input.parse()?;
        let component: LitStr = input.parse()?;
        let _: Token![,] = input.parse()?;

        // Determine if props are a typed struct or JSON-like.
        let props = if input.peek(syn::Ident) {
            let expr: Expr = input.parse()?;
            PropsKind::Typed(expr)
        } else {
            let props_content;
            syn::braced!(props_content in input);
            let props_tokens: proc_macro2::TokenStream = props_content.parse()?;
            PropsKind::Json(props_tokens)
        };

        let config = if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
            Some(input.parse::<Expr>()?)
        } else {
            None
        };

        Ok(InertiaResponseInput {
            request,
            component,
            props,
            config,
        })
    }
}

/// Implementation for the `InertiaProps` derive macro
pub fn derive_inertia_props_impl(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let fields = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            syn::Fields::Named(fields) => &fields.named,
            _ => {
                return syn::Error::new_spanned(
                    &input,
                    "InertiaProps only supports structs with named fields",
                )
                .to_compile_error()
                .into();
            }
        },
        _ => {
            return syn::Error::new_spanned(&input, "InertiaProps can only be derived for structs")
                .to_compile_error()
                .into();
        }
    };

    let container = match crate::serde_attrs::parse_container(&input.attrs, "InertiaProps") {
        Ok(container) => container,
        Err(e) => return e.to_compile_error().into(),
    };
    let mut field_names = Vec::new();
    let mut field_name_strings: Vec<String> = Vec::new();
    for field in fields {
        let names = match crate::serde_attrs::field_names(field, &container, "InertiaProps") {
            Ok(names) => names,
            Err(e) => return e.to_compile_error().into(),
        };
        // Props are only serialized: the derive reads the serialize side of
        // the attributes, and leaves a `deserialize` half and
        // `skip_deserializing` to a `Deserialize` derive on the same struct.
        if names.skip_serializing {
            continue;
        }
        if field_name_strings.contains(&names.serialize) {
            return syn::Error::new_spanned(
                field,
                format!(
                    "two props are sent under the key `{}`; give each its own name",
                    names.serialize
                ),
            )
            .to_compile_error()
            .into();
        }
        field_names.push(&field.ident);
        field_name_strings.push(names.serialize);
    }
    let field_count = field_names.len();

    let expanded = quote! {
        impl #impl_generics ::suprnova::serde::Serialize for #name #ty_generics #where_clause {
            fn serialize<S>(&self, serializer: S) -> ::core::result::Result<S::Ok, S::Error>
            where
                S: ::suprnova::serde::Serializer,
            {
                use ::suprnova::serde::ser::SerializeStruct;
                let mut state = serializer.serialize_struct(stringify!(#name), #field_count)?;
                #(
                    state.serialize_field(#field_name_strings, &self.#field_names)?;
                )*
                state.end()
            }
        }
    };

    expanded.into()
}

/// Implementation for the `inertia_response!` macro
pub fn inertia_response_impl(input: TokenStream) -> TokenStream {
    let parsed_input = parse_macro_input!(input as InertiaResponseInput);
    inertia_response_inner(parsed_input).into()
}

/// Pure-`proc_macro2` helper so unit tests can exercise the
/// expansion shape without leaving the proc-macro crate.
/// The validation against the frontend filesystem happens here too -
/// it's a no-op when `CARGO_MANIFEST_DIR` is unset (e.g. some IDE
/// states), exactly as before.
fn inertia_response_inner(input: InertiaResponseInput) -> proc_macro2::TokenStream {
    let component_name = input.component.value();
    let component_lit = &input.component;

    let tracked = match validate_component_exists(&component_name, component_lit.span()) {
        Ok(tracked) => tracked,
        Err(err) => return err.to_compile_error(),
    };

    track_inputs(render_inertia_response_expansion(&input), &tracked)
}

/// Prefix the expansion with one `include_bytes!` per file the check read.
///
/// Cargo re-runs a proc macro only when a file in the crate's dep-info
/// changes, and files a macro opens on its own never reach it: without
/// this, deleting a page or editing the lookup table in `Cargo.toml` (cargo
/// does not fingerprint `[package.metadata]`) leaves a stale check in place
/// until some Rust file changes. Naming the files in `include_bytes!` puts
/// them in the dep-info. The constants are unnamed and unused, so nothing
/// reaches the binary.
fn track_inputs(expansion: proc_macro2::TokenStream, files: &[PathBuf]) -> proc_macro2::TokenStream {
    let paths: Vec<LitStr> = files
        .iter()
        .filter_map(|file| file.to_str())
        .map(|file| LitStr::new(file, Span::call_site()))
        .collect();
    if paths.is_empty() {
        return expansion;
    }
    quote! {{
        #( const _: &[u8] = ::core::include_bytes!(#paths); )*
        #expansion
    }}
}

/// Render the macro expansion proper - the value-expr, the match
/// over the Object branch, and the trailing `resolve(...).await`.
/// Split out of `inertia_response_inner` so unit tests can target
/// the expansion shape without tripping the frontend-filesystem
/// validation that the outer helper applies.
fn render_inertia_response_expansion(input: &InertiaResponseInput) -> proc_macro2::TokenStream {
    let component_name = input.component.value();
    let component_lit = &input.component;
    let request_expr = &input.request;

    // Materialize props as a `serde_json::Value`, then unfold into individual
    // eager props on the `InertiaResponse` builder. Unfolding (one prop per
    // top-level key) is what makes partial-reload filtering work - the
    // framework needs to know each prop's name to honor X-Inertia-Partial-Data.
    //
    // Domain 5 audit M-D5-2: previously the typed-props branch panicked via
    // `.expect(...)` on a `Serialize` failure, and the non-object branch
    // panicked when the props expression resolved to a non-object Value.
    // Both now propagate `FrameworkError::internal` through the macro's
    // already-Result-returning expansion. The macro MUST be invoked in a
    // function whose return type accepts `FrameworkError` via `From`
    // (typically `Result<InertiaResponse, FrameworkError>` or the
    // framework's blanket `Result<_, E>` where `FrameworkError: Into<E>`);
    // this matches the existing contract on the `resolve(...).await`
    // call's `.map_err(Into::into)`.
    let value_expr = match &input.props {
        PropsKind::Typed(expr) => quote! {
            ::suprnova::serde_json::to_value(&#expr)
                .map_err(|__se| ::suprnova::FrameworkError::internal(::std::format!(
                    "inertia_response!({}): typed props failed to serialize: {}",
                    #component_name,
                    __se,
                )))?
        },
        PropsKind::Json(tokens) => quote! {
            ::suprnova::serde_json::json!({#tokens})
        },
    };

    let config_setup = match &input.config {
        Some(cfg) => quote! { __response = __response.with_config(#cfg); },
        None => quote! {},
    };

    let expanded = quote! {{
        let __value: ::suprnova::serde_json::Value = #value_expr;
        let mut __response = ::suprnova::InertiaResponse::new(#component_lit);
        #config_setup
        // resolve() is async (Lazy/Optional props may await). Errors flow
        // through the framework's Response type via the existing
        // From<FrameworkError> for HttpResponse conversion. The non-object
        // branch returns Err directly - Inertia v3's protocol requires
        // `props` to be an object, so a non-Object payload would render an
        // invalid response.
        match __value {
            ::suprnova::serde_json::Value::Object(__map) => {
                for (__k, __v) in __map {
                    __response.__add_eager(__k, __v);
                }
                __response
                    .resolve(#request_expr)
                    .await
                    .map_err(::core::convert::Into::into)
            }
            __other => ::core::result::Result::Err(
                ::core::convert::Into::into(
                    ::suprnova::FrameworkError::internal(::std::format!(
                        "inertia_response!({}): page props must serialize to a \
                         JSON object, got {}",
                        #component_name,
                        __other,
                    ))
                )
            ),
        }
    }};

    expanded
}

/// Checks the component against the crate's page lookup and returns the
/// files the check read, for [`track_inputs`].
fn validate_component_exists(component_name: &str, span: Span) -> Result<Vec<PathBuf>, syn::Error> {
    let manifest_dir = match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(dir) => dir,
        Err(_) => {
            // In environments where CARGO_MANIFEST_DIR isn't set (some IDEs,
            // rust-analyzer in odd states), skip validation gracefully.
            return Ok(Vec::new());
        }
    };

    crate::inertia_pages::check_component(Path::new(&manifest_dir), component_name)
        .map_err(|message| syn::Error::new(span, message))
}

#[cfg(test)]
mod tests {
    //! Domain 5 audit M-D5-2 regression: the `inertia_response!`
    //! expansion must propagate Serialize failures and non-Object
    //! props via `FrameworkError::internal` rather than panicking.
    //! Without these checks, a deploy that exposed a buggy `Serialize`
    //! impl would panic per-request - Domain 2's middleware safety
    //! net catches the panic but the operator loses the structured
    //! error path.

    use super::*;
    use syn::parse_quote;

    /// Render the macro on a typed-props input and assert the typed
    /// serialization branch propagates via `?` against `FrameworkError`,
    /// not `.expect`.
    #[test]
    fn typed_props_branch_uses_question_mark_not_expect() {
        let parsed: InertiaResponseInput = parse_quote! {
            &req, "Home", HomeProps { title: "Welcome".to_string() }
        };
        let rendered = render_inertia_response_expansion(&parsed).to_string();
        assert!(
            !rendered.contains(".expect ("),
            "typed-props branch must not panic via .expect; got: {rendered}"
        );
        assert!(
            rendered.contains("FrameworkError :: internal"),
            "typed-props branch must wrap Serialize errors in FrameworkError; got: {rendered}"
        );
        assert!(
            rendered.contains("typed props failed to serialize"),
            "typed-props branch must carry diagnostic; got: {rendered}"
        );
    }

    /// The non-Object branch must return `Err`, not panic, when the
    /// runtime Value is a non-Object (the v3 protocol requires
    /// `props` to be an object).
    #[test]
    fn non_object_branch_returns_err_not_panic() {
        let parsed: InertiaResponseInput = parse_quote! {
            &req, "Home", PropsAsString
        };
        let rendered = render_inertia_response_expansion(&parsed).to_string();
        assert!(
            !rendered.contains("panic !"),
            "non-Object branch must not panic; got: {rendered}"
        );
        // The Err arm carries the FrameworkError::internal carrying
        // the component name for debuggability. The source-level
        // line continuation in the macro's format string preserves
        // the backslash when round-tripped through `quote!.to_string()`,
        // so the assertion checks just the leading portion.
        assert!(
            rendered.contains("page props must serialize to a"),
            "Err arm must carry diagnostic; got: {rendered}"
        );
        assert!(
            rendered.contains("JSON object"),
            "Err arm message must mention JSON object requirement; got: {rendered}"
        );
        assert!(
            rendered.contains("Result :: Err"),
            "non-object branch must return Result::Err; got: {rendered}"
        );
    }

    /// JSON-syntax props go through `serde_json::json!` which is
    /// infallible - the typed-only error wrapper must NOT appear in
    /// the JSON branch's `value_expr`. (Both branches share the
    /// non-Object Err arm; this assertion targets just the value
    /// production phase.)
    #[test]
    fn json_props_branch_does_not_wrap_in_framework_error() {
        let parsed: InertiaResponseInput = parse_quote! {
            &req, "Home", { "title": "Welcome" }
        };
        let rendered = render_inertia_response_expansion(&parsed).to_string();
        // The JSON-shape props expression is constructed via
        // `serde_json::json!({...})` and never goes through the
        // Serialize-failure wrapper, so the typed-only error string
        // must not appear in the rendered output.
        assert!(
            !rendered.contains("typed props failed to serialize"),
            "JSON branch should not carry the typed-props error wrapper; got: {rendered}"
        );
        // The block must still terminate in a Result-returning
        // resolve/match shape so the caller's `?`/`Into` work.
        assert!(
            rendered.contains("resolve") && rendered.contains("map_err"),
            "block must end in resolve(...).map_err(...) shape; got: {rendered}"
        );
    }

    /// The page file and the manifest the lookup came from are read by the
    /// macro, not by rustc, so cargo does not know about them. Naming them
    /// in `include_bytes!` puts them in the crate's dep-info, which re-runs
    /// the check when either changes.
    #[test]
    fn tracked_inputs_become_include_bytes_items() {
        let parsed: InertiaResponseInput = parse_quote! {
            &req, "Home", { "title": "Welcome" }
        };
        let expansion = render_inertia_response_expansion(&parsed);
        let rendered = track_inputs(
            expansion.clone(),
            &[
                PathBuf::from("/srv/app/Cargo.toml"),
                PathBuf::from("/srv/app/frontend/src/pages/Home.svelte"),
            ],
        )
        .to_string();
        assert!(
            rendered.contains("include_bytes ! (\"/srv/app/Cargo.toml\")"),
            "the manifest must be tracked; got: {rendered}"
        );
        assert!(
            rendered.contains("include_bytes ! (\"/srv/app/frontend/src/pages/Home.svelte\")"),
            "the page must be tracked; got: {rendered}"
        );
        assert!(
            rendered.contains(&expansion.to_string()),
            "the expansion must follow the tracking items; got: {rendered}"
        );
    }

    #[test]
    fn no_tracked_inputs_leave_the_expansion_unchanged() {
        let parsed: InertiaResponseInput = parse_quote! {
            &req, "Home", { "title": "Welcome" }
        };
        let expansion = render_inertia_response_expansion(&parsed);
        assert_eq!(
            track_inputs(expansion.clone(), &[]).to_string(),
            expansion.to_string()
        );
    }
}
