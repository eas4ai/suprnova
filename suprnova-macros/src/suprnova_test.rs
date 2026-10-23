//! `#[suprnova_test]` attribute macro for database-enabled tests
//!
//! This macro simplifies writing tests that need database access by automatically
//! setting up a migrated test database: an in-memory SQLite database by
//! default, or the database `DATABASE_URL` names with `refresh`, and a seeder
//! run before the body with `seed`.

use proc_macro::TokenStream;
use quote::quote;
use syn::{FnArg, ItemFn, Pat, Type, parse_macro_input};

/// The keys `#[suprnova_test(...)]` takes, for the unknown-key error.
const KEYS: &str = "migrator, refresh, seed";

/// Parse the macro attributes
struct SuprnovaTestArgs {
    migrator: Option<syn::Path>,
    /// `refresh`: run the test on the configured database, in a
    /// transaction, instead of a fresh in-memory one.
    refresh: bool,
    /// `seed` or `seed = Path`: the seeder to run before the body.
    seed: Option<SeedArg>,
}

/// The seeder the `seed` key names.
enum SeedArg {
    /// Bare `seed`: the root seeder, as a bare `db:seed` runs it.
    Root,
    /// `seed = Path`: that seeder.
    Named(syn::Path),
}

impl syn::parse::Parse for SuprnovaTestArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut migrator = None;
        let mut refresh = None;
        let mut seed = None;

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            // Domain 5 audit M-D5-5: unknown keys (e.g. typo
            // `migrtor = MyMigrator`) used to be silently ignored
            // because the `if ident == "migrator"` branch had no else -
            // the next iteration just parsed the next ident as if
            // nothing was wrong. Reject unknown keys with a span-
            // pointed compile error so typos surface immediately. A key
            // given twice is an error too, so neither value is dropped.
            let given_twice = || {
                syn::Error::new(
                    ident.span(),
                    format!("#[suprnova_test(...)] key `{ident}` is given more than once"),
                )
            };
            if ident == "migrator" {
                input.parse::<syn::Token![=]>()?;
                if migrator.replace(input.parse()?).is_some() {
                    return Err(given_twice());
                }
            } else if ident == "refresh" {
                if input.peek(syn::Token![=]) {
                    return Err(syn::Error::new(
                        ident.span(),
                        "#[suprnova_test(...)] key `refresh` takes no value - write `refresh`",
                    ));
                }
                if refresh.replace(true).is_some() {
                    return Err(given_twice());
                }
            } else if ident == "seed" {
                let value = if input.peek(syn::Token![=]) {
                    input.parse::<syn::Token![=]>()?;
                    SeedArg::Named(input.parse()?)
                } else {
                    SeedArg::Root
                };
                if seed.replace(value).is_some() {
                    return Err(given_twice());
                }
            } else {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("unknown #[suprnova_test(...)] key `{ident}` - supported keys: {KEYS}"),
                ));
            }

            if input.peek(syn::Token![,]) {
                input.parse::<syn::Token![,]>()?;
            }
        }

        Ok(Self {
            migrator,
            refresh: refresh.unwrap_or(false),
            seed,
        })
    }
}

/// Check if a type is `TestDatabase`
fn is_test_database_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
    {
        return segment.ident == "TestDatabase";
    }
    false
}

/// Find the parameter name for TestDatabase if it exists
fn find_db_param_name(func: &ItemFn) -> Option<syn::Ident> {
    for arg in &func.sig.inputs {
        if let FnArg::Typed(pat_type) = arg
            && is_test_database_type(&pat_type.ty)
            && let Pat::Ident(pat_ident) = &*pat_type.pat
        {
            return Some(pat_ident.ident.clone());
        }
    }
    None
}

pub fn suprnova_test_impl(attr: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as SuprnovaTestArgs);
    let input_fn = parse_macro_input!(input as ItemFn);
    expand(args, input_fn).into()
}

/// The test the attribute expands to: the bootstrap, the database the
/// keys ask for, the seeder, then the body.
fn expand(args: SuprnovaTestArgs, input_fn: ItemFn) -> proc_macro2::TokenStream {
    let fn_name = &input_fn.sig.ident;
    let fn_block = &input_fn.block;
    let fn_attrs: Vec<_> = input_fn
        .attrs
        .iter()
        .filter(|attr| !attr.path().is_ident("suprnova_test"))
        .collect();
    let fn_vis = &input_fn.vis;

    // Default to crate::migrations::Migrator if not specified
    let migrator_type = args
        .migrator
        .unwrap_or_else(|| syn::parse_quote!(crate::migrations::Migrator));

    // `refresh` runs on the database DATABASE_URL names, in a transaction
    // rolled back when the helper drops; the default is a fresh in-memory
    // database.
    let constructor = if args.refresh {
        quote!(refresh)
    } else {
        quote!(fresh)
    };

    let seed = match args.seed {
        None => quote!(),
        Some(SeedArg::Root) => quote! {
            let __suprnova_db = __suprnova_db
                .seed_root()
                .await
                .expect("Failed to seed the test database");
        },
        Some(SeedArg::Named(seeder)) => quote! {
            let __suprnova_db = __suprnova_db
                .seed::<#seeder>()
                .await
                .expect("Failed to seed the test database");
        },
    };

    // Bind the database to the function's `TestDatabase` parameter, or
    // keep it alive unbound so DB::connection() still resolves to it.
    let binding = find_db_param_name(&input_fn).unwrap_or_else(|| syn::parse_quote!(_db));

    quote! {
        #(#fn_attrs)*
        #[::tokio::test]
        #fn_vis async fn #fn_name() {
            // Bootstrap services so #[injectable] types are available
            ::suprnova::App::init();
            ::suprnova::App::boot_services()
                .expect("App::boot_services() failed in #[suprnova_test] setup");
            let __suprnova_db = ::suprnova::testing::TestDatabase::#constructor::<#migrator_type>()
                .await
                .expect("Failed to set up test database");
            #seed
            let #binding = __suprnova_db;
            #fn_block
        }
    }
}

