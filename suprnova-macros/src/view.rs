use proc_macro::TokenStream;

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::parse::Parser as _;
use syn::punctuated::Punctuated;
use syn::{Field, ItemFn, ItemStruct, Lit, Meta, Token, Visibility};

pub(crate) fn expand_view(args: TokenStream, item: TokenStream) -> TokenStream {
    finish(expand_view_inner(TokenStream2::from(args), item))
}

pub(crate) fn expand_view_filter(args: TokenStream, item: TokenStream) -> TokenStream {
    finish(expand_view_filter_inner(TokenStream2::from(args), item))
}

fn expand_view_inner(args: TokenStream2, item: TokenStream) -> syn::Result<TokenStream2> {
    let path = parse_view_path(args)?;
    let mut item = syn::parse::<ItemStruct>(item)?;
    let ident = item.ident.clone();
    let module = format_ident!("__suprnova_view_{ident}");
    let export_visibility = item.vis.clone();

    item.vis = Visibility::Public(syn::token::Pub::default());
    for field in item.fields.iter_mut() {
        preserve_parent_field_access(field);
    }

    Ok(quote! {
        #[doc(hidden)]
        #[allow(non_snake_case, reason = "generated view module follows the application type name")]
        mod #module {
            use super::*;

            #[derive(::suprnova::live::__private::askama::Template)]
            #[template(
                askama = ::suprnova::live::__private::askama,
                path = #path
            )]
            #item
        }

        #export_visibility use #module::#ident;
    })
}

fn expand_view_filter_inner(args: TokenStream2, item: TokenStream) -> syn::Result<TokenStream2> {
    if !args.is_empty() {
        return Err(syn::Error::new_spanned(
            args,
            "#[view_filter] does not accept arguments",
        ));
    }

    let mut item = syn::parse::<ItemFn>(item)?;
    let ident = item.sig.ident.clone();
    let module = format_ident!("__suprnova_view_filter_{ident}");
    let export_visibility = item.vis.clone();
    item.vis = Visibility::Public(syn::token::Pub::default());

    Ok(quote! {
        #[doc(hidden)]
        #[allow(non_snake_case, reason = "generated filter module follows the application function name")]
        mod #module {
            use super::*;
            use ::suprnova::live::__private::askama as askama;

            #[::suprnova::live::__private::askama::filter_fn]
            #item
        }

        #export_visibility use #module::#ident;
    })
}

fn parse_view_path(args: TokenStream2) -> syn::Result<syn::LitStr> {
    let entries = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(args)?;
    let mut path = None;

    for entry in entries {
        match entry {
            Meta::NameValue(value) if value.path.is_ident("path") => {
                if path.is_some() {
                    return Err(syn::Error::new_spanned(value, "duplicate `path` argument"));
                }
                match value.value {
                    syn::Expr::Lit(expression) => match expression.lit {
                        Lit::Str(value) => path = Some(value),
                        other => {
                            return Err(syn::Error::new_spanned(
                                other,
                                "`path` must be a string literal",
                            ));
                        }
                    },
                    other => {
                        return Err(syn::Error::new_spanned(
                            other,
                            "`path` must be a string literal",
                        ));
                    }
                }
            }
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "expected exactly `path = \"relative/template.html\"`",
                ));
            }
        }
    }

    let path = path.ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "missing required `path = \"relative/template.html\"` argument",
        )
    })?;
    validate_view_path(&path)?;
    Ok(path)
}

fn validate_view_path(path: &syn::LitStr) -> syn::Result<()> {
    let value = path.value();
    let valid = !value.is_empty()
        && value.len() <= 256
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains(':')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'/' | b'-'))
        && value
            .split('/')
            .all(|segment| !segment.is_empty() && !matches!(segment, "." | ".."));
    if valid {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            path,
            "view path must be a bounded relative path without traversal segments",
        ))
    }
}

fn preserve_parent_field_access(field: &mut Field) {
    match &mut field.vis {
        Visibility::Inherited => field.vis = syn::parse_quote!(pub(super)),
        Visibility::Restricted(restricted) if restricted.path.is_ident("self") => {
            restricted.path = syn::parse_quote!(super);
        }
        Visibility::Restricted(restricted)
            if restricted
                .path
                .segments
                .first()
                .is_some_and(|segment| segment.ident == "self") =>
        {
            restricted
                .path
                .segments
                .first_mut()
                .expect("relative visibility has a first segment")
                .ident = format_ident!("super");
        }
        Visibility::Restricted(restricted)
            if restricted
                .path
                .segments
                .first()
                .is_some_and(|segment| segment.ident == "super") =>
        {
            let path = &restricted.path;
            restricted.path = syn::parse_quote!(super::#path);
            if restricted.path.segments.len() > 1 && restricted.in_token.is_none() {
                restricted.in_token = Some(syn::token::In::default());
            }
        }
        Visibility::Public(_) | Visibility::Restricted(_) => {}
    }
}

fn finish(result: syn::Result<TokenStream2>) -> TokenStream {
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}
