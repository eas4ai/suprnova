//! Development-only procedural macros for Suprnova Live.

#![forbid(unsafe_code)]
#![deny(missing_docs, rustdoc::broken_intra_doc_links)]

extern crate proc_macro;

mod attrs;
mod component;
mod expand;
mod live_impl;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use syn::{DeriveInput, Item};

/// Derives the canonical state-side metadata for a Live component.
#[proc_macro_derive(
    LiveComponent,
    attributes(
        live,
        __suprnova_live,
        public,
        model,
        locked,
        server_only,
        session,
        secret,
        transient,
        url,
        mount,
        action,
        computed,
        validate,
        hydrate,
        rendering,
        rendered,
        dehydrate,
        teardown,
        params_changed,
        lazy_complete
    )
)]
pub fn derive_live_component(input: TokenStream) -> TokenStream {
    expand::finish(syn::parse::<DeriveInput>(input).and_then(component::derive))
}

/// Declares either component metadata on a struct or registered behavior on its impl.
#[proc_macro_attribute]
pub fn live(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = TokenStream2::from(args);
    expand::finish(syn::parse::<Item>(item).and_then(|item| match item {
        Item::Struct(item) => component::rewrite_struct_helper(args, item),
        Item::Impl(item) => live_impl::expand(args, item),
        other => Err(syn::Error::new_spanned(
            other,
            "#[live] can only configure a LiveComponent struct or inherent impl",
        )),
    }))
}
