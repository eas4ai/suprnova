//! Workflow step attribute macro

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{FnArg, ItemFn, Pat, ReturnType, Type, parse_macro_input};

pub fn workflow_step_impl(_attr: TokenStream, input: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(input as ItemFn);

    if input_fn.sig.asyncness.is_none() {
        return syn::Error::new_spanned(
            input_fn.sig.fn_token,
            "#[workflow_step] requires an async function",
        )
        .to_compile_error()
        .into();
    }

    let fn_name = &input_fn.sig.ident;
    let fn_vis = &input_fn.vis;
    let fn_attrs = &input_fn.attrs;
    let fn_output = &input_fn.sig.output;
    let fn_block = &input_fn.block;
    let fn_inputs = &input_fn.sig.inputs;

    if let Err(err) = ensure_result_framework_error(fn_output) {
        return err.to_compile_error().into();
    }

    let mut arg_idents = Vec::new();
    for arg in fn_inputs.iter() {
        match arg {
            FnArg::Typed(pat_type) => match &*pat_type.pat {
                Pat::Ident(ident) => arg_idents.push(ident.ident.clone()),
                _ => {
                    return syn::Error::new_spanned(
                        &pat_type.pat,
                        "#[workflow_step] parameters must be simple identifiers",
                    )
                    .to_compile_error()
                    .into();
                }
            },
            FnArg::Receiver(_) => {
                return syn::Error::new_spanned(
                    arg,
                    "#[workflow_step] does not support methods with self",
                )
                .to_compile_error()
                .into();
            }
        }
    }

    let inner_name = format_ident!("__suprnova_workflow_step_inner_{}", fn_name);
    let input_json = build_input_json(&arg_idents);

    let expanded = quote! {
        #(#fn_attrs)*
        #fn_vis async fn #inner_name(#fn_inputs) #fn_output {
            #fn_block
        }

        #fn_vis async fn #fn_name(#fn_inputs) #fn_output {
            if let Some(ctx) = ::suprnova::workflow::WorkflowContext::current() {
                let __input_json = #input_json;
                // The step's identity is its module path and name, so two
                // steps with one name in two modules are two steps. A run
                // recorded under the bare name, before the module was part
                // of it, still replays under that name.
                ctx.run_named_step(
                    concat!(module_path!(), "::", stringify!(#fn_name)),
                    stringify!(#fn_name),
                    __input_json,
                    // `move`: the context takes a `'static` closure, and a
                    // borrowing one would hold a reference to every `Copy`
                    // argument instead of the value.
                    move || async move { #inner_name(#(#arg_idents),*).await },
                )
                .await
            } else {
                #inner_name(#(#arg_idents),*).await
            }
        }
    };

    TokenStream::from(expanded)
}

fn ensure_result_framework_error(output: &ReturnType) -> Result<(), syn::Error> {
    const EXPECTED: &str = "#[workflow_step] must return Result<T, FrameworkError>";

    let ReturnType::Type(_, ty) = output else {
        return Err(syn::Error::new(proc_macro2::Span::call_site(), EXPECTED));
    };
    let Type::Path(path) = &**ty else {
        return Err(syn::Error::new_spanned(ty, EXPECTED));
    };
    let last =
        path.path.segments.last().ok_or_else(|| {
            syn::Error::new_spanned(ty, "Invalid return type for #[workflow_step]")
        })?;
    if last.ident != "Result" {
        return Err(syn::Error::new_spanned(ty, EXPECTED));
    }
    let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
        return Err(syn::Error::new_spanned(ty, EXPECTED));
    };

    let mut iter = args.args.iter();
    iter.next()
        .ok_or_else(|| syn::Error::new_spanned(ty, "Result must have ok type"))?;
    let err = iter
        .next()
        .ok_or_else(|| syn::Error::new_spanned(ty, "Result must have error type"))?;
    let syn::GenericArgument::Type(err_ty) = err else {
        return Err(syn::Error::new_spanned(err, "Invalid error type"));
    };
    if !is_framework_error(err_ty) {
        return Err(syn::Error::new_spanned(err_ty, EXPECTED));
    }

    Ok(())
}

fn is_framework_error(ty: &Type) -> bool {
    if let Type::Path(path) = ty
        && let Some(last) = path.path.segments.last()
    {
        return last.ident == "FrameworkError";
    }
    false
}

fn build_input_json(arg_idents: &[syn::Ident]) -> TokenStream2 {
    if arg_idents.is_empty() {
        quote! {
            ::suprnova::serde_json::to_string(&())
                .map_err(|e| ::suprnova::FrameworkError::internal(format!("Workflow step serialize error: {}", e)))?
        }
    } else {
        quote! {
            ::suprnova::serde_json::to_string(&(#(&#arg_idents),*,))
                .map_err(|e| ::suprnova::FrameworkError::internal(format!("Workflow step serialize error: {}", e)))?
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ensure_result_framework_error;
    use syn::{ReturnType, parse_quote};

    const EXPECTED: &str = "#[workflow_step] must return Result<T, FrameworkError>";

    fn refusal(output: &ReturnType) -> String {
        ensure_result_framework_error(output)
            .expect_err("the return type is not the accepted shape")
            .to_string()
    }

    #[test]
    fn a_result_over_framework_error_is_accepted() {
        let plain: ReturnType = parse_quote!(-> Result<u32, FrameworkError>);
        let qualified: ReturnType =
            parse_quote!(-> std::result::Result<(), suprnova::FrameworkError>);

        assert!(ensure_result_framework_error(&plain).is_ok());
        assert!(ensure_result_framework_error(&qualified).is_ok());
    }

    #[test]
    fn every_other_shape_is_refused_with_the_accepted_shape_named() {
        let shapes: [ReturnType; 5] = [
            ReturnType::Default,
            parse_quote!(-> u32),
            parse_quote!(-> (u32, u32)),
            parse_quote!(-> Option<u32>),
            parse_quote!(-> Result<u32, String>),
        ];

        for shape in &shapes {
            assert_eq!(refusal(shape), EXPECTED);
        }
    }

    #[test]
    fn a_result_missing_its_error_type_says_so() {
        let one_argument: ReturnType = parse_quote!(-> Result<u32>);

        assert_eq!(refusal(&one_argument), "Result must have error type");
    }
}
