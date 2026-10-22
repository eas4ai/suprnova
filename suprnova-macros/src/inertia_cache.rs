//! Field attributes prevent an unclassified value from entering a shared page.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Token};

pub(crate) fn expand(input: &DeriveInput) -> syn::Result<Option<TokenStream>> {
    let mut cacheable = false;
    for attr in input
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("inertia"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("live") {
                let content;
                syn::parenthesized!(content in meta.input);
                let _: proc_macro2::TokenStream = content.parse()?;
                return Ok(());
            }
            if !meta.path.is_ident("cacheable") || cacheable {
                return Err(meta.error("expected one `#[inertia(cacheable)]`; declare public and per_request on fields"));
            }
            cacheable = true;
            Ok(())
        })?;
    }
    let syn::Data::Struct(data) = &input.data else {
        return if cacheable {
            Err(syn::Error::new_spanned(
                input,
                "#[inertia(cacheable)] requires an InertiaProps struct",
            ))
        } else {
            Ok(None)
        };
    };
    let mut public = Vec::new();
    let mut per_request = Vec::new();
    let mut conversions = Vec::new();
    let container = crate::serde_attrs::parse_container(&input.attrs, "InertiaProps")?;
    let mut wire_names = std::collections::BTreeSet::new();
    for field in &data.fields {
        let mut kind = None;
        for attr in field
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("inertia"))
        {
            attr.parse_nested_meta(|meta| {
                let value = if meta.path.is_ident("public") {
                    true
                } else if meta.path.is_ident("per_request") {
                    false
                } else {
                    return Err(
                        meta.error("expected `#[inertia(public)]` or `#[inertia(per_request)]`")
                    );
                };
                if kind.replace(value).is_some() {
                    return Err(
                        meta.error("a cacheable prop must have exactly one cache declaration")
                    );
                }
                if meta.input.peek(Token![=]) || meta.input.peek(syn::token::Paren) {
                    return Err(meta.error(
                        "mark the field itself; a cache declaration accepts no field names",
                    ));
                }
                Ok(())
            })?;
        }
        if !cacheable {
            if kind.is_some() {
                return Err(syn::Error::new_spanned(
                    field,
                    "cache field declarations require `#[inertia(cacheable)]` on the InertiaProps struct",
                ));
            }
            continue;
        }
        let name = field.ident.as_ref().ok_or_else(|| {
            syn::Error::new_spanned(field, "cacheable InertiaProps requires named fields")
        })?;
        let public_field = kind.ok_or_else(|| syn::Error::new_spanned(name, format!(
            "cacheable prop `{name}` must declare `#[inertia(public)]` or `#[inertia(per_request)]`"
        )))?;
        let names = crate::serde_attrs::field_names(field, &container, "InertiaProps")?;
        if names.skip_serializing {
            continue;
        }
        let wire = names.serialize;
        if wire.is_empty() || wire.contains('.') {
            return Err(syn::Error::new_spanned(
                field,
                "cacheable props require nonempty top-level wire names",
            ));
        }
        if !wire_names.insert(wire.clone()) {
            return Err(syn::Error::new_spanned(
                field,
                format!("two props are sent under the key `{wire}`"),
            ));
        }
        if public_field {
            public.push(wire.clone());
        } else {
            per_request.push(wire.clone());
        }
        let convert = if public_field {
            quote! { ::suprnova::inertia::IntoInertiaCacheProp::into_cache_prop(&self.#name) }
        } else {
            quote! { ::suprnova::inertia::IntoInertiaCacheProp::into_per_request_cache_prop(&self.#name) }
        };
        conversions.push(quote! { props.insert(#wire.to_owned(), #convert?); });
    }
    if !cacheable {
        return Ok(None);
    }
    if !matches!(data.fields, syn::Fields::Named(_)) {
        return Err(syn::Error::new_spanned(
            input,
            "cacheable InertiaProps requires named fields",
        ));
    }
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(Some(quote! {
        impl #impl_generics ::suprnova::inertia::InertiaPageProps for #name #ty_generics #where_clause {
            fn apply_props(&self, response: ::suprnova::InertiaResponse) -> ::core::result::Result<::suprnova::InertiaResponse, ::suprnova::FrameworkError> {
                response.cacheable(self)
            }
        }
        impl #impl_generics ::suprnova::inertia::CacheableInertiaProps for #name #ty_generics #where_clause {
            const PUBLIC: &'static [&'static str] = &[#(#public),*];
            const PER_REQUEST: &'static [&'static str] = &[#(#per_request),*];
            fn cache_props(&self) -> ::core::result::Result<
                ::suprnova::indexmap::IndexMap<::std::string::String, ::suprnova::Prop>,
                ::suprnova::FrameworkError,
            > {
                let mut props = ::suprnova::indexmap::IndexMap::new();
                #(#conversions)*
                Ok(props)
            }
        }
    }))
}
