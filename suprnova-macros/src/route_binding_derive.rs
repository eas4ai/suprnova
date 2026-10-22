//! `#[derive(RouteBinding)]` for a unit-only enum (BIND-010).
//!
//! Each variant binds from exactly one string: its `#[route(value = "...")]`
//! value, or else its name in snake case, matched exactly and
//! case-sensitively. A value that matches no variant answers 404 without
//! calling the route's `missing()` handler, as Laravel's enum binding does,
//! and its body names the enum without repeating the value. The same
//! string fills a parameter when `route()` is given the variant.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, LitStr};

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let ident = &input.ident;
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            ident,
            "#[derive(RouteBinding)] works on a unit-only enum; implement \
             `RouteBinding` by hand for any other type",
        ));
    };
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "#[derive(RouteBinding)] works on an enum without generic parameters",
        ));
    }
    let mut variants = Vec::with_capacity(data.variants.len());
    let mut seen: Vec<(String, &syn::Ident)> = Vec::new();
    for variant in &data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new_spanned(
                variant,
                "#[derive(RouteBinding)] binds unit variants only: a route \
                 parameter is one string, which cannot fill a variant's fields",
            ));
        }
        let value = route_value(variant)?.unwrap_or_else(|| snake_case(&variant.ident.to_string()));
        if let Some((_, other)) = seen.iter().find(|(taken, _)| *taken == value) {
            return Err(syn::Error::new_spanned(
                variant,
                format!(
                    "`{}` and `{other}` both bind from `{value}`; give one a \
                     `#[route(value = \"...\")]`",
                    variant.ident
                ),
            ));
        }
        seen.push((value.clone(), &variant.ident));
        variants.push((&variant.ident, value));
    }

    let from_value = variants.iter().map(|(variant, value)| {
        quote! { #value => ::core::option::Option::Some(Self::#variant), }
    });
    let to_value = variants.iter().map(|(variant, value)| {
        quote! { Self::#variant => #value, }
    });

    Ok(quote! {
        #[::suprnova::__async_trait::async_trait]
        impl ::suprnova::RouteBinding for #ident {
            fn route_key_name() -> &'static str {
                "value"
            }

            fn route_key(&self) -> ::std::string::String {
                ::std::string::String::from(match self {
                    #(#to_value)*
                })
            }

            fn route_field(&self, _field: &str) -> ::core::option::Option<::std::string::String> {
                ::core::option::Option::Some(self.route_key())
            }

            async fn resolve_route_binding(
                value: &str,
                _field: ::core::option::Option<&str>,
            ) -> ::core::result::Result<::core::option::Option<Self>, ::suprnova::FrameworkError> {
                ::core::result::Result::Ok(match value {
                    #(#from_value)*
                    _ => ::core::option::Option::None,
                })
            }

            fn route_binding_info() -> ::suprnova::RouteBindingInfo {
                ::suprnova::RouteBindingInfo::of::<Self>().unit_enum()
            }
        }

        impl ::suprnova::RouteValue for #ident {
            fn route_value(&self, field: ::core::option::Option<&str>) -> ::core::option::Option<::std::string::String> {
                ::suprnova::bound_route_value(self, field)
            }
        }
    })
}

/// The `value` of a variant's `#[route(value = "...")]`, if it has one.
fn route_value(variant: &syn::Variant) -> syn::Result<Option<String>> {
    let mut found = None;
    for attr in &variant.attrs {
        if !attr.path().is_ident("route") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("value") {
                let value: LitStr = meta.value()?.parse()?;
                if value.value().is_empty() {
                    return Err(meta.error("a route value cannot be empty"));
                }
                found = Some(value.value());
                Ok(())
            } else {
                Err(meta.error("expected `#[route(value = \"...\")]`"))
            }
        })?;
    }
    Ok(found)
}

/// A variant name in snake case, as Laravel's `Str::snake` and serde's
/// `snake_case` write it: `InReview` is `in_review`.
fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (index, ch) in name.char_indices() {
        if ch.is_uppercase() && index != 0 {
            out.push('_');
        }
        out.extend(ch.to_lowercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand_str(src: proc_macro2::TokenStream) -> String {
        match expand(syn::parse2(src).expect("an item")) {
            Ok(tokens) => tokens.to_string(),
            Err(error) => error.to_compile_error().to_string(),
        }
    }

    #[test]
    fn bind_010_variants_bind_from_their_value_or_snake_case_name() {
        let out = expand_str(quote! {
            enum Status { Draft, InReview, #[route(value = "live")] Published }
        });
        assert!(
            out.contains("\"draft\" => :: core :: option :: Option :: Some (Self :: Draft)"),
            "got:\n{out}"
        );
        assert!(out.contains("\"in_review\" =>"), "got:\n{out}");
        assert!(out.contains("\"live\" =>"), "got:\n{out}");
        assert!(!out.contains("\"published\""), "got:\n{out}");
        assert!(out.contains("unit_enum ()"), "got:\n{out}");
    }

    #[test]
    fn bind_010_a_variant_with_fields_is_refused() {
        let out = expand_str(quote! { enum Shape { Dot, Line(u32) } });
        assert!(out.contains("compile_error"), "got:\n{out}");
        assert!(out.contains("unit variants"), "got:\n{out}");
    }

    #[test]
    fn bind_010_two_variants_with_one_value_are_refused() {
        let out = expand_str(quote! {
            enum Status { Draft, #[route(value = "draft")] Pending }
        });
        assert!(out.contains("compile_error"), "got:\n{out}");
    }

    #[test]
    fn bind_010_a_struct_is_refused() {
        let out = expand_str(quote! { struct Status; });
        assert!(out.contains("compile_error"), "got:\n{out}");
    }
}
