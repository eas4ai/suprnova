use proc_macro::TokenStream;
use proc_macro2::{TokenStream as TokenStream2, TokenTree};

pub(crate) fn finish(result: syn::Result<TokenStream2>) -> TokenStream {
    match result {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Refuse generated code that names a development crate instead of the
/// `::suprnova::live` facade an application depends on.
///
/// Only identifiers are paths. A string literal such as a component name,
/// a view path or text in an action body is not a path, so it is not
/// checked: `test_support.page` is a valid component name. The forbidden
/// roots are the engine crate and its development crates, which all start
/// with `suprnova_live`, and `$crate`, which only resolves inside the crate
/// that defines a macro.
pub(crate) fn enforce_runtime_path_contract(tokens: &TokenStream2) -> syn::Result<()> {
    let mut previous_dollar = false;
    for tree in tokens.clone() {
        let forbidden = match &tree {
            TokenTree::Ident(ident) => {
                let name = ident.to_string();
                name.starts_with("suprnova_live") || (previous_dollar && name == "crate")
            }
            TokenTree::Group(group) => {
                enforce_runtime_path_contract(&group.stream())?;
                false
            }
            TokenTree::Punct(_) | TokenTree::Literal(_) => false,
        };
        if forbidden {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "generated runtime paths must use the final ::suprnova::live facade",
            ));
        }
        previous_dollar = matches!(&tree, TokenTree::Punct(punct) if punct.as_char() == '$');
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use quote::quote;
    use syn::{DeriveInput, ItemImpl};

    use super::enforce_runtime_path_contract;
    use crate::live::{component, live_impl};

    fn assert_public_live_facade(tokens: &proc_macro2::TokenStream) {
        let source = tokens.to_string();
        assert!(source.contains(":: suprnova :: live :: __private"));
        for forbidden in [
            "suprnova_live",
            "suprnova-live-macros",
            "$crate",
            "macro_fixture",
            "test_support",
            "crate :: live",
            "super ::",
        ] {
            assert!(
                !source.contains(forbidden),
                "generated tokens contained forbidden runtime path `{forbidden}`: {source}"
            );
        }
    }

    #[test]
    fn path_guard_rejects_development_runtime_names() {
        assert!(enforce_runtime_path_contract(&quote!(::suprnova_live::metadata)).is_err());
        assert!(
            enforce_runtime_path_contract(&quote!(::suprnova_live_test_support::host)).is_err()
        );
        assert!(enforce_runtime_path_contract(&quote!({ $crate::metadata })).is_err());
        assert!(enforce_runtime_path_contract(&quote!(::suprnova::live::metadata)).is_ok());
    }

    #[test]
    fn path_guard_ignores_names_and_text_that_are_not_paths() {
        assert!(
            enforce_runtime_path_contract(&quote!(
                ComponentName::parse("test_support.page");
                ViewName::parse("live/macro_fixture/suprnova_live.html");
                let crate_name = "suprnova-live-macros";
            ))
            .is_ok()
        );
    }

    #[test]
    fn component_and_impl_expansions_use_only_the_public_live_facade() {
        let component: DeriveInput = syn::parse_quote! {
            #[live(name = "macro.path", view = "live/macro/path.html")]
            pub struct MacroPath {
                #[model]
                value: String,
            }
        };
        let component_tokens = component::derive(component).expect("component expansion");
        assert_public_live_facade(&component_tokens);

        let implementation: ItemImpl = syn::parse_quote! {
            impl MacroPath {
                #[action]
                pub async fn save(&mut self) {}
            }
        };
        let implementation_tokens =
            live_impl::expand(proc_macro2::TokenStream::new(), implementation)
                .expect("impl expansion");
        assert_public_live_facade(&implementation_tokens);
    }
}
