//! `#[derive(MultipartRequest)]` - strongly-typed multipart extractor.
//!
//! Emits two impls per struct:
//! 1. `impl FromRequest` - runs the stages in order: `authorize`, the
//!    body parsed once via `parse_multipart_streaming_with_limits` with each
//!    `(name, value)` dispatched to its field, `after_validation`,
//!    `after_validation_async`. Each runs only after the one before it
//!    succeeded.
//! 2. `impl MultipartRequestHooks` - empty default unless the struct
//!    carries `#[multipart(custom_hooks)]`, in which case the user
//!    provides their own impl.
//!
//! Validators receive a bounded sniff buffer + the running size in
//! bytes; the parser captures both during streaming so neither
//! `validate_chunk` nor `validate_final` requires the full part in
//! memory.
//!
//! A field's failures are collected into one `ValidationErrors` under the
//! field's input name, so the client learns about every bad field at once.
//! The per-part logic lives in the framework (`take_file`, `take_text`);
//! the expansion only routes parts to fields.

use proc_macro::TokenStream;
use quote::quote;
use syn::ext::IdentExt;
use syn::{Data, DeriveInput, Fields, LitStr, Type, parse_macro_input};

pub fn expand(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_inner(input).into()
}

/// Pure-`proc_macro2` helper so unit tests can exercise the
/// expansion shape without leaving the proc-macro crate. Returns the
/// expansion (or a `compile_error!`-shaped token stream on bad input).
fn expand_inner(input: DeriveInput) -> proc_macro2::TokenStream {
    let struct_name = &input.ident;

    // Parse struct-level `#[multipart(...)]` options.
    //
    // `custom_hooks`         - caller provides the `MultipartRequestHooks` impl.
    // `max_body_bytes = N`   - per-struct cap on total request body size, in bytes.
    //                          When absent, the macro falls through to the
    //                          process-global cap at runtime.
    let mut emit_default_hooks = true;
    let mut max_body_bytes: Option<proc_macro2::TokenStream> = None;
    for attr in &input.attrs {
        if attr.path().is_ident("multipart") {
            // Domain 5 audit M-D5-3: propagate parse errors as a
            // compile_error rather than swallowing them via `let _ = ...`.
            // Previously a typo like `#[multipart(max_body_byte = 1024)]`
            // (missing the trailing `s`) silently kept the default cap.
            if let Err(e) = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("custom_hooks") {
                    emit_default_hooks = false;
                    return Ok(());
                }
                if meta.path.is_ident("max_body_bytes") {
                    let value: syn::Expr = meta.value()?.parse()?;
                    max_body_bytes = Some(quote::quote! { #value });
                    return Ok(());
                }
                Err(meta.error("unknown #[multipart(...)] option"))
            }) {
                return e.to_compile_error();
            }
        }
    }

    // Compute the cap expression once: per-struct override (if set), else
    // the process-global accessor evaluated at runtime.
    let max_body_bytes_expr: proc_macro2::TokenStream = if let Some(override_expr) = max_body_bytes
    {
        quote::quote! { (#override_expr) as usize }
    } else {
        quote::quote! { ::suprnova::http::upload::global_max_multipart_body_bytes() }
    };

    let Data::Struct(data) = &input.data else {
        return syn::Error::new_spanned(&input, "MultipartRequest requires a struct")
            .to_compile_error();
    };
    let Fields::Named(fields) = &data.fields else {
        return syn::Error::new_spanned(&data.fields, "MultipartRequest requires named fields")
            .to_compile_error();
    };

    let mut field_decls = Vec::new();
    let mut field_arms = Vec::new();
    let mut validator_arms = Vec::new();
    let mut validator_decls = Vec::new();
    let mut required_checks = Vec::new();
    let mut struct_init = Vec::new();
    // `(name a hook may use, input name)` pairs for renaming hook errors:
    // each field's Rust name, and its `#[field]` name when that carries
    // `[]`. Identity pairs are left out.
    let mut hook_names: Vec<(String, String)> = Vec::new();
    // `(wire_name, max_count)` pairs for every Vec field carrying a
    // `max_count` ceiling. Handed to the parser via
    // `MultipartLimits::per_field_max_counts` so the ceiling is enforced
    // during streaming, before the offending part allocates.
    let mut max_count_entries: Vec<proc_macro2::TokenStream> = Vec::new();

    for field in &fields.named {
        let ident = field.ident.clone().unwrap();
        let ty = &field.ty;

        // Parse `#[field("name")]` or `#[field("name", max_count = N)]`.
        //
        // `max_count = N` is a count ceiling on Vec fields. The total-byte
        // cap blocks raw payload bytes, but a `Vec<UploadedFile<()>>` field
        // could otherwise accept an unbounded number of parts within that
        // budget (multipart framing bytes are not counted toward the byte
        // cap). `max_count` is handed to the parser via
        // `MultipartLimits::per_field_max_counts` and enforced DURING
        // streaming: the (cap + 1)-th part carrying this name is rejected
        // with 413 before it is read, so the extra part never allocates.
        //
        // Honoured for `Vec<UploadedFile<V>>` (FileVec) and
        // `Vec<T: FromStr>` (TextVec). On scalar/option fields the
        // attribute is accepted but does nothing (those keep
        // first-write-wins semantics already).
        let mut field_name: Option<LitStr> = None;
        let mut max_count: Option<usize> = None;
        for attr in &field.attrs {
            if attr.path().is_ident("field") {
                let parsed = attr.parse_args_with(
                    |input: syn::parse::ParseStream| -> syn::Result<(LitStr, Option<usize>)> {
                        let name: LitStr = input.parse()?;
                        let mut max: Option<usize> = None;
                        while input.peek(syn::Token![,]) {
                            let _comma: syn::Token![,] = input.parse()?;
                            let key: syn::Ident = input.parse()?;
                            let _eq: syn::Token![=] = input.parse()?;
                            if key == "max_count" {
                                let lit: syn::LitInt = input.parse()?;
                                max = Some(lit.base10_parse()?);
                            } else {
                                return Err(syn::Error::new(
                                    key.span(),
                                    format!(
                                        "unknown #[field(...)] option `{key}`; \
                                         supported keys: max_count"
                                    ),
                                ));
                            }
                        }
                        Ok((name, max))
                    },
                );
                match parsed {
                    Ok((name, max)) => {
                        field_name = Some(name);
                        max_count = max;
                    }
                    Err(e) => return e.to_compile_error(),
                }
            }
        }
        let Some(field_name) = field_name else {
            return syn::Error::new_spanned(
                &ident,
                "each MultipartRequest field needs #[field(\"name\")]",
            )
            .to_compile_error();
        };
        let field_name_str = field_name.value();
        let input_name = field_name_str
            .strip_suffix("[]")
            .unwrap_or(&field_name_str)
            .to_string();
        for alias in [ident.unraw().to_string(), field_name_str.clone()] {
            if alias != input_name && !hook_names.iter().any(|(name, _)| *name == alias) {
                hook_names.push((alias, input_name.clone()));
            }
        }

        let shape = classify(ty);

        // `max_count` is only meaningful on Vec shapes; reject it on
        // scalar / option fields so a typo doesn't silently disable
        // the cap a caller intended.
        if max_count.is_some()
            && !matches!(
                shape,
                FieldShape::FileVec { .. } | FieldShape::TextVec { .. }
            )
        {
            return syn::Error::new_spanned(
                &ident,
                "#[field(..., max_count = N)] is only valid on `Vec<...>` fields; \
                 scalar and `Option<...>` fields already keep first-write-wins semantics",
            )
            .to_compile_error();
        }
        // `max_count` (when set) is enforced by the parser during streaming
        // via `MultipartLimits::per_field_max_counts`: the (cap + 1)-th part
        // with this name is rejected with 413 before it is read, so the
        // extra part never allocates.
        if let Some(cap) = max_count {
            max_count_entries.push(quote! { (#field_name_str, #cap) });
        }

        // Each part's zero-based index among the parts of this name, which
        // names its error when the input name ends in `[]`.
        let index_ident = quote::format_ident!("__index_{}", ident);
        // Set when a part of a required field failed, so a field reported
        // as invalid is not also reported as missing.
        let invalid_ident = quote::format_ident!("__invalid_{}", ident);
        field_decls.push(quote! {
            let mut #index_ident: usize = 0;
        });
        let next_index = quote! {
            let __index = #index_ident;
            #index_ident += 1;
        };

        // A scalar field is required; an `Option` or `Vec` one is not.
        let required = matches!(
            shape,
            FieldShape::FileScalar { .. } | FieldShape::TextScalar { .. }
        );

        match shape {
            FieldShape::FileScalar { validator } | FieldShape::FileOption { validator } => {
                let v_ident = quote::format_ident!("__v_{}", ident);
                let (validator_decl, validator_arm) =
                    validator_wiring(&validator, &v_ident, &field_name_str);
                validator_decls.push(validator_decl);
                validator_arms.push(validator_arm);
                let on_invalid = if required {
                    quote! { #invalid_ident = true; }
                } else {
                    quote! {}
                };
                field_arms.push(quote! {
                    #field_name_str => {
                        #next_index
                        // First write wins; a later part of the name is
                        // neither validated nor kept.
                        if #ident.is_none() {
                            match ::suprnova::http::upload::take_file(
                                &#v_ident, __value, #field_name_str, __index, &mut __errors,
                            )? {
                                ::suprnova::http::upload::Taken::Value(__file) => {
                                    #ident = ::core::option::Option::Some(__file);
                                }
                                ::suprnova::http::upload::Taken::Absent => {}
                                ::suprnova::http::upload::Taken::Invalid => { #on_invalid }
                            }
                        }
                    }
                });
                field_decls.push(quote! {
                    let mut #ident: ::core::option::Option<::suprnova::http::upload::UploadedFile<#validator>> = ::core::option::Option::None;
                });
                push_required(
                    required,
                    &ident,
                    &invalid_ident,
                    &field_name_str,
                    &mut field_decls,
                    &mut required_checks,
                    &mut struct_init,
                );
            }
            FieldShape::FileVec { validator } => {
                let v_ident = quote::format_ident!("__v_{}", ident);
                let (validator_decl, validator_arm) =
                    validator_wiring(&validator, &v_ident, &field_name_str);
                validator_decls.push(validator_decl);
                validator_arms.push(validator_arm);
                field_arms.push(quote! {
                    #field_name_str => {
                        #next_index
                        if let ::suprnova::http::upload::Taken::Value(__file) =
                            ::suprnova::http::upload::take_file(
                                &#v_ident, __value, #field_name_str, __index, &mut __errors,
                            )?
                        {
                            #ident.push(__file);
                        }
                    }
                });
                field_decls.push(quote! {
                    let mut #ident: ::std::vec::Vec<::suprnova::http::upload::UploadedFile<#validator>> = ::std::vec::Vec::new();
                });
                struct_init.push(quote! { #ident, });
            }
            FieldShape::TextScalar { inner_ty, failure }
            | FieldShape::TextOption { inner_ty, failure } => {
                let on_invalid = if required {
                    quote! { #invalid_ident = true; }
                } else {
                    quote! {}
                };
                field_arms.push(quote! {
                    #field_name_str => {
                        #next_index
                        // First write wins; a later part of the name is
                        // neither parsed nor kept.
                        if #ident.is_none() {
                            match ::suprnova::http::upload::take_text::<#inner_ty>(
                                __value, #field_name_str, __index, #failure, &mut __errors,
                            ) {
                                ::suprnova::http::upload::Taken::Value(__parsed) => {
                                    #ident = ::core::option::Option::Some(__parsed);
                                }
                                ::suprnova::http::upload::Taken::Absent => {}
                                ::suprnova::http::upload::Taken::Invalid => { #on_invalid }
                            }
                        }
                    }
                });
                field_decls.push(quote! {
                    let mut #ident: ::core::option::Option<#inner_ty> = ::core::option::Option::None;
                });
                push_required(
                    required,
                    &ident,
                    &invalid_ident,
                    &field_name_str,
                    &mut field_decls,
                    &mut required_checks,
                    &mut struct_init,
                );
            }
            FieldShape::TextVec { inner_ty, failure } => {
                field_arms.push(quote! {
                    #field_name_str => {
                        #next_index
                        if let ::suprnova::http::upload::Taken::Value(__parsed) =
                            ::suprnova::http::upload::take_text::<#inner_ty>(
                                __value, #field_name_str, __index, #failure, &mut __errors,
                            )
                        {
                            #ident.push(__parsed);
                        }
                    }
                });
                field_decls.push(quote! {
                    let mut #ident: ::std::vec::Vec<#inner_ty> = ::std::vec::Vec::new();
                });
                struct_init.push(quote! { #ident, });
            }
        }
    }

    let hook_name_pairs = hook_names
        .iter()
        .map(|(name, input)| quote! { (#name, #input) });

    let hooks_impl = if emit_default_hooks {
        quote! {
            #[automatically_derived]
            impl ::suprnova::http::upload::MultipartRequestHooks for #struct_name {}
        }
    } else {
        quote! {}
    };

    let expanded = quote! {
        #[::suprnova::__async_trait::async_trait]
        impl ::suprnova::http::FromRequest for #struct_name {
            async fn from_request(req: ::suprnova::http::Request)
                -> ::core::result::Result<Self, ::suprnova::FrameworkError>
            {
                // Stage 1: authorize, before any byte of the body is read.
                if !<Self as ::suprnova::http::upload::MultipartRequestHooks>::authorize(&req) {
                    return ::core::result::Result::Err(::suprnova::FrameworkError::Unauthorized);
                }

                // Construct one validator instance per file field, ONCE.
                // The non-`move` closure below and the post-parse field
                // loop both borrow these via `&#v_<ident>`, so stateful
                // validators (interior mutability - `Mutex`, `AtomicUsize`,
                // etc.) see coherent state across every chunk + the final
                // call. Without this hoist a fresh instance would be
                // constructed inside each match arm and any accumulated
                // state would be discarded.
                #(#validator_decls)*

                // Stage 2: extraction. A request-wide limit answers 413 and a
                // file refused while the body streams answers 422, both
                // without reading further.
                let __max_body_bytes: usize = #max_body_bytes_expr;
                let __spill_threshold: usize = ::suprnova::http::upload::global_upload_spill_threshold();
                let __limits = ::suprnova::http::upload::MultipartLimits {
                    max_body_bytes: __max_body_bytes,
                    max_parts: ::suprnova::http::upload::global_max_multipart_parts(),
                    spill_threshold: __spill_threshold,
                    per_field_max_counts: &[ #(#max_count_entries),* ],
                };
                let __payload = ::suprnova::http::upload::parse_multipart_streaming_with_limits(
                    req,
                    __limits,
                    |__name: &str, __sniff: &[u8], __size: u64| -> ::core::result::Result<(), ::suprnova::FrameworkError> {
                        match __name {
                            #(#validator_arms)*
                            _ => {}
                        }
                        ::core::result::Result::Ok(())
                    },
                ).await?;

                #(#field_decls)*
                let mut __errors = ::suprnova::ValidationErrors::new();

                for (__name, __value) in __payload.fields {
                    match __name.as_str() {
                        #(#field_arms)*
                        _ => {}
                    }
                }

                #(#required_checks)*

                // A field failed: answer with every field's errors. The
                // values built so far drop here, removing their temp files.
                if !__errors.is_empty() {
                    return ::core::result::Result::Err(
                        ::suprnova::FrameworkError::validation_errors(__errors),
                    );
                }

                let __constructed = Self { #(#struct_init)* };

                // Hook errors name fields as extraction errors do.
                fn __input_name(__key: &str) -> ::std::string::String {
                    ::suprnova::http::upload::hook_error_key(__key, &[ #(#hook_name_pairs),* ])
                }

                // Stage 3: the synchronous hook. An empty set is success.
                // (A `match`, not a let chain: the expansion compiles in the
                // caller's edition.)
                match <Self as ::suprnova::http::upload::MultipartRequestHooks>::after_validation(&__constructed) {
                    ::core::result::Result::Err(errs) if !errs.is_empty() => {
                        return ::core::result::Result::Err(
                            ::suprnova::FrameworkError::validation_errors(errs.rename_keys(__input_name)),
                        );
                    }
                    _ => {}
                }

                // Stage 4: the async hook, only once the sync one passed.
                match <Self as ::suprnova::http::upload::MultipartRequestHooks>::after_validation_async(&__constructed).await {
                    ::core::result::Result::Err(errs) if !errs.is_empty() => {
                        return ::core::result::Result::Err(
                            ::suprnova::FrameworkError::validation_errors(errs.rename_keys(__input_name)),
                        );
                    }
                    _ => {}
                }

                ::core::result::Result::Ok(__constructed)
            }
        }

        #hooks_impl
    };

    expanded
}

/// The declarations, the missing-field check and the struct initialiser
/// of one scalar or optional field. A required field missing with no
/// failure of its own is reported as `validation-required` under its
/// input name; an optional one is simply `None`.
fn push_required(
    required: bool,
    ident: &syn::Ident,
    invalid_ident: &syn::Ident,
    field_name_str: &str,
    field_decls: &mut Vec<proc_macro2::TokenStream>,
    required_checks: &mut Vec<proc_macro2::TokenStream>,
    struct_init: &mut Vec<proc_macro2::TokenStream>,
) {
    if !required {
        struct_init.push(quote! { #ident, });
        return;
    }
    field_decls.push(quote! {
        let mut #invalid_ident = false;
    });
    required_checks.push(quote! {
        if #ident.is_none() && !#invalid_ident {
            ::suprnova::http::upload::add_field_failure(
                &mut __errors,
                #field_name_str,
                ::core::option::Option::None,
                ::suprnova::http::upload::FieldFailure::Required,
            );
        }
    });
    // Unreachable: a missing required field was reported above, and the
    // error set was checked before construction. An error, not a panic.
    struct_init.push(quote! {
        #ident: #ident.ok_or_else(|| ::suprnova::FrameworkError::internal(
            format!("multipart field '{}' was neither extracted nor reported", #field_name_str)
        ))?,
    });
}

/// The validator instance declaration and the `validate_chunk` arm for
/// one file field. The arm reads the `__sniff` and `__size` names of the
/// chunk callback the caller emits.
fn validator_wiring(
    validator: &proc_macro2::TokenStream,
    v_ident: &syn::Ident,
    field_name_str: &str,
) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
    let decl = quote! {
        let #v_ident: #validator = <#validator as ::core::default::Default>::default();
    };
    let arm = quote! {
        #field_name_str => {
            <#validator as ::suprnova::http::upload::validators::UploadValidator>::validate_chunk(&#v_ident, __sniff, __size)?;
        }
    };
    (decl, arm)
}

enum FieldShape {
    FileScalar {
        validator: proc_macro2::TokenStream,
    },
    FileOption {
        validator: proc_macro2::TokenStream,
    },
    FileVec {
        validator: proc_macro2::TokenStream,
    },
    TextScalar {
        inner_ty: proc_macro2::TokenStream,
        failure: proc_macro2::TokenStream,
    },
    TextOption {
        inner_ty: proc_macro2::TokenStream,
        failure: proc_macro2::TokenStream,
    },
    TextVec {
        inner_ty: proc_macro2::TokenStream,
        failure: proc_macro2::TokenStream,
    },
}

/// The `FieldFailure` a text part that does not parse as `ty` reports,
/// chosen by the type's name as Laravel's rules split them: integer types,
/// float types, `bool`, and everything else as a format error.
fn parse_failure(ty: &Type) -> proc_macro2::TokenStream {
    let failure = match outer_segment_ident(ty).as_deref() {
        Some(
            "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128"
            | "usize" | "NonZeroI8" | "NonZeroI16" | "NonZeroI32" | "NonZeroI64" | "NonZeroI128"
            | "NonZeroIsize" | "NonZeroU8" | "NonZeroU16" | "NonZeroU32" | "NonZeroU64"
            | "NonZeroU128" | "NonZeroUsize",
        ) => quote! { Integer },
        Some("f32" | "f64") => quote! { Numeric },
        Some("bool") => quote! { Boolean },
        _ => quote! { Format },
    };
    quote! { ::suprnova::http::upload::FieldFailure::#failure }
}

fn classify(ty: &Type) -> FieldShape {
    let outer_kind = outer_segment_ident(ty);
    let outer_inner = outer_segment_first_generic(ty);

    match (outer_kind.as_deref(), outer_inner) {
        (Some("Vec"), Some(inner)) => {
            if let Some(validator) = uploaded_file_validator(&inner) {
                FieldShape::FileVec { validator }
            } else {
                FieldShape::TextVec {
                    failure: parse_failure(&inner),
                    inner_ty: quote! { #inner },
                }
            }
        }
        (Some("Option"), Some(inner)) => {
            if let Some(validator) = uploaded_file_validator(&inner) {
                FieldShape::FileOption { validator }
            } else {
                FieldShape::TextOption {
                    failure: parse_failure(&inner),
                    inner_ty: quote! { #inner },
                }
            }
        }
        _ => {
            if let Some(validator) = uploaded_file_validator(ty) {
                FieldShape::FileScalar { validator }
            } else {
                FieldShape::TextScalar {
                    failure: parse_failure(ty),
                    inner_ty: quote! { #ty },
                }
            }
        }
    }
}

fn outer_segment_ident(ty: &Type) -> Option<String> {
    if let Type::Path(p) = ty {
        return p.path.segments.last().map(|s| s.ident.to_string());
    }
    None
}

fn outer_segment_first_generic(ty: &Type) -> Option<Type> {
    if let Type::Path(p) = ty
        && let Some(seg) = p.path.segments.last()
        && let syn::PathArguments::AngleBracketed(args) = &seg.arguments
        && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
    {
        return Some(inner.clone());
    }
    None
}

fn uploaded_file_validator(ty: &Type) -> Option<proc_macro2::TokenStream> {
    if let Type::Path(p) = ty
        && let Some(seg) = p.path.segments.last()
        && seg.ident == "UploadedFile"
    {
        if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
            if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                return Some(quote! { #inner });
            }
            return Some(quote! { () });
        }
        return Some(quote! { () });
    }
    None
}

#[cfg(test)]
mod tests {
    //! Domain 5 audit M-D5-3 regression: unknown keys inside
    //! `#[multipart(...)]` must surface as compile errors. Before the
    //! fix, `let _ = attr.parse_nested_meta(|meta| { ... })` silently
    //! discarded the `Err(meta.error("unknown ..."))` returned for
    //! typos like `max_body_byte` - operators thought they'd set a
    //! larger per-struct cap but production kept the default.
    //!
    //! These tests also lock in the existing rejection paths
    //! (`tuple struct`, missing `#[field(...)]`, `max_count` on
    //! scalar fields) so a future refactor can't quietly silence
    //! them.

    use super::*;
    use syn::parse_quote;

    fn render(input: DeriveInput) -> String {
        expand_inner(input).to_string()
    }

    #[test]
    fn unknown_multipart_option_emits_compile_error() {
        // The `typo_key` is intentionally not one of `custom_hooks`
        // or `max_body_bytes`. Before M-D5-3 this silently went
        // through; now it must produce a `compile_error!` token.
        let input: DeriveInput = parse_quote! {
            #[multipart(typo_key = 1024)]
            struct Bad {
                #[field("file")]
                file: UploadedFile<()>,
            }
        };
        let rendered = render(input);
        assert!(
            rendered.contains("compile_error"),
            "unknown #[multipart(...)] option must produce a compile_error; got: {rendered}"
        );
        assert!(
            rendered.contains("unknown") || rendered.contains("multipart"),
            "compile_error message must reference the bad option; got: {rendered}"
        );
    }

    #[test]
    fn known_multipart_options_compile_through() {
        let input: DeriveInput = parse_quote! {
            #[multipart(max_body_bytes = 1024)]
            struct Good {
                #[field("file")]
                file: UploadedFile<()>,
            }
        };
        let rendered = render(input);
        assert!(
            !rendered.contains("compile_error"),
            "known multipart options should not error; got: {rendered}"
        );
    }

    #[test]
    fn tuple_struct_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            struct Bad(String);
        };
        let rendered = render(input);
        assert!(
            rendered.contains("compile_error"),
            "tuple structs must be rejected; got: {rendered}"
        );
    }

    #[test]
    fn missing_field_attr_emits_compile_error() {
        let input: DeriveInput = parse_quote! {
            struct Bad {
                file: UploadedFile<()>,
            }
        };
        let rendered = render(input);
        assert!(
            rendered.contains("compile_error"),
            "field without #[field(\"...\")] must be rejected; got: {rendered}"
        );
        assert!(
            rendered.contains("MultipartRequest field needs"),
            "diagnostic must explain the missing #[field] attribute; got: {rendered}"
        );
    }

    #[test]
    fn a_text_field_reports_the_parse_failure_of_its_type() {
        let failure = |ty: Type| parse_failure(&ty).to_string();
        assert!(failure(parse_quote!(u32)).ends_with("Integer"));
        assert!(failure(parse_quote!(std::num::NonZeroU64)).ends_with("Integer"));
        assert!(failure(parse_quote!(f64)).ends_with("Numeric"));
        assert!(failure(parse_quote!(bool)).ends_with("Boolean"));
        assert!(failure(parse_quote!(std::net::IpAddr)).ends_with("Format"));
        assert!(failure(parse_quote!(String)).ends_with("Format"));
    }

    #[test]
    fn hook_errors_are_renamed_from_rust_names_to_input_names() {
        let input: DeriveInput = parse_quote! {
            struct Album {
                #[field("photos[]")]
                photo: Vec<UploadedFile>,
                #[field("title")]
                r#name: String,
                #[field("slug")]
                slug: String,
            }
        };
        let rendered = render(input);
        assert!(
            rendered.contains(r#"("photo" , "photos")"#),
            "the Rust name maps to the input name; got: {rendered}"
        );
        assert!(
            rendered.contains(r#"("photos[]" , "photos")"#),
            "the bracketed name maps to the input name; got: {rendered}"
        );
        assert!(
            rendered.contains(r#"("name" , "title")"#),
            "a raw identifier maps by its plain name; got: {rendered}"
        );
        assert!(
            !rendered.contains(r#"("slug" , "slug")"#),
            "a field whose names agree needs no entry; got: {rendered}"
        );
    }
}
