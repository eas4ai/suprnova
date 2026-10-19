//! `#[suprnova_webmcp::tool_input]`: the WebAssembly crate's counterpart of
//! the server's `#[tool_input]`, for the input type of a page tool (AGENT-016).
//!
//! It derives `Deserialize`, `Validate` and `JsonSchema` and generates the
//! type's field table: each field's Rust name, which is what `validator`
//! reports, and the key the client sends it under. The page tool runtime
//! walks a validation error's key through that table, so its errors name
//! each field as the server's form request does (`items.1.unitPrice`).
//!
//! The names come from [`lenient_fields`], the reading of serde attributes
//! the server's registration uses, so the two cannot drift apart. Every
//! path the output names is in `suprnova_webmcp::__private`, so a front end
//! needs no `serde`, `schemars` or `validator` dependency of its own.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{DeriveInput, Field, parse_macro_input};

use crate::input_names::{lenient_fields, peel};

/// Implementation of `#[suprnova_webmcp::tool_input]`.
pub fn tool_input_attr_impl(
    attr: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    if !attr.is_empty() {
        return syn::Error::new(
            Span::call_site(),
            "#[suprnova_webmcp::tool_input] takes no arguments",
        )
        .to_compile_error()
        .into();
    }
    let input = parse_macro_input!(input as DeriveInput);
    match tool_input_tokens(&input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// The struct with its three derives, pointed at the crate's own copies of
/// serde, validator and schemars, and its `InputNames` table.
fn tool_input_tokens(input: &DeriveInput) -> syn::Result<TokenStream> {
    let Some(fields) = lenient_fields(input)? else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "#[suprnova_webmcp::tool_input] needs a struct with named fields and no generic or \
             lifetime parameters: a page tool's input is a JSON object",
        ));
    };
    let private = quote!(::suprnova_webmcp::__private);
    let mut any_nested = false;
    let mut arms = Vec::with_capacity(fields.len());
    for field in &fields {
        let rust = &field.rust;
        let client = &field.input;
        // Only a `#[validate(nested)]` field reports errors under longer
        // keys, so only its type must carry the table too. A type without
        // it fails the build here, with the message on `InputNames`.
        let (nested, dynamic) = if is_nested(input, rust)? {
            any_nested = true;
            let (inner, dynamic) = peel(field.ty);
            (
                quote! {
                    ::core::option::Option::Some(
                        <#inner as #private::InputNames>::input_field
                            as fn(&str) -> ::core::option::Option<#private::InputField>
                    )
                },
                dynamic,
            )
        } else {
            (quote!(::core::option::Option::None), 0)
        };
        arms.push(quote! {
            #rust => ::core::option::Option::Some(#private::InputField {
                input: #client,
                nested: #nested,
                dynamic_segments: #dynamic,
            }),
        });
    }
    // validator's derive validates a nested field with a `.validate()`
    // method call, which needs the trait in scope where the struct is.
    let validate_in_scope = any_nested.then(|| {
        quote! {
            #[allow(unused_imports)]
            use ::suprnova_webmcp::__private::validator::Validate as _;
        }
    });
    let name = &input.ident;
    Ok(quote! {
        #validate_in_scope
        #[derive(
            #private::serde::Deserialize,
            #private::validator::Validate,
            #private::schemars::JsonSchema
        )]
        #[serde(crate = "::suprnova_webmcp::__private::serde")]
        #[validate(crate = "::suprnova_webmcp::__private::validator")]
        #[schemars(crate = "::suprnova_webmcp::__private::schemars")]
        #input

        impl #private::InputNames for #name {
            fn input_field(rust: &str) -> ::core::option::Option<#private::InputField> {
                match rust {
                    #(#arms)*
                    _ => ::core::option::Option::None,
                }
            }
        }
    })
}

/// Whether the field named `rust` carries `#[validate(nested)]`.
fn is_nested(input: &DeriveInput, rust: &str) -> syn::Result<bool> {
    let syn::Data::Struct(data) = &input.data else {
        return Ok(false);
    };
    let Some(field) = data
        .fields
        .iter()
        .find(|field| field.ident.as_ref().is_some_and(|ident| ident == rust))
    else {
        return Ok(false);
    };
    validates_nested(field)
}

/// Whether one of `field`'s `#[validate(...)]` attributes lists `nested`.
fn validates_nested(field: &Field) -> syn::Result<bool> {
    let mut nested = false;
    for attr in field
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("validate"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("nested") {
                nested = true;
                Ok(())
            } else {
                crate::serde_attrs::skip_meta(&meta)
            }
        })?;
    }
    Ok(nested)
}
