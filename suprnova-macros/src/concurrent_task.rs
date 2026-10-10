//! `#[concurrent_task]` - register an async fn as a task the
//! `Concurrency` facade runs on any of its drivers.
//!
//! Applied to `async fn name(input: I) -> Result<O, FrameworkError>`, or to
//! one with no parameter, whose input is `()`, the macro keeps the function
//! as it is and adds:
//!
//!   1. a unit struct named after the function in UpperCamelCase
//!      (`count_users` gives `CountUsers`), the type a caller names in
//!      `Task::new::<CountUsers>(input)`, so the compiler checks the input;
//!   2. an `impl ConcurrentTask` for the struct with the input and output
//!      types, the registered name, and a call into the function;
//!   3. an `inventory::submit!` of the task's registry entry, so the
//!      `concurrency:run` console command finds the task by name in
//!      another process.
//!
//! Attributes:
//!
//!   - `name = "reports:count"` replaces the registered name, which is
//!     otherwise the function's module path and name. Both binaries of the
//!     process driver must register the task under one name.
//!   - `type_name = "CountReports"` replaces the struct's name, for a
//!     function whose UpperCamelCase name is taken, such as by its input.
//!
//! Modelled on `#[command]` (`command.rs`): an attribute on an async fn that
//! submits a registry entry through `inventory`.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::ext::IdentExt;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{Expr, FnArg, Ident, ItemFn, Lit, LitStr, Meta, ReturnType, Token, Type, parse_quote};

/// The shape every error about the return type names.
const EXPECTED_RETURN: &str = "#[concurrent_task] must return Result<T, FrameworkError>";

