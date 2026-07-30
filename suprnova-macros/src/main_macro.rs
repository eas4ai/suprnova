//! `#[suprnova::main]` — the application entry point.
//!
//! Why this exists rather than `#[tokio::main]`: loading `.env` mutates
//! the process environment, and `std::env::set_var` is only sound while
//! the process is single-threaded. `#[tokio::main]` builds the runtime
//! *around* the whole body, so every worker thread already exists by the
//! time the first line of `main` runs — and any of them may call `getenv`
//! through DNS resolution, time formatting, or a C dependency. This macro
//! keeps the same `async fn main` ergonomics while moving the env load
//! ahead of the runtime, where the mutation is actually sound.

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, parse_macro_input};

/// Which Tokio runtime the generated `main` builds.
enum Flavor {
    MultiThread,
    CurrentThread,
}

struct MainArgs {
    flavor: Flavor,
    worker_threads: Option<syn::LitInt>,
    /// Span of the `flavor` key, for pointing the `worker_threads`
    /// conflict error at the thing that actually conflicts.
    flavor_span: Option<proc_macro2::Span>,
}

impl syn::parse::Parse for MainArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut flavor = Flavor::MultiThread;
        let mut flavor_span = None;
        let mut worker_threads: Option<syn::LitInt> = None;

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            if ident == "flavor" {
                input.parse::<syn::Token![=]>()?;
                let lit: syn::LitStr = input.parse()?;
                flavor = match lit.value().as_str() {
                    "multi_thread" => Flavor::MultiThread,
                    "current_thread" => Flavor::CurrentThread,
                    other => {
                        return Err(syn::Error::new(
                            lit.span(),
                            format!(
                                "unknown runtime flavor `{other}` — supported: \
                                 \"multi_thread\", \"current_thread\""
                            ),
                        ));
                    }
                };
                flavor_span = Some(ident.span());
            } else if ident == "worker_threads" {
                input.parse::<syn::Token![=]>()?;
                worker_threads = Some(input.parse()?);
            } else {
                return Err(syn::Error::new(
                    ident.span(),
                    format!(
                        "unknown #[suprnova::main(...)] key `{ident}` — supported keys: \
                         flavor, worker_threads"
                    ),
                ));
            }

            if input.peek(syn::Token![,]) {
                input.parse::<syn::Token![,]>()?;
            }
        }

        // Same rejection `#[tokio::main]` makes, for the same reason: a
        // current_thread runtime has no worker pool to size, so accepting
        // the key would silently ignore it.
        if matches!(flavor, Flavor::CurrentThread)
            && let Some(threads) = &worker_threads
        {
            let span = flavor_span.unwrap_or_else(|| threads.span());
            return Err(syn::Error::new(
                span,
                "`worker_threads` has no meaning with `flavor = \"current_thread\"` — \
                 a current-thread runtime has no worker pool",
            ));
        }

        Ok(Self {
            flavor,
            worker_threads,
            flavor_span,
        })
    }
}

pub fn main_impl(attr: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as MainArgs);
    let input_fn = parse_macro_input!(input as ItemFn);

    // The macro's whole purpose is to own runtime construction, so an
    // already-synchronous fn means the author expected something this
    // macro does not do.
    if input_fn.sig.asyncness.is_none() {
        return syn::Error::new_spanned(
            input_fn.sig.fn_token,
            "#[suprnova::main] expects an `async fn` — it builds the Tokio runtime for you",
        )
        .to_compile_error()
        .into();
    }

    let attrs = &input_fn.attrs;
    let vis = &input_fn.vis;
    let name = &input_fn.sig.ident;
    let output = &input_fn.sig.output;
    let block = &input_fn.block;

    let builder = match args.flavor {
        Flavor::MultiThread => quote! {
            ::suprnova::tokio::runtime::Builder::new_multi_thread()
        },
        Flavor::CurrentThread => quote! {
            ::suprnova::tokio::runtime::Builder::new_current_thread()
        },
    };

    let worker_threads = match &args.worker_threads {
        Some(n) => quote! { .worker_threads(#n) },
        None => quote! {},
    };

    // `load_env` runs before the builder line on purpose, and the
    // ordering is the entire point of this macro — see the module doc.
    let expanded = quote! {
        #(#attrs)*
        #vis fn #name() #output {
            ::suprnova::boot::load_env_or_exit();

            let __suprnova_runtime = #builder
                #worker_threads
                .enable_all()
                .build()
                .expect("failed to build the Tokio runtime");

            __suprnova_runtime.block_on(async move #block)
        }
    };

    // Silence the unused-field warning when no conflict was reported.
    let _ = args.flavor_span;

    expanded.into()
}

#[cfg(test)]
mod tests {
    //! The parser is the part with branches; the expansion is a
    //! straight-line `quote!`. These pin the argument surface.

    use super::*;
    use syn::parse2;

    #[test]
    fn no_arguments_defaults_to_multi_thread() {
        let tokens: proc_macro2::TokenStream = "".parse().unwrap();
        let parsed: MainArgs = parse2(tokens).expect("empty attribute must parse");
        assert!(matches!(parsed.flavor, Flavor::MultiThread));
        assert!(parsed.worker_threads.is_none());
    }

    #[test]
    fn current_thread_flavor_parses() {
        let tokens: proc_macro2::TokenStream = r#"flavor = "current_thread""#.parse().unwrap();
        let parsed: MainArgs = parse2(tokens).expect("current_thread must parse");
        assert!(matches!(parsed.flavor, Flavor::CurrentThread));
    }

    #[test]
    fn worker_threads_parses_with_the_default_flavor() {
        let tokens: proc_macro2::TokenStream = "worker_threads = 4".parse().unwrap();
        let parsed: MainArgs = parse2(tokens).expect("worker_threads must parse");
        assert_eq!(
            parsed
                .worker_threads
                .expect("present")
                .base10_parse::<usize>()
                .expect("numeric"),
            4
        );
    }

    /// A typo in the flavor string must not silently downgrade the
    /// runtime — `flavor = "current-thread"` (hyphen) would otherwise
    /// be indistinguishable from the default at runtime.
    #[test]
    fn an_unknown_flavor_is_rejected() {
        let tokens: proc_macro2::TokenStream = r#"flavor = "current-thread""#.parse().unwrap();
        let err = parse2::<MainArgs>(tokens)
            .err()
            .expect("unknown flavor must reject");
        let msg = err.to_string();
        assert!(
            msg.contains("current-thread") && msg.contains("current_thread"),
            "error must show both what was written and what was meant; got: {msg}"
        );
    }

    #[test]
    fn an_unknown_key_is_rejected() {
        let tokens: proc_macro2::TokenStream = "flavour = \"multi_thread\"".parse().unwrap();
        let err = parse2::<MainArgs>(tokens)
            .err()
            .expect("unknown key must reject");
        let msg = err.to_string();
        assert!(
            msg.contains("flavour") && msg.contains("flavor"),
            "error must name the bad key and hint the right one; got: {msg}"
        );
    }

    /// Accepting this would size a pool that does not exist.
    #[test]
    fn worker_threads_conflicts_with_current_thread() {
        let tokens: proc_macro2::TokenStream =
            r#"flavor = "current_thread", worker_threads = 4"#.parse().unwrap();
        let err = parse2::<MainArgs>(tokens)
            .err()
            .expect("the combination must reject");
        assert!(
            err.to_string().contains("worker_threads"),
            "error must name the offending key; got: {err}"
        );
    }
}
