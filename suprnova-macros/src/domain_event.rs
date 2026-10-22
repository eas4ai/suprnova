//! Stable schema declarations and snapshot registration.
use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemStruct, LitInt, LitStr, Path, parse_macro_input};

pub fn domain_event(args: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as ItemStruct);
    let mut name: Option<LitStr> = None;
    let mut version: Option<LitInt> = None;
    let mut upgrade: Option<Path> = None;
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("name") {
            name = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("version") {
            version = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("upgrade") {
            let value: LitStr = meta.value()?.parse()?;
            upgrade = Some(value.parse()?);
        } else {
            return Err(meta.error("expected name, version or upgrade"));
        }
        Ok(())
    });
    parse_macro_input!(args with parser);
    let expand = || -> syn::Result<proc_macro2::TokenStream> {
        if !item.generics.params.is_empty() {
            return Err(syn::Error::new_spanned(
                &item.generics,
                "domain events must be concrete types",
            ));
        }
        let name = name.ok_or_else(|| {
            syn::Error::new_spanned(&item.ident, "domain_event requires a stable name")
        })?;
        if name.value().is_empty() || name.value().len() > 191 || !name.value().is_ascii() {
            return Err(syn::Error::new_spanned(
                name,
                "event name must contain 1 to 191 ASCII bytes",
            ));
        }
        let version = version
            .ok_or_else(|| syn::Error::new_spanned(&item.ident, "domain_event requires version"))?;
        if version.base10_parse::<i32>()? < 1 {
            return Err(syn::Error::new_spanned(
                version,
                "event version starts at one",
            ));
        }
        let ident = &item.ident;
        let upgrade = upgrade.map(|path| quote! {
            fn upgrade(version: i32, value: ::suprnova::serde_json::Value) -> ::std::result::Result<::suprnova::serde_json::Value, ::suprnova::FrameworkError> {
                #path(version, value)
            }
        });
        Ok(quote! {
            #item
            impl ::suprnova::DomainEvent for #ident {
                const NAME: &'static str = #name;
                const VERSION: i32 = #version;
                #upgrade
            }
            ::suprnova::inventory::submit! {
                ::suprnova::DomainEventEntry { name: #name, decode: ::suprnova::DomainEventEntry::decode::<#ident> }
            }
        })
    };
    expand()
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

pub fn aggregate(args: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as ItemStruct);
    if !args.is_empty() || !item.generics.params.is_empty() {
        return syn::Error::new_spanned(
            &item.ident,
            "aggregate takes no options and requires a concrete AggregateRoot",
        )
        .to_compile_error()
        .into();
    }
    let ident = &item.ident;
    quote! {
        #item
        ::suprnova::inventory::submit! {
            ::suprnova::AggregateEntry {
                name: <#ident as ::suprnova::AggregateRoot>::NAME,
                snapshot: ::suprnova::AggregateEntry::snapshot::<#ident>,
            }
        }
    }
    .into()
}

pub fn handler(args: TokenStream, input: TokenStream, reactor: bool) -> TokenStream {
    let item = parse_macro_input!(input as ItemStruct);
    let args: proc_macro2::TokenStream = args.into();
    let queued = args.to_string() == "queued";
    if !item.generics.params.is_empty() || (!args.is_empty() && (!queued || reactor)) {
        return syn::Error::new_spanned(
            &item.ident,
            "event handlers require a concrete Default type; only projector accepts queued",
        )
        .to_compile_error()
        .into();
    }
    let ident = &item.ident;
    let entry = if reactor {
        quote! {
            ::suprnova::ReactorEntry { build: || ::std::boxed::Box::new(<#ident as ::std::default::Default>::default()) }
        }
    } else {
        quote! {
            ::suprnova::ProjectorEntry { build: || ::std::boxed::Box::new(<#ident as ::std::default::Default>::default()), queued: #queued }
        }
    };
    quote! { #item ::suprnova::inventory::submit! { #entry } }.into()
}