#[cfg(test)]
mod tests {
    //! Domain 5 audit M-D5-5 regression: unknown
    //! `#[suprnova_test(...)]` keys must produce a compile error
    //! rather than being silently ignored. Previously the parser
    //! had `if ident == "migrator"` with no else branch, so a typo
    //! like `migrtor = MyMigrator` advanced past the `=` token to
    //! the next iteration with no signal anything went wrong.

    use super::*;
    use syn::parse2;

    #[test]
    fn known_key_parses_cleanly() {
        let tokens: proc_macro2::TokenStream = "migrator = crate::Migrator".parse().unwrap();
        let parsed: SuprnovaTestArgs = parse2(tokens).expect("known key must parse");
        assert!(parsed.migrator.is_some());
    }

    #[test]
    fn empty_attribute_parses_cleanly() {
        // `#[suprnova_test]` with no args is the common case - must
        // remain valid.
        let tokens: proc_macro2::TokenStream = "".parse().unwrap();
        let parsed: SuprnovaTestArgs = parse2(tokens).expect("empty attribute must parse");
        assert!(parsed.migrator.is_none());
    }

    #[test]
    fn unknown_key_is_rejected() {
        // Typo: `migrtor` is what the user wrote when they meant
        // `migrator`. Old behaviour silently kept the default
        // migrator; new behaviour rejects with a span-pointed error.
        let tokens: proc_macro2::TokenStream = "migrtor = crate::Migrator".parse().unwrap();
        let err = parse2::<SuprnovaTestArgs>(tokens)
            .err()
            .expect("unknown key must reject");
        let msg = err.to_string();
        assert!(
            msg.contains("unknown") && msg.contains("migrtor"),
            "error must name the bad key; got: {msg}"
        );
        assert!(
            msg.contains("migrator"),
            "error must hint at the supported key; got: {msg}"
        );
    }

    fn expand_str(attr: &str, item: &str) -> String {
        let args: SuprnovaTestArgs = parse2(attr.parse().unwrap()).expect("attribute parses");
        let func: ItemFn = syn::parse_str(item).expect("function parses");
        expand(args, func).to_string()
    }

    #[test]
    fn refresh_and_seed_keys_parse() {
        let parsed: SuprnovaTestArgs = parse2(
            "refresh, seed = crate::seeders::UsersSeeder, migrator = crate::M"
                .parse()
                .unwrap(),
        )
        .expect("refresh, seed and migrator parse together");
        assert!(parsed.refresh);
        assert!(matches!(parsed.seed, Some(SeedArg::Named(_))));
        assert!(parsed.migrator.is_some());

        let parsed: SuprnovaTestArgs = parse2("seed".parse().unwrap()).expect("bare seed parses");
        assert!(!parsed.refresh);
        assert!(matches!(parsed.seed, Some(SeedArg::Root)));
    }

    #[test]
    fn a_key_given_twice_is_rejected() {
        for attr in [
            "refresh, refresh",
            "seed, seed = crate::S",
            "migrator = a::M, migrator = b::M",
        ] {
            let err = parse2::<SuprnovaTestArgs>(attr.parse().unwrap())
                .err()
                .unwrap_or_else(|| panic!("`{attr}` must reject"));
            assert!(err.to_string().contains("more than once"), "got: {err}");
        }
    }

    #[test]
    fn refresh_takes_no_value() {
        let err = parse2::<SuprnovaTestArgs>("refresh = true".parse().unwrap())
            .err()
            .expect("`refresh = true` must reject");
        assert!(err.to_string().contains("refresh"), "got: {err}");
    }

    #[test]
    fn the_unknown_key_error_lists_every_key() {
        let err = parse2::<SuprnovaTestArgs>("refrsh".parse().unwrap())
            .err()
            .expect("an unknown key must reject");
        let msg = err.to_string();
        for key in ["migrator", "refresh", "seed"] {
            assert!(msg.contains(key), "the error lists `{key}`: {msg}");
        }
    }

    #[test]
    fn the_default_expansion_builds_a_fresh_database_and_seeds_nothing() {
        let out = expand_str("", "async fn t(db: TestDatabase) {}");
        assert!(
            out.contains("TestDatabase :: fresh :: < crate :: migrations :: Migrator >"),
            "{out}"
        );
        assert!(!out.contains("refresh"), "{out}");
        assert!(!out.contains("seed"), "{out}");
    }

    #[test]
    fn refresh_expands_to_the_configured_database_helper() {
        let out = expand_str(
            "refresh, migrator = crate::M",
            "async fn t(db: TestDatabase) {}",
        );
        assert!(
            out.contains("TestDatabase :: refresh :: < crate :: M >"),
            "{out}"
        );
        assert!(!out.contains(":: fresh ::"), "{out}");
    }

    #[test]
    fn seed_expands_to_the_named_or_the_root_seeder_before_the_body() {
        let out = expand_str(
            "seed = crate::UsersSeeder",
            "async fn t(db: TestDatabase) { body(); }",
        );
        let seeded = out
            .find(". seed :: < crate :: UsersSeeder > ()")
            .unwrap_or_else(|| panic!("the named seeder runs: {out}"));
        let body = out.find("body ()").expect("the body is kept");
        assert!(seeded < body, "the seeder runs before the body: {out}");

        let out = expand_str("seed", "async fn t() { body(); }");
        assert!(
            out.contains(". seed_root ()"),
            "the root seeder runs: {out}"
        );
    }
}