pub fn concurrent_task_impl(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    match expand(attr.into(), item.into()) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[derive(Default)]
struct Attrs {
    name: Option<LitStr>,
    type_name: Option<Ident>,
}

fn parse_attrs(attr: TokenStream) -> syn::Result<Attrs> {
    let mut out = Attrs::default();
    if attr.is_empty() {
        return Ok(out);
    }
    let metas = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(attr)?;
    for meta in metas {
        let Meta::NameValue(pair) = meta else {
            return Err(syn::Error::new_spanned(
                meta,
                "#[concurrent_task(...)] takes `name = \"...\"` and `type_name = \"...\"`",
            ));
        };
        let key = pair
            .path
            .get_ident()
            .map(ToString::to_string)
            .unwrap_or_default();
        let Expr::Lit(literal) = &pair.value else {
            return Err(syn::Error::new_spanned(
                &pair.value,
                format!("`{key}` in #[concurrent_task(...)] takes a string literal"),
            ));
        };
        let Lit::Str(text) = &literal.lit else {
            return Err(syn::Error::new_spanned(
                &literal.lit,
                format!("`{key}` in #[concurrent_task(...)] takes a string literal"),
            ));
        };
        match key.as_str() {
            "name" => {
                if text.value().is_empty() {
                    return Err(syn::Error::new_spanned(
                        text,
                        "the `name` of a concurrent task cannot be empty",
                    ));
                }
                out.name = Some(text.clone());
            }
            "type_name" => out.type_name = Some(text.parse::<Ident>()?),
            _ => {
                return Err(syn::Error::new_spanned(
                    &pair.path,
                    format!(
                        "unknown key `{key}` in #[concurrent_task(...)]; the keys are name and \
                         type_name"
                    ),
                ));
            }
        }
    }
    Ok(out)
}

/// The expansion, apart from the proc-macro types so it can be tested.
pub(crate) fn expand(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let attrs = parse_attrs(attr)?;
    let function: ItemFn = syn::parse2(item)?;
    let signature = &function.sig;

    if signature.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            signature.fn_token,
            "#[concurrent_task] requires an async fn",
        ));
    }
    if !signature.generics.params.is_empty() || signature.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            &signature.generics,
            "#[concurrent_task] cannot be generic: a task is registered under one name, for \
             one input and one output type",
        ));
    }
    if let Some(variadic) = &signature.variadic {
        return Err(syn::Error::new_spanned(
            variadic,
            "#[concurrent_task] cannot take variadic arguments",
        ));
    }
    let input_type: Option<Type> = match signature.inputs.len() {
        0 => None,
        1 => match signature.inputs.first() {
            Some(FnArg::Typed(parameter)) => Some((*parameter.ty).clone()),
            Some(receiver @ FnArg::Receiver(_)) => {
                return Err(syn::Error::new_spanned(
                    receiver,
                    "#[concurrent_task] goes on a free function, not a method",
                ));
            }
            None => None,
        },
        _ => {
            return Err(syn::Error::new_spanned(
                &signature.inputs,
                "#[concurrent_task] takes one input parameter; put several values in one \
                 serde type",
            ));
        }
    };
    let output_type = ok_type(&signature.output, signature.ident.span())?;

    let function_name = &signature.ident;
    let plain_name = function_name.unraw().to_string();
    let struct_name = match attrs.type_name {
        Some(name) => name,
        None => {
            let camel = upper_camel(&plain_name);
            if camel.is_empty() {
                return Err(syn::Error::new_spanned(
                    function_name,
                    "#[concurrent_task] cannot name a type after this function; give one \
                     with type_name = \"...\"",
                ));
            }
            format_ident!("{}", camel, span = function_name.span())
        }
    };
    let registered = match &attrs.name {
        Some(name) => quote!(#name),
        None => quote!(::std::concat!(::std::module_path!(), "::", #plain_name)),
    };
    let visibility = &function.vis;
    let (input_type, input_binding, call) = match input_type {
        Some(input_type) => (input_type, quote!(input), quote!(#function_name(input))),
        None => (parse_quote!(()), quote!(_input), quote!(#function_name())),
    };
    let doc = format!(
        "The concurrent task `{plain_name}`: `Task::new::<{struct_name}>(input)` runs it on a \
         `Concurrency` driver."
    );

    Ok(quote! {
        #function

        #[doc = #doc]
        #[derive(Debug, Clone, Copy)]
        #visibility struct #struct_name;

        impl ::suprnova::concurrency::ConcurrentTask for #struct_name {
            type Input = #input_type;
            type Output = #output_type;
            const NAME: &'static str = #registered;

            fn call(
                #input_binding: Self::Input,
            ) -> ::suprnova::concurrency::TaskFuture<Self::Output> {
                ::std::boxed::Box::pin(#call)
            }
        }

        ::suprnova::inventory::submit! {
            ::suprnova::concurrency::ConcurrentTaskEntry::of::<#struct_name>()
        }
    })
}

/// The `T` of `Result<T, FrameworkError>`.
fn ok_type(output: &ReturnType, span: Span) -> syn::Result<Type> {
    let ReturnType::Type(_, returned) = output else {
        return Err(syn::Error::new(span, EXPECTED_RETURN));
    };
    let Type::Path(path) = &**returned else {
        return Err(syn::Error::new_spanned(returned, EXPECTED_RETURN));
    };
    let Some(last) = path.path.segments.last() else {
        return Err(syn::Error::new_spanned(returned, EXPECTED_RETURN));
    };
    if last.ident != "Result" {
        return Err(syn::Error::new_spanned(returned, EXPECTED_RETURN));
    }
    let syn::PathArguments::AngleBracketed(arguments) = &last.arguments else {
        return Err(syn::Error::new_spanned(returned, EXPECTED_RETURN));
    };
    let mut arguments = arguments.args.iter();
    let (Some(syn::GenericArgument::Type(ok)), Some(syn::GenericArgument::Type(error)), None) =
        (arguments.next(), arguments.next(), arguments.next())
    else {
        return Err(syn::Error::new_spanned(returned, EXPECTED_RETURN));
    };
    let is_framework_error = matches!(
        error,
        Type::Path(error) if error
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "FrameworkError")
    );
    if !is_framework_error {
        return Err(syn::Error::new_spanned(error, EXPECTED_RETURN));
    }
    Ok(ok.clone())
}

/// `count_users` as `CountUsers`.
fn upper_camel(snake: &str) -> String {
    snake
        .split('_')
        .flat_map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{expand, upper_camel};
    use quote::quote;

    fn expanded(attr: proc_macro2::TokenStream, item: proc_macro2::TokenStream) -> String {
        expand(attr, item)
            .expect("the function is a concurrent task")
            .to_string()
    }

    fn refusal(attr: proc_macro2::TokenStream, item: proc_macro2::TokenStream) -> String {
        expand(attr, item)
            .expect_err("the function is not a concurrent task")
            .to_string()
    }

    #[test]
    fn the_type_is_the_function_name_in_upper_camel_case() {
        assert_eq!(upper_camel("count_users"), "CountUsers");
        assert_eq!(upper_camel("_private_task"), "PrivateTask");
        assert_eq!(upper_camel("fetch__v2"), "FetchV2");
        assert_eq!(upper_camel("already"), "Already");
        assert_eq!(upper_camel("_"), "");
    }

    #[test]
    fn a_task_with_one_input_registers_its_struct_types_and_name() {
        let text = expanded(
            quote!(),
            quote! {
                pub async fn count_rows(table: String) -> Result<u64, FrameworkError> { Ok(0) }
            },
        );
        assert!(text.contains("pub struct CountRows"), "{text}");
        assert!(text.contains("type Input = String"), "{text}");
        assert!(text.contains("type Output = u64"), "{text}");
        assert!(text.contains("module_path"), "{text}");
        assert!(text.contains("count_rows (input)"), "{text}");
        assert!(
            text.contains("ConcurrentTaskEntry :: of :: < CountRows >"),
            "{text}"
        );
    }

    #[test]
    fn a_task_without_a_parameter_takes_unit() {
        let text = expanded(
            quote!(),
            quote! { async fn answer() -> Result<u64, FrameworkError> { Ok(42) } },
        );
        assert!(text.contains("type Input = ()"), "{text}");
        assert!(text.contains("answer ()"), "{text}");
        assert!(text.contains("struct Answer"), "{text}");
    }

    #[test]
    fn name_and_type_name_replace_the_defaults() {
        let text = expanded(
            quote!(name = "reports:count", type_name = "CountReports"),
            quote! { async fn count(input: u8) -> Result<u8, suprnova::FrameworkError> { Ok(input) } },
        );
        assert!(text.contains("\"reports:count\""), "{text}");
        assert!(!text.contains("module_path"), "{text}");
        assert!(text.contains("struct CountReports"), "{text}");
    }

    #[test]
    fn every_other_shape_is_refused_with_the_reason() {
        let cases = [
            (
                quote!(),
                quote! { fn plain(input: u8) -> Result<u8, FrameworkError> { Ok(input) } },
                "requires an async fn",
            ),
            (
                quote!(),
                quote! { async fn generic<T>(input: T) -> Result<u8, FrameworkError> { Ok(0) } },
                "cannot be generic",
            ),
            (
                quote!(),
                quote! { async fn two(a: u8, b: u8) -> Result<u8, FrameworkError> { Ok(a) } },
                "takes one input parameter",
            ),
            (
                quote!(),
                quote! { async fn method(&self) -> Result<u8, FrameworkError> { Ok(0) } },
                "not a method",
            ),
            (
                quote!(),
                quote! { async fn bare(input: u8) -> u8 { input } },
                "must return Result<T, FrameworkError>",
            ),
            (
                quote!(),
                quote! { async fn other(input: u8) -> Result<u8, String> { Ok(input) } },
                "must return Result<T, FrameworkError>",
            ),
            (
                quote!(queue = "x"),
                quote! { async fn keyed(input: u8) -> Result<u8, FrameworkError> { Ok(input) } },
                "unknown key `queue`",
            ),
            (
                quote!(name = ""),
                quote! { async fn empty(input: u8) -> Result<u8, FrameworkError> { Ok(input) } },
                "cannot be empty",
            ),
        ];
        for (attr, item, reason) in cases {
            let text = refusal(attr, item);
            assert!(text.contains(reason), "expected `{reason}`, got `{text}`");
        }
    }
}
