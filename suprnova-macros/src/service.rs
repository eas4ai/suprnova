//! Service trait macro for the Suprnova framework
//!
//! Provides the `#[service]` attribute macro that:
//! 1. Adds `Send + Sync + 'static` bounds to trait definitions
//! 2. Optionally auto-registers a concrete implementation with the container
//! 3. Optionally registers implementations chosen by the environment
//! 4. Optionally generates a `fake()` method for testing

use proc_macro::TokenStream;
use quote::quote;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, ItemTrait, LitStr, Path, Token, bracketed, parenthesized, parse_macro_input};

/// One `bind(Concrete, env = ["pattern", ...])` entry: the type bound when
/// the environment matches one of the patterns.
struct EnvironmentBinding {
    concrete: Path,
    environments: Vec<LitStr>,
}

/// Parsed arguments from the service attribute
struct ServiceArgs {
    impl_type: Option<Path>,
    fake_type: Option<Path>,
    bindings: Vec<EnvironmentBinding>,
}

/// What the attribute accepts, for the errors that name a misuse.
const USAGE: &str = "expected `#[service]`, `#[service(Concrete)]`, or named entries: \
                     `impl = Concrete`, `fake = Fake`, `bind(Concrete, env = [\"pattern\", ...])`";

impl Parse for EnvironmentBinding {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let content;
        parenthesized!(content in input);
        let concrete: Path = content.parse()?;
        content.parse::<Token![,]>()?;
        let key: Ident = content.call(Ident::parse_any)?;
        if key != "env" {
            return Err(syn::Error::new(
                key.span(),
                "expected `env = [\"pattern\", ...]` after the concrete type in `bind(...)`",
            ));
        }
        content.parse::<Token![=]>()?;
        let list;
        bracketed!(list in content);
        let patterns = list.parse_terminated(|item| item.parse::<LitStr>(), Token![,])?;
        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        }
        if !content.is_empty() {
            return Err(content.error("unexpected token after `env = [...]` in `bind(...)`"));
        }
        let environments: Vec<LitStr> = patterns.into_iter().collect();
        if environments.is_empty() {
            return Err(syn::Error::new(
                concrete
                    .segments
                    .last()
                    .map_or_else(proc_macro2::Span::call_site, |segment| segment.ident.span()),
                "`bind(...)` needs at least one environment pattern in `env = [...]`",
            ));
        }
        if let Some(empty) = environments
            .iter()
            .find(|pattern| pattern.value().is_empty())
        {
            return Err(syn::Error::new(
                empty.span(),
                "an environment pattern cannot be empty; `\"*\"` matches every environment",
            ));
        }
        Ok(Self {
            concrete,
            environments,
        })
    }
}

impl Parse for ServiceArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut args = ServiceArgs {
            impl_type: None,
            fake_type: None,
            bindings: Vec::new(),
        };
        if input.is_empty() {
            return Ok(args);
        }

        // Named entries start with `impl =`, `fake =` or `bind(`; anything
        // else is the old positional form, whose one argument is the impl
        // type. `impl` is a keyword, so it is read with `parse_any`.
        let fork = input.fork();
        let is_named = match fork.call(Ident::parse_any) {
            Ok(name) => fork.peek(Token![=]) || (name == "bind" && fork.peek(syn::token::Paren)),
            Err(_) => false,
        };
        if !is_named {
            args.impl_type = Some(input.parse()?);
            if !input.is_empty() {
                return Err(input.error(USAGE));
            }
            return Ok(args);
        }

        while !input.is_empty() {
            let name: Ident = input.call(Ident::parse_any)?;
            match name.to_string().as_str() {
                "impl" => {
                    input.parse::<Token![=]>()?;
                    args.impl_type = Some(input.parse()?);
                }
                "fake" => {
                    input.parse::<Token![=]>()?;
                    args.fake_type = Some(input.parse()?);
                }
                "bind" => args.bindings.push(input.parse()?),
                _ => {
                    return Err(syn::Error::new(
                        name.span(),
                        format!("unknown parameter '{name}', expected 'impl', 'fake' or 'bind'"),
                    ));
                }
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else if !input.is_empty() {
                return Err(input.error(USAGE));
            }
        }
        Ok(args)
    }
}

