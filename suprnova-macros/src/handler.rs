//! Handler attribute macro implementation
//!
//! Transforms controller functions to automatically extract typed parameters
//! from HTTP requests, including path parameters and route model binding.
//!
//! ## Extractor combination rules
//!
//! The macro generates a single transformed `fn(req: Request)` shape that
//! moves `req` into at most one of the body-consuming extractors. `Request`
//! and `FormRequest` both consume the request body, so the macro **rejects
//! at expansion time** any combination with more than one of them - the
//! emitted code would otherwise trip E0382 (use of moved value) with no
//! actionable diagnostic for the user.
//!
//! Legal shapes (compile):
//! - zero params (e.g. `fn index()`)
//! - a single `Request` (e.g. `fn show(req: Request)`)
//! - a single `FormRequest`-derived extractor (e.g. `fn store(form: CreateUser)`)
//! - any number of `Primitive` + `Model` params alongside at most one consumer
//!   (e.g. `fn update(user: user::Model, form: UpdateUser)`)
//!
//! Rejected at expansion (clear macro error):
//! - two or more `FormRequest` params
//! - `Request` plus any `FormRequest` param
//!
//! ## `#[authorize]`
//!
//! The macro applies every `#[authorize(...)]` on the function (parsed in
//! `authorize.rs`). Without one, the extractions run in declaration order,
//! as always. With one, the route-bound extractions (`Primitive`, `Model`)
//! run first, then one gate check per attribute in the order written, then
//! the body-consuming extraction, then the body. That is the order of
//! Laravel's `SubstituteBindings` and `can` middleware ahead of a form
//! request: the check sees the bound model, a missing model is a 404 before
//! the check, and a denied request never reaches validation.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{FnArg, Ident, ItemFn, Pat, PatType, Type};

use crate::authorize::{AuthorizeSpec, Target, is_authorize_attr, parse_spec};

/// Parameter classification for extraction strategy
enum ParamKind {
    /// Request type - pass through unchanged
    Request,
    /// Primitive type (i32, String, etc.) - extract from path params via FromParam
    Primitive,
    /// Model type (*::Model) - extract via RouteBinding
    Model,
    /// Other types - extract via FromRequest (FormRequest, etc.)
    FormRequest,
}

/// Implementation of the `#[handler]` attribute macro
///
/// Supports multiple parameter extraction:
///
/// - `Request` - passes through unchanged
/// - Primitives (`i32`, `String`, etc.) - extracted from path params via `FromParam`
/// - Model types (`user::Model`) - extracted via `RouteBinding` (auto 404 if not found)
/// - Other types - extracted via `FromRequest` (FormRequest validation)
///
/// # Examples
///
/// ```rust,ignore
/// // No parameters
/// #[handler]
/// pub async fn index() -> Response { ... }
///
/// // Request passthrough
/// #[handler]
/// pub async fn show(req: Request) -> Response { ... }
///
/// // Path parameter extraction
/// #[handler]
/// pub async fn show(id: i32) -> Response { ... }
///
/// // Route model binding
/// #[handler]
/// pub async fn show(user: user::Model) -> Response { ... }
///
/// // FormRequest validation
/// #[handler]
/// pub async fn store(form: CreateUserRequest) -> Response { ... }
///
/// // Mixed parameters
/// #[handler]
/// pub async fn update(user: user::Model, form: UpdateUserRequest) -> Response { ... }
/// ```
pub fn handler_impl(_attr: TokenStream, input: TokenStream) -> TokenStream {
    handler_impl_inner(input.into()).into()
}

