use std::collections::BTreeMap;

use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::ext::IdentExt as _;
use syn::spanned::Spanned as _;
use syn::{Attribute, FnArg, ImplItem, ImplItemFn, ItemImpl, Meta, Receiver, Visibility};

use crate::attrs::{
    is_field_helper, is_method_helper, parse_action_args, validate_registered_name,
};
use crate::expand::enforce_runtime_path_contract;

pub(crate) fn expand(args: TokenStream2, mut item: ItemImpl) -> syn::Result<TokenStream2> {
    if !args.is_empty() {
        return Err(syn::Error::new(
            Span::call_site(),
            "the outer #[live] impl attribute does not accept arguments",
        ));
    }
    if item.trait_.is_some() {
        return Err(syn::Error::new(
            item.impl_token.span(),
            "#[live] requires an inherent impl",
        ));
    }
    if !item.generics.params.is_empty() || item.generics.where_clause.is_some() {
        return Err(syn::Error::new(
            item.generics.span(),
            "a Live component impl cannot be generic",
        ));
    }

    let mut actions = BTreeMap::<String, (u16, Span)>::new();
    let mut singleton_helpers = BTreeMap::<String, Span>::new();
    for impl_item in &mut item.items {
        let ImplItem::Fn(method) = impl_item else {
            reject_helpers_on_non_method(impl_item)?;
            continue;
        };
        let helper = take_method_helper(method)?;
        let Some((helper_name, attribute)) = helper else {
            continue;
        };
        validate_common_signature(method)?;
        match helper_name.as_str() {
            "action" => {
                validate_action_signature(method)?;
                let args = parse_action_args(&attribute)?;
                let name = args
                    .name
                    .as_ref()
                    .map_or_else(|| method.sig.ident.unraw().to_string(), syn::LitStr::value);
                let literal = args
                    .name
                    .unwrap_or_else(|| syn::LitStr::new(&name, method.sig.ident.span()));
                validate_registered_name(&literal)?;
                if actions
                    .insert(name, (args.version, attribute.span()))
                    .is_some()
                {
                    return Err(syn::Error::new(
                        attribute.span(),
                        "duplicate registered Live action name",
                    ));
                }
            }
            "mount" => {
                ensure_singleton(&mut singleton_helpers, &helper_name, attribute.span())?;
                validate_mount_signature(method)?;
                ensure_path_helper(&attribute)?;
            }
            "computed" => {
                validate_computed_signature(method)?;
                ensure_path_helper(&attribute)?;
            }
            "validate" => {
                validate_receiver_method(method, false)?;
                ensure_path_helper(&attribute)?;
            }
            _ => {
                ensure_singleton(&mut singleton_helpers, &helper_name, attribute.span())?;
                validate_receiver_method(method, true)?;
                ensure_path_helper(&attribute)?;
            }
        }
    }

    let self_ty = &item.self_ty;
    let action_values = actions.into_iter().map(|(name, (version, _))| {
        quote! {
            ::suprnova::live::__private::metadata::ActionMetadata::new(
                ::suprnova::live::__private::identity::ActionName::parse(#name)
                    .expect("macro-validated Live action identity"),
                #version,
            )?
        }
    });
    let tokens = quote! {
        #item

        impl ::suprnova::live::__private::metadata::LiveComponentContract for #self_ty {
            fn descriptor() -> ::std::result::Result<
                ::suprnova::live::__private::registry::ComponentDescriptor,
                ::suprnova::live::__private::metadata::MetadataError,
            > {
                let metadata = <Self as
                    ::suprnova::live::__private::metadata::LiveComponentDefinitionMetadata
                >::component_metadata(::std::vec![#(#action_values),*])?;
                ::std::result::Result::Ok(
                    ::suprnova::live::__private::registry::ComponentDescriptor::new(metadata),
                )
            }
        }
    };
    enforce_runtime_path_contract(&tokens)?;
    Ok(tokens)
}

fn take_method_helper(method: &mut ImplItemFn) -> syn::Result<Option<(String, Attribute)>> {
    let mut helper = None;
    let mut retained = Vec::with_capacity(method.attrs.len());
    for attribute in method.attrs.drain(..) {
        let Some(name) = attribute
            .path()
            .get_ident()
            .map(|ident| ident.unraw().to_string())
        else {
            retained.push(attribute);
            continue;
        };
        if is_field_helper(&name) {
            return Err(syn::Error::new(
                attribute.span(),
                "field helper cannot be placed on a Live impl method",
            ));
        }
        if is_method_helper(&name) {
            if helper.replace((name, attribute)).is_some() {
                return Err(syn::Error::new(
                    method.sig.ident.span(),
                    "a Live method may declare only one method helper",
                ));
            }
        } else {
            retained.push(attribute);
        }
    }
    method.attrs = retained;
    Ok(helper)
}

fn reject_helpers_on_non_method(item: &ImplItem) -> syn::Result<()> {
    let attributes = match item {
        ImplItem::Const(item) => &item.attrs,
        ImplItem::Type(item) => &item.attrs,
        ImplItem::Macro(item) => &item.attrs,
        ImplItem::Verbatim(_) | _ => return Ok(()),
    };
    if let Some(attribute) = attributes.iter().find(|attribute| {
        attribute.path().get_ident().is_some_and(|ident| {
            let name = ident.unraw().to_string();
            is_method_helper(&name) || is_field_helper(&name)
        })
    }) {
        return Err(syn::Error::new(
            attribute.span(),
            "Live helpers can only be placed on their documented item kind",
        ));
    }
    Ok(())
}

fn validate_common_signature(method: &ImplItemFn) -> syn::Result<()> {
    let signature = &method.sig;
    if signature.constness.is_some()
        || signature.unsafety.is_some()
        || signature.abi.is_some()
        || signature.variadic.is_some()
        || !signature.generics.params.is_empty()
        || signature.generics.where_clause.is_some()
    {
        return Err(syn::Error::new(
            signature.span(),
            "Live methods cannot be const, unsafe, extern, variadic, or generic",
        ));
    }
    if !matches!(method.vis, Visibility::Public(_)) {
        return Err(syn::Error::new(
            method.sig.ident.span(),
            "registered Live methods must be public",
        ));
    }
    Ok(())
}

fn validate_action_signature(method: &ImplItemFn) -> syn::Result<()> {
    if method.sig.asyncness.is_none() {
        return Err(syn::Error::new(
            method.sig.ident.span(),
            "Live actions must be async",
        ));
    }
    let receiver = method.sig.inputs.first().and_then(receiver);
    if !receiver.is_some_and(|value| value.reference.is_some() && value.mutability.is_some()) {
        return Err(syn::Error::new(
            method.sig.ident.span(),
            "Live actions require `&mut self` as their first argument",
        ));
    }
    Ok(())
}

fn validate_mount_signature(method: &ImplItemFn) -> syn::Result<()> {
    if method
        .sig
        .inputs
        .iter()
        .any(|argument| matches!(argument, FnArg::Receiver(_)))
    {
        return Err(syn::Error::new(
            method.sig.ident.span(),
            "mount must be an associated constructor without a self receiver",
        ));
    }
    Ok(())
}

fn validate_computed_signature(method: &ImplItemFn) -> syn::Result<()> {
    if method.sig.asyncness.is_some() {
        return Err(syn::Error::new(
            method.sig.ident.span(),
            "computed values must be synchronous",
        ));
    }
    let receiver = method.sig.inputs.first().and_then(receiver);
    if !receiver.is_some_and(|value| value.reference.is_some() && value.mutability.is_none()) {
        return Err(syn::Error::new(
            method.sig.ident.span(),
            "computed values require `&self`",
        ));
    }
    Ok(())
}

fn validate_receiver_method(method: &ImplItemFn, require_mutable: bool) -> syn::Result<()> {
    let receiver = method.sig.inputs.first().and_then(receiver);
    let valid = receiver.is_some_and(|value| {
        value.reference.is_some() && (!require_mutable || value.mutability.is_some())
    });
    if !valid {
        return Err(syn::Error::new(
            method.sig.ident.span(),
            if require_mutable {
                "lifecycle hooks require `&mut self`"
            } else {
                "validation methods require `&self` or `&mut self`"
            },
        ));
    }
    Ok(())
}

fn receiver(argument: &FnArg) -> Option<&Receiver> {
    match argument {
        FnArg::Receiver(receiver) => Some(receiver),
        FnArg::Typed(_) => None,
    }
}

fn ensure_singleton(
    helpers: &mut BTreeMap<String, Span>,
    name: &str,
    span: Span,
) -> syn::Result<()> {
    if helpers.insert(name.to_owned(), span).is_some() {
        return Err(syn::Error::new(
            span,
            format!("duplicate #[{name}] lifecycle helper"),
        ));
    }
    Ok(())
}

fn ensure_path_helper(attribute: &Attribute) -> syn::Result<()> {
    if matches!(attribute.meta, Meta::Path(_)) {
        Ok(())
    } else {
        Err(syn::Error::new(
            attribute.span(),
            "this Live method helper does not accept arguments",
        ))
    }
}
