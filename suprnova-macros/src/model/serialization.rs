//! Model serialization shares one policy across serde, arrays and JSON.
//! Unfiltered attributes remain available to persistence without recursive serialization.

use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

use super::parse::ModelInput;

/// Emit direct column access so collections read identity without visibility filters.
pub fn emit_field_value(idents: &[Ident]) -> TokenStream {
    let arms = idents.iter().map(|ident| {
        let name = ident.to_string();
        quote! { #name => ::suprnova::serde_json::to_value(&self.#ident).ok(), }
    });
    quote! {
        fn field_value(&self, name: &str) -> ::core::option::Option<::suprnova::serde_json::Value> {
            match name { #(#arms)* _ => ::core::option::Option::None }
        }
    }
}

// Read serde options so the view excludes skipped fields and preserves
// the argument types of custom serializers.
fn has_serde_option(field: &syn::Field, names: &[&str]) -> bool {
    field
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("serde"))
        .any(|attr| {
            attr.parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            )
            .is_ok_and(|items| {
                items
                    .iter()
                    .any(|item| names.iter().any(|name| item.path().is_ident(*name)))
            })
        })
}

/// Emit a borrowed serde view so model output honors field attributes without recursion.
/// The view retains serde renames and custom serializers but excludes runtime state.
pub fn emit_serialize(input: &ModelInput) -> TokenStream {
    let ident = &input.item.ident;
    let attrs = input
        .item
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("serde"));
    let fields: Vec<_> = input
        .item
        .fields
        .iter()
        .filter(|field| {
            field
                .ident
                .as_ref()
                .is_some_and(|name| name != "__eager" && name != "__pivot")
                && !has_serde_option(field, &["skip", "skip_serializing"])
        })
        .map(|field| (field, has_serde_option(field, &["serialize_with", "with"])))
        .collect();
    let lifetime = fields
        .iter()
        .any(|(_, custom)| !custom)
        .then(|| quote! { <'a> });
    let declarations = fields.iter().map(|(field, custom)| {
        let ident = &field.ident;
        let ty = &field.ty;
        let attrs = field
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("serde"));
        let ty = if *custom {
            quote! { #ty }
        } else {
            quote! { &'a #ty }
        };
        quote! { #(#attrs)* #ident: #ty }
    });
    let values = fields.iter().map(|(field, custom)| {
        let ident = &field.ident;
        if *custom {
            // Generic callbacks expect the original field type, not a reference type.
            quote! { #ident: ::core::clone::Clone::clone(&self.#ident) }
        } else {
            quote! { #ident: &self.#ident }
        }
    });
    quote! {
        impl #ident {
            fn __suprnova_attributes(&self) -> ::core::result::Result<
                ::suprnova::serde_json::Value, ::suprnova::serde_json::Error,
            > {
                #[derive(::suprnova::serde::Serialize)]
                #(#attrs)*
                struct __SuprnovaAttributes #lifetime { #(#declarations,)* }
                ::suprnova::serde_json::to_value(__SuprnovaAttributes { #(#values,)* })
            }
        }

        impl ::suprnova::serde::Serialize for #ident {
            fn serialize<S>(&self, serializer: S) -> ::core::result::Result<S::Ok, S::Error>
            where S: ::suprnova::serde::Serializer {
                let value = <Self as ::suprnova::Model>::__serialization_value(self)
                    .map_err(<S::Error as ::suprnova::serde::ser::Error>::custom)?;
                ::suprnova::serde::Serialize::serialize(&value, serializer)
            }
        }
    }
}

/// Emit declared policies and unfiltered runtime fields for shared model serialization.
pub fn emit_to_array_override(
    hidden: &[String],
    visible: Option<&[String]>,
    appends: &[String],
) -> TokenStream {
    let visible = visible.unwrap_or(&[]);
    quote! {
        const HIDDEN: &'static [&'static str] = &[#(#hidden),*];
        const VISIBLE: &'static [&'static str] = &[#(#visible),*];
        const APPENDS: &'static [&'static str] = &[#(#appends),*];

        fn __attributes_to_value(&self) -> ::core::result::Result<
            ::suprnova::serde_json::Value, ::suprnova::FrameworkError,
        > {
            self.__suprnova_attributes().map_err(|error| {
                ::suprnova::FrameworkError::internal(error.to_string())
            })
        }
    }
}

/// Register accessor dispatch so runtime appends can select a declared method.
/// Conversion errors reach serde instead of silently emitting null.
pub fn emit_append_accessor_dispatch(appends: &[String]) -> TokenStream {
    let arms = appends.iter().map(|name| {
        let method: syn::Ident = syn::parse_str(name).expect("validated accessor name");
        quote! {
            #name => ::suprnova::serde_json::to_value(self.#method())
                .map(::core::option::Option::Some)
                .map_err(|error| ::suprnova::FrameworkError::validation(#name, error.to_string())),
        }
    });
    quote! {
        fn __has_append_accessor(name: &str) -> bool {
            [#(#appends),*].contains(&name)
        }
        fn __try_append_accessor(&self, name: &str) -> ::core::result::Result<
            ::core::option::Option<::suprnova::serde_json::Value>, ::suprnova::FrameworkError,
        > {
            match name { #(#arms)* _ => ::core::result::Result::Ok(::core::option::Option::None) }
        }
    }
}
