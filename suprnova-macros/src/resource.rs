//! `resource!` and `api_resource!`: a resource whose actions are the
//! `#[handler]` functions of one module, found by action name (BIND-011).
//!
//! ```rust,ignore
//! resource!("posts", controllers::posts)
//! resource!("users.posts", controllers::user_posts, only = [index, show])
//! api_resource!("photos", controllers::photos, except = [destroy])
//! ```
//!
//! The expansion names `<module>::<action>` for every selected action, so
//! a selected action the module does not define fails to compile, and an
//! action `only` or `except` leaves out is never named.

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Path, Token, bracketed, punctuated::Punctuated};

/// The seven actions, in Laravel's order, with their `ResourceAction`
/// variants.
const ACTIONS: [(&str, &str); 7] = [
    ("index", "Index"),
    ("create", "Create"),
    ("store", "Store"),
    ("show", "Show"),
    ("edit", "Edit"),
    ("update", "Update"),
    ("destroy", "Destroy"),
];

/// The actions an API resource has: no `create` or `edit` form.
const API_ACTIONS: [&str; 5] = ["index", "store", "show", "update", "destroy"];

/// `only = [...]` or `except = [...]`.
enum Selection {
    Only(Vec<Ident>),
    Except(Vec<Ident>),
}

struct ResourceInput {
    name: LitStr,
    module: Path,
    selection: Option<Selection>,
}

impl Parse for ResourceInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: LitStr = input.parse()?;
        input.parse::<Token![,]>()?;
        let module: Path = input.parse()?;
        let mut selection = None;
        if input.parse::<Option<Token![,]>>()?.is_some() && !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            let content;
            bracketed!(content in input);
            let actions: Punctuated<Ident, Token![,]> =
                content.parse_terminated(Ident::parse, Token![,])?;
            let actions: Vec<Ident> = actions.into_iter().collect();
            selection = Some(match key.to_string().as_str() {
                "only" => Selection::Only(actions),
                "except" => Selection::Except(actions),
                other => {
                    return Err(syn::Error::new_spanned(
                        &key,
                        format!("expected `only = [...]` or `except = [...]`, found `{other}`"),
                    ));
                }
            });
            input.parse::<Option<Token![,]>>()?;
        }
        if !input.is_empty() {
            return Err(input.error("unexpected tokens after the resource's actions"));
        }
        Ok(Self {
            name,
            module,
            selection,
        })
    }
}

/// Expand `resource!` (`api` false) or `api_resource!` (`api` true).
pub fn expand(input: TokenStream, api: bool) -> syn::Result<TokenStream> {
    let ResourceInput {
        name,
        module,
        selection,
    } = syn::parse2(input)?;
    let defaults: Vec<&str> = ACTIONS
        .iter()
        .map(|(action, _)| *action)
        .filter(|action| !api || API_ACTIONS.contains(action))
        .collect();
    let check = |idents: &[Ident]| -> syn::Result<Vec<String>> {
        idents
            .iter()
            .map(|ident| {
                let action = ident.to_string();
                if ACTIONS.iter().any(|(known, _)| *known == action) {
                    Ok(action)
                } else {
                    Err(syn::Error::new_spanned(
                        ident,
                        format!(
                            "`{action}` is not a resource action; the actions are index, \
                             create, store, show, edit, update and destroy"
                        ),
                    ))
                }
            })
            .collect()
    };
    let selected: Vec<&str> = match &selection {
        None => defaults,
        Some(Selection::Only(idents)) => {
            let only = check(idents)?;
            ACTIONS
                .iter()
                .map(|(action, _)| *action)
                .filter(|action| only.iter().any(|picked| picked == action))
                .collect()
        }
        Some(Selection::Except(idents)) => {
            let except = check(idents)?;
            defaults
                .into_iter()
                .filter(|action| !except.iter().any(|left| left == action))
                .collect()
        }
    };

    let variants: Vec<TokenStream> = selected
        .iter()
        .map(|action| {
            let variant = ACTIONS
                .iter()
                .find(|(known, _)| known == action)
                .map(|(_, variant)| Ident::new(variant, proc_macro2::Span::call_site()))
                .expect("selected actions are known actions");
            quote! { ::suprnova::routing::ResourceAction::#variant }
        })
        .collect();
    let functions = selected.iter().zip(&variants).map(|(action, variant)| {
        let function = Ident::new(action, name.span());
        quote! { .__action(#variant, #module::#function) }
    });
    Ok(quote! {
        ::suprnova::routing::ResourceDef::__new(#name, &[#(#variants),*])
            #(#functions)*
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand_str(src: TokenStream, api: bool) -> String {
        match expand(src, api) {
            Ok(tokens) => tokens.to_string(),
            Err(error) => error.to_compile_error().to_string(),
        }
    }

    #[test]
    fn bind_011_a_resource_names_every_action_function() {
        let out = expand_str(quote! { "posts", controllers::posts }, false);
        for action in [
            "index", "create", "store", "show", "edit", "update", "destroy",
        ] {
            assert!(
                out.contains(&format!("controllers :: posts :: {action}")),
                "`{action}` missing; got:\n{out}"
            );
        }
    }

    #[test]
    fn bind_011_only_and_except_name_just_the_selected_functions() {
        let out = expand_str(quote! { "posts", posts, only = [show, index] }, false);
        assert!(out.contains("posts :: index"), "got:\n{out}");
        assert!(out.contains("posts :: show"), "got:\n{out}");
        assert!(!out.contains("posts :: store"), "got:\n{out}");

        let out = expand_str(quote! { "posts", posts, except = [destroy] }, true);
        assert!(!out.contains("posts :: destroy"), "got:\n{out}");
        assert!(
            !out.contains("posts :: create"),
            "an API resource has no create; got:\n{out}"
        );
        assert!(out.contains("posts :: update"), "got:\n{out}");
    }

    #[test]
    fn bind_011_an_unknown_action_is_refused() {
        let out = expand_str(quote! { "posts", posts, only = [list] }, false);
        assert!(out.contains("compile_error"), "got:\n{out}");
    }
}
