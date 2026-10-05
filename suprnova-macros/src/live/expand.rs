use proc_macro::TokenStream;
use proc_macro2::{TokenStream as TokenStream2, TokenTree};

pub(crate) fn finish(result: syn::Result<TokenStream2>) -> TokenStream {
    match result {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// The Rust names of the Live engine crate and its development crates. A
/// path rooted at one of them builds only inside this workspace.
const DEVELOPMENT_CRATES: &[&str] = &[
    "suprnova_live",
    "suprnova_live_fuzz",
    "suprnova_live_macro_fixture",
    "suprnova_live_test_support",
];

/// Refuse generated code that names a development crate instead of the
/// `::suprnova::live` facade an application depends on.
///
/// Only paths are checked: one of [`DEVELOPMENT_CRATES`] followed by `::`,
/// and `$crate`, which only resolves inside the crate that defines a macro.
/// A string literal such as a component name, a view path or text in an
/// action body is not a path: `test_support.page` is a valid component
/// name. Nor is an identifier of the application that no `::` follows, such
/// as a field `suprnova_live_count` or an argument named `suprnova_live`:
/// the generated code carries the application's names, and those are not
/// crates.
pub(crate) fn enforce_runtime_path_contract(tokens: &TokenStream2) -> syn::Result<()> {
    let trees: Vec<TokenTree> = tokens.clone().into_iter().collect();
    for (index, tree) in trees.iter().enumerate() {
        let forbidden = match tree {
            TokenTree::Ident(ident) => {
                let name = ident.to_string();
                let name = name.strip_prefix("r#").unwrap_or(&name);
                let after_dollar = index
                    .checked_sub(1)
                    .and_then(|previous| trees.get(previous))
                    .is_some_and(|previous| is_punct(previous, '$'));
                (after_dollar && name == "crate")
                    || (DEVELOPMENT_CRATES.contains(&name)
                        && starts_with_path_separator(&trees[index + 1..]))
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
    }
    Ok(())
}

/// Whether `tree` is the punctuation `ch`.
fn is_punct(tree: &TokenTree, ch: char) -> bool {
    matches!(tree, TokenTree::Punct(punct) if punct.as_char() == ch)
}

/// Whether `rest` starts with the path separator `::`: two `:` with nothing
/// between them, which is how a token stream spells it.
fn starts_with_path_separator(rest: &[TokenTree]) -> bool {
    match rest {
        [TokenTree::Punct(first), second, ..] => {
            first.as_char() == ':'
                && first.spacing() == proc_macro2::Spacing::Joint
                && is_punct(second, ':')
        }
        _ => false,
    }
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

    // IDENTITY-038: an application identifier that starts with
    // `suprnova_live`, or even equals a development crate's name, is not a
    // path to that crate unless a `::` follows it.
    #[test]
    fn path_guard_accepts_application_identifiers_that_share_the_prefix() {
        assert!(
            enforce_runtime_path_contract(&quote!(
                self.suprnova_live_count += 1;
                let suprnova_live = suprnova_live_total(SuprnovaLive { suprnova_live_test_support: 1 });
                fn suprnova_live_step(suprnova_live_fuzz: u8) {}
            ))
            .is_ok()
        );
        assert!(enforce_runtime_path_contract(&quote!(suprnova_live_count::helper())).is_ok());
        assert!(
            enforce_runtime_path_contract(&quote!(suprnova_live_macro_fixture::Component)).is_err()
        );
        assert!(enforce_runtime_path_contract(&quote!(r#suprnova_live::metadata)).is_err());
    }

    #[test]
    fn a_component_with_an_application_field_named_like_the_engine_expands() {
        let component: DeriveInput = syn::parse_quote! {
            #[live(name = "macro.counter", view = "live/macro/counter.html")]
            pub struct Counter {
                #[model]
                suprnova_live_count: u64,
            }
        };
        component::derive(component).expect("component expansion");

        let implementation: ItemImpl = syn::parse_quote! {
            impl Counter {
                #[action]
                pub async fn bump(&mut self, suprnova_live_step: u64) {
                    self.suprnova_live_count += suprnova_live_step;
                }
            }
        };
        live_impl::expand(proc_macro2::TokenStream::new(), implementation).expect("impl expansion");
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