/// Implements the `#[service]` attribute macro
///
/// This macro transforms a trait definition to add `Send + Sync + 'static` bounds,
/// making it suitable for use with the App container.
///
/// # Without arguments (just adds bounds)
///
/// ```rust,ignore
/// #[service]
/// pub trait HttpClient {
///     async fn get(&self, url: &str) -> Result<String, Error>;
/// }
/// ```
///
/// # With impl type (auto-registration, backwards compatible)
///
/// ```rust,ignore
/// #[service(RedisCache)]  // or #[service(impl = RedisCache)]
/// pub trait CacheStore {
///     fn get(&self, key: &str) -> Option<String>;
/// }
/// ```
///
/// # With implementations chosen by the environment
///
/// Each `bind(Concrete, env = [...])` entry names the environments it is
/// bound in, `*` matching any run of characters. At boot, the first entry
/// with a pattern that matches `Config::environment()` is bound, and `impl`
/// when none matches, as Laravel's `#[Bind]` attribute with `environments`
/// chooses a concrete type.
///
/// ```rust,ignore
/// #[service(impl = SmtpMailer, bind(LogMailer, env = ["local", "testing"]))]
/// pub trait Mailer {
///     fn send(&self, to: &str);
/// }
/// ```
///
/// # With fake type (generates fake() method for testing)
///
/// ```rust,ignore
/// #[service(impl = RealCache, fake = FakeCache)]
/// pub trait CacheStore {
///     fn get(&self, key: &str) -> Option<String>;
/// }
///
/// // In tests:
/// let _guard = <dyn CacheStore>::fake();  // Binds FakeCache, returns TestContainerGuard
/// ```
pub fn service_impl(attr: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as ServiceArgs);
    let mut item_trait = parse_macro_input!(input as ItemTrait);

    // Add Send + Sync + 'static to the trait's supertraits
    let send_bound: syn::TypeParamBound = syn::parse_quote!(Send);
    let sync_bound: syn::TypeParamBound = syn::parse_quote!(Sync);
    let static_bound: syn::TypeParamBound = syn::parse_quote!('static);

    // Check if bounds already exist to avoid duplicates
    let has_send = item_trait.supertraits.iter().any(|bound| {
        if let syn::TypeParamBound::Trait(trait_bound) = bound {
            trait_bound
                .path
                .segments
                .last()
                .map(|s| s.ident == "Send")
                .unwrap_or(false)
        } else {
            false
        }
    });

    let has_sync = item_trait.supertraits.iter().any(|bound| {
        if let syn::TypeParamBound::Trait(trait_bound) = bound {
            trait_bound
                .path
                .segments
                .last()
                .map(|s| s.ident == "Sync")
                .unwrap_or(false)
        } else {
            false
        }
    });

    let has_static = item_trait
        .supertraits
        .iter()
        .any(|bound| matches!(bound, syn::TypeParamBound::Lifetime(lt) if lt.ident == "static"));

    // Add missing bounds
    if !has_send {
        item_trait.supertraits.push(send_bound);
    }
    if !has_sync {
        item_trait.supertraits.push(sync_bound);
    }
    if !has_static {
        item_trait.supertraits.push(static_bound);
    }

    let trait_name = &item_trait.ident;
    let trait_name_str = trait_name.to_string();

    // Generate impl registration if impl_type is specified
    let bind_concrete = |concrete_type: &Path| {
        quote! {
            || -> ::std::result::Result<(), ::std::string::String> {
                ::suprnova::App::bind_if_absent::<dyn #trait_name>(
                    ::std::sync::Arc::new(<#concrete_type as ::std::default::Default>::default())
                );
                ::std::result::Result::Ok(())
            }
        }
    };
    let impl_registration = if args.bindings.is_empty() {
        args.impl_type.as_ref().map(|concrete_type| {
            let register = bind_concrete(concrete_type);
            quote! {
                // Auto-register this service binding at startup.
                //
                // `bind_if_absent` keeps boot idempotent: re-running the bootstrap
                // (e.g. on `Server::from_config` for the second time, or from a
                // test that already installed a fake before booting) leaves the
                // existing binding in place rather than replacing it with a fresh
                // `Default::default()` instance.
                //
                // The closure returns `Result<(), String>` so the bootstrap loop
                // can distinguish "registered" from "still waiting on a
                // dependency". Service bindings construct via `Default::default()`
                // and never touch the container, so they always return `Ok`.
                ::suprnova::inventory::submit! {
                    ::suprnova::container::provider::ServiceBindingEntry {
                        register: #register,
                        name: #trait_name_str,
                    }
                }
            }
        })
    } else {
        // The environment chooses among the `bind(...)` entries at boot, and
        // `impl` (when given) is bound when none matches. One entry carries
        // every choice, so exactly one binding is installed.
        let choices = args.bindings.iter().map(|binding| {
            let register = bind_concrete(&binding.concrete);
            let environments = &binding.environments;
            quote! {
                ::suprnova::container::provider::EnvironmentBinding {
                    environments: &[#(#environments),*],
                    register: #register,
                }
            }
        });
        let fallback = match args.impl_type.as_ref() {
            Some(concrete_type) => {
                let register = bind_concrete(concrete_type);
                quote! { ::std::option::Option::Some(#register) }
            }
            None => quote! { ::std::option::Option::None },
        };
        Some(quote! {
            ::suprnova::inventory::submit! {
                ::suprnova::container::provider::EnvironmentServiceBindingEntry {
                    name: #trait_name_str,
                    choices: &[#(#choices),*],
                    fallback: #fallback,
                }
            }
        })
    };

    // Generate fake() method if fake_type is specified
    let fake_impl = args.fake_type.as_ref().map(|fake_type| {
        quote! {
            impl dyn #trait_name {
                /// Create a test container with the fake implementation bound.
                ///
                /// Returns a guard that clears the test container when dropped.
                ///
                /// # Example
                /// ```rust,ignore
                /// #[test]
                /// fn test_something() {
                ///     let _guard = <dyn MyService>::fake();
                ///     // App::make::<dyn MyService>() now returns the fake
                /// }
                /// ```
                pub fn fake() -> ::suprnova::container::testing::TestContainerGuard {
                    let guard = ::suprnova::container::testing::TestContainer::fake();
                    ::suprnova::container::testing::TestContainer::bind::<dyn #trait_name>(
                        ::std::sync::Arc::new(<#fake_type as ::std::default::Default>::default())
                    );
                    guard
                }
            }
        }
    });

    let expanded = quote! {
        #item_trait
        #impl_registration
        #fake_impl
    };

    TokenStream::from(expanded)
}