/// `proc_macro2`-flavoured entry point. The outer `handler_impl` is a thin
/// shim that converts the host `proc_macro::TokenStream`. Splitting the work
/// here lets the unit tests below feed in token streams directly and assert
/// on the rendered output - the host `proc_macro::TokenStream` cannot be
/// constructed outside a real macro-expansion context.
fn handler_impl_inner(input: TokenStream2) -> TokenStream2 {
    let mut input_fn: ItemFn = match syn::parse2(input) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error(),
    };

    // `#[authorize]` attributes are applied here, so take them off the
    // function: left on, each would expand on its own.
    let (authorize_attrs, fn_attrs): (Vec<_>, Vec<_>) = std::mem::take(&mut input_fn.attrs)
        .into_iter()
        .partition(is_authorize_attr);
    let mut specs = Vec::with_capacity(authorize_attrs.len());
    for attr in &authorize_attrs {
        match parse_spec(attr) {
            Ok(spec) => specs.push(spec),
            Err(e) => return e.to_compile_error(),
        }
    }

    let fn_vis = &input_fn.vis;
    let fn_name = &input_fn.sig.ident;
    let fn_generics = &input_fn.sig.generics;
    let fn_output = &input_fn.sig.output;
    let fn_block = &input_fn.block;

    let is_async = input_fn.sig.asyncness.is_some();
    let async_token = if is_async {
        quote! { async }
    } else {
        quote! {}
    };

    if let Some(first) = authorize_attrs.first()
        && !is_async
    {
        return syn::Error::new_spanned(
            first,
            "#[authorize] needs an `async fn` handler: the check awaits the gate",
        )
        .to_compile_error();
    }

    // Collect all parameters
    let params: Vec<_> = input_fn.sig.inputs.iter().collect();

    // First pass: classify every param and count the body-consuming
    // extractors so we can reject `Request` + `FormRequest` /
    // `FormRequest` × 2 / etc. with a clear macro error before we try
    // to emit code that would move `__suprnova_req` twice and trip E0382.
    let mut classifications = Vec::with_capacity(params.len());
    let mut request_consumer_count = 0usize;
    let mut last_consumer_span: Option<&FnArg> = None;
    for param in &params {
        let pat_type = match param {
            FnArg::Typed(pt) => pt,
            FnArg::Receiver(_) => {
                return syn::Error::new_spanned(
                    param,
                    "#[handler] does not support methods with self receiver",
                )
                .to_compile_error();
            }
        };
        let kind = classify_param_type(&pat_type.ty);
        if matches!(kind, ParamKind::Request | ParamKind::FormRequest) {
            request_consumer_count += 1;
            last_consumer_span = Some(*param);
        }
        classifications.push((pat_type, kind));
    }

    let checks = match authorize_checks(&specs, &classifications, fn_name) {
        Ok(checks) => checks,
        Err(e) => return e.to_compile_error(),
    };

    // Handle no parameters case
    if params.is_empty() {
        return quote! {
            #(#fn_attrs)*
            #fn_vis #async_token fn #fn_name #fn_generics(_: ::suprnova::Request) #fn_output {
                #(#checks)*
                #fn_block
            }
        };
    }

    if request_consumer_count > 1 {
        // Point the diagnostic at the most-recent offending parameter so
        // the user's eye lands somewhere meaningful in the signature.
        let span_target = last_consumer_span.unwrap_or(params[0]);
        return syn::Error::new_spanned(
            span_target,
            "#[handler] supports at most one body-consuming extractor \
             per signature (Request or any FormRequest). Combining two \
             would move the underlying `Request` twice. Split the work \
             across separate handlers, or fold the extra extractor into \
             a single FormRequest struct.",
        )
        .to_compile_error();
    }

    // Second pass: emit extractions now that we know the signature is legal.
    // With checks, the body-consuming extraction waits until they pass (see
    // the module docs); without, every extraction keeps its place.
    let mut extractions = Vec::with_capacity(classifications.len());
    let mut body_extractions = Vec::new();
    for (pat_type, kind) in &classifications {
        let param_pat = &pat_type.pat;
        let param_type = &pat_type.ty;
        let param_name = extract_param_name(param_pat);
        let extraction = generate_extraction(param_pat, param_type, &param_name, kind);
        if !checks.is_empty() && reads_body(kind) {
            body_extractions.push(extraction);
        } else {
            extractions.push(extraction);
        }
    }

    quote! {
        #(#fn_attrs)*
        #fn_vis #async_token fn #fn_name #fn_generics(__suprnova_req: ::suprnova::Request) #fn_output {
            let __suprnova_params = __suprnova_req.params().clone();
            #(#extractions)*
            #(#checks)*
            #(#body_extractions)*
            #fn_block
        }
    }
}

