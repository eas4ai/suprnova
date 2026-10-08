//! `#[inertia_root(path = "...")]`: an application's Inertia root document.
//!
//! The struct the attribute sits on is a unit marker the application names
//! in its configuration. Every value the template reads comes from the
//! framework, so the attribute generates the Askama struct itself: its only
//! field is the framework's parts, reached through `Deref`, so a template
//! that names anything else fails to compile.

use proc_macro::TokenStream;

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{Fields, ItemStruct};

pub(crate) fn expand_inertia_root(args: TokenStream, item: TokenStream) -> TokenStream {
    expand(TokenStream2::from(args), item)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand(args: TokenStream2, item: TokenStream) -> syn::Result<TokenStream2> {
    let path = crate::view::parse_view_path(args)?;
    let item = syn::parse::<ItemStruct>(item)?;
    if !matches!(item.fields, Fields::Unit) {
        return Err(syn::Error::new_spanned(
            &item.fields,
            "an Inertia root template takes every value from the framework's parts; \
             declare it as a unit struct (`pub struct AppDocument;`)",
        ));
    }
    if !item.generics.params.is_empty() || item.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            &item.generics,
            "an Inertia root template takes no generic parameters",
        ));
    }

    let ident = &item.ident;
    let name = ident.to_string();
    // In the caller's module rather than a generated one, so a custom
    // filter the template names resolves through the caller's `filters`
    // module exactly as it does for `#[view]`, with no glob import left
    // unused when the template names none.
    let document = format_ident!("__SuprnovaInertiaRoot{ident}");

    Ok(quote! {
        #item

        #[doc(hidden)]
        #[derive(::suprnova::live::__private::askama::Template)]
        #[template(
            askama = ::suprnova::live::__private::askama,
            path = #path
        )]
        struct #document<'parts, 'page>(&'parts ::suprnova::InertiaRootParts<'page>);

        impl<'parts, 'page> ::core::ops::Deref for #document<'parts, 'page> {
            type Target = ::suprnova::InertiaRootParts<'page>;

            fn deref(&self) -> &Self::Target {
                self.0
            }
        }

        impl ::suprnova::InertiaRoot for #ident {
            const NAME: &'static str = #name;

            fn render(
                parts: &::suprnova::InertiaRootParts<'_>,
                output: &mut dyn ::core::fmt::Write,
            ) -> ::core::result::Result<(), ::suprnova::view::TemplateFailure> {
                ::suprnova::view::ViewTemplate::render_view(&#document(parts), output)
            }
        }
    })
}
