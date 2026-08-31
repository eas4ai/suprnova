mod attrs;
mod component;
mod expand;
mod live_impl;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use syn::{DeriveInput, Item};

pub(crate) fn derive_live_component(input: TokenStream) -> TokenStream {
    expand::finish(syn::parse::<DeriveInput>(input).and_then(component::derive))
}

pub(crate) fn expand_live(args: TokenStream, item: TokenStream) -> TokenStream {
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