/// Whether an extraction of this kind consumes the request.
fn reads_body(kind: &ParamKind) -> bool {
    matches!(kind, ParamKind::Request | ParamKind::FormRequest)
}

/// Emit one gate check per `#[authorize]`, in the order written.
///
/// A parameter target must be a parameter of the handler, bound by an
/// identifier pattern, that the route supplies; anything else is a
/// compile error spanned on the name in the attribute.
fn authorize_checks(
    specs: &[AuthorizeSpec],
    classifications: &[(&PatType, ParamKind)],
    fn_name: &Ident,
) -> syn::Result<Vec<TokenStream2>> {
    specs
        .iter()
        .map(|spec| {
            let ability = &spec.ability;
            let name = match &spec.target {
                Target::Type(ty) => {
                    return Ok(quote! {
                        ::suprnova::authorization::__authorize_handler_type::<#ty>(#ability).await?;
                    });
                }
                Target::Param(name) => name,
            };
            let found = classifications.iter().find(
                |(pat_type, _)| matches!(&*pat_type.pat, Pat::Ident(pat) if pat.ident == *name),
            );
            let Some((pat_type, kind)) = found else {
                return Err(syn::Error::new_spanned(
                    name,
                    format!(
                        "#[authorize] names `{name}`, but `{fn_name}` takes no parameter \
                         named `{name}`; name a parameter the route binds, written \
                         as `{name}: RouteParam<Model>`"
                    ),
                ));
            };
            if reads_body(kind) {
                return Err(syn::Error::new_spanned(
                    name,
                    format!(
                        "#[authorize] cannot check `{name}`: it reads the request body, \
                         and the check runs before the body is read; name a route-bound \
                         model (`RouteParam<M>` or `...::Model`) or a path parameter"
                    ),
                ));
            }
            // A `RouteParam<M>` is checked as the `M` inside it, the type
            // its policy is registered for.
            let resource = if is_route_param(&pat_type.ty) {
                quote! { ::core::ops::Deref::deref(&#name) }
            } else {
                quote! { &#name }
            };
            Ok(quote! {
                ::suprnova::authorization::__authorize_handler(#ability, #resource).await?;
            })
        })
        .collect()
}

/// Extract the parameter name as a string from the pattern
fn extract_param_name(pat: &Pat) -> String {
    match pat {
        Pat::Ident(pat_ident) => pat_ident.ident.to_string(),
        Pat::Wild(_) => "_".to_string(),
        _ => "param".to_string(),
    }
}

/// Classify the parameter type to determine extraction strategy
fn classify_param_type(ty: &Type) -> ParamKind {
    match ty {
        Type::Path(type_path) => {
            let segments = &type_path.path.segments;

            // Check for Request type
            if segments.len() == 1 && segments[0].ident == "Request" {
                return ParamKind::Request;
            }
            if segments.len() == 2
                && segments[0].ident == "suprnova"
                && segments[1].ident == "Request"
            {
                return ParamKind::Request;
            }

            // Check for primitive types
            if segments.len() == 1 {
                let ident = segments[0].ident.to_string();
                if is_primitive_type_name(&ident) {
                    return ParamKind::Primitive;
                }
            }

            // Check for Model type (path ends with ::Model) - the
            // unscoped escape hatch. See `RouteParam<M>` below for
            // the scoped default.
            if let Some(last_segment) = segments.last()
                && last_segment.ident == "Model"
                && segments.len() >= 2
            {
                return ParamKind::Model;
            }

            // Check for RouteParam<M> - the scoped binding wrapper.
            // Routes through M::find (Eloquent's CRUD entrypoint) so
            // global scopes, soft-delete filter, and per-model
            // connection apply. See `suprnova::RouteParam` rustdoc.
            if is_route_param(ty) {
                return ParamKind::Model;
            }

            // Default to FormRequest for other types
            ParamKind::FormRequest
        }
        _ => ParamKind::FormRequest,
    }
}

/// Whether `ty` is the scoped binding wrapper `RouteParam<M>`.
fn is_route_param(ty: &Type) -> bool {
    matches!(ty, Type::Path(type_path)
        if type_path.path.segments.last().is_some_and(|last| last.ident == "RouteParam"))
}

/// Check if a type name is a primitive that should use FromParam
fn is_primitive_type_name(name: &str) -> bool {
    matches!(
        name,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "isize"
            | "String"
    )
}

/// Generate extraction code for a parameter based on its classification.
///
/// The caller is responsible for ensuring at most one of the body-consuming
/// kinds (`Request`, `FormRequest`) is emitted per signature; this fn does
/// not re-check.
fn generate_extraction(pat: &Pat, ty: &Type, param_name: &str, kind: &ParamKind) -> TokenStream2 {
    match kind {
        ParamKind::Request => quote! {
            let #pat: #ty = __suprnova_req;
        },
        ParamKind::Primitive => {
            // Extract from path params using FromParam
            quote! {
                let #pat: #ty = {
                    let __value = __suprnova_params.get(#param_name)
                        .ok_or_else(|| ::suprnova::FrameworkError::param(#param_name))?;
                    <#ty as ::suprnova::FromParam>::from_param(__value)?
                };
            }
        }
        ParamKind::Model => {
            // Route model binding using AutoRouteBinding trait
            // The parameter name comes from the function signature
            quote! {
                let #pat: #ty = {
                    let __value = __suprnova_params.get(#param_name)
                        .ok_or_else(|| ::suprnova::FrameworkError::param(#param_name))?;
                    <#ty as ::suprnova::AutoRouteBinding>::from_route_param(__value).await?
                };
            }
        }
        ParamKind::FormRequest => quote! {
            let #pat: #ty = <#ty as ::suprnova::FromRequest>::from_request(__suprnova_req).await?;
        },
    }
}

#[cfg(test)]
mod tests {
    //! Macro-expansion regressions for the body-consumer constraint.
    //!
    //! Two-FormRequest, Request + FormRequest, and friends would emit
    //! `let _ = __suprnova_req; let _ = …(__suprnova_req).await?;` -
    //! moving the same value twice. rustc reports E0382 deep inside
    //! generated code, far from the user's signature, with no hint at
    //! the actual constraint. The macro now rejects those signatures
    //! at expansion with a single span-pointed diagnostic.
    //!
    //! Legal signatures (zero/one consumer, plus any Primitive/Model
    //! params) must keep round-tripping cleanly.
    use super::*;
    use quote::quote;

    /// Render `handler_impl_inner` against a function and look for a
    /// span-rendered diagnostic marker. `compile_error! { … }` is the
    /// surface form `syn::Error::to_compile_error()` produces, so a
    /// rejection produces a token stream whose string form contains
    /// the macro path and our message.
    fn expansion(src: proc_macro2::TokenStream) -> String {
        handler_impl_inner(src).to_string()
    }

    #[test]
    fn rejects_two_form_request_params() {
        let out = expansion(quote! {
            pub async fn store(a: CreateUser, b: UpdateUser) -> Response { todo!() }
        });
        assert!(
            out.contains("compile_error"),
            "two FormRequest params must reject; got:\n{out}"
        );
        assert!(
            out.contains("body-consuming"),
            "rejection message must mention the constraint; got:\n{out}"
        );
    }

    #[test]
    fn rejects_request_plus_form_request() {
        let out = expansion(quote! {
            pub async fn store(req: Request, form: CreateUser) -> Response { todo!() }
        });
        assert!(
            out.contains("compile_error"),
            "Request + FormRequest must reject; got:\n{out}"
        );
    }

    #[test]
    fn rejects_three_form_request_params() {
        // Defensive: the cap is "at most one", not "exactly two".
        let out = expansion(quote! {
            pub async fn store(a: A, b: B, c: C) -> Response { todo!() }
        });
        assert!(
            out.contains("compile_error"),
            "three FormRequest params must reject; got:\n{out}"
        );
    }

    #[test]
    fn accepts_single_request() {
        let out = expansion(quote! {
            pub async fn show(req: Request) -> Response { todo!() }
        });
        assert!(
            !out.contains("compile_error"),
            "single Request must compile; got:\n{out}"
        );
        // Confirms the request actually got forwarded into the body.
        assert!(out.contains("__suprnova_req"));
    }

    #[test]
    fn accepts_single_form_request() {
        let out = expansion(quote! {
            pub async fn store(form: CreateUser) -> Response { todo!() }
        });
        assert!(
            !out.contains("compile_error"),
            "single FormRequest must compile; got:\n{out}"
        );
        assert!(out.contains("from_request"));
    }

    #[test]
    fn accepts_zero_params() {
        let out = expansion(quote! {
            pub async fn index() -> Response { todo!() }
        });
        assert!(
            !out.contains("compile_error"),
            "zero-param handler must compile; got:\n{out}"
        );
    }

    #[test]
    fn accepts_model_plus_form_request_mix() {
        // The documented Mixed example: `update(user: user::Model,
        // form: UpdateUserRequest)`. Model reads from the cloned
        // params map and never touches `__suprnova_req`, so this
        // counts as one body-consumer overall - legal.
        let out = expansion(quote! {
            pub async fn update(user: user::Model, form: UpdateUserRequest) -> Response { todo!() }
        });
        assert!(
            !out.contains("compile_error"),
            "Model + FormRequest must compile (Model is non-consuming); got:\n{out}"
        );
    }

    #[test]
    fn accepts_primitive_plus_form_request_mix() {
        let out = expansion(quote! {
            pub async fn update(id: i32, form: UpdateUserRequest) -> Response { todo!() }
        });
        assert!(
            !out.contains("compile_error"),
            "Primitive + FormRequest must compile (Primitive is non-consuming); got:\n{out}"
        );
    }

    #[test]
    fn accepts_primitive_plus_request_mix() {
        // Request is a consumer, but only ONE of it - Primitive reads
        // from the cloned params clone, so the combination is legal.
        let out = expansion(quote! {
            pub async fn show(id: i32, req: Request) -> Response { todo!() }
        });
        assert!(
            !out.contains("compile_error"),
            "Primitive + Request must compile (Primitive is non-consuming); got:\n{out}"
        );
    }

    // ── #[authorize] ─────────────────────────────────────────────────────────

    /// Byte offset of `needle` in `out`, failing the test when it is absent.
    fn position(out: &str, needle: &str) -> usize {
        out.find(needle)
            .unwrap_or_else(|| panic!("`{needle}` missing from expansion:\n{out}"))
    }

    #[test]
    fn authorize_param_missing_from_the_signature_is_a_compile_error() {
        let out = expansion(quote! {
            #[authorize("update", post)]
            pub async fn update(id: i64) -> Response { todo!() }
        });
        assert!(out.contains("compile_error"), "got:\n{out}");
        assert!(
            out.contains("`update` takes no parameter named `post`"),
            "the message must name the handler and the missing parameter; got:\n{out}"
        );
    }

    #[test]
    fn authorize_names_the_binding_inside_a_route_param_pattern() {
        // `RouteParam(post)` binds `post` to the model inside the wrapper,
        // so the check runs against `post` itself, with no `Deref`.
        let out = expansion(quote! {
            #[authorize("update", post)]
            pub async fn update(RouteParam(post): RouteParam<Post>) -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(
            out.contains("__authorize_handler (\"update\" , & post)"),
            "got:\n{out}"
        );
        assert!(!out.contains("Deref"), "got:\n{out}");
    }

    #[test]
    fn destructured_route_param_is_looked_up_by_its_binding() {
        // The route parameter is named after the pattern's binding, as for
        // a plain identifier: `RouteParam(user)` reads `{user}`.
        for param in [
            quote! { RouteParam(user): RouteParam<User> },
            quote! { RouteParam(mut user): RouteParam<User> },
            quote! { suprnova::RouteParam(user): suprnova::RouteParam<User> },
            quote! { user: RouteParam<User> },
        ] {
            let out = expansion(quote! {
                pub async fn show(#param) -> Response { todo!() }
            });
            assert!(!out.contains("compile_error"), "got:\n{out}");
            assert!(
                out.contains("get (\"user\")"),
                "`{param}` must read the route parameter `user`; got:\n{out}"
            );
            assert!(!out.contains("\"param\""), "got:\n{out}");
        }
    }

    #[test]
    fn route_bound_pattern_without_one_binding_is_a_compile_error() {
        // No single binding to name the route parameter after: reject it
        // rather than read a parameter the route does not have.
        for param in [
            quote! { user::Model { id, name, .. }: user::Model },
            quote! { RouteParam(User { id, .. }): RouteParam<User> },
        ] {
            let out = expansion(quote! {
                pub async fn show(#param) -> Response { todo!() }
            });
            assert!(
                out.contains("compile_error"),
                "`{param}` must be rejected; got:\n{out}"
            );
            assert!(out.contains("route parameter"), "got:\n{out}");
        }
    }

    #[test]
    fn authorize_param_that_reads_the_body_is_a_compile_error() {
        let out = expansion(quote! {
            #[authorize("update", form)]
            pub async fn update(form: UpdatePost) -> Response { todo!() }
        });
        assert!(out.contains("compile_error"), "got:\n{out}");
        assert!(
            out.contains("request body"),
            "the message must say why a body extractor cannot be named; got:\n{out}"
        );

        let out = expansion(quote! {
            #[authorize("update", req)]
            pub async fn update(req: Request) -> Response { todo!() }
        });
        assert!(out.contains("compile_error"), "got:\n{out}");
    }

    #[test]
    fn authorize_on_a_sync_handler_is_a_compile_error() {
        let out = expansion(quote! {
            #[authorize("create", Post)]
            pub fn store() -> Response { todo!() }
        });
        assert!(out.contains("compile_error"), "got:\n{out}");
        assert!(out.contains("async"), "got:\n{out}");
    }

    #[test]
    fn authorize_with_malformed_arguments_is_a_compile_error() {
        for attr in [
            quote! { #[authorize(post)] },
            quote! { #[authorize("update")] },
            quote! { #[authorize("update", post, extra)] },
            quote! { #[authorize("", post)] },
            quote! { #[authorize(update, post)] },
            quote! { #[authorize] },
        ] {
            let out = expansion(quote! {
                #attr
                pub async fn update(post: RouteParam<Post>) -> Response { todo!() }
            });
            assert!(
                out.contains("compile_error"),
                "`{attr}` must be rejected; got:\n{out}"
            );
        }
    }

    #[test]
    fn authorize_param_form_checks_after_binding_and_before_body_and_form() {
        // The form comes first in the signature, yet it must be read after
        // the check: a denied user never sees what the form would reject.
        let out = expansion(quote! {
            #[authorize("update", post)]
            pub async fn update(form: UpdatePost, post: post::Model) -> Response {
                let _ = "BODY_MARKER";
                todo!()
            }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        let binding = position(&out, "from_route_param");
        let check = position(&out, "__authorize_handler (");
        let form = position(&out, "from_request");
        let body = position(&out, "BODY_MARKER");
        assert!(
            binding < check && check < form && form < body,
            "order must be binding, check, form, body; got:\n{out}"
        );
        assert!(
            !out.contains("# [authorize"),
            "the attribute must not survive into the output; got:\n{out}"
        );
    }

    #[test]
    fn authorize_route_param_checks_the_inner_model() {
        let out = expansion(quote! {
            #[authorize("update", post)]
            pub async fn update(post: RouteParam<Post>) -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(
            out.contains("Deref :: deref (& post)"),
            "a RouteParam<M> must be authorized as the M inside it; got:\n{out}"
        );
    }

    #[test]
    fn authorize_type_form_checks_the_type() {
        for (attr, ty) in [
            (quote! { #[authorize("create", Post)] }, "< Post >"),
            (
                quote! { #[authorize("create", post::Model)] },
                "< post :: Model >",
            ),
            (
                quote! { #[authorize("create", crate::models::Post)] },
                "< crate :: models :: Post >",
            ),
        ] {
            let out = expansion(quote! {
                #attr
                pub async fn store(form: StorePost) -> Response {
                    let _ = "BODY_MARKER";
                    todo!()
                }
            });
            assert!(!out.contains("compile_error"), "got:\n{out}");
            let check = position(&out, "__authorize_handler_type");
            assert!(out[check..].contains(ty), "`{ty}` missing; got:\n{out}");
            assert!(
                check < position(&out, "from_request") && check < position(&out, "BODY_MARKER"),
                "the check must precede the form and the body; got:\n{out}"
            );
        }
    }

    #[test]
    fn authorize_on_a_zero_parameter_handler_checks_before_the_body() {
        let out = expansion(quote! {
            #[authorize("create", Post)]
            pub async fn create() -> Response {
                let _ = "BODY_MARKER";
                todo!()
            }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(position(&out, "__authorize_handler_type") < position(&out, "BODY_MARKER"));
    }

    #[test]
    fn authorize_attributes_all_apply_in_written_order() {
        let out = expansion(quote! {
            #[authorize("view", post)]
            #[suprnova::authorize("publish", post)]
            pub async fn publish(post: RouteParam<Post>) -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(
            position(&out, "\"view\"") < position(&out, "\"publish\""),
            "checks must run in the order written; got:\n{out}"
        );
        assert!(!out.contains("authorize ("), "got:\n{out}");
    }

    // ── #[authorize] expanding on its own ────────────────────────────────────

    /// What the standalone attribute expands to.
    fn authorize_expansion(
        attr: proc_macro2::TokenStream,
        item: proc_macro2::TokenStream,
    ) -> String {
        crate::authorize::authorize_impl_inner(attr, item).to_string()
    }

    #[test]
    fn authorize_above_handler_moves_below_it() {
        // Written above `#[handler]`, the attribute expands first. It hands
        // itself back below `#[handler]`, which then applies every check.
        let out = authorize_expansion(
            quote! { "view", post },
            quote! {
                #[authorize("publish", post)]
                #[handler]
                pub async fn publish(post: RouteParam<Post>) -> Response { todo!() }
            },
        );
        assert!(!out.contains("compile_error"), "got:\n{out}");
        let handler = position(&out, "# [handler]");
        let view = position(&out, "\"view\"");
        let publish = position(&out, "\"publish\"");
        assert!(
            handler < view && view < publish,
            "`#[handler]` must come first, then the checks in written order; got:\n{out}"
        );
        assert_eq!(out.matches("handler").count(), 1, "got:\n{out}");
    }

    #[test]
    fn authorize_without_handler_is_a_compile_error() {
        let out = authorize_expansion(
            quote! { "update", post },
            quote! { pub async fn update(post: RouteParam<Post>) -> Response { todo!() } },
        );
        assert!(out.contains("compile_error"), "got:\n{out}");
        assert!(out.contains("#[handler]"), "got:\n{out}");

        let out = authorize_expansion(quote! { "update", Post }, quote! { pub struct Post; });
        assert!(out.contains("compile_error"), "got:\n{out}");
    }

    #[test]
    fn handler_without_authorize_keeps_declaration_order() {
        // No attribute, no change: extractions stay in signature order.
        let out = expansion(quote! {
            pub async fn update(form: UpdatePost, post: post::Model) -> Response { todo!() }
        });
        assert!(position(&out, "from_request") < position(&out, "from_route_param"));
        assert!(!out.contains("__authorize_handler"), "got:\n{out}");
    }
}
