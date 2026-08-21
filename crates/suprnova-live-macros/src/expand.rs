use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;

pub(crate) fn finish(result: syn::Result<TokenStream2>) -> TokenStream {
    match result {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

pub(crate) fn enforce_runtime_path_contract(tokens: &TokenStream2) -> syn::Result<()> {
    let source = tokens.to_string();
    for forbidden in [
        "suprnova_live",
        "suprnova-live-macros",
        "$crate",
        "macro_fixture",
        "test_support",
    ] {
        if source.contains(forbidden) {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "generated runtime paths must use the final ::suprnova::live facade",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use super::enforce_runtime_path_contract;

    #[test]
    fn path_guard_rejects_development_runtime_names() {
        assert!(enforce_runtime_path_contract(&quote!(::suprnova_live::metadata)).is_err());
        assert!(enforce_runtime_path_contract(&quote!(::suprnova::live::metadata)).is_ok());
    }
}
