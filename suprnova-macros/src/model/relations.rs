//! Phase 10B T1 - relation emission for `#[suprnova::model]`.
//!
//! Reads the `relations = { ... }` block parsed by [`super::parse`] and
//! emits, per model:
//!
//! 1. Two auto-injected struct fields (`__eager: EagerLoadCache`,
//!    `__pivot: Option<Arc<dyn Any + Send + Sync>>`). The rewrite
//!    happens in [`super::expand`] before the struct definition is
//!    emitted; this module only emits the impl blocks that need
//!    `__eager` / `__pivot` to exist.
//! 2. Four dispatcher methods on the user struct (`__eager_load`,
//!    `__recurse_eager_load`, `__count_relation`,
//!    `__aggregate_relation`). T1 emits the skeletons with empty
//!    matches - T2-T7 add arms per concrete relation type.
//! 3. The `pivot::<P>()` accessor for reading per-row pivot context
//!    set by `BelongsToMany` loaders.
//! 4. Per declared relation: `<rel>_loaded()` / `<rel>_count()`
//!    accessors that read from `__eager`, plus a
//!    `RelationEntry` inventory submission for Phase 8 enumeration.
//!
//! T2-T7 will add: a concrete `Relation` impl per relation type, the
//! per-kind relation methods (`fn posts(&self) -> HasMany<Self, Post>`),
//! and the per-kind dispatcher arms inside the four methods above.

use proc_macro2::TokenStream;
use quote::quote;
use syn::Result;

use super::parse::{ModelInput, RelationDecl, RelationKindAttr, RelationOpt, to_snake};

/// Top-level entry point. Emits every relation-related artifact for
/// the model (dispatchers + accessors + inventory submissions + the
/// per-kind relation methods).
pub fn emit(input: &ModelInput) -> Result<TokenStream> {
    let struct_ident = &input.item.ident;
    let dispatchers = emit_dispatchers(input)?;
    let pivot_accessor = emit_pivot_accessor(struct_ident);
    let with_helper = emit_with_helper(struct_ident);
    let dispatch_impl = emit_dispatch_impl(struct_ident);

    // Build per-relation accessors + relation methods + inventory
    // submissions. Each lives in its own `impl Self { ... }` block -
    // a subsequent `cargo expand` clearly shows which methods came
    // from which relation declarations.
    let mut relation_methods: Vec<TokenStream> = Vec::new();
    let mut relation_accessors: Vec<TokenStream> = Vec::new();
    let mut relation_inventory: Vec<TokenStream> = Vec::new();
    for rel in input.relations.as_deref().unwrap_or(&[]) {
        relation_methods.push(emit_relation_method(input, rel)?);
        relation_accessors.push(emit_relation_accessors(struct_ident, rel));
        relation_inventory.push(emit_relation_inventory(struct_ident, input, rel));
    }

    Ok(quote! {
        #dispatchers
        #pivot_accessor
        #with_helper
        #dispatch_impl
        #( #relation_methods )*
        #( #relation_accessors )*
        #( #relation_inventory )*
    })
}

/// Emit the `EagerLoadDispatch` impl that lets `Builder<M>::get` call
/// `M::eager_load(...)` without needing inherent-method access. Each
/// method delegates straight to the matching inherent dispatcher
/// (`__eager_load`, `__count_relation`, `__aggregate_relation`,
/// `__recurse_eager_load`).
///
/// Also emits the `Sealed` supertrait impl. `EagerLoadDispatch` is
/// language-sealed in the framework via a `__sealed::Sealed`
/// supertrait - user code can't write `impl EagerLoadDispatch for X`
/// because it can't write `impl Sealed for X` (the trait is reachable
/// only through the doc-hidden `__private_eloquent` path; reaching it
/// is the explicit "I know what I'm doing" gesture).
fn emit_dispatch_impl(struct_ident: &syn::Ident) -> TokenStream {
    quote! {
        impl ::suprnova::__private_eloquent::Sealed for #struct_ident {}

        impl ::suprnova::EagerLoadDispatch for #struct_ident {
            fn eager_load<'a>(
                relation: &'a str,
                parents: &'a mut [&'a mut Self],
                db: &'a ::suprnova::sea_orm::DatabaseConnection,
                predicate: ::core::option::Option<
                    ::std::sync::Arc<dyn ::std::any::Any + ::core::marker::Send + ::core::marker::Sync>,
                >,
            ) -> ::core::pin::Pin<
                ::std::boxed::Box<
                    dyn ::core::future::Future<
                            Output = ::core::result::Result<(), ::suprnova::FrameworkError>,
                        > + ::core::marker::Send + 'a,
                >,
            > {
                ::std::boxed::Box::pin(Self::__eager_load(relation, parents, db, predicate))
            }

            fn count_relation<'a>(
                relation: &'a str,
                parents: &'a mut [&'a mut Self],
                db: &'a ::suprnova::sea_orm::DatabaseConnection,
            ) -> ::core::pin::Pin<
                ::std::boxed::Box<
                    dyn ::core::future::Future<
                            Output = ::core::result::Result<(), ::suprnova::FrameworkError>,
                        > + ::core::marker::Send + 'a,
                >,
            > {
                ::std::boxed::Box::pin(Self::__count_relation(relation, parents, db))
            }

            fn aggregate_relation<'a>(
                relation: &'a str,
                column: &'a str,
                kind: ::suprnova::AggregateKind,
                parents: &'a mut [&'a mut Self],
                db: &'a ::suprnova::sea_orm::DatabaseConnection,
            ) -> ::core::pin::Pin<
                ::std::boxed::Box<
                    dyn ::core::future::Future<
                            Output = ::core::result::Result<(), ::suprnova::FrameworkError>,
                        > + ::core::marker::Send + 'a,
                >,
            > {
                ::std::boxed::Box::pin(Self::__aggregate_relation(relation, column, kind, parents, db))
            }

            fn recurse_eager_load<'a>(
                &'a mut self,
                relation: &'a str,
                rest: &'a str,
                db: &'a ::suprnova::sea_orm::DatabaseConnection,
                missing_only: bool,
            ) -> ::core::pin::Pin<
                ::std::boxed::Box<
                    dyn ::core::future::Future<
                            Output = ::core::result::Result<(), ::suprnova::FrameworkError>,
                        > + ::core::marker::Send + 'a,
                >,
            > {
                ::std::boxed::Box::pin(self.__recurse_eager_load(relation, rest, db, missing_only))
            }

            fn recurse_eager_load_batched<'a>(
                parents: &'a mut [&'a mut Self],
                relation: &'a str,
                rest: &'a str,
                db: &'a ::suprnova::sea_orm::DatabaseConnection,
                missing_only: bool,
            ) -> ::core::pin::Pin<
                ::std::boxed::Box<
                    dyn ::core::future::Future<
                            Output = ::core::result::Result<(), ::suprnova::FrameworkError>,
                        > + ::core::marker::Send + 'a,
                >,
            > {
                ::std::boxed::Box::pin(Self::__recurse_eager_load_batched(parents, relation, rest, db, missing_only))
            }

            fn set_pivot_arc(
                &mut self,
                pivot: ::core::option::Option<
                    ::std::sync::Arc<dyn ::std::any::Any + ::core::marker::Send + ::core::marker::Sync>,
                >,
            ) {
                self.__pivot = pivot;
            }

            fn has_eager(&self, name: &str) -> bool {
                self.__eager.has(name)
            }
        }
    }
}

/// Emit the four dispatcher methods + per-relation match arms.
///
/// T1 shipped the skeletons (no-relation error path only); T2 adds
/// the `HasOne` and `BelongsTo` arms. T3-T7 will keep extending the
/// per-relation arm lists as more relation kinds land. The
/// `predicate` parameter on `__eager_load` carries the user's
/// optional `with_where` closure type-erased - concrete arms downcast
/// it before applying (T9 wires the closure plumbing; T2 only fills
/// the `HasOne` / `BelongsTo` arms which ignore the predicate for
/// now).
fn emit_dispatchers(input: &ModelInput) -> Result<TokenStream> {
    let struct_ident = &input.item.ident;

    // Collect per-kind match arms for the four dispatchers.
    let mut eager_arms: Vec<TokenStream> = Vec::new();
    let mut count_arms: Vec<TokenStream> = Vec::new();
    let mut aggregate_arms: Vec<TokenStream> = Vec::new();
    let mut recurse_arms: Vec<TokenStream> = Vec::new();
    let mut recurse_batched_arms: Vec<TokenStream> = Vec::new();
    for rel in input.relations.as_deref().unwrap_or(&[]) {
        if let Some(arm) = emit_eager_arm(input, rel)? {
            eager_arms.push(arm);
        }
        if let Some(arm) = emit_count_arm(input, rel)? {
            count_arms.push(arm);
        }
        if let Some(arm) = emit_aggregate_arm(input, rel)? {
            aggregate_arms.push(arm);
        }
        if let Some(arm) = emit_recurse_arm(input, rel)? {
            recurse_arms.push(arm);
        }
        if let Some(arm) = emit_recurse_batched_arm(input, rel)? {
            recurse_batched_arms.push(arm);
        }
    }

    Ok(quote! {
        impl #struct_ident {
            /// Eager-load a relation by name. Called by `Builder::with`
            /// (T9) and `Collection::load_missing` (T9) to populate
            /// the per-row `__eager` cache. T1 emits the no-relation
            /// arm only; relation tasks (T2-T7) extend the match.
            ///
            /// The `predicate` carries a type-erased `with_where`
            /// closure - concrete arms downcast to the relation's
            /// `Box<dyn FnOnce(Builder<R>) -> Builder<R>>` and apply
            /// before issuing the IN query. T1 ignores it.
            #[doc(hidden)]
            pub async fn __eager_load(
                relation: &str,
                parents: &mut [&mut Self],
                db: &::suprnova::sea_orm::DatabaseConnection,
                predicate: ::core::option::Option<
                    ::std::sync::Arc<dyn ::std::any::Any + ::core::marker::Send + ::core::marker::Sync>,
                >,
            ) -> ::core::result::Result<(), ::suprnova::FrameworkError> {
                // `predicate` is consumed by the matching arm via
                // downcast to the relation's typed
                // `Box<dyn FnOnce(Builder<R>) -> Builder<R>>`. Each
                // arm declares its own `mut predicate` shadow so the
                // ones that don't use it don't warn.
                let _ = db;
                let mut predicate = predicate;
                match relation {
                    #(#eager_arms)*
                    other => ::core::result::Result::Err(
                        ::suprnova::FrameworkError::internal(::std::format!(
                            "model `{}` has no relation `{}`",
                            ::std::any::type_name::<Self>(),
                            other,
                        )),
                    ),
                }
            }

            /// Recurse into an already-loaded relation to load its own
            /// relations. Used by T9's nested-path eager loader
            /// (`with(["posts.comments"])`). T1 emits the skeleton;
            /// T2-T7 add arms; T9's orchestrator calls this after
            /// `__eager_load` for the head segment of a dotted path.
            ///
            /// `missing_only`: when `true`, per-relation arms skip the
            /// bulk eager-load step for the next path segment if any
            /// cached child already has it. Used by
            /// `Collection::load_missing` to fill only the missing
            /// tail of a dotted path whose head is already cached.
            #[doc(hidden)]
            pub async fn __recurse_eager_load(
                &mut self,
                relation: &str,
                rest: &str,
                db: &::suprnova::sea_orm::DatabaseConnection,
                missing_only: bool,
            ) -> ::core::result::Result<(), ::suprnova::FrameworkError> {
                let _ = (rest, db, missing_only);
                match relation {
                    #(#recurse_arms)*
                    other => ::core::result::Result::Err(
                        ::suprnova::FrameworkError::internal(::std::format!(
                            "model `{}` has no relation `{}` to recurse into",
                            ::std::any::type_name::<Self>(),
                            other,
                        )),
                    ),
                }
            }

            /// Batched sibling of [`Self::__recurse_eager_load`]: recurse
            /// the next path segment across EVERY parent at once. Gathers
            /// all parents' cached children of `relation` into one slice,
            /// issues a single IN query for the next segment, then recurses -
            /// so a dotted path stays a constant number of queries
            /// instead of N+1 per nested level. Used by the eager-load
            /// orchestrator for both `with(...)` and `load_missing(...)`.
            #[doc(hidden)]
            pub async fn __recurse_eager_load_batched(
                parents: &mut [&mut Self],
                relation: &str,
                rest: &str,
                db: &::suprnova::sea_orm::DatabaseConnection,
                missing_only: bool,
            ) -> ::core::result::Result<(), ::suprnova::FrameworkError> {
                let _ = (rest, db, missing_only);
                match relation {
                    #(#recurse_batched_arms)*
                    other => ::core::result::Result::Err(
                        ::suprnova::FrameworkError::internal(::std::format!(
                            "model `{}` has no relation `{}` to recurse into",
                            ::std::any::type_name::<Self>(),
                            other,
                        )),
                    ),
                }
            }

            /// Count rows for a relation (`with_count(["posts"])`).
            /// T1 emits the skeleton; T2-T7 add per-relation arms
            /// running GROUP BY queries.
            #[doc(hidden)]
            pub async fn __count_relation(
                relation: &str,
                parents: &mut [&mut Self],
                db: &::suprnova::sea_orm::DatabaseConnection,
            ) -> ::core::result::Result<(), ::suprnova::FrameworkError> {
                let _ = db;
                match relation {
                    #(#count_arms)*
                    other => ::core::result::Result::Err(
                        ::suprnova::FrameworkError::internal(::std::format!(
                            "model `{}` has no relation `{}` for with_count",
                            ::std::any::type_name::<Self>(),
                            other,
                        )),
                    ),
                }
            }

            /// Aggregate (SUM/AVG/MIN/MAX) over a relation column.
            /// Called by `with_sum(("posts", "views"))` and friends.
            /// T1 emits skeleton; T2-T7 add arms for the kinds that
            /// have a target column (HasMany / BelongsToMany /
            /// Through / Morph many-to-* - NOT HasOne / BelongsTo).
            #[doc(hidden)]
            pub async fn __aggregate_relation(
                relation: &str,
                column: &str,
                kind: ::suprnova::AggregateKind,
                parents: &mut [&mut Self],
                db: &::suprnova::sea_orm::DatabaseConnection,
            ) -> ::core::result::Result<(), ::suprnova::FrameworkError> {
                let _ = (column, kind, db);
                match relation {
                    #(#aggregate_arms)*
                    other => ::core::result::Result::Err(
                        ::suprnova::FrameworkError::internal(::std::format!(
                            "model `{}` has no relation `{}` for aggregate",
                            ::std::any::type_name::<Self>(),
                            other,
                        )),
                    ),
                }
            }
        }
    })
}

/// Emit the `pivot::<P>()` accessor. T4 (BelongsToMany) fills
/// `__pivot` on each row at load time; this accessor reads it back.
/// Panics when the row has no pivot context, matching the spec's
/// explicit "clear error message" requirement.
///
/// The accessor distinguishes the two failure modes:
///
/// - `__pivot` is `None` → the row was fetched without a m2m loader
///   (typically via `find()` instead of `BelongsToMany::get()`). The
///   panic message tells the caller to load through the m2m path.
/// - `__pivot` is `Some(_)` but the downcast to `P` fails → the data
///   is there but the caller asked for the wrong pivot type. The panic
///   message names the actual struct and the requested type so the
///   typo is obvious.
fn emit_pivot_accessor(struct_ident: &syn::Ident) -> TokenStream {
    quote! {
        impl #struct_ident {
            /// Read pivot context attached by a `BelongsToMany` load.
            ///
            /// Panics with one of two distinct messages depending on
            /// the failure mode:
            ///
            /// - If `__pivot` is empty, the row wasn't loaded through
            ///   the m2m path - call `BelongsToMany::get()` instead of
            ///   `find()`.
            /// - If `__pivot` is populated but the requested `P` type
            ///   doesn't match what was stored, the call site passed
            ///   the wrong pivot type - fix the turbofish.
            pub fn pivot<P: ::std::any::Any + ::core::marker::Send + ::core::marker::Sync>(&self) -> &P {
                match self.__pivot.as_ref() {
                    ::core::option::Option::None => ::std::panic!(
                        "`{}` row has no pivot context; load via `BelongsToMany::get()`",
                        ::std::any::type_name::<Self>(),
                    ),
                    ::core::option::Option::Some(arc) => match arc.downcast_ref::<P>() {
                        ::core::option::Option::Some(p) => p,
                        ::core::option::Option::None => ::std::panic!(
                            "`{}` row's pivot is not of type `{}` - pass the correct pivot type to `pivot::<P>()`",
                            ::std::any::type_name::<Self>(),
                            ::std::any::type_name::<P>(),
                        ),
                    },
                }
            }
        }
    }
}

/// Emit `<rel>_loaded()`, `<rel>_count()`, and the four per-kind
/// aggregate accessors for one relation. The return type of
/// `<rel>_loaded()` depends on the relation's kind:
///
/// - HasOne / BelongsTo / MorphTo / MorphOne / HasOneThrough →
///   `Option<&Target>` (the cache stores `Option<T>`)
/// - HasMany / BelongsToMany / HasManyThrough / MorphMany /
///   MorphToMany / MorphedByMany → `&[Target]`
///
/// `<rel>_count()` always returns `u64` and panics with a clear
/// message when `with_count(["..."])` wasn't called.
///
/// The four aggregate accessors -
/// `<rel>_sum_of(col)` / `<rel>_avg_of(col)` returning `Option<f64>`
/// and `<rel>_min_of(col)` / `<rel>_max_of(col)` returning
/// `Option<Option<f64>>` (outer `Option` = "was `with_min`/`with_max`
/// called?", inner `Option` = "is the result NULL because the group
/// was empty?") - read the wide `<rel>_<kind>_<col>` cache cells.
/// They return `None` when the matching `with_*` call was not made
/// against the column; reading after the corresponding `with_*` call
/// returns `Some(value)`.
fn emit_relation_accessors(struct_ident: &syn::Ident, rel: &RelationDecl) -> TokenStream {
    let name = &rel.name;
    let name_str = name.to_string();
    let loaded_fn = quote::format_ident!("{}_loaded", name);
    let count_fn = quote::format_ident!("{}_count", name);
    let sum_of_fn = quote::format_ident!("{}_sum_of", name);
    let avg_of_fn = quote::format_ident!("{}_avg_of", name);
    let min_of_fn = quote::format_ident!("{}_min_of", name);
    let max_of_fn = quote::format_ident!("{}_max_of", name);
    let with_where_fn = quote::format_ident!("with_where_{}", name);
    // For Through kinds the parser stores generics left-to-right as
    // `(rel.target, rel.through)` where the first generic is the
    // intermediate B and the second is the final target C. The
    // accessor must surface the FINAL target so user code reads
    // `country.posts_loaded()` as `&[Post]` (not `&[User]`). For all
    // other kinds the parser-side `rel.target` IS the user-facing
    // target.
    let target_ty: &syn::Type = match rel.kind {
        RelationKindAttr::HasManyThrough | RelationKindAttr::HasOneThrough => {
            rel.through.as_ref().unwrap_or(&rel.target)
        }
        _ => &rel.target,
    };

    // The "loaded" accessor - kind-dependent return type.
    let loaded = match rel.kind {
        // Single-value kinds - read via get_one.
        RelationKindAttr::HasOne
        | RelationKindAttr::BelongsTo
        | RelationKindAttr::HasOneThrough
        | RelationKindAttr::MorphOne => quote! {
            #[doc = "Read the eager-loaded row for this relation."]
            #[doc = ""]
            #[doc = "Returns `None` if the relation was not eager-loaded \
                     (call `.with([\"...\"])` on the query builder) OR if \
                     the FK on the parent row was null."]
            pub fn #loaded_fn(&self) -> ::core::option::Option<&#target_ty> {
                self.__eager.get_one::<#target_ty>(#name_str)
            }
        },

        // MorphTo: the eager loader caches the per-family `<Name>Morph`
        // enum, so a loaded row reads it back without a query.
        RelationKindAttr::MorphTo => {
            let enum_ident = morph_enum_ident(rel);
            quote! {
                #[doc = "Read the eager-loaded `MorphTo` parent: the per-family enum \
                         holding the target row, or `Unknown` when the row points at \
                         no target."]
                #[doc = ""]
                #[doc = "Returns `None` if the relation was not eager-loaded \
                         (call `.with([\"...\"])` on the query builder)."]
                pub fn #loaded_fn(&self) -> ::core::option::Option<&#enum_ident> {
                    self.__eager.get_one::<#enum_ident>(#name_str)
                }
            }
        }

        // Collection kinds - read via get_many; panics if not loaded.
        RelationKindAttr::HasMany
        | RelationKindAttr::BelongsToMany
        | RelationKindAttr::HasManyThrough
        | RelationKindAttr::MorphMany
        | RelationKindAttr::MorphToMany
        | RelationKindAttr::MorphedByMany => quote! {
            #[doc = "Read the eager-loaded rows for this relation."]
            #[doc = ""]
            #[doc = "Panics with a clear message if the relation was not \
                     eager-loaded - call `.with([\"...\"])` on the query \
                     builder before iterating."]
            pub fn #loaded_fn(&self) -> &[#target_ty] {
                self.__eager.get_many::<#target_ty>(#name_str)
            }
        },
    };

    // P4: typed `with_where_<rel>(closure)` method per relation. The
    // generic `with_where((name, closure))` needs the caller to spell
    // out `Builder<Target>` on the closure parameter because the
    // predicate is type-erased through `Arc<dyn Any>`. The macro knows
    // the target type, so it can emit a typed wrapper that lets
    // inference do the work. `MorphTo` is skipped - its target is `()`
    // / a per-family enum at T1, not a single `Model`, so no
    // `Builder<Target>` exists.
    let with_where_block = match rel.kind {
        RelationKindAttr::MorphTo => quote! {},
        _ => quote! {
            #[doc = "Eager-load this relation with a typed predicate applied to its builder."]
            #[doc = ""]
            #[doc = "Identical to `Self::with_where((\"<rel>\", |q| ...))` but with type \
                     inference picking up the closure parameter type from the method \
                     signature - users don't need to spell out `Builder<Target>`."]
            pub fn #with_where_fn<F>(f: F) -> ::suprnova::Builder<Self>
            where
                F: ::core::ops::Fn(
                        ::suprnova::Builder<#target_ty>,
                    ) -> ::suprnova::Builder<#target_ty>
                    + ::core::marker::Send
                    + ::core::marker::Sync
                    + 'static,
            {
                <Self as ::suprnova::eloquent::Model>::query()
                    .with_where((#name_str, f))
            }
        },
    };

    // NOTE: we deliberately do NOT emit `impl Builder<#struct_ident>` -
    // Rust's orphan rules forbid inherent impls on a foreign generic
    // type even when its parameter is local, and threading a per-relation
    // extension trait into scope would defeat the ergonomic win. Users
    // chain on a builder via the existing generic
    // `.with_where(("rel", |q: Builder<T>| ...))`; type inference works
    // on the static `Self::with_where_<rel>(closure)` entrypoint, which
    // is the path the typed shorthand is intended for.
    let with_where_builder_impl = quote! {};

    quote! {
        impl #struct_ident {
            #loaded

            #[doc = "Read the `with_count(\"...\")` aggregate for this relation."]
            #[doc = ""]
            #[doc = "Panics with a clear message if `with_count` wasn't called \
                     for this relation - the spec requires loud failures over \
                     silent zeros."]
            pub fn #count_fn(&self) -> u64 {
                self.__eager
                    .get_count(#name_str)
                    .unwrap_or_else(|| ::std::panic!(
                        "`{}::{}` requires `with_count([\"{}\"])`",
                        ::std::any::type_name::<Self>(),
                        ::core::stringify!(#count_fn),
                        #name_str,
                    ))
            }

            #[doc = "Read the `with_sum((\"...\", col))` aggregate for this \
                     relation and the given column."]
            #[doc = ""]
            #[doc = "Returns `None` if `with_sum` was not called for \
                     this relation/column pair (silent miss - multiple \
                     aggregates can compose on the same relation, so we \
                     don't panic on absent reads)."]
            pub fn #sum_of_fn(&self, col: &str) -> ::core::option::Option<f64> {
                let key = ::suprnova::eloquent::relations::aggregate_cache_key(
                    #name_str,
                    ::suprnova::AggregateKind::Sum,
                    col,
                );
                self.__eager.get_aggregate::<f64>(&key).copied()
            }

            #[doc = "Read the `with_avg((\"...\", col))` aggregate for this \
                     relation and the given column."]
            #[doc = ""]
            #[doc = "Returns `None` if `with_avg` was not called for \
                     this relation/column pair."]
            pub fn #avg_of_fn(&self, col: &str) -> ::core::option::Option<f64> {
                let key = ::suprnova::eloquent::relations::aggregate_cache_key(
                    #name_str,
                    ::suprnova::AggregateKind::Avg,
                    col,
                );
                self.__eager.get_aggregate::<f64>(&key).copied()
            }

            #[doc = "Read the `with_min((\"...\", col))` aggregate for this \
                     relation and the given column."]
            #[doc = ""]
            #[doc = "Outer `Option` is \"did `with_min` populate this cell?\" \
                     - `None` means the call was not made. Inner `Option` is \
                     \"is the result NULL?\" - `Some(None)` means `with_min` \
                     was called but the group was empty (SQL's NULL-on-empty). \
                     `Some(Some(value))` is the populated, non-empty case."]
            pub fn #min_of_fn(
                &self,
                col: &str,
            ) -> ::core::option::Option<::core::option::Option<f64>> {
                let key = ::suprnova::eloquent::relations::aggregate_cache_key(
                    #name_str,
                    ::suprnova::AggregateKind::Min,
                    col,
                );
                self.__eager
                    .get_aggregate::<::core::option::Option<f64>>(&key)
                    .copied()
            }

            #[doc = "Read the `with_max((\"...\", col))` aggregate for this \
                     relation and the given column. See `<rel>_min_of` for \
                     the double-`Option` shape rationale."]
            pub fn #max_of_fn(
                &self,
                col: &str,
            ) -> ::core::option::Option<::core::option::Option<f64>> {
                let key = ::suprnova::eloquent::relations::aggregate_cache_key(
                    #name_str,
                    ::suprnova::AggregateKind::Max,
                    col,
                );
                self.__eager
                    .get_aggregate::<::core::option::Option<f64>>(&key)
                    .copied()
            }

            #with_where_block
        }

        #with_where_builder_impl
    }
}

/// Emit the `inventory::submit!(RelationEntry { ... })` for one
/// declared relation. Phase 8 (Admin) walks this registry to
/// enumerate every relation in the binary; the has/where-has engine
/// reads the join-metadata fields populated below.
///
/// For `MorphTo` declarations the target type is the unit type `()` -
/// the per-family enum that stands in as the "real" target is
/// generated locally by T6.
fn emit_relation_inventory(
    struct_ident: &syn::Ident,
    input: &ModelInput,
    rel: &RelationDecl,
) -> TokenStream {
    let name_str = rel.name.to_string();
    let parent_type_name = struct_ident.to_string();
    // Mirror `emit_relation_accessors`' kind-aware target choice so
    // Phase 8 admin renders the FINAL target (Post) for Through
    // relations rather than the intermediate (User).
    let target_ty: &syn::Type = match rel.kind {
        RelationKindAttr::HasManyThrough | RelationKindAttr::HasOneThrough => {
            rel.through.as_ref().unwrap_or(&rel.target)
        }
        _ => &rel.target,
    };
    let kind_variant = kind_to_runtime(rel.kind);
    // `RelationEntry::target_type_name` is `&'static str`, so we need
    // a string literal at macro expansion time - `type_name::<T>()`
    // isn't a `const fn` and can't be used in an `inventory::submit!`
    // constant initialiser. We render the `syn::Type` via
    // `TokenStream::to_string()` and strip the spaces that `quote`
    // inserts between tokens, so `Vec<Post>` is stored as
    // `"Vec<Post>"` (not `"Vec < Post >"`) and `Option<i64>` as
    // `"Option<i64>"` - Phase 8 admin renders this in the UI and
    // the padded form is visually wrong.
    let target_type_lit = format_target_type(target_ty);
    let target_type_name = match rel.kind {
        // MorphTo has no single concrete target; T6 emits the
        // per-family enum and overrides this entry. T1 stores
        // `"<morph>"` as a placeholder so admin tooling can render
        // something meaningful even before T6 lands.
        RelationKindAttr::MorphTo => "<morph>".to_string(),
        _ => target_type_lit,
    };

    // ---- Has/where-has join metadata ------------------------------
    //
    // Each branch computes the four/seven string slots
    // (target_table, foreign_key, parent_key, pivot_*, morph_*)
    // the existence-engine renders into `EXISTS (...)`. We emit
    // `&'static str` literals - every value is either a parsed
    // attribute string or a `const` accessor on `EloquentModel`,
    // which is const-evaluable inside `inventory::submit!`.

    let parent_struct_name = struct_ident.to_string();
    let parent_struct_snake = to_snake(&parent_struct_name);

    // For most kinds the target table is the related model's
    // declared `EloquentModel::TABLE`. Through resolves to the FINAL
    // target (already chosen above by `target_ty`). MorphTo has no
    // single target table; we emit "" as the per-spec sentinel.
    let target_table_expr: TokenStream = match rel.kind {
        RelationKindAttr::MorphTo => quote! { "" },
        _ => quote! {
            <#target_ty as ::suprnova::eloquent::EloquentModel>::TABLE
        },
    };

    // Target key - used by the existence engine to render
    // `pivot.related = target.<key>` joins. A many-to-many joins on the
    // column its pivot's related key holds, the declared `related_key`
    // or else the related model's `const PRIMARY_KEY`, as its reads do
    // (`related_key_expr`). Every other kind reads the related model's
    // primary key. MorphTo has no single target table and no single PK
    // column; emit `""` as the sentinel.
    let target_primary_key_expr: TokenStream = match rel.kind {
        RelationKindAttr::MorphTo => quote! { "" },
        RelationKindAttr::BelongsToMany
        | RelationKindAttr::MorphToMany
        | RelationKindAttr::MorphedByMany => related_key_expr(rel, target_ty),
        _ => quote! {
            <#target_ty as ::suprnova::eloquent::EloquentModel>::PRIMARY_KEY
        },
    };

    // Related model's soft-delete column. The macro-emitted const on
    // the target's `impl EloquentModel` returns `""` when the related
    // model does NOT opt into `#[model(soft_deletes)]`, so the engine
    // can treat empty as "no auto soft-delete filter". MorphTo cannot
    // bind to a single target type at the parent's expansion site.
    let related_soft_deletes_column_expr: TokenStream = match rel.kind {
        RelationKindAttr::MorphTo => quote! { "" },
        _ => quote! {
            <#target_ty as ::suprnova::eloquent::EloquentModel>::SOFT_DELETES_COLUMN
        },
    };

    // The owner's `updated_at` column, or "" when the owner disclaims
    // timestamps. `touch_column` collapses the pair at link time so the
    // parent-touch cascade reads one string instead of two consts.
    let related_updated_at_column_expr: TokenStream = match rel.kind {
        RelationKindAttr::MorphTo => quote! { "" },
        _ => quote! {
            ::suprnova::eloquent::relations::touch_column(
                <#target_ty as ::suprnova::eloquent::EloquentModel>::HAS_TIMESTAMPS,
                <#target_ty as ::suprnova::eloquent::EloquentModel>::UPDATED_AT_COLUMN,
            )
        },
    };
    // How the owner's `updated_at` cast stores the time, so the touch
    // cascade binds what the owner's column takes. A `MorphTo` owner
    // varies by row and brings its own.
    let related_bind_column_expr: TokenStream = match rel.kind {
        RelationKindAttr::MorphTo => {
            quote! { ::suprnova::eloquent::relations::no_column_binder }
        }
        _ => quote! {
            <#target_ty as ::suprnova::eloquent::EloquentModel>::bind_column
        },
    };
    let related_updated_at_storage_expr: TokenStream = match rel.kind {
        RelationKindAttr::MorphTo => {
            quote! { ::suprnova::eloquent::relations::morph_to_touch_storage }
        }
        _ => quote! {
            <#target_ty as ::suprnova::eloquent::EloquentModel>::updated_at_storage
        },
    };

    // Parent key (the key on the OWNER's side). For the has-family
    // relations it is this model's column the relation matches its
    // foreign key against: the `lk` override, else this model's primary
    // key - the same column `local_key_ident` reads for the relation's
    // own queries. A model keyed on `uid` correlates `has("kids")` on
    // `uid`, not on an `id` column it may not have. BelongsTo's
    // "parent_key" maps to the OWNED model's key column ("id" by
    // default).
    let parent_key_str = match rel.kind {
        RelationKindAttr::HasOne
        | RelationKindAttr::HasMany
        | RelationKindAttr::MorphOne
        | RelationKindAttr::MorphMany
        | RelationKindAttr::BelongsToMany
        | RelationKindAttr::MorphToMany
        | RelationKindAttr::MorphedByMany
        | RelationKindAttr::HasOneThrough
        | RelationKindAttr::HasManyThrough => lk_override(rel)
            .unwrap_or(input.primary_key.as_str())
            .to_string(),
        // BelongsTo: parent_key is the COLUMN on the related table the
        // child's FK references (defaults to "id").
        RelationKindAttr::BelongsTo => lk_override(rel).unwrap_or("id").to_string(),
        // MorphTo: parent_key is the PK on the (variable) target
        // table - Laravel default "id".
        RelationKindAttr::MorphTo => "id".to_string(),
    };

    // Foreign key - what the child / pivot / morph row carries.
    let foreign_key_str = match rel.kind {
        RelationKindAttr::HasOne | RelationKindAttr::HasMany => fk_override(rel)
            .map(str::to_string)
            .unwrap_or_else(|| default_has_fk(&parent_struct_name)),
        RelationKindAttr::BelongsTo => fk_override(rel)
            .map(str::to_string)
            .unwrap_or_else(|| default_belongs_to_fk(target_ty)),
        RelationKindAttr::MorphOne | RelationKindAttr::MorphMany => {
            format!("{}_id", morph_name_or_default(rel))
        }
        RelationKindAttr::MorphTo => format!("{}_id", morph_name_or_default(rel)),
        // Pivot kinds: the existence-engine joins through the pivot
        // table; the top-level "foreign_key" slot isn't the discriminating
        // join. Emit "" so the engine selects the pivot path.
        RelationKindAttr::BelongsToMany
        | RelationKindAttr::MorphToMany
        | RelationKindAttr::MorphedByMany => String::new(),
        // Through: first_key on the intermediate, falling back to
        // `<snake(parent)>_id`. The engine uses this for the
        // intermediate-side join.
        RelationKindAttr::HasOneThrough | RelationKindAttr::HasManyThrough => {
            first_key_override(rel)
                .map(str::to_string)
                .unwrap_or_else(|| format!("{}_id", parent_struct_snake))
        }
    };

    let (pivot_table_str, pivot_parent_key_str, pivot_related_key_str) = match rel.kind {
        RelationKindAttr::BelongsToMany => {
            // Pivot table: either the user-declared `pivot_table = "..."`
            // override OR the pivot type's `EloquentModel::TABLE` const.
            // We can't read the const inside the macro; emit a token
            // tree at submission time.
            let pivot_table_token: TokenStream = match pivot_table_override(rel) {
                Some(s) => {
                    let lit = s.to_string();
                    quote! { #lit }
                }
                None => {
                    let pivot_ty = rel
                        .through
                        .as_ref()
                        .expect("parser guarantees BelongsToMany declares `Pivot` after `Target`");
                    quote! { <#pivot_ty as ::suprnova::eloquent::EloquentModel>::TABLE }
                }
            };
            let parent_pivot_col = pivot_fk_override(rel)
                .map(str::to_string)
                .unwrap_or_else(|| default_has_fk(&parent_struct_name));
            let related_pivot_col = pivot_related_override(rel)
                .map(str::to_string)
                .unwrap_or_else(|| default_belongs_to_fk(target_ty));
            return emit_inventory_token(&InventoryFields {
                struct_ident,
                target_ty,
                name: &name_str,
                kind_variant: &kind_variant,
                parent_type_name: &parent_type_name,
                target_type_name: &target_type_name,
                target_table_expr: &target_table_expr,
                foreign_key: "",
                parent_key: &parent_key_str,
                pivot_table_expr: &pivot_table_token,
                pivot_parent_key: &parent_pivot_col,
                pivot_related_key: &related_pivot_col,
                morph_type_column: "",
                morph_type_value: "",
                target_primary_key_expr: &target_primary_key_expr,
                related_soft_deletes_column_expr: &related_soft_deletes_column_expr,
                related_updated_at_column_expr: &related_updated_at_column_expr,
                related_updated_at_storage_expr: &related_updated_at_storage_expr,
                related_bind_column_expr: &related_bind_column_expr,
            });
        }
        RelationKindAttr::MorphToMany | RelationKindAttr::MorphedByMany => {
            let pivot_table_token: TokenStream = match pivot_table_override(rel) {
                Some(s) => {
                    let lit = s.to_string();
                    quote! { #lit }
                }
                None => {
                    let pivot_ty = rel.through.as_ref().expect(
                        "parser guarantees MorphToMany/MorphedByMany declares `Pivot` after `Target`",
                    );
                    quote! { <#pivot_ty as ::suprnova::eloquent::EloquentModel>::TABLE }
                }
            };
            let morph_name = morph_name_or_default(rel);
            let morph_col = format!("{morph_name}_id");
            let related_col = pivot_related_override(rel)
                .map(str::to_string)
                .unwrap_or_else(|| default_belongs_to_fk(target_ty));
            let parent_morph_type_col = format!("{morph_name}_type");
            // For MorphToMany the parent IS the morph side; the
            // discriminator value is THIS model's morph_type string.
            // For MorphedByMany the related model is the morph side;
            // we honour the parser-required `target_morph_type` option.
            let morph_type_value_str = match rel.kind {
                RelationKindAttr::MorphToMany => morph_type_of(input),
                RelationKindAttr::MorphedByMany => target_morph_type_override(rel)
                    .map(str::to_string)
                    .unwrap_or_else(|| morph_type_of(input)),
                _ => String::new(),
            };
            // For MorphedByMany the pivot column that points at this
            // (parent) model is its `pivot_foreign_key`, defaulted to
            // `<snake(parent)>_id` as the relation method defaults it,
            // and the morph column points at the related model (the
            // morph side). It used to default to `<snake(target)>_id`,
            // a column the pivot does not have, so `has` failed.
            let (pivot_parent_col, pivot_related_col) = match rel.kind {
                RelationKindAttr::MorphedByMany => {
                    let parent_col = pivot_fk_override(rel)
                        .map(str::to_string)
                        .unwrap_or_else(|| default_has_fk(&parent_struct_name));
                    (parent_col, morph_col.clone())
                }
                _ => (morph_col.clone(), related_col.clone()),
            };
            return emit_inventory_token(&InventoryFields {
                struct_ident,
                target_ty,
                name: &name_str,
                kind_variant: &kind_variant,
                parent_type_name: &parent_type_name,
                target_type_name: &target_type_name,
                target_table_expr: &target_table_expr,
                foreign_key: "",
                parent_key: &parent_key_str,
                pivot_table_expr: &pivot_table_token,
                pivot_parent_key: &pivot_parent_col,
                pivot_related_key: &pivot_related_col,
                morph_type_column: &parent_morph_type_col,
                morph_type_value: &morph_type_value_str,
                target_primary_key_expr: &target_primary_key_expr,
                related_soft_deletes_column_expr: &related_soft_deletes_column_expr,
                related_updated_at_column_expr: &related_updated_at_column_expr,
                related_updated_at_storage_expr: &related_updated_at_storage_expr,
                related_bind_column_expr: &related_bind_column_expr,
            });
        }
        _ => (String::new(), String::new(), String::new()),
    };

    let (morph_type_column_str, morph_type_value_str) = match rel.kind {
        RelationKindAttr::MorphOne | RelationKindAttr::MorphMany => {
            let morph_name = morph_name_or_default(rel);
            (format!("{morph_name}_type"), morph_type_of(input))
        }
        RelationKindAttr::MorphTo => {
            let morph_name = morph_name_or_default(rel);
            (format!("{morph_name}_type"), String::new())
        }
        _ => (String::new(), String::new()),
    };

    let pivot_table_expr = quote! { #pivot_table_str };
    emit_inventory_token(&InventoryFields {
        struct_ident,
        target_ty,
        name: &name_str,
        kind_variant: &kind_variant,
        parent_type_name: &parent_type_name,
        target_type_name: &target_type_name,
        target_table_expr: &target_table_expr,
        foreign_key: &foreign_key_str,
        parent_key: &parent_key_str,
        pivot_table_expr: &pivot_table_expr,
        pivot_parent_key: &pivot_parent_key_str,
        pivot_related_key: &pivot_related_key_str,
        morph_type_column: &morph_type_column_str,
        morph_type_value: &morph_type_value_str,
        target_primary_key_expr: &target_primary_key_expr,
        related_soft_deletes_column_expr: &related_soft_deletes_column_expr,
        related_updated_at_column_expr: &related_updated_at_column_expr,
        related_updated_at_storage_expr: &related_updated_at_storage_expr,
        related_bind_column_expr: &related_bind_column_expr,
    })
}

/// One named field per slot of the `RelationEntry` inventory record.
/// Naming the slots at each call site keeps two same-typed values (the
/// pivot keys, the morph column and value) from being swapped unnoticed.
struct InventoryFields<'a> {
    struct_ident: &'a syn::Ident,
    target_ty: &'a syn::Type,
    name: &'a str,
    kind_variant: &'a TokenStream,
    parent_type_name: &'a str,
    target_type_name: &'a str,
    target_table_expr: &'a TokenStream,
    foreign_key: &'a str,
    parent_key: &'a str,
    pivot_table_expr: &'a TokenStream,
    pivot_parent_key: &'a str,
    pivot_related_key: &'a str,
    morph_type_column: &'a str,
    morph_type_value: &'a str,
    target_primary_key_expr: &'a TokenStream,
    related_soft_deletes_column_expr: &'a TokenStream,
    related_updated_at_column_expr: &'a TokenStream,
    related_updated_at_storage_expr: &'a TokenStream,
    related_bind_column_expr: &'a TokenStream,
}

/// Single emission point for the inventory token. Keeps the kind-arms
/// in [`emit_relation_inventory`] readable - every branch tail-calls
/// here with the per-kind values.
fn emit_inventory_token(fields: &InventoryFields<'_>) -> TokenStream {
    let InventoryFields {
        struct_ident,
        target_ty,
        name: name_str,
        kind_variant,
        parent_type_name,
        target_type_name,
        target_table_expr,
        foreign_key,
        parent_key,
        pivot_table_expr,
        pivot_parent_key,
        pivot_related_key,
        morph_type_column,
        morph_type_value,
        target_primary_key_expr,
        related_soft_deletes_column_expr,
        related_updated_at_column_expr,
        related_updated_at_storage_expr,
        related_bind_column_expr,
    } = fields;
    quote! {
        ::suprnova::inventory::submit! {
            ::suprnova::RelationEntry {
                parent_type: ::std::any::TypeId::of::<#struct_ident>,
                target_type: ::std::any::TypeId::of::<#target_ty>,
                name: #name_str,
                kind: #kind_variant,
                parent_type_name: #parent_type_name,
                target_type_name: #target_type_name,
                target_table: #target_table_expr,
                foreign_key: #foreign_key,
                parent_key: #parent_key,
                pivot_table: #pivot_table_expr,
                pivot_parent_key: #pivot_parent_key,
                pivot_related_key: #pivot_related_key,
                morph_type_column: #morph_type_column,
                morph_type_value: #morph_type_value,
                target_primary_key: #target_primary_key_expr,
                related_soft_deletes_column: #related_soft_deletes_column_expr,
                related_updated_at_column: #related_updated_at_column_expr,
                related_updated_at_storage: #related_updated_at_storage_expr,
                related_bind_column: #related_bind_column_expr,
            }
        }
    }
}

/// Map the parse-time [`RelationKindAttr`] to the runtime
/// `::suprnova::RelationKind` enum value.
fn kind_to_runtime(kind: RelationKindAttr) -> TokenStream {
    match kind {
        RelationKindAttr::HasOne => quote! { ::suprnova::RelationKind::HasOne },
        RelationKindAttr::BelongsTo => quote! { ::suprnova::RelationKind::BelongsTo },
        RelationKindAttr::HasMany => quote! { ::suprnova::RelationKind::HasMany },
        RelationKindAttr::BelongsToMany => quote! { ::suprnova::RelationKind::BelongsToMany },
        RelationKindAttr::HasOneThrough => quote! { ::suprnova::RelationKind::HasOneThrough },
        RelationKindAttr::HasManyThrough => quote! { ::suprnova::RelationKind::HasManyThrough },
        RelationKindAttr::MorphTo => quote! { ::suprnova::RelationKind::MorphTo },
        RelationKindAttr::MorphOne => quote! { ::suprnova::RelationKind::MorphOne },
        RelationKindAttr::MorphMany => quote! { ::suprnova::RelationKind::MorphMany },
        RelationKindAttr::MorphToMany => quote! { ::suprnova::RelationKind::MorphToMany },
        RelationKindAttr::MorphedByMany => quote! { ::suprnova::RelationKind::MorphedByMany },
    }
}

// ---- T2: HasOne / BelongsTo emission helpers ----------------------------
//
// Each helper takes the model `input` plus the parsed `RelationDecl`
// and emits one chunk of code: the relation method, the
// `__eager_load` match arm, or the (currently empty) count /
// aggregate / recurse stubs. T3-T7 extend these with their own kinds;
// the dispatch is `match rel.kind { ... }` so each kind owns its
// own emission branch.

/// Default FK column name for a HasOne / HasMany relation on
/// parent `<P>`. Laravel convention: `<snake(P)>_id`. Override via
/// the inline `fk = "..."` option on the relation declaration.
fn default_has_fk(parent_struct_name: &str) -> String {
    format!("{}_id", to_snake(parent_struct_name))
}

/// Default FK column name for a BelongsTo on child `<C>` pointing at
/// parent `<P>`. Laravel convention: `<snake(target_type)>_id`. The
/// `target_type` is the `<P>` in `BelongsTo<P>`. Override via inline
/// `fk = "..."`.
fn default_belongs_to_fk(target_ty: &syn::Type) -> String {
    // Extract the last path segment as a string - covers
    // `Post`, `crate::models::Post`, `super::Post`. Falls back to
    // formatting the whole type if the path is empty.
    let target_name = match target_ty {
        syn::Type::Path(p) => p
            .path
            .segments
            .last()
            .map(|seg| seg.ident.to_string())
            .unwrap_or_else(|| quote::quote!(#target_ty).to_string()),
        _ => quote::quote!(#target_ty).to_string(),
    };
    format!("{}_id", to_snake(&target_name))
}

/// Look up the user-declared `fk = "..."` override on a relation
/// declaration. `None` when the user didn't override.
fn fk_override(rel: &RelationDecl) -> Option<&str> {
    rel.options.iter().find_map(|o| match o {
        RelationOpt::ForeignKey(s) => Some(s.as_str()),
        _ => None,
    })
}

/// Look up the user-declared `lk = "..."` override.
fn lk_override(rel: &RelationDecl) -> Option<&str> {
    rel.options.iter().find_map(|o| match o {
        RelationOpt::LocalKey(s) => Some(s.as_str()),
        _ => None,
    })
}

/// The parent field whose value a relation matches against its foreign
/// key: the field `lk = "..."` names, else the primary key.
///
/// Every has-family path reads the parent's key through this one ident:
/// the lazy relation method, the eager, count and aggregate arms, and so
/// the pivot writes of a many-to-many. The inventory entry the existence
/// engine reads names the same column. A `BelongsTo`'s `lk` names the
/// owner's column on the target instead, and a `MorphTo` has no parent
/// key of its own, so both keep the primary key, which they never read.
fn local_key_ident(input: &ModelInput, rel: &RelationDecl) -> Result<syn::Ident> {
    let pk = quote::format_ident!("{}", input.primary_key);
    match rel.kind {
        RelationKindAttr::BelongsTo | RelationKindAttr::MorphTo => Ok(pk),
        _ => match lk_override(rel) {
            None => Ok(pk),
            Some(lk) if field_type(input, lk).is_some() => Ok(quote::format_ident!("{lk}")),
            Some(lk) => Err(syn::Error::new_spanned(
                &rel.name,
                format!(
                    "`lk = \"{lk}\"` names no field of `{}`: the local key a relation \
                     reads must be a field of the model",
                    input.item.ident
                ),
            )),
        },
    }
}

/// Look up the user-declared `with_default = || ...` closure on a
/// BelongsTo relation. Returns the parsed expression; emission wraps
/// it in `.with_default(<expr>)` at the call site.
fn with_default_expr(rel: &RelationDecl) -> Option<&syn::Expr> {
    rel.options.iter().find_map(|o| match o {
        RelationOpt::WithDefault(e) => Some(e),
        _ => None,
    })
}

/// Look up the user-declared `pivot_table = "..."` override.
/// Returns `None` when the user relies on the pivot type's own
/// `EloquentModel::TABLE` const (the recommended path).
fn pivot_table_override(rel: &RelationDecl) -> Option<&str> {
    rel.options.iter().find_map(|o| match o {
        RelationOpt::PivotTable(s) => Some(s.as_str()),
        _ => None,
    })
}

/// Look up the user-declared `pivot_foreign_key = "..."` override.
fn pivot_fk_override(rel: &RelationDecl) -> Option<&str> {
    rel.options.iter().find_map(|o| match o {
        RelationOpt::PivotForeignKey(s) => Some(s.as_str()),
        _ => None,
    })
}

/// Look up the user-declared `pivot_related_key = "..."` override.
fn pivot_related_override(rel: &RelationDecl) -> Option<&str> {
    rel.options.iter().find_map(|o| match o {
        RelationOpt::PivotRelatedKey(s) => Some(s.as_str()),
        _ => None,
    })
}

/// Look up the user-declared `related_key = "..."` override - the
/// related-side COLUMN a many-to-many pivot's related key holds. See
/// [`related_key_expr`] for the default.
fn related_key_override(rel: &RelationDecl) -> Option<&str> {
    rel.options.iter().find_map(|o| match o {
        RelationOpt::RelatedKey(s) => Some(s.as_str()),
        _ => None,
    })
}

/// The pivot table a many-to-many relation reads and writes, as a
/// `&str` expression: the declared `pivot_table`, else the table the
/// pivot model is declared over. The eager arms read pivot rows from it,
/// as the lazy relation and the pivot writes do.
fn pivot_table_str(rel: &RelationDecl, pivot_ty: &syn::Type) -> TokenStream {
    match pivot_table_override(rel) {
        Some(t) => quote! { #t },
        None => quote! { <#pivot_ty as ::suprnova::eloquent::EloquentModel>::TABLE },
    }
}

/// The related column a many-to-many pivot's related key holds, as a
/// `&str` expression: the declared `related_key`, else the related
/// model's own primary key, as Laravel's `$relatedKey` defaults to the
/// related model's key name.
///
/// The lazy relation, the eager arm and the aggregate join all read
/// this one expression. The eager arm used to hardcode `id`, so a pivot
/// holding another column matched the wrong rows when loaded eagerly.
fn related_key_expr(rel: &RelationDecl, target_ty: &syn::Type) -> TokenStream {
    match related_key_override(rel) {
        Some(rk) => quote! { #rk },
        None => quote! { <#target_ty as ::suprnova::eloquent::EloquentModel>::PRIMARY_KEY },
    }
}

/// Look up `with_pivot = ["col1", ...]` extra columns. Returns an
/// empty slice when omitted.
fn with_pivot_cols(rel: &RelationDecl) -> &[String] {
    for o in &rel.options {
        if let RelationOpt::WithPivot(cols) = o {
            return cols.as_slice();
        }
    }
    &[]
}

/// Look up the user-declared `first_key = "..."` override for
/// `HasOneThrough` / `HasManyThrough` - the column on the intermediate
/// `B` table that points at the parent `A`. Default:
/// `<snake(parent_struct)>_id`.
fn first_key_override(rel: &RelationDecl) -> Option<&str> {
    rel.options.iter().find_map(|o| match o {
        RelationOpt::FirstKey(s) => Some(s.as_str()),
        _ => None,
    })
}

/// Look up the user-declared `second_key = "..."` override for
/// `HasOneThrough` / `HasManyThrough` - the column on the target `C`
/// table that points at the intermediate `B`. Default:
/// `<snake(through_type)>_id`.
fn second_key_override(rel: &RelationDecl) -> Option<&str> {
    rel.options.iter().find_map(|o| match o {
        RelationOpt::SecondKey(s) => Some(s.as_str()),
        _ => None,
    })
}

/// Look up the user-declared `second_local_key = "..."` override for
/// `HasOneThrough` / `HasManyThrough` - the column on the intermediate
/// `B` matched by `second_key`. Defaults to `"id"`. Required when the
/// intermediate model declares `#[model(primary_key = "...")]` with a
/// non-`id` PK.
fn second_local_key_override(rel: &RelationDecl) -> Option<&str> {
    rel.options.iter().find_map(|o| match o {
        RelationOpt::SecondLocalKey(s) => Some(s.as_str()),
        _ => None,
    })
}

/// True when `with_timestamps` (bare flag or `= true`) is declared.
fn with_timestamps_flag(rel: &RelationDecl) -> bool {
    rel.options
        .iter()
        .any(|o| matches!(o, RelationOpt::WithTimestamps))
}

/// Look up the user-declared `name = "..."` morph-family override.
/// Defaults to the relation name itself when omitted - e.g. a relation
/// declared as `commentable: MorphTo { targets = [...] }` derives a
/// morph-name of `"commentable"` without needing the redundant
/// `name = "commentable"` option.
fn morph_name_or_default(rel: &RelationDecl) -> String {
    rel.options
        .iter()
        .find_map(|o| match o {
            RelationOpt::MorphName(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| rel.name.to_string())
}

/// Look up the user-declared `target_morph_type = "..."` option on a
/// `MorphedByMany` declaration. Returns the explicit morph-type string
/// for the target model. The parser enforces that this option is
/// present for `MorphedByMany` declarations (see
/// `parse_one_relation`'s post-validation block), so `expect` callers
/// are on the unreachable path.
fn target_morph_type_override(rel: &RelationDecl) -> Option<&str> {
    rel.options.iter().find_map(|o| match o {
        RelationOpt::TargetMorphType(s) => Some(s.as_str()),
        _ => None,
    })
}

/// Look up the user-declared `targets = [...]` list on a `MorphTo`
/// declaration. The parser guarantees this option is present for
/// `MorphTo` declarations (see `parse_one_relation`); the `expect`
/// / `ok_or_else` callers handle the unreachable case.
fn morph_targets(rel: &RelationDecl) -> Option<&[syn::Type]> {
    rel.options.iter().find_map(|o| match o {
        RelationOpt::MorphTargets(types) => Some(types.as_slice()),
        _ => None,
    })
}

/// The per-family enum a `MorphTo` relation emits: `commentable` →
/// `CommentableMorph`, `something_polymorphic` →
/// `SomethingPolymorphicMorph`. The relation method, the loaded
/// accessor, the eager loader and the nested loader all name it, so the
/// name is derived in this one place.
fn morph_enum_ident(rel: &RelationDecl) -> syn::Ident {
    let s = rel.name.to_string();
    let mut chars = s.chars();
    let first = chars
        .next()
        .map(|c| c.to_ascii_uppercase().to_string())
        .unwrap_or_default();
    // Strip underscores + capitalise each segment so
    // `something_polymorphic` becomes `SomethingPolymorphicMorph`.
    let mut camel = String::with_capacity(s.len());
    camel.push_str(&first);
    let mut upper_next = false;
    for c in chars {
        if c == '_' {
            upper_next = true;
        } else if upper_next {
            camel.push(c.to_ascii_uppercase());
            upper_next = false;
        } else {
            camel.push(c);
        }
    }
    quote::format_ident!("{camel}Morph")
}

/// The variant of the per-family enum for each target of a `MorphTo`
/// relation, in declaration order: the target's last path segment
/// (`MorphPost` from `crate::models::MorphPost`).
fn morph_variant_idents(targets: &[syn::Type]) -> Vec<syn::Ident> {
    targets
        .iter()
        .map(|ty| quote::format_ident!("{}", last_segment_name(ty)))
        .collect()
}

/// The declared type of the named field on the model struct, `None`
/// when the struct has no field by that name.
fn field_type<'a>(input: &'a ModelInput, field_name: &str) -> Option<&'a syn::Type> {
    let syn::Fields::Named(named) = &input.item.fields else {
        return None;
    };
    named
        .named
        .iter()
        .find(|f| f.ident.as_ref().is_some_and(|i| i == field_name))
        .map(|f| &f.ty)
}

/// The `T` of an `Option<T>` field type, or the type itself when it is
/// not an `Option`. A nullable morph declares `<name>_id: Option<K>`,
/// and it holds the same key `K` as a required one.
fn strip_option(ty: &syn::Type) -> &syn::Type {
    if let syn::Type::Path(p) = ty
        && let Some(seg) = p.path.segments.last()
        && seg.ident == "Option"
        && let syn::PathArguments::AngleBracketed(args) = &seg.arguments
        && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
    {
        return inner;
    }
    ty
}

/// The compile-time key checks of one `MorphTo` relation, as `const`
/// items: the child's `<name>_id` field holds the key type of the first
/// target, and every other target has that key type too. A mismatch is
/// a compile error whose message names the models (see
/// `suprnova::eloquent::relations::morph::MorphTargetsShareKey`).
///
/// The relation reads its id and type string from the `<name>_id` and
/// `<name>_type` fields, so a declaration without them is refused here
/// with a message that says which fields to add.
fn emit_morph_key_checks(
    input: &ModelInput,
    rel: &RelationDecl,
    targets: &[syn::Type],
    enum_ident: &syn::Ident,
) -> Result<TokenStream> {
    let struct_ident = &input.item.ident;
    let morph_name = morph_name_or_default(rel);
    let id_col = format!("{morph_name}_id");
    let type_col = format!("{morph_name}_type");
    let (Some(id_ty), Some(_)) = (field_type(input, &id_col), field_type(input, &type_col)) else {
        return Err(syn::Error::new_spanned(
            &rel.name,
            format!(
                "the `MorphTo` relation `{}` reads the `{id_col}` and `{type_col}` columns; \
                 declare both fields on `{struct_ident}`",
                rel.name,
            ),
        ));
    };
    let Some((first, others)) = targets.split_first() else {
        // The parser refuses an empty `targets = [...]`.
        return Ok(TokenStream::new());
    };
    let column_ty = strip_option(id_ty);
    let share_checks = others.iter().map(|other| {
        quote! {
            const _: () = ::suprnova::eloquent::relations::morph::assert_morph_targets_share_key::<
                <#first as ::suprnova::eloquent::EloquentModel>::Key,
                <#other as ::suprnova::eloquent::EloquentModel>::Key,
                #first,
                #other,
                #enum_ident,
            >();
        }
    });
    Ok(quote! {
        const _: () = ::suprnova::eloquent::relations::morph::assert_morph_id_column_holds_key::<
            #column_ty,
            <#first as ::suprnova::eloquent::EloquentModel>::Key,
            #struct_ident,
            #first,
            #enum_ident,
        >();
        #( #share_checks )*
    })
}

/// The morph-type string a model registers under. Read from the
/// model's `morph_type = "..."` attribute when present; defaults to
/// `to_snake(struct_name)` otherwise (Laravel convention - `Post`
/// becomes `"post"`).
///
/// This is the string the parent puts into the child's
/// `<morph_name>_type` column at insert time, and the string the
/// `MorphMany` / `MorphOne` runtime uses to filter the child table
/// by. Per the brief: T8's morph registry adds a runtime warn-log
/// cross-check; T6 trusts the per-model attribute + Laravel default.
fn morph_type_of(input: &ModelInput) -> String {
    input
        .morph_type
        .clone()
        .unwrap_or_else(|| to_snake(&input.item.ident.to_string()))
}

/// Heuristic set of structural morph-type match keys for one target,
/// used ONLY by parse-time overlap detection. Phase 10B P2 moved the
/// runtime fetch-helper dispatch onto the T8 `MorphTypeEntry`
/// inventory (`find_morph_type_by_id`) - the heuristic match-key
/// surface is no longer authoritative at runtime.
///
/// The macro expanding a `MorphTo` declaration can't see another
/// struct's `morph_type = "..."` attribute (it lives in a separate
/// macro invocation), so the parse-time check uses these structural
/// shorthands to catch the obvious collision cases at declaration
/// time. Runtime is authoritative; the parse-time check is a heuristic
/// safety net.
///
/// For a target named `MorphPost`, this yields:
/// - `"morph_post"` - `to_snake(TargetTypeName)` (the macro default)
/// - `"morphpost"` - no-underscore form
/// - `"post"` - Laravel convention (struct name minus a `Morph`
///   prefix when one exists; if there's no obvious prefix this falls
///   through to the snake form, deduped via `sort + dedup`).
///
/// For a target named `Post` (no prefix), the result collapses to
/// `["post"]` - `to_snake`, no-underscore, and the no-prefix branch
/// all produce the same string.
///
/// Exposed to `parse.rs` so the parser can detect overlapping dispatch
/// keys across a `MorphTo`'s declared targets at declaration time
/// (e.g. `targets = [MorphPost, Post]` - both produce `"post"` and
/// were ambiguous under the old heuristic dispatch; even though P2
/// now resolves the ambiguity at runtime via the registry, the
/// parse-time check still catches the obvious cases up front so the
/// user gets a clearer error than "first match wins").
pub(super) fn morph_target_keys(ty: &syn::Type) -> Vec<String> {
    let name = match ty {
        syn::Type::Path(p) => p
            .path
            .segments
            .last()
            .map(|seg| seg.ident.to_string())
            .unwrap_or_else(|| quote::quote!(#ty).to_string()),
        _ => quote::quote!(#ty).to_string(),
    };
    let snake = to_snake(&name);
    let no_underscore = snake.replace('_', "");
    // Laravel convention: strip a leading `Morph` prefix when present
    // so `MorphPost` → `"post"`. Falls back to the snake form when no
    // prefix is found.
    let stripped = if let Some(rest) = name.strip_prefix("Morph") {
        if !rest.is_empty() {
            to_snake(rest)
        } else {
            snake.clone()
        }
    } else {
        snake.clone()
    };
    let mut out = vec![snake.clone(), no_underscore, stripped];
    out.sort();
    out.dedup();
    out
}

/// Extract the last path segment of a type ident, e.g. `Post` from
/// `crate::models::Post`. Used for default pivot-key derivation
/// (`<snake(name)>_id`) when the user omits `pivot_foreign_key`
/// / `pivot_related_key`. Falls back to the full token stream
/// rendering when the type isn't a path (rare; mostly defensive).
fn last_segment_name(ty: &syn::Type) -> String {
    match ty {
        syn::Type::Path(p) => p
            .path
            .segments
            .last()
            .map(|seg| seg.ident.to_string())
            .unwrap_or_else(|| quote::quote!(#ty).to_string()),
        _ => quote::quote!(#ty).to_string(),
    }
}

/// Whether the named field on the user struct has type `Option<T>`.
/// Used by BelongsTo emission to decide between
/// `Some(serde_json::to_value(&self.<fk>).ok()?)` (non-Option) and
/// `self.<fk>.as_ref().map(|v| serde_json::to_value(v).ok()).flatten()`
/// (Option). Looks at the last path segment of the field type - same
/// shape as `classify_datetime` in `parse.rs`.
fn field_is_optional(input: &ModelInput, field_name: &str) -> bool {
    let fields = match &input.item.fields {
        syn::Fields::Named(named) => &named.named,
        _ => return false,
    };
    for f in fields {
        let ident = match f.ident.as_ref() {
            Some(i) => i,
            None => continue,
        };
        if ident == field_name {
            return matches!(
                &f.ty,
                syn::Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "Option")
            );
        }
    }
    false
}

/// The lazy-loading check a relation method hands the relation it
/// builds, over the row's relation cache. The read of every kind
/// (`get` and `first`, and `get` of a `MorphTo` fetch helper) runs it
/// before its query, so this one expression is where the macro wires
/// lazy-loading prevention into every relation method.
fn emit_lazy_load_guard(parent_name: &str, rel: &RelationDecl) -> TokenStream {
    let name_str = rel.name.to_string();
    quote! {
        ::suprnova::eloquent::lazy_loading::LazyLoadGuard::for_relation(
            &self.__eager,
            #parent_name,
            #name_str,
        )
    }
}

/// Emit the relation method (`fn profile(&self) -> HasOne<Self, Profile>`)
/// per declared HasOne / BelongsTo. Other kinds will land in T3-T7;
/// T2 returns an empty stream for them so the macro compiles for
/// users who declared a kind T2 doesn't own yet (e.g. T1's smoke
/// tests with `relations = {}`).
fn emit_relation_method(input: &ModelInput, rel: &RelationDecl) -> Result<TokenStream> {
    let struct_ident = &input.item.ident;
    let parent_name = struct_ident.to_string();
    let pk_name = &input.primary_key;
    let pk_ident = local_key_ident(input, rel)?;
    let method_ident = &rel.name;
    let target_ty = &rel.target;
    let lazy_load = emit_lazy_load_guard(&parent_name, rel);

    match rel.kind {
        RelationKindAttr::HasOne => {
            // FK on the child = <snake(parent_struct)>_id by default.
            // LK on the parent = the parent's PK by default ("id").
            let fk = fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| default_has_fk(&parent_name));
            let lk = lk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| pk_name.clone());

            Ok(quote! {
                impl #struct_ident {
                    #[doc = "Construct a `HasOne` relation builder for this row."]
                    #[doc = ""]
                    #[doc = "Chainable - `user.profile().filter(...).first().await?`."]
                    pub fn #method_ident(&self) -> ::suprnova::HasOne<Self, #target_ty> {
                        let parent_value = ::suprnova::serde_json::to_value(&self.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null);
                        ::suprnova::HasOne::<Self, #target_ty>::__new(
                            parent_value,
                            ::std::string::String::from(#fk),
                            ::std::string::String::from(#lk),
                        )
                        .__lazy_load(#lazy_load)
                    }
                }
            })
        }
        RelationKindAttr::BelongsTo => {
            // FK on this child row = <snake(target)>_id by default.
            // The child's FK column on `self` is what the macro reads
            // to build the lookup; the resulting field access is
            // `self.<fk_ident>` (e.g. `self.user_id`).
            let fk = fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| default_belongs_to_fk(target_ty));
            // owner key on parent = parent's PK by default ("id").
            // T2: BelongsTo's parent PK isn't introspectable from this
            // macro (the parent struct lives in a different `#[model]`
            // invocation), so we default to "id" + honour an explicit
            // `lk = "..."` override.
            let owner_key = lk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| "id".to_string());

            let fk_ident = quote::format_ident!("{}", fk);
            // Inspect the FK field type on the child struct. If
            // `Option<T>`, emit a flat_map over `.as_ref()`; otherwise
            // emit `Some(serde_json::to_value(&self.<fk>)...)`.
            let parent_value_expr = if field_is_optional(input, &fk) {
                quote! {
                    self.#fk_ident
                        .as_ref()
                        .and_then(|v| ::suprnova::serde_json::to_value(v).ok())
                }
            } else {
                quote! {
                    ::core::option::Option::Some(
                        ::suprnova::serde_json::to_value(&self.#fk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null)
                    )
                }
            };

            let with_default_chain = match with_default_expr(rel) {
                Some(expr) => quote! { .with_default(#expr) },
                None => quote! {},
            };

            Ok(quote! {
                impl #struct_ident {
                    #[doc = "Construct a `BelongsTo` relation lookup for this row."]
                    #[doc = ""]
                    #[doc = "Looks up the parent identified by this row's foreign-key \
                             column. Honours `with_default(closure)` declared inline \
                             on the relation."]
                    pub fn #method_ident(&self) -> ::suprnova::BelongsTo<Self, #target_ty> {
                        let parent_value: ::core::option::Option<::suprnova::serde_json::Value>
                            = #parent_value_expr;
                        ::suprnova::BelongsTo::<Self, #target_ty>::__new(
                            parent_value,
                            ::std::string::String::from(#fk),
                            ::std::string::String::from(#owner_key),
                        )#with_default_chain
                        .__lazy_load(#lazy_load)
                    }
                }
            })
        }
        RelationKindAttr::HasMany => {
            // FK on the child table = <snake(parent_struct)>_id by
            // default - same default as HasOne. LK = parent's PK by
            // default ("id"), configurable via `lk = "..."`.
            let fk = fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| default_has_fk(&parent_name));
            let lk = lk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| pk_name.clone());

            Ok(quote! {
                impl #struct_ident {
                    #[doc = "Construct a `HasMany` relation builder for this row."]
                    #[doc = ""]
                    #[doc = "Chainable - `user.posts().latest().take(5).get().await?`."]
                    pub fn #method_ident(&self) -> ::suprnova::HasMany<Self, #target_ty> {
                        let parent_value = ::suprnova::serde_json::to_value(&self.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null);
                        ::suprnova::HasMany::<Self, #target_ty>::__new(
                            parent_value,
                            ::std::string::String::from(#fk),
                            ::std::string::String::from(#lk),
                        )
                        .__lazy_load(#lazy_load)
                    }
                }
            })
        }
        RelationKindAttr::BelongsToMany => {
            // Pivot model - the user wrote `BelongsToMany<R, P>`,
            // parsed into `rel.through`. The parser already validates
            // that BelongsToMany requires a second generic argument,
            // so the `expect` is unreachable on the happy path.
            let pivot_ty = rel.through.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    method_ident,
                    "BelongsToMany requires a pivot type (see parse-time validation)",
                )
            })?;

            // pivot_foreign_key default: <snake(parent_struct)>_id.
            let pivot_fk = pivot_fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&parent_name)));
            // pivot_related_key default: <snake(target_struct_name)>_id.
            let pivot_related = pivot_related_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&last_segment_name(target_ty))));
            // pivot_table: either the user-supplied literal, or - at
            // runtime - `<P as EloquentModel>::TABLE` so the pivot
            // struct's own `#[suprnova::model(table = "...")]` declaration
            // is the single source of truth.
            let pivot_table_expr: TokenStream = match pivot_table_override(rel) {
                Some(t) => quote! { ::std::string::String::from(#t) },
                None => quote! {
                    <#pivot_ty as ::suprnova::eloquent::EloquentModel>::TABLE.to_string()
                },
            };
            // Local key (parent's PK column name). Defaults to the
            // model's declared primary_key.
            let lk = lk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| pk_name.clone());

            // `with_pivot([...])` and `.with_timestamps()` chain calls.
            let pivot_extras = with_pivot_cols(rel);
            let with_pivot_chain = if pivot_extras.is_empty() {
                quote! {}
            } else {
                let lits = pivot_extras
                    .iter()
                    .map(|c| quote! { #c })
                    .collect::<Vec<_>>();
                quote! { .with_pivot(::std::vec![#(#lits),*]) }
            };
            let with_timestamps_chain = if with_timestamps_flag(rel) {
                quote! { .with_timestamps() }
            } else {
                quote! {}
            };
            let local_key_chain = if lk == "id" {
                quote! {}
            } else {
                quote! { .local_key(#lk) }
            };
            // Related-side key column - see `related_key_expr`. Chained
            // as `.related_pk(...)` so the runtime IN-filter (`.get()`)
            // reads the column the eager arm and the aggregate JOIN read.
            let related_key = related_key_expr(rel, target_ty);
            let related_key_chain = quote! { .related_pk(#related_key) };

            Ok(quote! {
                impl #struct_ident {
                    #[doc = "Construct a `BelongsToMany` relation for this row."]
                    #[doc = ""]
                    #[doc = "Use `.attach(id)` / `.detach(id)` / `.sync([...])` to \
                             mutate the pivot, `.get()` to load related rows with \
                             pivot context."]
                    pub fn #method_ident(&self) -> ::suprnova::BelongsToMany<Self, #target_ty, #pivot_ty> {
                        let parent_value = ::suprnova::serde_json::to_value(&self.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null);
                        ::suprnova::BelongsToMany::<Self, #target_ty, #pivot_ty>::__new(
                            parent_value,
                            #pivot_table_expr,
                            ::std::string::String::from(#pivot_fk),
                            ::std::string::String::from(#pivot_related),
                        )
                        #local_key_chain
                        #related_key_chain
                        #with_pivot_chain
                        #with_timestamps_chain
                        .__lazy_load(#lazy_load)
                    }
                }
            })
        }
        RelationKindAttr::HasManyThrough | RelationKindAttr::HasOneThrough => {
            // The user wrote `HasManyThrough<B, C>` where `B` is the
            // intermediate model and `C` is the final target. The
            // parser stores generics left-to-right as
            // `(rel.target, rel.through)` - so for Through kinds, the
            // semantic mapping is:
            //
            //   rel.target  = first generic = B (intermediate)
            //   rel.through = second generic = C (final target)
            //
            // This is intentionally different from `BelongsToMany<R, P>`
            // where `rel.target = R` (final) and `rel.through = P`
            // (pivot). Through relations declare the chain in traversal
            // order; m2m declares target-then-pivot. The macro absorbs
            // the inconsistency so user code reads naturally for each
            // kind.
            let through_ty = &rel.target; // intermediate B
            let final_target_ty = rel.through.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    method_ident,
                    "HasOneThrough / HasManyThrough require a final target type, e.g. \
                     `HasManyThrough<Intermediate, Target>` (parser bug if reached)",
                )
            })?; // final target C

            // first_key default: <snake(parent_struct)>_id.
            let first_key = first_key_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&parent_name)));
            // second_key default: <snake(last_segment(through_ty))>_id -
            // column on the FINAL target table pointing at the
            // intermediate.
            let second_key = second_key_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&last_segment_name(through_ty))));
            // Local key (parent's PK column name). Defaults to the
            // model's declared primary_key. Honoured via the runtime
            // `.local_key(...)` setter so the metadata stays on the
            // Relation impl.
            let lk = lk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| pk_name.clone());
            let local_key_chain = if lk == "id" {
                quote! {}
            } else {
                quote! { .local_key(#lk) }
            };
            // Second local key - column on the intermediate `B`
            // matched by `second_key`. Defaults to `"id"`. Chained as
            // `.second_local_key(...)` so the runtime JOIN reads the
            // right column for intermediates declaring a non-`id` PK.
            let second_local_key = second_local_key_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| "id".to_string());
            let second_local_key_chain = if second_local_key == "id" {
                quote! {}
            } else {
                quote! { .second_local_key(#second_local_key) }
            };

            // Pick the runtime struct name based on the kind. Both
            // wrappers share the same `__new` shape.
            let wrapper = match rel.kind {
                RelationKindAttr::HasManyThrough => quote! { HasManyThrough },
                RelationKindAttr::HasOneThrough => quote! { HasOneThrough },
                _ => unreachable!("guarded by outer match arm"),
            };
            let doc_kind = match rel.kind {
                RelationKindAttr::HasManyThrough => "HasManyThrough",
                RelationKindAttr::HasOneThrough => "HasOneThrough",
                _ => unreachable!("guarded by outer match arm"),
            };
            let doc_str = format!("Construct a `{doc_kind}` relation for this row.");

            Ok(quote! {
                impl #struct_ident {
                    #[doc = #doc_str]
                    #[doc = ""]
                    #[doc = "Two-hop traversal via the intermediate model - \
                             `.get()` issues a single `INNER JOIN` query."]
                    pub fn #method_ident(&self) -> ::suprnova::#wrapper<Self, #through_ty, #final_target_ty> {
                        let parent_value = ::suprnova::serde_json::to_value(&self.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null);
                        ::suprnova::#wrapper::<Self, #through_ty, #final_target_ty>::__new(
                            parent_value,
                            ::std::string::String::from(#first_key),
                            ::std::string::String::from(#second_key),
                        )
                        #local_key_chain
                        #second_local_key_chain
                        .__lazy_load(#lazy_load)
                    }
                }
            })
        }
        RelationKindAttr::MorphMany | RelationKindAttr::MorphOne => {
            // Polymorphic one-to-many / one-to-one. The relation
            // declaration lives on the PARENT side (e.g.
            // `comments: MorphMany<Comment> { name = "commentable" }`
            // on Post + Video). The macro emits a method that returns
            // a runtime `MorphMany<Self, Comment>` (or `MorphOne<...>`)
            // pre-filtered with both `commentable_id = self.id` and
            // `commentable_type = "post"`.
            //
            // Two pieces of metadata flow from the model attributes:
            //
            // 1. `morph_name` - controls the `<name>_id` /
            //    `<name>_type` column names on the child table.
            //    Defaults to the relation name itself.
            //
            // 2. `morph_type_value` - the parent's `morph_type = "..."`
            //    attribute (defaulted to `to_snake(struct_name)`).
            //    This is the string the child's `*_type` column has
            //    to equal for the row to belong to this parent.
            let morph_name = morph_name_or_default(rel);
            let morph_type_value = morph_type_of(input);
            let wrapper = match rel.kind {
                RelationKindAttr::MorphMany => quote! { MorphMany },
                RelationKindAttr::MorphOne => quote! { MorphOne },
                _ => unreachable!("guarded by outer match arm"),
            };
            let doc_kind = match rel.kind {
                RelationKindAttr::MorphMany => "MorphMany",
                RelationKindAttr::MorphOne => "MorphOne",
                _ => unreachable!("guarded by outer match arm"),
            };
            let doc_str =
                format!("Construct a `{doc_kind}` polymorphic relation builder for this row.");
            Ok(quote! {
                impl #struct_ident {
                    #[doc = #doc_str]
                    #[doc = ""]
                    #[doc = "Chainable - both `<morph_name>_id` and `<morph_name>_type` \
                             predicates are pre-applied. Children pointing at OTHER \
                             parents (different `*_type` values) never appear in \
                             results."]
                    pub fn #method_ident(&self) -> ::suprnova::#wrapper<Self, #target_ty> {
                        let parent_value = ::suprnova::serde_json::to_value(&self.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null);
                        ::suprnova::#wrapper::<Self, #target_ty>::__new(
                            parent_value,
                            ::std::string::String::from(#morph_name),
                            ::std::string::String::from(#morph_type_value),
                        )
                        .__lazy_load(#lazy_load)
                    }
                }
            })
        }
        RelationKindAttr::MorphTo => {
            // `MorphTo` is the inverse side - the user declared
            // `commentable: MorphTo { name = "commentable",
            //  targets = [MorphPost, MorphVideo] }` on the morph-table
            // model (Comment). The macro emits FOUR things at this
            // declaration site:
            //
            // 1. A per-family enum `<Name>Morph` with one variant per
            //    target + `Unknown(String, serde_json::Value)` for rows
            //    that point at no target.
            // 2. A per-family fetch helper `<Name>MorphFetch` carrying
            //    the id + type-string, with a `.get()` method that
            //    matches the type-string against each target's morph
            //    type and returns the per-family enum. The eager loader
            //    and the parent-touch cascade pick the target type
            //    through the same helper, so the three paths cannot
            //    disagree on which model a row names.
            // 3. An inherent method on the morph-table model that
            //    constructs the fetch helper from the row's
            //    `<name>_id` + `<name>_type` columns.
            // 4. Compile-time checks that every target has the key type
            //    of the first one and that the `<name>_id` field holds
            //    that type.
            //
            // The user's call site reads:
            //
            //     match comment.commentable().get().await? {
            //         CommentableMorph::MorphPost(p) => ...,
            //         CommentableMorph::MorphVideo(v) => ...,
            //         CommentableMorph::Unknown(t, id) => ...,
            //     }
            //
            // No runtime `MorphTo<C>` instance is built at the call
            // site - `MorphTo<C>` is purely metadata for the relation
            // registry + a re-export users can name in turbofish.
            let name_str = rel.name.to_string();
            let morph_name = morph_name_or_default(rel);
            let id_col = format!("{morph_name}_id");
            let type_col = format!("{morph_name}_type");
            // Enum + fetch struct names - `commentable` →
            // `CommentableMorph` / `CommentableMorphFetch`.
            let enum_ident = morph_enum_ident(rel);
            let fetch_ident = quote::format_ident!("{enum_ident}Fetch");

            let targets = morph_targets(rel).ok_or_else(|| {
                syn::Error::new_spanned(
                    method_ident,
                    "MorphTo requires `targets = [...]` (parser bug if reached)",
                )
            })?;

            // Variant idents = the target's last path segment (e.g.
            // `MorphPost` from `crate::models::MorphPost`). Mechanically
            // required - enum variants name the user type, not a
            // generic placeholder.
            let variant_idents = morph_variant_idents(targets);
            let key_checks = emit_morph_key_checks(input, rel, targets, &enum_ident)?;

            // Per-target if-branches inside `<Name>MorphFetch::__target()`.
            // Each branch consults the runtime morph registry (T8's
            // `MorphTypeEntry` inventory) for the target's `TypeId`
            // to get the canonical `morph_type` string the parent
            // declared via `#[suprnova::model(morph_type = "...")]`.
            // A match names the target by its position in `targets`;
            // no match means the row points at no declared target.
            //
            // Registry-first is what makes user-declared custom
            // `morph_type` strings dispatch correctly (e.g. a target
            // named `Post` carrying `morph_type = "blog_post"` -
            // none of the structural heuristics on the type name
            // would have matched the runtime `"blog_post"`).
            //
            // When the target struct DIDN'T declare an explicit
            // `morph_type` attribute, it's absent from the registry
            // (Phase 10B T8's `morph_type_not_registered_for_non_morph_models`
            // pins this). In that case we fall back to comparing
            // `self.morph_type` against `to_snake(TypeName)` - the
            // same convention the parent's `MorphMany` / `MorphOne`
            // uses to STAMP the type-string into the child column
            // (see `morph_type_of` in this file). Preserves the
            // documented implicit-default contract in
            // `docs/core/eloquent.md#MorphTo`.
            let mut target_arms: Vec<TokenStream> = Vec::with_capacity(targets.len());
            // `.get()` arms: the JSON id turns back into the target's
            // typed key, then `Target::find(key)` runs through the
            // standard Eloquent CRUD path. A missing row falls through
            // to the `Unknown` variant.
            let mut get_arms: Vec<TokenStream> = Vec::with_capacity(targets.len());
            // `__owner_of` arms: where the parent-touch cascade writes.
            let mut owner_arms: Vec<TokenStream> = Vec::with_capacity(targets.len());
            for (index, (ty, variant)) in targets.iter().zip(variant_idents.iter()).enumerate() {
                // The snake-form fallback string for the implicit-
                // default path. Computed at macro-expansion time from
                // the target type's last path segment.
                let snake_fallback = to_snake(&last_segment_name(ty));
                let index = proc_macro2::Literal::usize_unsuffixed(index);
                target_arms.push(quote! {
                    {
                        // Look up the target's registered `morph_type`
                        // string via the T8 inventory. When the target
                        // didn't declare `morph_type = "..."`, the
                        // registry returns None and we compare against
                        // the snake-cased type name - the same default
                        // the parent-side MorphMany / MorphOne uses to
                        // write the type-string column.
                        let registered: ::std::option::Option<&'static str> =
                            ::suprnova::find_morph_type_by_id(
                                ::std::any::TypeId::of::<#ty>(),
                            )
                            .map(|e| e.morph_type);
                        let expected: &str = registered.unwrap_or(#snake_fallback);
                        if expected == morph_type {
                            return ::core::option::Option::Some(#index);
                        }
                    }
                });
                get_arms.push(quote! {
                    ::core::option::Option::Some(#index) => {
                        // The error names the column and the target
                        // but never the value, which is user data.
                        let key: <#ty as ::suprnova::eloquent::EloquentModel>::Key =
                            ::suprnova::serde_json::from_value(self.morph_id.clone()).map_err(
                                |_| {
                                    ::suprnova::FrameworkError::internal(::std::format!(
                                        "the MorphTo relation `{}` of `{}`: the `{}` column does \
                                         not hold a key of `{}`",
                                        #name_str,
                                        #parent_name,
                                        #id_col,
                                        ::std::any::type_name::<#ty>(),
                                    ))
                                },
                            )?;
                        let row: ::core::option::Option<#ty> =
                            <#ty as ::suprnova::eloquent::Model>::find(key).await?;
                        ::core::result::Result::Ok(match row {
                            ::core::option::Option::Some(r) => #enum_ident::#variant(r),
                            ::core::option::Option::None => {
                                #enum_ident::Unknown(self.morph_type, self.morph_id)
                            }
                        })
                    }
                });
                owner_arms.push(quote! {
                    ::core::option::Option::Some(#index) => ::core::result::Result::Ok(
                        ::core::option::Option::Some(
                            ::suprnova::eloquent::relations::morph::MorphOwner::of::<#ty>(
                                morph_id.clone(),
                            ),
                        ),
                    ),
                });
            }

            Ok(quote! {
                #key_checks

                /// Per-family morph enum generated by the
                /// `#[suprnova::model(relations = { ...: MorphTo {
                /// targets = [...] } })]` declaration on the
                /// morph-table struct. One variant per declared target
                /// + `Unknown(type_string, id)` for rows that point at
                /// no target.
                #[derive(::std::fmt::Debug, ::core::clone::Clone)]
                pub enum #enum_ident {
                    #(
                        #variant_idents(#targets),
                    )*
                    /// The row points at no target: its `<morph_name>_type`
                    /// column didn't match any declared target, the target
                    /// row is absent, or its `<morph_name>_id` column is
                    /// null. Carries the type string + the id as the
                    /// column holds it (a JSON number or string, `Null`
                    /// for a null column) so callers can log or migrate
                    /// the stale data.
                    Unknown(::std::string::String, ::suprnova::serde_json::Value),
                }

                /// Fetch helper that dispatches into the per-family
                /// enum. Built by the morph-table model's
                /// `<rel>()` method; calling `.get().await?` resolves
                /// the parent row via the standard Eloquent CRUD path.
                pub struct #fetch_ident {
                    morph_id: ::suprnova::serde_json::Value,
                    morph_type: ::std::string::String,
                    /// The lazy-loading check `get()` runs before its
                    /// query.
                    lazy_load: ::suprnova::eloquent::lazy_loading::LazyLoadGuard,
                }

                impl #fetch_ident {
                    /// The position in `targets = [...]` of the target
                    /// the row's `<name>_type` column names, or `None`
                    /// when it names none of them.
                    ///
                    /// Each declared target is checked in declaration
                    /// order via the T8 morph registry: the runtime
                    /// `<name>_type` string is compared against the
                    /// target's registered `morph_type` value, with a
                    /// snake-cased type-name fallback for targets that
                    /// didn't declare an explicit `morph_type`. First
                    /// match wins.
                    fn __target_of(morph_type: &str) -> ::core::option::Option<usize> {
                        #(#target_arms)*
                        ::core::option::Option::None
                    }

                    /// [`Self::__target_of`] for this helper's own
                    /// `<name>_type` value.
                    fn __target(&self) -> ::core::option::Option<usize> {
                        Self::__target_of(self.morph_type.as_str())
                    }

                    /// The owner row the parent-touch cascade of
                    /// `#[model(touches = [...])]` writes, for a row
                    /// whose `<name>_id` and `<name>_type` values are
                    /// given. `None` when the id is null: the row has no
                    /// owner. A type that names none of the targets is
                    /// an error.
                    ///
                    /// The cascade calls this before the write, through
                    /// `Model::__morph_owner`, so the error means that
                    /// no statement ran and no observer was called. It
                    /// names the relation, the model and the type
                    /// string, and never the id.
                    fn __owner_of(
                        morph_id: &::suprnova::serde_json::Value,
                        morph_type: &str,
                    ) -> ::core::result::Result<
                        ::core::option::Option<::suprnova::eloquent::relations::morph::MorphOwner>,
                        ::suprnova::FrameworkError,
                    > {
                        if ::suprnova::serde_json::Value::is_null(morph_id) {
                            return ::core::result::Result::Ok(::core::option::Option::None);
                        }
                        match Self::__target_of(morph_type) {
                            #(#owner_arms)*
                            _ => ::core::result::Result::Err(
                                ::suprnova::FrameworkError::internal(::std::format!(
                                    "the MorphTo relation `{}` of `{}` cannot touch its owner: \
                                     `{}` is the morph type of none of its targets",
                                    #name_str,
                                    #parent_name,
                                    morph_type,
                                )),
                            ),
                        }
                    }

                    /// Resolve the polymorphic parent. Returns the
                    /// per-family enum's `Unknown` variant when the
                    /// `<name>_type` column doesn't match any declared
                    /// target, when the looked-up row is absent
                    /// (legacy / soft-deleted / renamed model), or when
                    /// the `<name>_id` column is null.
                    ///
                    /// The target is chosen as `__target_of` describes;
                    /// the id is turned into that target's key type before
                    /// the lookup, so an `i64`, `String`, UUID or ULID
                    /// key all resolve.
                    ///
                    /// The lookup is by key and applies no global scope
                    /// of the target, so a target hidden by a scope is
                    /// found (a soft-delete target stays scoped, as its
                    /// `find` is). The eager load,
                    /// `with(["<relation>"])`, runs the target's query
                    /// and applies its global scopes, so the same row
                    /// comes back as `Unknown` there.
                    ///
                    /// Refused without a query when it is a lazy load
                    /// that lazy-loading prevention catches, whether or
                    /// not the id is null, as in Laravel.
                    pub async fn get(
                        self,
                    ) -> ::core::result::Result<#enum_ident, ::suprnova::FrameworkError> {
                        self.lazy_load.check()?;
                        if ::suprnova::serde_json::Value::is_null(&self.morph_id) {
                            return ::core::result::Result::Ok(#enum_ident::Unknown(
                                self.morph_type,
                                self.morph_id,
                            ));
                        }
                        match self.__target() {
                            #(#get_arms)*
                            _ => ::core::result::Result::Ok(#enum_ident::Unknown(
                                self.morph_type,
                                self.morph_id,
                            )),
                        }
                    }
                }

                impl #struct_ident {
                    #[doc = "Construct a `MorphTo` fetch helper for this row."]
                    #[doc = ""]
                    #[doc = "Resolves the polymorphic parent via the row's \
                             `<morph_name>_id` + `<morph_name>_type` columns. \
                             Awaiting `.get()` returns the per-family enum with \
                             one variant per declared target."]
                    pub fn #method_ident(&self) -> #fetch_ident {
                        // The id is the column's JSON value - the same
                        // value `field_value` hands every other
                        // key-carrying path - so any key type binds as
                        // it is.
                        #fetch_ident {
                            morph_id: <Self as ::suprnova::eloquent::Model>::field_value(
                                self,
                                #id_col,
                            )
                            .unwrap_or(::suprnova::serde_json::Value::Null),
                            morph_type: match <Self as ::suprnova::eloquent::Model>::field_value(
                                self,
                                #type_col,
                            ) {
                                ::core::option::Option::Some(
                                    ::suprnova::serde_json::Value::String(s),
                                ) => s,
                                _ => ::std::string::String::new(),
                            },
                            lazy_load: #lazy_load,
                        }
                    }
                }
            })
        }
        RelationKindAttr::MorphToMany => {
            // Polymorphic m2m, parent side (`post.tags()`). The user
            // wrote `MorphToMany<R, P>` where R is the m2m partner
            // (Tag) and P is the polymorphic pivot model (Taggable).
            //
            // The parser stores the pivot in `rel.through`; the
            // `expect` is unreachable on the happy path because
            // `MorphToMany`'s `needs_through()` is true.
            let pivot_ty = rel.through.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    method_ident,
                    "MorphToMany requires a pivot type (see parse-time validation)",
                )
            })?;

            let morph_name = morph_name_or_default(rel);
            let parent_morph_type = morph_type_of(input);
            // pivot_related_key default: <snake(target_struct)>_id.
            let pivot_related = pivot_related_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&last_segment_name(target_ty))));
            // pivot_table: user-supplied literal or `<P as
            // EloquentModel>::TABLE` at runtime.
            let pivot_table_expr: TokenStream = match pivot_table_override(rel) {
                Some(t) => quote! { ::std::string::String::from(#t) },
                None => quote! {
                    <#pivot_ty as ::suprnova::eloquent::EloquentModel>::TABLE.to_string()
                },
            };
            let lk = lk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| pk_name.clone());

            let pivot_extras = with_pivot_cols(rel);
            let with_pivot_chain = if pivot_extras.is_empty() {
                quote! {}
            } else {
                let lits = pivot_extras
                    .iter()
                    .map(|c| quote! { #c })
                    .collect::<Vec<_>>();
                quote! { .with_pivot(::std::vec![#(#lits),*]) }
            };
            let with_timestamps_chain = if with_timestamps_flag(rel) {
                quote! { .with_timestamps() }
            } else {
                quote! {}
            };
            let local_key_chain = if lk == "id" {
                quote! {}
            } else {
                quote! { .local_key(#lk) }
            };
            let related_key = related_key_expr(rel, target_ty);
            let related_key_chain = quote! { .related_pk(#related_key) };

            Ok(quote! {
                impl #struct_ident {
                    #[doc = "Construct a `MorphToMany` polymorphic m2m relation for this row."]
                    #[doc = ""]
                    #[doc = "Use `.attach(id)` / `.detach(id)` / `.sync([...])` to mutate \
                             the pivot, `.get()` to load related rows with pivot context. \
                             The pivot's `<morph_name>_type` column is filtered to \
                             `Self`'s `morph_type` so children of other morph families are \
                             excluded automatically."]
                    pub fn #method_ident(&self)
                        -> ::suprnova::MorphToMany<Self, #target_ty, #pivot_ty>
                    {
                        let parent_value = ::suprnova::serde_json::to_value(&self.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null);
                        ::suprnova::MorphToMany::<Self, #target_ty, #pivot_ty>::__new(
                            parent_value,
                            ::std::string::String::from(#parent_morph_type),
                            ::std::string::String::from(#morph_name),
                            #pivot_table_expr,
                            ::std::string::String::from(#pivot_related),
                        )
                        #local_key_chain
                        #related_key_chain
                        #with_pivot_chain
                        #with_timestamps_chain
                        .__lazy_load(#lazy_load)
                    }
                }
            })
        }
        RelationKindAttr::MorphedByMany => {
            // Polymorphic m2m, inverse side (`tag.posts()`). The user
            // wrote `MorphedByMany<R, P>` where R is one specific
            // morph target family (Post) and P is the polymorphic
            // pivot model (Taggable). One declaration per target type.
            //
            // The parser enforces presence of `target_morph_type =
            // "..."`; the macro at `Self`'s expansion site can't
            // introspect R's `morph_type` attribute. Default the rest
            // of the keys from Laravel conventions.
            let pivot_ty = rel.through.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    method_ident,
                    "MorphedByMany requires a pivot type (see parse-time validation)",
                )
            })?;

            let morph_name = morph_name_or_default(rel);
            let target_morph_type = target_morph_type_override(rel).ok_or_else(|| {
                syn::Error::new_spanned(
                    method_ident,
                    "MorphedByMany requires `target_morph_type = \"...\"` \
                         (parse-time validation should reject this earlier)",
                )
            })?;
            // pivot_foreign_key (pivot column → L=Tag): <snake(parent_struct)>_id.
            let pivot_fk = pivot_fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&parent_name)));
            let pivot_table_expr: TokenStream = match pivot_table_override(rel) {
                Some(t) => quote! { ::std::string::String::from(#t) },
                None => quote! {
                    <#pivot_ty as ::suprnova::eloquent::EloquentModel>::TABLE.to_string()
                },
            };
            let lk = lk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| pk_name.clone());
            let local_key_chain = if lk == "id" {
                quote! {}
            } else {
                quote! { .local_key(#lk) }
            };
            let related_key = related_key_expr(rel, target_ty);
            let related_key_chain = quote! { .related_pk(#related_key) };

            Ok(quote! {
                impl #struct_ident {
                    #[doc = "Construct a `MorphedByMany` inverse polymorphic m2m relation \
                             for this row."]
                    #[doc = ""]
                    #[doc = "Filters to one specific morph target family per declaration \
                             via `target_morph_type = \"...\"`. `.get()` returns rows of \
                             that family only - never mixing target families in a single \
                             collection."]
                    pub fn #method_ident(&self)
                        -> ::suprnova::MorphedByMany<Self, #target_ty, #pivot_ty>
                    {
                        let tag_value = ::suprnova::serde_json::to_value(&self.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null);
                        ::suprnova::MorphedByMany::<Self, #target_ty, #pivot_ty>::__new(
                            tag_value,
                            ::std::string::String::from(#target_morph_type),
                            ::std::string::String::from(#morph_name),
                            #pivot_table_expr,
                            ::std::string::String::from(#pivot_fk),
                        )
                        #local_key_chain
                        #related_key_chain
                        .__lazy_load(#lazy_load)
                    }
                }
            })
        }
    }
}

/// Emit a `let mut __sn_pred = ...;` token stream that extracts an
/// optional typed predicate closure from the dispatcher's
/// `predicate: Option<Box<dyn Any>>` parameter.
///
/// The closure shape is
/// `Box<dyn FnOnce(Builder<R>) -> Builder<R> + Send + Sync + 'static>` -
/// exactly what `Builder::with_where` boxes up. On a well-typed
/// program the downcast targets the statically-known `#target_ty` and
/// succeeds. If the caller spelled the closure's `Builder<T>` parameter
/// with the WRONG `T` for the named relation (e.g.
/// `.with_where(("posts", |q: Builder<Comment>| ...))` against a
/// `posts: HasMany<Post>` declaration), the closure was boxed under
/// `Box<dyn Any>` with the wrong typed shape and the downcast fails.
///
/// Failing silently here is the bug: the predicate gets dropped and the
/// eager-load runs UNFILTERED - the user thinks they constrained the
/// relation when they didn't. The emitted extractor turns that into a
/// loud `FrameworkError::internal` that names the relation and the
/// statically-known target type, so the dispatcher returns `Err` and
/// the call site sees the mismatch instead of silent corruption.
///
/// Emits a let-binding `__sn_pred: Option<Box<dyn FnOnce(Builder<R>) ->
/// Builder<R>>>`. The matching arm consumes the closure exactly once
/// before issuing `.get()`. Other arms ignore the predicate entirely
/// (the binding is shadowed by `_` later).
fn emit_predicate_extractor(target_ty: &syn::Type, name_str: &str) -> TokenStream {
    quote! {
        let mut __sn_pred: ::std::option::Option<
            ::std::sync::Arc<
                dyn ::core::ops::Fn(
                        ::suprnova::Builder<#target_ty>,
                    ) -> ::suprnova::Builder<#target_ty>
                    + ::core::marker::Send
                    + ::core::marker::Sync
                    + 'static,
            >,
        > = match predicate.take() {
            ::core::option::Option::None => ::core::option::Option::None,
            ::core::option::Option::Some(p) => {
                match p.downcast::<
                    ::std::sync::Arc<
                        dyn ::core::ops::Fn(
                                ::suprnova::Builder<#target_ty>,
                            )
                                -> ::suprnova::Builder<#target_ty>
                            + ::core::marker::Send
                            + ::core::marker::Sync
                            + 'static,
                    >,
                >() {
                    ::core::result::Result::Ok(b) => ::core::option::Option::Some((*b).clone()),
                    ::core::result::Result::Err(_) => {
                        return ::core::result::Result::Err(
                            ::suprnova::FrameworkError::internal(::std::format!(
                                "with_where(`{}`, ...) closure type mismatch \
                                 \u{2014} expected `Builder<{}>`",
                                #name_str,
                                ::std::any::type_name::<#target_ty>(),
                            )),
                        );
                    }
                }
            }
        };
    }
}

/// Emit a `<name> => { ... }` arm for `__eager_load`. T2 owns HasOne
/// and BelongsTo; other kinds return `None` (no arm).
fn emit_eager_arm(input: &ModelInput, rel: &RelationDecl) -> Result<Option<TokenStream>> {
    let struct_ident = &input.item.ident;
    let name_str = rel.name.to_string();
    let pk_ident = local_key_ident(input, rel)?;
    let target_ty = &rel.target;
    let parent_name = struct_ident.to_string();

    match rel.kind {
        RelationKindAttr::HasOne => {
            // FK column on the child table.
            let fk = fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| default_has_fk(&parent_name));

            // Predicate extractor - see HasMany arm for the full
            // contract. The `with_where(("profile", |q| ...))` user
            // call lands here type-erased; the downcast targets the
            // statically-known `#target_ty`.
            let pred_extractor = emit_predicate_extractor(target_ty, &name_str);

            // Build a JSON Vec of parent PK values, issue an
            // `IN (...)` against the child table, group by FK on each
            // returned row, and stuff into each parent's `__eager`.
            //
            // The FK is read off the target row by the target's own
            // `Model::field_value(&r, #fk)`, through `eager_row_column`,
            // rather than `r.<fk_ident>` field access. The field-access
            // form would force the macro to assume the target struct
            // declared a field by exactly that ident, which it can't
            // verify (the target's `#[model]` invocation is a separate
            // macro expansion). The column is serialized alone; the whole
            // row is serialized only when the target's `field_value` does
            // not know the column.
            //
            // PK values use `serde_json::to_value(&p.<pk>)`
            // serialisation as `HashMap` keys so the lookup is total
            // across PK shapes (i64 / String / Uuid-via-string).
            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }
                    #pred_extractor
                    let pk_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();
                    let __sn_builder: ::suprnova::Builder<#target_ty> =
                        <#target_ty as ::suprnova::eloquent::Model>::query()
                            .filter_in(#fk, pk_values);
                    let __sn_builder = match __sn_pred.take() {
                        ::core::option::Option::Some(f) => f(__sn_builder),
                        ::core::option::Option::None => __sn_builder,
                    };
                    let rows: ::std::vec::Vec<#target_ty> =
                        __sn_builder.get().await?.into_vec();
                    use ::std::collections::HashMap;
                    let mut by_fk: HashMap<::std::string::String, #target_ty> = HashMap::new();
                    for r in rows.into_iter() {
                        let key = ::suprnova::eloquent::relations::eager_row_column(
                            &r,
                            ::suprnova::eloquent::Model::field_value(&r, #fk),
                            #fk,
                        )
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        by_fk.insert(key, r);
                    }
                    for p in parents.iter_mut() {
                        let key = ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        let row = by_fk.remove(&key);
                        p.__eager.set_one::<#target_ty>(#name_str, row);
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::BelongsTo => {
            // FK on the child = <snake(target)>_id by default.
            let fk = fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| default_belongs_to_fk(target_ty));
            let fk_ident = quote::format_ident!("{}", fk);
            // Owner key on the parent.
            let owner_key = lk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| "id".to_string());
            let fk_is_optional = field_is_optional(input, &fk);
            let with_default_chain = match with_default_expr(rel) {
                Some(expr) => quote! { .with_default(#expr) },
                None => quote! {},
            };

            // For Option<T> FKs the per-row JSON extraction unwraps
            // the inner value; for non-Option FKs it's always present.
            let per_parent_key_expr = if fk_is_optional {
                quote! {
                    p.#fk_ident
                        .as_ref()
                        .and_then(|v| ::suprnova::serde_json::to_value(v).ok())
                }
            } else {
                quote! {
                    ::core::option::Option::Some(
                        ::suprnova::serde_json::to_value(&p.#fk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null),
                    )
                }
            };

            // Predicate extractor - see HasMany arm for the full
            // contract. The `with_where(("user", |q| ...))` user
            // call lands here type-erased; downcast targets
            // `#target_ty`.
            let pred_extractor = emit_predicate_extractor(target_ty, &name_str);

            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }
                    #pred_extractor
                    // Distinct FK values to query (skip null FKs).
                    let fk_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .filter_map(|p| {
                            let v: ::core::option::Option<::suprnova::serde_json::Value> =
                                #per_parent_key_expr;
                            v
                        })
                        .collect();
                    let parent_rows: ::std::vec::Vec<#target_ty> = if fk_values.is_empty() {
                        ::std::vec::Vec::new()
                    } else {
                        let __sn_builder: ::suprnova::Builder<#target_ty> =
                            <#target_ty as ::suprnova::eloquent::Model>::query()
                                .filter_in(#owner_key, fk_values);
                        let __sn_builder = match __sn_pred.take() {
                            ::core::option::Option::Some(f) => f(__sn_builder),
                            ::core::option::Option::None => __sn_builder,
                        };
                        __sn_builder.get().await?.into_vec()
                    };
                    use ::std::collections::HashMap;
                    // Group parents by their PK (which is matched by
                    // the BelongsTo's owner_key) as JSON-encoded string.
                    // The target's owner-key column resolution at
                    // emission time uses the primary_key field name
                    // unless the user overrode `lk = "..."`. T2 names
                    // the parent's PK field via `<owner_key>` directly
                    // as an ident, which assumes the parent struct
                    // declared a field by that name. Models with a
                    // non-`id` PK can use `lk = "<pk>"` to align.
                    let mut by_pk: HashMap<::std::string::String, #target_ty> = HashMap::new();
                    for row in parent_rows.into_iter() {
                        // The owner-key column is read off the parent
                        // target through its `field_value` - works
                        // uniformly for any field name the user wrote,
                        // without requiring the macro here to know the
                        // parent struct's field layout.
                        let key = ::suprnova::eloquent::relations::eager_row_column(
                            &row,
                            ::suprnova::eloquent::Model::field_value(&row, #owner_key),
                            #owner_key,
                        )
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        by_pk.insert(key, row);
                    }
                    // Per parent row: look up the parent by FK; if
                    // missing OR FK was null, invoke the
                    // `with_default` closure (if installed). The
                    // lookup is `.get().cloned()` rather than
                    // `.remove()` because multiple children can share
                    // the same FK and each needs its own copy.
                    for p in parents.iter_mut() {
                        let p_fk_json: ::core::option::Option<::suprnova::serde_json::Value> =
                            #per_parent_key_expr;
                        let parent_row: ::core::option::Option<#target_ty> = match &p_fk_json {
                            ::core::option::Option::Some(v) => {
                                by_pk.get(&v.to_string()).cloned().or_else(|| {
                                    // Parent missing - invoke
                                    // `with_default` closure if
                                    // installed.
                                    let tmpl: ::suprnova::BelongsTo<Self, #target_ty> =
                                        ::suprnova::BelongsTo::<Self, #target_ty>::__new(
                                            ::core::option::Option::None,
                                            ::std::string::String::from(#fk),
                                            ::std::string::String::from(#owner_key),
                                        )#with_default_chain;
                                    tmpl.__default_fn().map(|f| f())
                                })
                            }
                            ::core::option::Option::None => {
                                // FK is null - same `with_default` path.
                                let tmpl: ::suprnova::BelongsTo<Self, #target_ty> =
                                    ::suprnova::BelongsTo::<Self, #target_ty>::__new(
                                        ::core::option::Option::None,
                                        ::std::string::String::from(#fk),
                                        ::std::string::String::from(#owner_key),
                                    )#with_default_chain;
                                tmpl.__default_fn().map(|f| f())
                            }
                        };
                        p.__eager.set_one::<#target_ty>(#name_str, parent_row);
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::HasMany => {
            // FK column on the child table - same default as HasOne.
            let fk = fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| default_has_fk(&parent_name));

            // Predicate extractor - downcasts the dispatcher's
            // `predicate` parameter to a typed
            // `FnOnce(Builder<R>) -> Builder<R>` and binds it as
            // `__sn_pred`. The arm body applies it just before
            // `.get()` on the inner builder.
            let pred_extractor = emit_predicate_extractor(target_ty, &name_str);

            // Same `eager_row_column` FK-reading pattern as HasOne's eager
            // arm - see the long-form comment there for why we don't
            // do field-access on the target struct. The difference is
            // we accumulate into `HashMap<key, Vec<R>>` rather than
            // `HashMap<key, R>`, and stuff via `set_many` instead of
            // `set_one`. Parents whose group is empty still get an
            // explicit `set_many(name, Vec::new())` so the loaded
            // accessor returns `&[]` (not a panic).
            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }
                    #pred_extractor
                    let pk_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();
                    let __sn_builder: ::suprnova::Builder<#target_ty> =
                        <#target_ty as ::suprnova::eloquent::Model>::query()
                            .filter_in(#fk, pk_values);
                    let __sn_builder = match __sn_pred.take() {
                        ::core::option::Option::Some(f) => f(__sn_builder),
                        ::core::option::Option::None => __sn_builder,
                    };
                    let rows: ::std::vec::Vec<#target_ty> =
                        __sn_builder.get().await?.into_vec();
                    use ::std::collections::HashMap;
                    let mut by_fk: HashMap<::std::string::String, ::std::vec::Vec<#target_ty>>
                        = HashMap::new();
                    for r in rows.into_iter() {
                        let key = ::suprnova::eloquent::relations::eager_row_column(
                            &r,
                            ::suprnova::eloquent::Model::field_value(&r, #fk),
                            #fk,
                        )
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        by_fk.entry(key).or_default().push(r);
                    }
                    for p in parents.iter_mut() {
                        let key = ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        let group = by_fk.remove(&key).unwrap_or_default();
                        p.__eager.set_many::<#target_ty>(#name_str, group);
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::BelongsToMany => {
            // Pivot model + key names. The parser already validates a
            // pivot type exists for BelongsToMany; the `expect` is
            // unreachable on the happy path but defensive.
            let pivot_ty = rel.through.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "BelongsToMany requires a pivot type (parser bug if reached)",
                )
            })?;
            let pivot_fk = pivot_fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&parent_name)));
            let pivot_related = pivot_related_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&last_segment_name(target_ty))));
            // Two-query strategy:
            //
            // 1. Fetch all pivot rows whose FK points at any of the
            //    parent PKs in this batch.
            // 2. Fetch all related rows whose PK is in the set of
            //    pivot.related_key values.
            // 3. Walk each pivot row, look up the matching related
            //    row, clone it, stamp `__pivot = Some(Arc::new(pivot))`
            //    onto the clone, and push into the per-parent vec keyed
            //    by pivot.foreign_key.
            //
            // The clone-per-attachment is load-bearing: when a single
            // R is attached to multiple Ls via different pivot rows,
            // each L's copy must carry its OWN pivot context. The
            // `Model: Clone` supertrait makes this cheap (no new
            // bounds needed on this arm).
            // Predicate extractor - `with_where(("roles", |q| ...))`
            // applies its closure to the RELATED-table query (not the
            // pivot scan). The downcast targets `#target_ty`.
            let pred_extractor = emit_predicate_extractor(target_ty, &name_str);
            // The related column the pivot's related key holds - see
            // `related_key_expr`.
            let related_key = related_key_expr(rel, target_ty);
            // The pivot table the relation names - see `pivot_table_str`.
            let pivot_table = pivot_table_str(rel, pivot_ty);

            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }
                    #pred_extractor
                    let pk_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();

                    // Step 1: pivot rows where FK ∈ pk_values, read from
                    // the relation's own pivot table.
                    let pivots: ::std::vec::Vec<#pivot_ty> =
                        ::suprnova::eloquent::relations::belongs_to_many::__eager_pivot_rows::<#pivot_ty>(
                            #pivot_table,
                            <Self as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                            #pivot_fk,
                            pk_values.clone(),
                            ::core::option::Option::None,
                        )
                        .await?;

                    if pivots.is_empty() {
                        // Every parent gets an empty slice so the
                        // loaded accessor returns `&[]` instead of
                        // panicking.
                        for p in parents.iter_mut() {
                            p.__eager.set_many::<#target_ty>(
                                #name_str,
                                ::std::vec::Vec::<#target_ty>::new(),
                            );
                        }
                        return ::core::result::Result::Ok(());
                    }

                    // Collect the distinct related-key values for the
                    // IN query.
                    use ::std::collections::HashMap;
                    let mut related_ids: ::std::vec::Vec<::suprnova::serde_json::Value>
                        = ::std::vec::Vec::with_capacity(pivots.len());
                    let mut seen_rel: ::std::collections::HashSet<::std::string::String>
                        = ::std::collections::HashSet::new();
                    for pv in pivots.iter() {
                        if let ::core::option::Option::Some(v) = ::suprnova::eloquent::relations::eager_row_column(
                            pv,
                            ::suprnova::eloquent::Model::field_value(pv, #pivot_related),
                            #pivot_related,
                        ) {
                            let s = v.to_string();
                            if seen_rel.insert(s) {
                                related_ids.push(v);
                            }
                        }
                    }

                    // Step 2: related rows whose related key is in
                    // related_ids - the declared `related_key`, else the
                    // related model's primary key, as the lazy read uses.
                    let related_rows: ::std::vec::Vec<#target_ty> = if related_ids.is_empty() {
                        ::std::vec::Vec::new()
                    } else {
                        let __sn_builder: ::suprnova::Builder<#target_ty> =
                            <#target_ty as ::suprnova::eloquent::Model>::query()
                                .filter_in(#related_key, related_ids);
                        let __sn_builder = match __sn_pred.take() {
                            ::core::option::Option::Some(f) => f(__sn_builder),
                            ::core::option::Option::None => __sn_builder,
                        };
                        __sn_builder.get().await?.into_vec()
                    };

                    // Index related rows by their related key (JSON-
                    // string form) for fast lookup.
                    let mut by_related_id: HashMap<::std::string::String, #target_ty>
                        = HashMap::new();
                    for r in related_rows.into_iter() {
                        let key = ::suprnova::eloquent::relations::eager_row_column(
                            &r,
                            ::suprnova::eloquent::Model::field_value(&r, #related_key),
                            #related_key,
                        )
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        by_related_id.insert(key, r);
                    }

                    // Step 3: per pivot row, clone the matching
                    // related row, stamp __pivot, and append to the
                    // per-parent vec.
                    let mut by_parent: HashMap<
                        ::std::string::String,
                        ::std::vec::Vec<#target_ty>,
                    > = HashMap::new();
                    for pv in pivots.into_iter() {
                        let parent_key = ::suprnova::eloquent::relations::eager_row_column(
                            &pv,
                            ::suprnova::eloquent::Model::field_value(&pv, #pivot_fk),
                            #pivot_fk,
                        )
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        let related_key = ::suprnova::eloquent::relations::eager_row_column(
                            &pv,
                            ::suprnova::eloquent::Model::field_value(&pv, #pivot_related),
                            #pivot_related,
                        )
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        if let ::core::option::Option::Some(template)
                            = by_related_id.get(&related_key)
                        {
                            let mut row: #target_ty = template.clone();
                            row.__pivot = ::core::option::Option::Some(
                                ::std::sync::Arc::new(pv),
                            );
                            by_parent.entry(parent_key).or_default().push(row);
                        }
                    }

                    // Distribute per parent. Parents with no
                    // attachments get an explicit empty slice.
                    for p in parents.iter_mut() {
                        let key = ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        let group = by_parent.remove(&key).unwrap_or_default();
                        p.__eager.set_many::<#target_ty>(#name_str, group);
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::HasManyThrough | RelationKindAttr::HasOneThrough => {
            // Two-query eager-load strategy (cleaner than a single
            // JOIN-with-extra-column because we get to reuse the
            // existing `Builder<C>` SeaORM deserialisation path):
            //
            // 1. Raw SQL: `SELECT id, {first_key} FROM B WHERE
            //    {first_key} IN (parent_ids)` - build a map
            //    `b_id -> parent_id`.
            // 2. `<C as Model>::query().filter_in({second_key}, b_ids)
            //    .get()` - uses the existing model pipeline so C
            //    deserialises correctly even with casts / accessors.
            // 3. Group C by `row.{second_key}` (which is a B.id) →
            //    look up the parent_id via the map → distribute via
            //    `set_many` (HasManyThrough) or `set_one`
            //    (HasOneThrough - first row wins per parent).
            //
            // Type rebinding: for Through kinds the parser stores
            // `(rel.target, rel.through)` as `(B, C)` - same swap as
            // `emit_relation_accessors`. We shadow the function-scope
            // `target_ty` (which would be `B`) with the final target
            // `C` taken from `rel.through`.
            let through_ty = &rel.target; // intermediate B
            let target_ty = rel.through.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "HasOneThrough / HasManyThrough require a final target type \
                     (parser bug if reached)",
                )
            })?; // final target C
            let first_key = first_key_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&parent_name)));
            let second_key = second_key_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&last_segment_name(through_ty))));
            // Column on B matched by `second_key`. Defaults to "id";
            // overridable for intermediates declaring a non-`id` PK
            // via `second_local_key = "..."`. Query 1 below `SELECT`s
            // this column as `__sn_b_id` so the b->parent map keys
            // off the correct join target.
            let second_local_key = second_local_key_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| "id".to_string());
            let is_one = matches!(rel.kind, RelationKindAttr::HasOneThrough);
            // Distribute branch: HasOneThrough stores `set_one`
            // (None if no row); HasManyThrough stores `set_many`
            // (empty Vec if no rows). Per-parent group reduction
            // happens client-side over the already-grouped HashMap.
            //
            // Key shape: `__sn_parent_key_to_match_cast` (declared in
            // the outer arm body below) unwraps `Value::String(s)` to
            // raw `s` rather than the JSON-quoted form, so String PKs
            // line up with the `CAST(... AS TEXT)` column on the
            // `b_to_parent` lookup. The count and aggregate arms use
            // the same helper for the same reason - the eager arm
            // previously used `to_value(...).to_string()` directly,
            // which produced `"\"abc\""` for String PKs and silently
            // missed the lookup.
            let distribute = if is_one {
                quote! {
                    for p in parents.iter_mut() {
                        let pk_str = __sn_parent_key_to_match_cast(
                            ::suprnova::serde_json::to_value(&p.#pk_ident)
                                .unwrap_or(::suprnova::serde_json::Value::Null),
                        );
                        let row: ::core::option::Option<#target_ty> =
                            by_parent.remove(&pk_str).and_then(|mut g| g.pop());
                        p.__eager.set_one::<#target_ty>(#name_str, row);
                    }
                }
            } else {
                quote! {
                    for p in parents.iter_mut() {
                        let pk_str = __sn_parent_key_to_match_cast(
                            ::suprnova::serde_json::to_value(&p.#pk_ident)
                                .unwrap_or(::suprnova::serde_json::Value::Null),
                        );
                        let group = by_parent.remove(&pk_str).unwrap_or_default();
                        p.__eager.set_many::<#target_ty>(#name_str, group);
                    }
                }
            };

            // Predicate extractor - `with_where(("posts", |q| ...))`
            // applies its closure to the final-target `C` query.
            let pred_extractor = emit_predicate_extractor(target_ty, &name_str);

            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }
                    #pred_extractor

                    // Per-parent FK-key derivation - matches the SQL
                    // CAST output of Query 1 below. `Value::String(s)`
                    // unwraps to raw `s` rather than the JSON-quoted
                    // form so the String PK case lines up with the raw
                    // `CAST(... AS TEXT)` result on the b->parent map.
                    // Mirrors the helper in the count and aggregate
                    // arms; spliced into both `#distribute` branches.
                    fn __sn_parent_key_to_match_cast(
                        v: ::suprnova::serde_json::Value,
                    ) -> ::std::string::String {
                        match v {
                            ::suprnova::serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        }
                    }

                    let pk_json_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();

                    // Phase 10C audit-fix AF1 - resolve through ExecutorChoice so
                    // raw eager-load SQL honors any ambient CURRENT_TX. Without
                    // this every leaf would run on `DB::connection()` (the pool)
                    // and miss in-tx state under a `DB::transaction` closure.
                    let __sn_exec = ::suprnova::database::transaction::ExecutorChoice::resolve_read(
                        ::core::option::Option::None,
                        ::core::option::Option::None,
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                    ).await?;
                    let db_backend = __sn_exec.backend();

                    // Backend-aware placeholder rendering for the
                    // IN-list on the intermediate table query.
                    let mut placeholders: ::std::vec::Vec<::std::string::String> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    let mut binds: ::std::vec::Vec<::suprnova::sea_orm::Value> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    for (i, v) in pk_json_values.iter().enumerate() {
                        let ph = match db_backend {
                            ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                                ::std::format!("${}", i + 1)
                            }
                            _ => ::std::string::String::from("?"),
                        };
                        placeholders.push(ph);
                        binds.push(
                            ::suprnova::eloquent::model::json_value_to_sea_value(v),
                        );
                    }

                    let __sn_b_table = <#through_ty as
                        ::suprnova::eloquent::EloquentModel>::TABLE;

                    // Query 1 - pull (b_id, parent_id) mapping. We
                    // CAST both columns to TEXT/CHAR so the
                    // HashMap key shape lines up regardless of the
                    // underlying integer vs string column type. The
                    // `Value::String(s) => s` normalisation on the
                    // parent side matches what HasMany's count arm
                    // does.
                    let __sn_cast_kw = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::MySql => "CHAR",
                        _ => "TEXT",
                    };
                    // A trashed intermediate maps to no parent, so its
                    // rows are left out, as the lazy read leaves them out.
                    let __sn_map_sql = ::std::format!(
                        "SELECT CAST({slk} AS {cast}) AS __sn_b_id, \
                                CAST({fk} AS {cast}) AS __sn_parent_id \
                           FROM {table} \
                          WHERE {fk} IN ({phs}){alive}",
                        cast = __sn_cast_kw,
                        fk = #first_key,
                        slk = #second_local_key,
                        table = __sn_b_table,
                        phs = placeholders.join(", "),
                        alive = ::suprnova::eloquent::relations::__soft_delete_guard::<#through_ty>(
                            __sn_b_table,
                        ),
                    );
                    let __sn_map_stmt = ::suprnova::sea_orm::Statement::from_sql_and_values(
                        db_backend,
                        &__sn_map_sql,
                        binds,
                    );
                    // DATA-033: this raw read names its tables to the render
                    // cache, so a write to any of them invalidates the page.
                    ::suprnova::render_cache::collector::observe_table_read(__sn_b_table);
                    let __sn_map_rows = __sn_exec.query_all(__sn_map_stmt)
                        .await
                        .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;

                    use ::std::collections::HashMap;
                    // b_id -> parent_id (both as string keys).
                    let mut b_to_parent: HashMap<::std::string::String, ::std::string::String>
                        = HashMap::new();
                    // The IN-set of B's `id`s - we re-issue Query 2 on
                    // C with these, keeping the existing model-level
                    // typed deserialisation.
                    let mut b_ids: ::std::vec::Vec<::suprnova::serde_json::Value>
                        = ::std::vec::Vec::with_capacity(__sn_map_rows.len());
                    for r in __sn_map_rows.iter() {
                        let b_id = r
                            .try_get::<::std::string::String>("", "__sn_b_id")
                            .unwrap_or_default();
                        let parent_id = r
                            .try_get::<::std::string::String>("", "__sn_parent_id")
                            .unwrap_or_default();
                        if b_id.is_empty() { continue; }
                        b_ids.push(::suprnova::serde_json::Value::from(b_id.clone()));
                        b_to_parent.insert(b_id, parent_id);
                    }

                    // Group container declared up-front so the
                    // `#distribute` block (which `.remove()`s per
                    // parent key) compiles for both the empty-set
                    // short-circuit AND the populated path. Empty
                    // `b_ids` means no rows go in; per-parent
                    // distribution still runs so every parent gets
                    // an explicit empty cache entry (not a panic).
                    let mut by_parent: HashMap<
                        ::std::string::String,
                        ::std::vec::Vec<#target_ty>,
                    > = HashMap::new();

                    // Short-circuit when no intermediate rows match -
                    // every parent gets an empty / None entry so the
                    // loaded accessor doesn't panic on "you forgot
                    // `with([\"...\"])`".
                    if b_ids.is_empty() {
                        #distribute
                        return ::core::result::Result::Ok(());
                    }

                    // Query 2 - pull C rows via the existing
                    // Model::query() pipeline. `filter_in` runs the
                    // same bind / placeholder / typed-deserialisation
                    // path the rest of the framework uses.
                    //
                    // T9 with_where: predicate (if present) applies
                    // to the final-target `C` query, downcast to
                    // `Box<dyn FnOnce(Builder<C>) -> Builder<C>>` via
                    // `__sn_pred` (extracted at the arm top).
                    let __sn_builder: ::suprnova::Builder<#target_ty> =
                        <#target_ty as ::suprnova::eloquent::Model>::query()
                            .filter_in(#second_key, b_ids);
                    let __sn_builder = match __sn_pred.take() {
                        ::core::option::Option::Some(f) => f(__sn_builder),
                        ::core::option::Option::None => __sn_builder,
                    };
                    let c_rows: ::std::vec::Vec<#target_ty> = __sn_builder.get().await?.into_vec();

                    // Group C rows by parent_id, via the b->parent
                    // map. The per-row C.second_key is read through
                    // `eager_row_column` (as in HasMany's eager arm) and
                    // normalised to the raw string form so it lines
                    // up with the CAST-as-TEXT keys in `b_to_parent`.
                    for r in c_rows.into_iter() {
                        let b_id_key = match ::suprnova::eloquent::relations::eager_row_column(
                            &r,
                            ::suprnova::eloquent::Model::field_value(&r, #second_key),
                            #second_key,
                        ) {
                            ::core::option::Option::Some(
                                ::suprnova::serde_json::Value::String(s),
                            ) => s,
                            ::core::option::Option::Some(other) => other.to_string(),
                            ::core::option::Option::None => ::std::string::String::new(),
                        };
                        if let ::core::option::Option::Some(parent_id)
                            = b_to_parent.get(&b_id_key)
                        {
                            by_parent
                                .entry(parent_id.clone())
                                .or_default()
                                .push(r);
                        }
                    }

                    // Distribute - branches on HasOne vs HasMany
                    // through the `#distribute` token block above.
                    #distribute
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::MorphMany | RelationKindAttr::MorphOne => {
            // Polymorphic eager-load. Same shape as the HasMany arm
            // (IN-query by parent IDs, group by FK, distribute into
            // each parent's `__eager` cache) but with an additional
            // `<name>_type = '<morph_type>'` predicate so children of
            // OTHER morph families (e.g. comments on Video when the
            // parent is Post) are excluded.
            //
            // The id column on the child = `<morph_name>_id` and the
            // type column = `<morph_name>_type` - both baked from
            // `morph_name` (which defaults to the relation name).
            //
            // MorphOne distributes via `set_one` (first row wins per
            // parent); MorphMany distributes via `set_many`.
            let morph_name = morph_name_or_default(rel);
            let morph_type_value = morph_type_of(input);
            let id_col = format!("{morph_name}_id");
            let type_col = format!("{morph_name}_type");
            let is_one = matches!(rel.kind, RelationKindAttr::MorphOne);
            let distribute = if is_one {
                quote! {
                    for p in parents.iter_mut() {
                        let key = ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        // First row wins (per HasOne semantics) - we
                        // sort by id ASC implicitly via the order the
                        // groups were built, but explicit `LIMIT 1`
                        // logic isn't worth a separate dispatch path
                        // because MorphOne is by contract 0-or-1 row.
                        let row = by_fk.remove(&key).and_then(|mut v| v.pop());
                        p.__eager.set_one::<#target_ty>(#name_str, row);
                    }
                }
            } else {
                quote! {
                    for p in parents.iter_mut() {
                        let key = ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        let group = by_fk.remove(&key).unwrap_or_default();
                        p.__eager.set_many::<#target_ty>(#name_str, group);
                    }
                }
            };
            // Predicate extractor - applies to the child-table query
            // before the IN + type filter are issued.
            let pred_extractor = emit_predicate_extractor(target_ty, &name_str);

            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }
                    #pred_extractor
                    let pk_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();
                    // Type-string predicate goes through the
                    // standard `filter` path; the inner builder
                    // serialises it to a bind parameter via
                    // `IntoVal`. We pre-wrap it in a JSON `String`
                    // value so the WhereTerm storage stays
                    // homogeneous with the IN-list above.
                    let morph_type_predicate =
                        ::suprnova::serde_json::Value::String(
                            ::std::string::String::from(#morph_type_value),
                        );
                    let __sn_builder: ::suprnova::Builder<#target_ty> =
                        <#target_ty as ::suprnova::eloquent::Model>::query()
                            .filter_in(#id_col, pk_values)
                            .filter(#type_col, morph_type_predicate);
                    let __sn_builder = match __sn_pred.take() {
                        ::core::option::Option::Some(f) => f(__sn_builder),
                        ::core::option::Option::None => __sn_builder,
                    };
                    let rows: ::std::vec::Vec<#target_ty> = __sn_builder.get().await?.into_vec();
                    use ::std::collections::HashMap;
                    let mut by_fk: HashMap<::std::string::String, ::std::vec::Vec<#target_ty>>
                        = HashMap::new();
                    for r in rows.into_iter() {
                        // Read the morph-id column off the returned row
                        // through `eager_row_column` - as in the HasMany
                        // arm. Avoids requiring the macro at THIS
                        // expansion site to know the target struct's
                        // field layout.
                        let key = ::suprnova::eloquent::relations::eager_row_column(
                            &r,
                            ::suprnova::eloquent::Model::field_value(&r, #id_col),
                            #id_col,
                        )
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        by_fk.entry(key).or_default().push(r);
                    }
                    #distribute
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::MorphToMany => {
            // Polymorphic m2m eager load - same two-query strategy as
            // BelongsToMany (T4), with the morph `<name>_type` filter
            // layered on top so pivot rows pointing at other morph
            // families are excluded.
            //
            //   1. SELECT pivot rows WHERE <name>_id IN (parent ids)
            //                          AND <name>_type = '<self_morph_type>'.
            //   2. SELECT related rows WHERE id IN (pivot.related_key).
            //   3. Per pivot row, clone the matching related row, stamp
            //      `__pivot`, append into the per-parent vec keyed by
            //      pivot.<name>_id.
            let pivot_ty = rel.through.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "MorphToMany requires a pivot type (parser bug if reached)",
                )
            })?;
            let morph_name = morph_name_or_default(rel);
            let parent_morph_type = morph_type_of(input);
            let id_col = format!("{morph_name}_id");
            let type_col = format!("{morph_name}_type");
            let pivot_related = pivot_related_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&last_segment_name(target_ty))));
            // Predicate extractor - `with_where` applies to the
            // RELATED-table query (Step 2), mirroring BelongsToMany.
            let pred_extractor = emit_predicate_extractor(target_ty, &name_str);
            // The related column the pivot's related key holds - see
            // `related_key_expr`.
            let related_key = related_key_expr(rel, target_ty);
            // The pivot table the relation names - see `pivot_table_str`.
            let pivot_table = pivot_table_str(rel, pivot_ty);
            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }
                    #pred_extractor
                    let pk_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();

                    // Step 1: pivot rows where <name>_id ∈ pk_values
                    //         AND <name>_type = '<self_morph_type>', read
                    //         from the relation's own pivot table.
                    let pivots: ::std::vec::Vec<#pivot_ty> =
                        ::suprnova::eloquent::relations::belongs_to_many::__eager_pivot_rows::<#pivot_ty>(
                            #pivot_table,
                            <Self as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                            #id_col,
                            pk_values.clone(),
                            ::core::option::Option::Some((#type_col, #parent_morph_type)),
                        )
                        .await?;

                    if pivots.is_empty() {
                        for p in parents.iter_mut() {
                            p.__eager.set_many::<#target_ty>(
                                #name_str,
                                ::std::vec::Vec::<#target_ty>::new(),
                            );
                        }
                        return ::core::result::Result::Ok(());
                    }

                    use ::std::collections::HashMap;
                    let mut related_ids: ::std::vec::Vec<::suprnova::serde_json::Value>
                        = ::std::vec::Vec::with_capacity(pivots.len());
                    let mut seen_rel: ::std::collections::HashSet<::std::string::String>
                        = ::std::collections::HashSet::new();
                    for pv in pivots.iter() {
                        if let ::core::option::Option::Some(v) = ::suprnova::eloquent::relations::eager_row_column(
                            pv,
                            ::suprnova::eloquent::Model::field_value(pv, #pivot_related),
                            #pivot_related,
                        ) {
                            let s = v.to_string();
                            if seen_rel.insert(s) {
                                related_ids.push(v);
                            }
                        }
                    }

                    // Step 2: related rows by PK IN-set.
                    let related_rows: ::std::vec::Vec<#target_ty> = if related_ids.is_empty() {
                        ::std::vec::Vec::new()
                    } else {
                        let __sn_builder: ::suprnova::Builder<#target_ty> =
                            <#target_ty as ::suprnova::eloquent::Model>::query()
                                .filter_in(#related_key, related_ids);
                        let __sn_builder = match __sn_pred.take() {
                            ::core::option::Option::Some(f) => f(__sn_builder),
                            ::core::option::Option::None => __sn_builder,
                        };
                        __sn_builder.get().await?.into_vec()
                    };

                    let mut by_related_id: HashMap<::std::string::String, #target_ty>
                        = HashMap::new();
                    for r in related_rows.into_iter() {
                        let key = ::suprnova::eloquent::relations::eager_row_column(
                            &r,
                            ::suprnova::eloquent::Model::field_value(&r, #related_key),
                            #related_key,
                        )
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        by_related_id.insert(key, r);
                    }

                    // Step 3: per pivot row, clone matching related
                    // row, stamp __pivot via the EagerLoadDispatch
                    // hook, append into per-parent vec keyed by
                    // pivot.<name>_id.
                    let mut by_parent: HashMap<
                        ::std::string::String,
                        ::std::vec::Vec<#target_ty>,
                    > = HashMap::new();
                    for pv in pivots.into_iter() {
                        let parent_key = ::suprnova::eloquent::relations::eager_row_column(
                            &pv,
                            ::suprnova::eloquent::Model::field_value(&pv, #id_col),
                            #id_col,
                        )
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        let related_key = ::suprnova::eloquent::relations::eager_row_column(
                            &pv,
                            ::suprnova::eloquent::Model::field_value(&pv, #pivot_related),
                            #pivot_related,
                        )
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        if let ::core::option::Option::Some(template)
                            = by_related_id.get(&related_key)
                        {
                            let mut row: #target_ty = template.clone();
                            <#target_ty as ::suprnova::eloquent::EagerLoadDispatch>::set_pivot_arc(
                                &mut row,
                                ::core::option::Option::Some(
                                    ::std::sync::Arc::new(pv),
                                ),
                            );
                            by_parent.entry(parent_key).or_default().push(row);
                        }
                    }

                    for p in parents.iter_mut() {
                        let key = ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        let group = by_parent.remove(&key).unwrap_or_default();
                        p.__eager.set_many::<#target_ty>(#name_str, group);
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::MorphedByMany => {
            // Inverse polymorphic m2m eager load. Same two-query
            // strategy as MorphToMany, but the pivot filter switches
            // sides: filter on `pivot.<pivot_foreign_key> IN
            // (tag_ids)` AND `pivot.<name>_type = '<target_morph_type>'`.
            // The `<name>_id` column then holds R's primary-key values.
            //
            // Per-attachment cloning + `__pivot` stamping mirrors the
            // BelongsToMany / MorphToMany contract - the inverse
            // direction also surfaces pivot context via
            // `tag.posts_loaded()[0].pivot::<Taggable>()`. Even though
            // the UNIQUE constraint on (`<pfk>`, `<id_col>`,
            // `<type_col>`) typically gives one pivot row per (tag,
            // target) pair, the clone path covers the general case
            // when a deployment doesn't enforce that constraint.
            let pivot_ty = rel.through.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "MorphedByMany requires a pivot type (parser bug if reached)",
                )
            })?;
            let morph_name = morph_name_or_default(rel);
            let target_morph_type = target_morph_type_override(rel).ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "MorphedByMany requires `target_morph_type = \"...\"` \
                         (parse-time validation should reject this earlier)",
                )
            })?;
            let id_col = format!("{morph_name}_id");
            let type_col = format!("{morph_name}_type");
            let pivot_fk = pivot_fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&parent_name)));
            // Predicate extractor - `with_where` applies to the
            // TARGET-table query (Step 2). Mirrors MorphToMany.
            let pred_extractor = emit_predicate_extractor(target_ty, &name_str);
            // The related column the pivot's related key holds - see
            // `related_key_expr`.
            let related_key = related_key_expr(rel, target_ty);
            // The pivot table the relation names - see `pivot_table_str`.
            let pivot_table = pivot_table_str(rel, pivot_ty);
            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }
                    #pred_extractor
                    let pk_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();

                    // Step 1: pivot rows for these tags filtered by
                    // the declared target morph type, read from the
                    // relation's own pivot table.
                    let pivots: ::std::vec::Vec<#pivot_ty> =
                        ::suprnova::eloquent::relations::belongs_to_many::__eager_pivot_rows::<#pivot_ty>(
                            #pivot_table,
                            <Self as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                            #pivot_fk,
                            pk_values.clone(),
                            ::core::option::Option::Some((#type_col, #target_morph_type)),
                        )
                        .await?;

                    if pivots.is_empty() {
                        for p in parents.iter_mut() {
                            p.__eager.set_many::<#target_ty>(
                                #name_str,
                                ::std::vec::Vec::<#target_ty>::new(),
                            );
                        }
                        return ::core::result::Result::Ok(());
                    }

                    use ::std::collections::HashMap;
                    let mut target_ids: ::std::vec::Vec<::suprnova::serde_json::Value>
                        = ::std::vec::Vec::with_capacity(pivots.len());
                    let mut seen_target: ::std::collections::HashSet<::std::string::String>
                        = ::std::collections::HashSet::new();
                    for pv in pivots.iter() {
                        if let ::core::option::Option::Some(v) = ::suprnova::eloquent::relations::eager_row_column(
                            pv,
                            ::suprnova::eloquent::Model::field_value(pv, #id_col),
                            #id_col,
                        ) {
                            let s = v.to_string();
                            if seen_target.insert(s) {
                                target_ids.push(v);
                            }
                        }
                    }

                    // Step 2: target rows by PK IN-set.
                    let target_rows: ::std::vec::Vec<#target_ty> = if target_ids.is_empty() {
                        ::std::vec::Vec::new()
                    } else {
                        let __sn_builder: ::suprnova::Builder<#target_ty> =
                            <#target_ty as ::suprnova::eloquent::Model>::query()
                                .filter_in(#related_key, target_ids);
                        let __sn_builder = match __sn_pred.take() {
                            ::core::option::Option::Some(f) => f(__sn_builder),
                            ::core::option::Option::None => __sn_builder,
                        };
                        __sn_builder.get().await?.into_vec()
                    };

                    // Index targets by id (JSON-string).
                    let mut by_target_id: HashMap<::std::string::String, #target_ty>
                        = HashMap::new();
                    for r in target_rows.into_iter() {
                        let key = ::suprnova::eloquent::relations::eager_row_column(
                            &r,
                            ::suprnova::eloquent::Model::field_value(&r, #related_key),
                            #related_key,
                        )
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        by_target_id.insert(key, r);
                    }

                    // Group: per pivot row, look up the target by its
                    // id, clone, stamp `__pivot` via the
                    // EagerLoadDispatch hook, push into per-tag vec.
                    // Mirrors the BelongsToMany / MorphToMany contract
                    // so `tag.posts_loaded()[0].pivot::<Taggable>()`
                    // works the same as the parent-side
                    // `post.tags_loaded()[0].pivot::<Taggable>()`.
                    let mut by_parent: HashMap<
                        ::std::string::String,
                        ::std::vec::Vec<#target_ty>,
                    > = HashMap::new();
                    for pv in pivots.into_iter() {
                        let tag_key = ::suprnova::eloquent::relations::eager_row_column(
                            &pv,
                            ::suprnova::eloquent::Model::field_value(&pv, #pivot_fk),
                            #pivot_fk,
                        )
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        let target_key = ::suprnova::eloquent::relations::eager_row_column(
                            &pv,
                            ::suprnova::eloquent::Model::field_value(&pv, #id_col),
                            #id_col,
                        )
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        if let ::core::option::Option::Some(template)
                            = by_target_id.get(&target_key)
                        {
                            let mut row: #target_ty = template.clone();
                            <#target_ty as ::suprnova::eloquent::EagerLoadDispatch>::set_pivot_arc(
                                &mut row,
                                ::core::option::Option::Some(
                                    ::std::sync::Arc::new(pv),
                                ),
                            );
                            by_parent.entry(tag_key).or_default().push(row);
                        }
                    }

                    for p in parents.iter_mut() {
                        let key = ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        let group = by_parent.remove(&key).unwrap_or_default();
                        p.__eager.set_many::<#target_ty>(#name_str, group);
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        // MorphTo: every parent's `<name>_type` + `<name>_id` pair is
        // read once through the relation's fetch helper, the ids are
        // grouped by the target they name, and each target that is
        // present loads with ONE IN query on its primary key - one query
        // per target type, never one per row. The ids are bound as the
        // JSON values the column holds, so any key type works. The query
        // runs through `Target::query()`, so the target's global scopes
        // and soft-delete filter apply, as they do for `BelongsTo`. Each
        // parent caches the per-family enum: the target row, or
        // `Unknown` when the type names no target, the row is absent or
        // the id is null.
        //
        // The targets are different models, so there is no single
        // `Builder<Target>` a `with_where` closure could narrow. A
        // predicate is refused rather than silently dropped.
        RelationKindAttr::MorphTo => {
            let method_ident = &rel.name;
            let enum_ident = morph_enum_ident(rel);
            let fetch_ident = quote::format_ident!("{enum_ident}Fetch");
            let targets = morph_targets(rel).ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "MorphTo requires `targets = [...]` (parser bug if reached)",
                )
            })?;
            let variant_idents = morph_variant_idents(targets);
            let per_target: Vec<TokenStream> = targets
                .iter()
                .zip(variant_idents.iter())
                .enumerate()
                .map(|(index, (ty, variant))| {
                    let index = proc_macro2::Literal::usize_unsuffixed(index);
                    quote! {
                        {
                            // Distinct non-null ids of the rows that name
                            // this target.
                            let mut __sn_ids: ::std::vec::Vec<::suprnova::serde_json::Value> =
                                ::std::vec::Vec::new();
                            let mut __sn_seen: ::std::collections::HashSet<::std::string::String> =
                                ::std::collections::HashSet::new();
                            for (link, target) in __sn_links.iter().zip(__sn_targets.iter()) {
                                if *target == ::core::option::Option::Some(#index)
                                    && !::suprnova::serde_json::Value::is_null(&link.morph_id)
                                    && __sn_seen.insert(link.morph_id.to_string())
                                {
                                    __sn_ids.push(link.morph_id.clone());
                                }
                            }
                            if !__sn_ids.is_empty() {
                                let __sn_key_col: &'static str =
                                    <#ty as ::suprnova::eloquent::EloquentModel>::PRIMARY_KEY;
                                let rows: ::std::vec::Vec<#ty> =
                                    <#ty as ::suprnova::eloquent::Model>::query()
                                        .filter_in(__sn_key_col, __sn_ids)
                                        .get()
                                        .await?
                                        .into_vec();
                                let mut by_key: ::std::collections::HashMap<::std::string::String, #ty> =
                                    ::std::collections::HashMap::new();
                                for r in rows.into_iter() {
                                    let key = <#ty as ::suprnova::eloquent::Model>::field_value(
                                        &r,
                                        __sn_key_col,
                                    )
                                    .map(|v| v.to_string())
                                    .unwrap_or_default();
                                    by_key.insert(key, r);
                                }
                                for ((link, target), loaded) in __sn_links
                                    .iter()
                                    .zip(__sn_targets.iter())
                                    .zip(__sn_loaded.iter_mut())
                                {
                                    let hit: ::core::option::Option<&#ty> =
                                        if *target == ::core::option::Option::Some(#index) {
                                            by_key.get(&link.morph_id.to_string())
                                        } else {
                                            ::core::option::Option::None
                                        };
                                    if let ::core::option::Option::Some(row) = hit {
                                        *loaded = ::core::option::Option::Some(
                                            #enum_ident::#variant(row.clone()),
                                        );
                                    }
                                }
                            }
                        }
                    }
                })
                .collect();
            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }
                    if predicate.take().is_some() {
                        return ::core::result::Result::Err(
                            ::suprnova::FrameworkError::internal(::std::format!(
                                "with_where on the MorphTo relation `{}` of `{}` is not \
                                 supported: its targets are different models, so no one \
                                 query builder can take the constraint",
                                #name_str,
                                #parent_name,
                            )),
                        );
                    }
                    // What each parent points at, and the position of the
                    // target its type string names.
                    let __sn_links: ::std::vec::Vec<#fetch_ident> =
                        parents.iter().map(|p| p.#method_ident()).collect();
                    let __sn_targets: ::std::vec::Vec<::core::option::Option<usize>> =
                        __sn_links.iter().map(|link| link.__target()).collect();
                    let mut __sn_loaded: ::std::vec::Vec<::core::option::Option<#enum_ident>> =
                        ::std::vec![::core::option::Option::None; parents.len()];
                    #( #per_target )*
                    for ((p, link), loaded) in parents
                        .iter_mut()
                        .zip(__sn_links.into_iter())
                        .zip(__sn_loaded.into_iter())
                    {
                        let value: #enum_ident = match loaded {
                            ::core::option::Option::Some(v) => v,
                            ::core::option::Option::None => {
                                #enum_ident::Unknown(link.morph_type, link.morph_id)
                            }
                        };
                        p.__eager.set_one::<#enum_ident>(
                            #name_str,
                            ::core::option::Option::Some(value),
                        );
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
    }
}

/// `__count_relation` arm for one relation. HasOne / BelongsTo both
/// produce 0-or-1 row counts; T2 wires both to keep the API uniform
/// (the spec lets `with_count(["profile"])` return 0 or 1). T3+ will
/// extend this for HasMany / BelongsToMany where COUNT actually
/// branches into real GROUP BY queries.
fn emit_count_arm(input: &ModelInput, rel: &RelationDecl) -> Result<Option<TokenStream>> {
    let struct_ident = &input.item.ident;
    let name_str = rel.name.to_string();
    let pk_ident = local_key_ident(input, rel)?;
    let target_ty = &rel.target;
    let parent_name = struct_ident.to_string();

    match rel.kind {
        RelationKindAttr::HasOne => {
            let fk = fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| default_has_fk(&parent_name));
            // Same shape as `__eager_load`: run an IN query, group by
            // FK (via `eager_row_column` - see eager arm for why), store the
            // per-parent count via `set_count`.
            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }
                    let pk_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();
                    let rows: ::std::vec::Vec<#target_ty> =
                        <#target_ty as ::suprnova::eloquent::Model>::query()
                            .filter_in(#fk, pk_values)
                            .get()
                            .await?
                            .into_vec();
                    use ::std::collections::HashMap;
                    let mut counts: HashMap<::std::string::String, u64> = HashMap::new();
                    for r in rows.iter() {
                        let key = ::suprnova::eloquent::relations::eager_row_column(
                            r,
                            ::suprnova::eloquent::Model::field_value(r, #fk),
                            #fk,
                        )
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        *counts.entry(key).or_insert(0) += 1;
                    }
                    for p in parents.iter_mut() {
                        let key = ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        p.__eager.set_count(#name_str, *counts.get(&key).unwrap_or(&0));
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::BelongsTo => {
            let fk = fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| default_belongs_to_fk(target_ty));
            let owner_key = lk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| "id".to_string());
            let fk_ident = quote::format_ident!("{}", fk);
            let fk_is_optional = field_is_optional(input, &fk);
            let per_parent_key_expr = if fk_is_optional {
                quote! {
                    p.#fk_ident
                        .as_ref()
                        .and_then(|v| ::suprnova::serde_json::to_value(v).ok())
                }
            } else {
                quote! {
                    ::core::option::Option::Some(
                        ::suprnova::serde_json::to_value(&p.#fk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null),
                    )
                }
            };
            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }
                    let fk_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .filter_map(|p| {
                            let v: ::core::option::Option<::suprnova::serde_json::Value> =
                                #per_parent_key_expr;
                            v
                        })
                        .collect();
                    let parent_rows: ::std::vec::Vec<#target_ty> = if fk_values.is_empty() {
                        ::std::vec::Vec::new()
                    } else {
                        <#target_ty as ::suprnova::eloquent::Model>::query()
                            .filter_in(#owner_key, fk_values)
                            .get()
                            .await?
                            .into_vec()
                    };
                    use ::std::collections::HashSet;
                    let mut existing_keys: HashSet<::std::string::String> = HashSet::new();
                    for r in parent_rows.iter() {
                        if let ::core::option::Option::Some(v) = ::suprnova::eloquent::relations::eager_row_column(
                            r,
                            ::suprnova::eloquent::Model::field_value(r, #owner_key),
                            #owner_key,
                        ) {
                            existing_keys.insert(v.to_string());
                        }
                    }
                    for p in parents.iter_mut() {
                        let v: ::core::option::Option<::suprnova::serde_json::Value> =
                            #per_parent_key_expr;
                        let count: u64 = match &v {
                            ::core::option::Option::Some(jv) => {
                                if existing_keys.contains(&jv.to_string()) { 1 } else { 0 }
                            }
                            ::core::option::Option::None => 0,
                        };
                        p.__eager.set_count(#name_str, count);
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::HasMany => {
            let fk = fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| default_has_fk(&parent_name));

            // Server-side `GROUP BY` count - one round trip regardless
            // of fan-out. The previous implementation fetched every
            // child row into memory and counted client-side via a
            // HashMap; at 10K children per parent that's 10K rows over
            // the wire just to learn the count. This arm issues:
            //
            //   SELECT CAST(<fk> AS TEXT) AS __sn_fk_key,
            //          COUNT(*)           AS __sn_count
            //     FROM <child_table>
            //    WHERE <fk> IN (?, ?, ...)
            //    GROUP BY <fk>
            //
            // and distributes the per-FK counts into each parent's
            // `__eager.set_count(name, n)`. Parents whose PK didn't
            // appear in any child row get 0 - set explicitly so the
            // `<rel>_count()` accessor doesn't panic on "you forgot
            // `with_count`".
            //
            // ## FK key matching
            //
            // The SQL `CAST(... AS TEXT)` form produces the raw
            // stringified column value for both integer FKs (`"42"`)
            // and string FKs (`"abc"`) on SQLite + Postgres; MySQL's
            // `CAST(... AS CHAR)` produces the same shape and the
            // backend branch below picks the right form. The
            // parent-side key is derived to MATCH that raw form: a
            // `serde_json::Value::String("abc")` is unwrapped to its
            // inner `String` rather than rendered as `"\"abc\""` via
            // `Value::to_string()`. This is internal to the dispatcher -
            // the cache key for `set_count` is the relation name,
            // not the FK key, so internal consistency is all that
            // matters.
            //
            // T4-T7's count arms should follow the same server-side
            // pattern. The HasMany aggregate arm above still reduces
            // client-side; converting it lands under its own task
            // because the aggregate dispatcher signature carries the
            // `kind` branch and a different SQL shape per aggregate.
            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }

                    // Per-parent FK-key derivation - matches the SQL
                    // CAST output below. `Value::String(s)` unwraps to
                    // raw `s` rather than the JSON-quoted form so the
                    // string FK case lines up with the raw CAST result.
                    fn __sn_parent_key_to_match_cast(
                        v: ::suprnova::serde_json::Value,
                    ) -> ::std::string::String {
                        match v {
                            ::suprnova::serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        }
                    }

                    let pk_json_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();

                    // Phase 10C audit-fix AF1 - resolve through ExecutorChoice so
                    // raw eager-load SQL honors any ambient CURRENT_TX. Without
                    // this every leaf would run on `DB::connection()` (the pool)
                    // and miss in-tx state under a `DB::transaction` closure.
                    let __sn_exec = ::suprnova::database::transaction::ExecutorChoice::resolve_read(
                        ::core::option::Option::None,
                        ::core::option::Option::None,
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                    ).await?;
                    let db_backend = __sn_exec.backend();
                    // The related rows as the related model's `query()` reads
                    // them: its soft-delete filter and global scopes apply, so
                    // the count or total covers the rows `with` loads. The
                    // source binds come first.
                    let (__sn_source, __sn_source_binds) =
                        ::suprnova::Builder::<#target_ty>::__relation_source(
                            db_backend,
                            <#target_ty as ::suprnova::eloquent::EloquentModel>::TABLE,
                        )?;
                    let __sn_offset = __sn_source_binds.len();

                    // Build the placeholder list. Per-backend dialect
                    // matches the inner `Builder` renderer: Postgres
                    // uses `$N`, others use `?`. `parents.is_empty()`
                    // already short-circuited above so the bind list
                    // is non-empty here.
                    let mut placeholders: ::std::vec::Vec<::std::string::String> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    let mut binds: ::std::vec::Vec<::suprnova::sea_orm::Value> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    binds.extend(__sn_source_binds);
                    for (i, v) in pk_json_values.iter().enumerate() {
                        let ph = match db_backend {
                            ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                                ::std::format!("${}", i + 1 + __sn_offset)
                            }
                            _ => ::std::string::String::from("?"),
                        };
                        placeholders.push(ph);
                        binds.push(
                            ::suprnova::eloquent::model::json_value_to_sea_value(v),
                        );
                    }

                    // `CAST(... AS CHAR)` on MySQL, `CAST(... AS TEXT)`
                    // elsewhere - both yield the raw stringified column
                    // value the parent-side key derivation matches.
                    let __sn_cast_kw = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::MySql => "CHAR",
                        _ => "TEXT",
                    };
                    let __sn_sql = ::std::format!(
                        "SELECT CAST({fk} AS {cast}) AS __sn_fk_key, \
                                COUNT(*) AS __sn_count \
                           FROM {table} \
                          WHERE {fk} IN ({phs}) \
                          GROUP BY {fk}",
                        fk = #fk,
                        cast = __sn_cast_kw,
                        table = __sn_source,
                        phs = placeholders.join(", "),
                    );

                    let stmt = ::suprnova::sea_orm::Statement::from_sql_and_values(
                        db_backend,
                        &__sn_sql,
                        binds,
                    );
                    // DATA-033: this raw read names its tables to the render
                    // cache, so a write to any of them invalidates the page.
                    ::suprnova::render_cache::collector::observe_table_read(
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::TABLE,
                    );
                    let rows = __sn_exec.query_all(stmt)
                        .await
                        .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;

                    use ::std::collections::HashMap;
                    let mut counts: HashMap<::std::string::String, u64> = HashMap::new();
                    for r in rows.iter() {
                        // Both columns come back via `try_get` against
                        // their declared aliases. COUNT(*) is a 64-bit
                        // signed integer on every backend SeaORM
                        // supports here.
                        let key: ::std::string::String = r
                            .try_get::<::std::string::String>("", "__sn_fk_key")
                            .unwrap_or_default();
                        let n: i64 = r.try_get::<i64>("", "__sn_count").unwrap_or(0);
                        // Negative COUNT shouldn't happen on real
                        // backends, but the saturating cast guards
                        // against pathological drivers without
                        // panicking the dispatcher.
                        counts.insert(key, ::core::cmp::Ord::max(n, 0) as u64);
                    }

                    for p in parents.iter_mut() {
                        let key = __sn_parent_key_to_match_cast(
                            ::suprnova::serde_json::to_value(&p.#pk_ident)
                                .unwrap_or(::suprnova::serde_json::Value::Null),
                        );
                        p.__eager.set_count(#name_str, *counts.get(&key).unwrap_or(&0));
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::BelongsToMany => {
            let pivot_ty = rel.through.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "BelongsToMany requires a pivot type (parser bug if reached)",
                )
            })?;
            let pivot_fk = pivot_fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&parent_name)));
            let pivot_table_expr: TokenStream = match pivot_table_override(rel) {
                Some(t) => {
                    let lit = syn::LitStr::new(t, proc_macro2::Span::call_site());
                    quote! { #lit }
                }
                None => quote! {
                    <#pivot_ty as ::suprnova::eloquent::EloquentModel>::TABLE
                },
            };

            // Server-side GROUP BY count over the pivot table - one
            // round trip regardless of fan-out. Identical pattern to
            // the HasMany count arm, except the GROUP-BY target is the
            // pivot's FK column and the source table is the pivot.
            // See the HasMany arm's long-form comment for the
            // CAST-as-text key-matching contract.
            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }

                    fn __sn_parent_key_to_match_cast(
                        v: ::suprnova::serde_json::Value,
                    ) -> ::std::string::String {
                        match v {
                            ::suprnova::serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        }
                    }

                    let pk_json_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();

                    // Phase 10C audit-fix AF1 - resolve through ExecutorChoice so
                    // raw eager-load SQL honors any ambient CURRENT_TX. Without
                    // this every leaf would run on `DB::connection()` (the pool)
                    // and miss in-tx state under a `DB::transaction` closure.
                    let __sn_exec = ::suprnova::database::transaction::ExecutorChoice::resolve_read(
                        ::core::option::Option::None,
                        ::core::option::Option::None,
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                    ).await?;
                    let db_backend = __sn_exec.backend();

                    let mut placeholders: ::std::vec::Vec<::std::string::String> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    let mut binds: ::std::vec::Vec<::suprnova::sea_orm::Value> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    for (i, v) in pk_json_values.iter().enumerate() {
                        let ph = match db_backend {
                            ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                                ::std::format!("${}", i + 1)
                            }
                            _ => ::std::string::String::from("?"),
                        };
                        placeholders.push(ph);
                        binds.push(
                            ::suprnova::eloquent::model::json_value_to_sea_value(v),
                        );
                    }

                    let __sn_cast_kw = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::MySql => "CHAR",
                        _ => "TEXT",
                    };
                    let __sn_table = #pivot_table_expr;
                    let __sn_sql = ::std::format!(
                        "SELECT CAST({fk} AS {cast}) AS __sn_fk_key, \
                                COUNT(*) AS __sn_count \
                           FROM {table} \
                          WHERE {fk} IN ({phs}) \
                          GROUP BY {fk}",
                        fk = #pivot_fk,
                        cast = __sn_cast_kw,
                        table = __sn_table,
                        phs = placeholders.join(", "),
                    );

                    let stmt = ::suprnova::sea_orm::Statement::from_sql_and_values(
                        db_backend,
                        &__sn_sql,
                        binds,
                    );
                    // DATA-033: this raw read names its tables to the render
                    // cache, so a write to any of them invalidates the page.
                    ::suprnova::render_cache::collector::observe_table_read(__sn_table);
                    let rows = __sn_exec.query_all(stmt)
                        .await
                        .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;

                    use ::std::collections::HashMap;
                    let mut counts: HashMap<::std::string::String, u64> = HashMap::new();
                    for r in rows.iter() {
                        let key: ::std::string::String = r
                            .try_get::<::std::string::String>("", "__sn_fk_key")
                            .unwrap_or_default();
                        let n: i64 = r.try_get::<i64>("", "__sn_count").unwrap_or(0);
                        counts.insert(key, ::core::cmp::Ord::max(n, 0) as u64);
                    }

                    for p in parents.iter_mut() {
                        let key = __sn_parent_key_to_match_cast(
                            ::suprnova::serde_json::to_value(&p.#pk_ident)
                                .unwrap_or(::suprnova::serde_json::Value::Null),
                        );
                        p.__eager.set_count(#name_str, *counts.get(&key).unwrap_or(&0));
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::HasManyThrough | RelationKindAttr::HasOneThrough => {
            // Server-side GROUP BY count via the two-hop JOIN. One
            // round trip regardless of fan-out across the C table.
            // Mirrors HasMany's count arm but the COUNT source is
            // `<C> INNER JOIN <B>` and we group by `B.<first_key>`
            // (which is the parent's PK value, normalised via
            // CAST AS TEXT/CHAR).
            //
            //   SELECT CAST(b.<first_key> AS TEXT|CHAR) AS __sn_fk_key,
            //          COUNT(*) AS __sn_count
            //     FROM <C> c
            //     JOIN <B> b ON c.<second_key> = b.<second_local_key>
            //    WHERE b.<first_key> IN (?, ?, ...)
            //    GROUP BY b.<first_key>
            //
            // HasOneThrough reports the real COUNT(*) here - the JOIN
            // itself can return multiple C rows per parent if the HasOne
            // contract is violated, and we'd rather surface the real
            // count + let tests catch a malformed dataset than silently
            // truncate at the SQL layer.
            //
            // Type rebinding: same swap as the eager arm - for
            // Through kinds the parser stores `(B, C)` as
            // `(rel.target, rel.through)`. Shadow the function-scope
            // `target_ty` with the final target `C`.
            let through_ty = &rel.target; // intermediate B
            let target_ty = rel.through.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "HasOneThrough / HasManyThrough require a final target type \
                     (parser bug if reached)",
                )
            })?; // final target C
            let first_key = first_key_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&parent_name)));
            let second_key = second_key_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&last_segment_name(through_ty))));
            // JOIN-target column on B. Defaults to `"id"`; overridable
            // via `second_local_key = "..."` for intermediates with a
            // non-`id` PK.
            let second_local_key = second_local_key_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| "id".to_string());

            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }

                    fn __sn_parent_key_to_match_cast(
                        v: ::suprnova::serde_json::Value,
                    ) -> ::std::string::String {
                        match v {
                            ::suprnova::serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        }
                    }

                    let pk_json_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();

                    // Phase 10C audit-fix AF1 - resolve through ExecutorChoice so
                    // raw eager-load SQL honors any ambient CURRENT_TX. Without
                    // this every leaf would run on `DB::connection()` (the pool)
                    // and miss in-tx state under a `DB::transaction` closure.
                    let __sn_exec = ::suprnova::database::transaction::ExecutorChoice::resolve_read(
                        ::core::option::Option::None,
                        ::core::option::Option::None,
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                    ).await?;
                    let db_backend = __sn_exec.backend();
                    // The related rows as the related model's `query()` reads
                    // them: its soft-delete filter and global scopes apply, so
                    // the count or total covers the rows `with` loads. The
                    // source binds come first.
                    let (__sn_source, __sn_source_binds) =
                        ::suprnova::Builder::<#target_ty>::__relation_source(
                            db_backend,
                            "__sn_c",
                        )?;
                    let __sn_offset = __sn_source_binds.len();

                    let mut placeholders: ::std::vec::Vec<::std::string::String> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    let mut binds: ::std::vec::Vec<::suprnova::sea_orm::Value> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    binds.extend(__sn_source_binds);
                    for (i, v) in pk_json_values.iter().enumerate() {
                        let ph = match db_backend {
                            ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                                ::std::format!("${}", i + 1 + __sn_offset)
                            }
                            _ => ::std::string::String::from("?"),
                        };
                        placeholders.push(ph);
                        binds.push(
                            ::suprnova::eloquent::model::json_value_to_sea_value(v),
                        );
                    }

                    let __sn_cast_kw = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::MySql => "CHAR",
                        _ => "TEXT",
                    };
                    let __sn_b_table = <#through_ty as
                        ::suprnova::eloquent::EloquentModel>::TABLE;
                    let __sn_sql = ::std::format!(
                        "SELECT CAST(__sn_b.{fk} AS {cast}) AS __sn_fk_key, \
                                COUNT(*) AS __sn_count \
                           FROM {c_table} \
                           JOIN {b_table} __sn_b \
                             ON __sn_c.{second_key} = __sn_b.{slk} \
                          WHERE __sn_b.{fk} IN ({phs}){b_alive} \
                          GROUP BY __sn_b.{fk}",
                        fk = #first_key,
                        second_key = #second_key,
                        slk = #second_local_key,
                        cast = __sn_cast_kw,
                        c_table = __sn_source,
                        b_table = __sn_b_table,
                        b_alive = ::suprnova::eloquent::relations::__soft_delete_guard::<#through_ty>("__sn_b"),
                        phs = placeholders.join(", "),
                    );

                    let stmt = ::suprnova::sea_orm::Statement::from_sql_and_values(
                        db_backend,
                        &__sn_sql,
                        binds,
                    );
                    // DATA-033: this raw read names its tables to the render
                    // cache, so a write to any of them invalidates the page.
                    ::suprnova::render_cache::collector::observe_table_read(
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::TABLE,
                    );
                    ::suprnova::render_cache::collector::observe_table_read(__sn_b_table);
                    let rows = __sn_exec.query_all(stmt)
                        .await
                        .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;

                    use ::std::collections::HashMap;
                    let mut counts: HashMap<::std::string::String, u64> = HashMap::new();
                    for r in rows.iter() {
                        let key: ::std::string::String = r
                            .try_get::<::std::string::String>("", "__sn_fk_key")
                            .unwrap_or_default();
                        let n: i64 = r.try_get::<i64>("", "__sn_count").unwrap_or(0);
                        counts.insert(key, ::core::cmp::Ord::max(n, 0) as u64);
                    }

                    for p in parents.iter_mut() {
                        let key = __sn_parent_key_to_match_cast(
                            ::suprnova::serde_json::to_value(&p.#pk_ident)
                                .unwrap_or(::suprnova::serde_json::Value::Null),
                        );
                        p.__eager.set_count(#name_str, *counts.get(&key).unwrap_or(&0));
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::MorphMany | RelationKindAttr::MorphOne => {
            // Server-side GROUP BY count over the child table, with
            // both the `<name>_id IN (...)` and
            // `<name>_type = '<morph_type>'` predicates applied so
            // children of other morph families are excluded from the
            // count.
            //
            //   SELECT CAST(<id_col> AS TEXT|CHAR) AS __sn_fk_key,
            //          COUNT(*)                   AS __sn_count
            //     FROM <child_table>
            //    WHERE <id_col> IN (?, ?, ...)
            //      AND <type_col> = ?
            //    GROUP BY <id_col>
            //
            // Same CAST-as-text key-matching contract as the HasMany
            // count arm - see that arm for the long-form rationale.
            //
            // MorphOne's count surface is 0-or-1 in practice (the
            // contract says one child per parent), so the real count
            // is reported here even when violated upstream - tests
            // catch the malformed dataset rather than silently
            // truncating at the SQL layer.
            let morph_name = morph_name_or_default(rel);
            let morph_type_value = morph_type_of(input);
            let id_col = format!("{morph_name}_id");
            let type_col = format!("{morph_name}_type");
            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }

                    fn __sn_parent_key_to_match_cast(
                        v: ::suprnova::serde_json::Value,
                    ) -> ::std::string::String {
                        match v {
                            ::suprnova::serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        }
                    }

                    let pk_json_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();

                    // Phase 10C audit-fix AF1 - resolve through ExecutorChoice so
                    // raw eager-load SQL honors any ambient CURRENT_TX. Without
                    // this every leaf would run on `DB::connection()` (the pool)
                    // and miss in-tx state under a `DB::transaction` closure.
                    let __sn_exec = ::suprnova::database::transaction::ExecutorChoice::resolve_read(
                        ::core::option::Option::None,
                        ::core::option::Option::None,
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                    ).await?;
                    let db_backend = __sn_exec.backend();
                    // The related rows as the related model's `query()` reads
                    // them: its soft-delete filter and global scopes apply, so
                    // the count or total covers the rows `with` loads. The
                    // source binds come first.
                    let (__sn_source, __sn_source_binds) =
                        ::suprnova::Builder::<#target_ty>::__relation_source(
                            db_backend,
                            <#target_ty as ::suprnova::eloquent::EloquentModel>::TABLE,
                        )?;
                    let __sn_offset = __sn_source_binds.len();

                    let mut placeholders: ::std::vec::Vec<::std::string::String> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    let mut binds: ::std::vec::Vec<::suprnova::sea_orm::Value> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len() + 1);
                    binds.extend(__sn_source_binds);
                    for (i, v) in pk_json_values.iter().enumerate() {
                        let ph = match db_backend {
                            ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                                ::std::format!("${}", i + 1 + __sn_offset)
                            }
                            _ => ::std::string::String::from("?"),
                        };
                        placeholders.push(ph);
                        binds.push(
                            ::suprnova::eloquent::model::json_value_to_sea_value(v),
                        );
                    }
                    // The morph-type predicate gets the next sequential
                    // Postgres placeholder ($N+1) or `?` on the
                    // remaining backends. Bound after the IN-list.
                    let type_ph = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                            ::std::format!("${}", pk_json_values.len() + 1 + __sn_offset)
                        }
                        _ => ::std::string::String::from("?"),
                    };
                    binds.push(::suprnova::sea_orm::Value::from(#morph_type_value));

                    let __sn_cast_kw = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::MySql => "CHAR",
                        _ => "TEXT",
                    };
                    let __sn_sql = ::std::format!(
                        "SELECT CAST({id} AS {cast}) AS __sn_fk_key, \
                                COUNT(*) AS __sn_count \
                           FROM {table} \
                          WHERE {id} IN ({phs}) \
                            AND {type_col} = {type_ph} \
                          GROUP BY {id}",
                        id = #id_col,
                        cast = __sn_cast_kw,
                        table = __sn_source,
                        type_col = #type_col,
                        type_ph = type_ph,
                        phs = placeholders.join(", "),
                    );

                    let stmt = ::suprnova::sea_orm::Statement::from_sql_and_values(
                        db_backend,
                        &__sn_sql,
                        binds,
                    );
                    // DATA-033: this raw read names its tables to the render
                    // cache, so a write to any of them invalidates the page.
                    ::suprnova::render_cache::collector::observe_table_read(
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::TABLE,
                    );
                    let rows = __sn_exec.query_all(stmt)
                        .await
                        .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;

                    use ::std::collections::HashMap;
                    let mut counts: HashMap<::std::string::String, u64> = HashMap::new();
                    for r in rows.iter() {
                        let key: ::std::string::String = r
                            .try_get::<::std::string::String>("", "__sn_fk_key")
                            .unwrap_or_default();
                        let n: i64 = r.try_get::<i64>("", "__sn_count").unwrap_or(0);
                        counts.insert(key, ::core::cmp::Ord::max(n, 0) as u64);
                    }

                    for p in parents.iter_mut() {
                        let key = __sn_parent_key_to_match_cast(
                            ::suprnova::serde_json::to_value(&p.#pk_ident)
                                .unwrap_or(::suprnova::serde_json::Value::Null),
                        );
                        p.__eager.set_count(#name_str, *counts.get(&key).unwrap_or(&0));
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::MorphToMany => {
            // Server-side GROUP BY count over the polymorphic pivot
            // table - one round trip regardless of fan-out across the
            // parent set. Same shape as BelongsToMany's count arm,
            // with the extra `<name>_type = '<self_morph_type>'`
            // predicate so children of OTHER morph families are
            // excluded.
            //
            //   SELECT CAST(<id_col> AS TEXT|CHAR) AS __sn_fk_key,
            //          COUNT(*)                   AS __sn_count
            //     FROM <pivot_table>
            //    WHERE <id_col> IN (?, ?, ...)
            //      AND <type_col> = ?
            //    GROUP BY <id_col>
            let pivot_ty = rel.through.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "MorphToMany requires a pivot type (parser bug if reached)",
                )
            })?;
            let morph_name = morph_name_or_default(rel);
            let parent_morph_type = morph_type_of(input);
            let id_col = format!("{morph_name}_id");
            let type_col = format!("{morph_name}_type");
            let pivot_table_expr: TokenStream = match pivot_table_override(rel) {
                Some(t) => {
                    let lit = syn::LitStr::new(t, proc_macro2::Span::call_site());
                    quote! { #lit }
                }
                None => quote! {
                    <#pivot_ty as ::suprnova::eloquent::EloquentModel>::TABLE
                },
            };
            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }

                    fn __sn_parent_key_to_match_cast(
                        v: ::suprnova::serde_json::Value,
                    ) -> ::std::string::String {
                        match v {
                            ::suprnova::serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        }
                    }

                    let pk_json_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();

                    // Phase 10C audit-fix AF1 - resolve through ExecutorChoice so
                    // raw eager-load SQL honors any ambient CURRENT_TX. Without
                    // this every leaf would run on `DB::connection()` (the pool)
                    // and miss in-tx state under a `DB::transaction` closure.
                    let __sn_exec = ::suprnova::database::transaction::ExecutorChoice::resolve_read(
                        ::core::option::Option::None,
                        ::core::option::Option::None,
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                    ).await?;
                    let db_backend = __sn_exec.backend();

                    let mut placeholders: ::std::vec::Vec<::std::string::String> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    let mut binds: ::std::vec::Vec<::suprnova::sea_orm::Value> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len() + 1);
                    for (i, v) in pk_json_values.iter().enumerate() {
                        let ph = match db_backend {
                            ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                                ::std::format!("${}", i + 1)
                            }
                            _ => ::std::string::String::from("?"),
                        };
                        placeholders.push(ph);
                        binds.push(
                            ::suprnova::eloquent::model::json_value_to_sea_value(v),
                        );
                    }
                    let type_ph = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                            ::std::format!("${}", pk_json_values.len() + 1)
                        }
                        _ => ::std::string::String::from("?"),
                    };
                    binds.push(::suprnova::sea_orm::Value::from(#parent_morph_type));

                    let __sn_cast_kw = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::MySql => "CHAR",
                        _ => "TEXT",
                    };
                    let __sn_table = #pivot_table_expr;
                    let __sn_sql = ::std::format!(
                        "SELECT CAST({id} AS {cast}) AS __sn_fk_key, \
                                COUNT(*) AS __sn_count \
                           FROM {table} \
                          WHERE {id} IN ({phs}) \
                            AND {type_col} = {type_ph} \
                          GROUP BY {id}",
                        id = #id_col,
                        cast = __sn_cast_kw,
                        table = __sn_table,
                        type_col = #type_col,
                        type_ph = type_ph,
                        phs = placeholders.join(", "),
                    );

                    let stmt = ::suprnova::sea_orm::Statement::from_sql_and_values(
                        db_backend,
                        &__sn_sql,
                        binds,
                    );
                    // DATA-033: this raw read names its tables to the render
                    // cache, so a write to any of them invalidates the page.
                    ::suprnova::render_cache::collector::observe_table_read(__sn_table);
                    let rows = __sn_exec.query_all(stmt)
                        .await
                        .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;

                    use ::std::collections::HashMap;
                    let mut counts: HashMap<::std::string::String, u64> = HashMap::new();
                    for r in rows.iter() {
                        let key: ::std::string::String = r
                            .try_get::<::std::string::String>("", "__sn_fk_key")
                            .unwrap_or_default();
                        let n: i64 = r.try_get::<i64>("", "__sn_count").unwrap_or(0);
                        counts.insert(key, ::core::cmp::Ord::max(n, 0) as u64);
                    }

                    for p in parents.iter_mut() {
                        let key = __sn_parent_key_to_match_cast(
                            ::suprnova::serde_json::to_value(&p.#pk_ident)
                                .unwrap_or(::suprnova::serde_json::Value::Null),
                        );
                        p.__eager.set_count(#name_str, *counts.get(&key).unwrap_or(&0));
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::MorphedByMany => {
            // Inverse-side server-side GROUP BY count. Same SQL
            // shape as MorphToMany but the grouped column is the
            // pivot's FK to the m2m side (tag_id) and the type
            // predicate is the explicit `target_morph_type`.
            //
            //   SELECT CAST(<pivot_fk> AS TEXT|CHAR) AS __sn_fk_key,
            //          COUNT(*) AS __sn_count
            //     FROM <pivot_table>
            //    WHERE <pivot_fk> IN (?, ?, ...)
            //      AND <type_col> = ?
            //    GROUP BY <pivot_fk>
            let pivot_ty = rel.through.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "MorphedByMany requires a pivot type (parser bug if reached)",
                )
            })?;
            let morph_name = morph_name_or_default(rel);
            let target_morph_type = target_morph_type_override(rel).ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "MorphedByMany requires `target_morph_type = \"...\"` \
                         (parse-time validation should reject this earlier)",
                )
            })?;
            let type_col = format!("{morph_name}_type");
            let pivot_fk = pivot_fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&parent_name)));
            let pivot_table_expr: TokenStream = match pivot_table_override(rel) {
                Some(t) => {
                    let lit = syn::LitStr::new(t, proc_macro2::Span::call_site());
                    quote! { #lit }
                }
                None => quote! {
                    <#pivot_ty as ::suprnova::eloquent::EloquentModel>::TABLE
                },
            };
            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }

                    fn __sn_parent_key_to_match_cast(
                        v: ::suprnova::serde_json::Value,
                    ) -> ::std::string::String {
                        match v {
                            ::suprnova::serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        }
                    }

                    let pk_json_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();

                    // Phase 10C audit-fix AF1 - resolve through ExecutorChoice so
                    // raw eager-load SQL honors any ambient CURRENT_TX. Without
                    // this every leaf would run on `DB::connection()` (the pool)
                    // and miss in-tx state under a `DB::transaction` closure.
                    let __sn_exec = ::suprnova::database::transaction::ExecutorChoice::resolve_read(
                        ::core::option::Option::None,
                        ::core::option::Option::None,
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                    ).await?;
                    let db_backend = __sn_exec.backend();

                    let mut placeholders: ::std::vec::Vec<::std::string::String> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    let mut binds: ::std::vec::Vec<::suprnova::sea_orm::Value> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len() + 1);
                    for (i, v) in pk_json_values.iter().enumerate() {
                        let ph = match db_backend {
                            ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                                ::std::format!("${}", i + 1)
                            }
                            _ => ::std::string::String::from("?"),
                        };
                        placeholders.push(ph);
                        binds.push(
                            ::suprnova::eloquent::model::json_value_to_sea_value(v),
                        );
                    }
                    let type_ph = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                            ::std::format!("${}", pk_json_values.len() + 1)
                        }
                        _ => ::std::string::String::from("?"),
                    };
                    binds.push(::suprnova::sea_orm::Value::from(#target_morph_type));

                    let __sn_cast_kw = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::MySql => "CHAR",
                        _ => "TEXT",
                    };
                    let __sn_table = #pivot_table_expr;
                    let __sn_sql = ::std::format!(
                        "SELECT CAST({fk} AS {cast}) AS __sn_fk_key, \
                                COUNT(*) AS __sn_count \
                           FROM {table} \
                          WHERE {fk} IN ({phs}) \
                            AND {type_col} = {type_ph} \
                          GROUP BY {fk}",
                        fk = #pivot_fk,
                        cast = __sn_cast_kw,
                        table = __sn_table,
                        type_col = #type_col,
                        type_ph = type_ph,
                        phs = placeholders.join(", "),
                    );

                    let stmt = ::suprnova::sea_orm::Statement::from_sql_and_values(
                        db_backend,
                        &__sn_sql,
                        binds,
                    );
                    // DATA-033: this raw read names its tables to the render
                    // cache, so a write to any of them invalidates the page.
                    ::suprnova::render_cache::collector::observe_table_read(__sn_table);
                    let rows = __sn_exec.query_all(stmt)
                        .await
                        .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;

                    use ::std::collections::HashMap;
                    let mut counts: HashMap<::std::string::String, u64> = HashMap::new();
                    for r in rows.iter() {
                        let key: ::std::string::String = r
                            .try_get::<::std::string::String>("", "__sn_fk_key")
                            .unwrap_or_default();
                        let n: i64 = r.try_get::<i64>("", "__sn_count").unwrap_or(0);
                        counts.insert(key, ::core::cmp::Ord::max(n, 0) as u64);
                    }

                    for p in parents.iter_mut() {
                        let key = __sn_parent_key_to_match_cast(
                            ::suprnova::serde_json::to_value(&p.#pk_ident)
                                .unwrap_or(::suprnova::serde_json::Value::Null),
                        );
                        p.__eager.set_count(#name_str, *counts.get(&key).unwrap_or(&0));
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        // MorphTo: a row points at one owner in one of several tables,
        // so `with_count(["commentable"])` has no canonical SQL shape.
        // Emit an explicit error rather than falling through to the
        // dispatcher catch-all ("no relation X"); `with(["commentable"])`
        // is the eager path for this kind.
        RelationKindAttr::MorphTo => Ok(Some(quote! {
            #name_str => {
                return ::core::result::Result::Err(
                    ::suprnova::FrameworkError::internal(::std::format!(
                        "with_count on the MorphTo relation `{}` of `{}` is not supported: \
                         the relation points at one row in one of several tables; load it \
                         with `with([\"{}\"])` instead",
                        #name_str,
                        #parent_name,
                        #name_str,
                    )),
                );
            }
        })),
    }
}

/// `__aggregate_relation` arm for HasOne / BelongsTo. Same shape as
/// count - we run the IN query, then per parent pick a single row (or
/// none) and apply the SUM/AVG/MIN/MAX, which over 0-or-1 row is
/// either the column value itself or 0 / null. For T2 the column is
/// stored as `f64` for SUM/AVG (matching `with_sum`'s usual signature)
/// and we honour the same for MIN/MAX. T9 may extend the shape for
/// non-numeric MIN/MAX once the eager loading orchestrator lands.
///
/// NB: HasOne / BelongsTo `with_sum`/`avg`/`min`/`max` rarely make
/// sense in practice (the result is over at most one row), but the
/// spec lets users call them, so we wire the path here for parity.
/// Users querying real aggregates use HasMany (T3).
fn emit_aggregate_arm(input: &ModelInput, rel: &RelationDecl) -> Result<Option<TokenStream>> {
    let struct_ident = &input.item.ident;
    let name_str = rel.name.to_string();
    let pk_ident = local_key_ident(input, rel)?;
    let target_ty = &rel.target;
    let parent_name = struct_ident.to_string();

    match rel.kind {
        RelationKindAttr::HasOne => {
            let fk = fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| default_has_fk(&parent_name));
            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }
                    let pk_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();
                    let rows: ::std::vec::Vec<#target_ty> =
                        <#target_ty as ::suprnova::eloquent::Model>::query()
                            .filter_in(#fk, pk_values)
                            .get()
                            .await?
                            .into_vec();
                    use ::std::collections::HashMap;
                    let mut by_fk: HashMap<::std::string::String, f64> = HashMap::new();
                    for r in rows.iter() {
                        let key = ::suprnova::eloquent::relations::eager_row_column(
                            r,
                            ::suprnova::eloquent::Model::field_value(r, #fk),
                            #fk,
                        )
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        let col_val = ::suprnova::eloquent::relations::eager_row_column(
                            r,
                            ::suprnova::eloquent::Model::field_value(r, column),
                            column,
                        )
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);
                        // Each parent's group has 0-or-1 row, so the
                        // aggregate function is the same on every kind -
                        // just record the column value.
                        by_fk.insert(key, col_val);
                    }
                    // Sum/Avg over an empty group stores 0.0
                    // (consistent with the framework's COALESCE
                    // behaviour). Min/Max over an empty group stores
                    // Option::<f64>::None (matches SQL's NULL-on-empty
                    // semantics + the existing Builder::min/max
                    // Option<T> return type). Non-empty groups always
                    // store Some(value) for Min/Max.
                    //
                    // Cache key is the wide `<rel>_<kind>_<col>` form
                    // (P1 fix). Multiple aggregates on the same
                    // relation coexist on the same row without
                    // overwriting; the per-relation
                    // `<rel>_sum_of(col)` / `_avg_of` / `_min_of` /
                    // `_max_of` accessors read using the same helper.
                    let __sn_agg_key: ::std::string::String =
                        ::suprnova::eloquent::relations::aggregate_cache_key(
                            #name_str, kind, column,
                        );
                    for p in parents.iter_mut() {
                        let key = ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        let opt_v: ::core::option::Option<f64> = by_fk.get(&key).copied();
                        match kind {
                            ::suprnova::AggregateKind::Sum
                            | ::suprnova::AggregateKind::Avg => {
                                p.__eager.set_aggregate::<f64>(
                                    &__sn_agg_key,
                                    opt_v.unwrap_or(0.0),
                                );
                            }
                            ::suprnova::AggregateKind::Min
                            | ::suprnova::AggregateKind::Max => {
                                p.__eager.set_aggregate::<::core::option::Option<f64>>(
                                    &__sn_agg_key,
                                    opt_v,
                                );
                            }
                        }
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::BelongsTo => {
            let fk = fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| default_belongs_to_fk(target_ty));
            let owner_key = lk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| "id".to_string());
            let fk_ident = quote::format_ident!("{}", fk);
            let fk_is_optional = field_is_optional(input, &fk);
            let per_parent_key_expr = if fk_is_optional {
                quote! {
                    p.#fk_ident
                        .as_ref()
                        .and_then(|v| ::suprnova::serde_json::to_value(v).ok())
                }
            } else {
                quote! {
                    ::core::option::Option::Some(
                        ::suprnova::serde_json::to_value(&p.#fk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null),
                    )
                }
            };
            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }
                    let fk_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .filter_map(|p| {
                            let v: ::core::option::Option<::suprnova::serde_json::Value> =
                                #per_parent_key_expr;
                            v
                        })
                        .collect();
                    let parent_rows: ::std::vec::Vec<#target_ty> = if fk_values.is_empty() {
                        ::std::vec::Vec::new()
                    } else {
                        <#target_ty as ::suprnova::eloquent::Model>::query()
                            .filter_in(#owner_key, fk_values)
                            .get()
                            .await?
                            .into_vec()
                    };
                    use ::std::collections::HashMap;
                    let mut by_pk: HashMap<::std::string::String, f64> = HashMap::new();
                    for r in parent_rows.iter() {
                        let key = ::suprnova::eloquent::relations::eager_row_column(
                            r,
                            ::suprnova::eloquent::Model::field_value(r, #owner_key),
                            #owner_key,
                        )
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        let col_val = ::suprnova::eloquent::relations::eager_row_column(
                            r,
                            ::suprnova::eloquent::Model::field_value(r, column),
                            column,
                        )
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);
                        by_pk.insert(key, col_val);
                    }
                    // Sum/Avg over an empty group stores 0.0
                    // (framework COALESCE behaviour). Min/Max over an
                    // empty group stores Option::<f64>::None (SQL's
                    // NULL-on-empty + Builder::min/max Option<T>
                    // return type). Non-empty groups always store
                    // Some(value) for Min/Max.
                    //
                    // Cache key is the wide `<rel>_<kind>_<col>` form
                    // (P1 fix) - see the HasOne arm above.
                    let __sn_agg_key: ::std::string::String =
                        ::suprnova::eloquent::relations::aggregate_cache_key(
                            #name_str, kind, column,
                        );
                    for p in parents.iter_mut() {
                        let v: ::core::option::Option<::suprnova::serde_json::Value> =
                            #per_parent_key_expr;
                        let opt_v: ::core::option::Option<f64> = match &v {
                            ::core::option::Option::Some(jv) => {
                                by_pk.get(&jv.to_string()).copied()
                            }
                            ::core::option::Option::None => ::core::option::Option::None,
                        };
                        match kind {
                            ::suprnova::AggregateKind::Sum
                            | ::suprnova::AggregateKind::Avg => {
                                p.__eager.set_aggregate::<f64>(
                                    &__sn_agg_key,
                                    opt_v.unwrap_or(0.0),
                                );
                            }
                            ::suprnova::AggregateKind::Min
                            | ::suprnova::AggregateKind::Max => {
                                p.__eager.set_aggregate::<::core::option::Option<f64>>(
                                    &__sn_agg_key,
                                    opt_v,
                                );
                            }
                        }
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::HasMany => {
            let fk = fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| default_has_fk(&parent_name));

            // Server-side GROUP BY query, one round-trip per aggregate
            // kind invocation. Mirrors the count arm's pattern: build
            // a `SELECT CAST(<fk> AS TEXT|CHAR) AS __sn_fk_key,
            // <AGG>(<col>) AS __sn_agg FROM <table> WHERE <fk> IN (...)
            // GROUP BY <fk>` statement, then distribute per-FK results
            // into each parent's `__eager.set_aggregate` cell.
            //
            // The aggregate expression is picked at runtime from the
            // dispatcher's `kind` arg - Sum/Avg/Min/Max each map to the
            // corresponding SQL function. Sum/Avg over an empty group
            // store 0.0 (matches the framework's COALESCE behaviour);
            // Min/Max over an empty group store `Option::None` (matches
            // SQL's NULL-on-empty + the Builder::min/max Option<T>
            // return type). Non-empty groups always produce Some(value)
            // for Min/Max.
            //
            // `__sn_agg` is read as `Option<f64>` because AVG over an
            // empty group is NULL in SQL - and SUM is too, even though
            // our user-facing default is 0.0; the None vs Some(v)
            // branch below normalises that.
            //
            // Cache key is the wide `<rel>_<kind>_<col>` form built
            // by `aggregate_cache_key()`. Multiple aggregates on the
            // same relation (e.g. `with_sum` then `with_avg`) coexist
            // without clobbering each other.
            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }

                    // Per-parent FK-key derivation - matches the SQL
                    // CAST output below. `Value::String(s)` unwraps to
                    // raw `s` rather than the JSON-quoted form so the
                    // string FK case lines up with the raw CAST result.
                    // Identical to the count arm's helper.
                    fn __sn_parent_key_to_match_cast(
                        v: ::suprnova::serde_json::Value,
                    ) -> ::std::string::String {
                        match v {
                            ::suprnova::serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        }
                    }

                    let pk_json_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();

                    // Phase 10C audit-fix AF1 - resolve through ExecutorChoice so
                    // raw eager-load SQL honors any ambient CURRENT_TX. Without
                    // this every leaf would run on `DB::connection()` (the pool)
                    // and miss in-tx state under a `DB::transaction` closure.
                    let __sn_exec = ::suprnova::database::transaction::ExecutorChoice::resolve_read(
                        ::core::option::Option::None,
                        ::core::option::Option::None,
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                    ).await?;
                    let db_backend = __sn_exec.backend();
                    // The related rows as the related model's `query()` reads
                    // them: its soft-delete filter and global scopes apply, so
                    // the count or total covers the rows `with` loads. The
                    // source binds come first.
                    let (__sn_source, __sn_source_binds) =
                        ::suprnova::Builder::<#target_ty>::__relation_source(
                            db_backend,
                            <#target_ty as ::suprnova::eloquent::EloquentModel>::TABLE,
                        )?;
                    let __sn_offset = __sn_source_binds.len();

                    // Build the placeholder list. Per-backend dialect
                    // matches the inner `Builder` renderer: Postgres
                    // uses `$N`, others use `?`. `parents.is_empty()`
                    // already short-circuited above so the bind list
                    // is non-empty here.
                    let mut placeholders: ::std::vec::Vec<::std::string::String> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    let mut binds: ::std::vec::Vec<::suprnova::sea_orm::Value> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    binds.extend(__sn_source_binds);
                    for (i, v) in pk_json_values.iter().enumerate() {
                        let ph = match db_backend {
                            ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                                ::std::format!("${}", i + 1 + __sn_offset)
                            }
                            _ => ::std::string::String::from("?"),
                        };
                        placeholders.push(ph);
                        binds.push(
                            ::suprnova::eloquent::model::json_value_to_sea_value(v),
                        );
                    }

                    // `CAST(... AS CHAR)` on MySQL, `CAST(... AS TEXT)`
                    // elsewhere - both yield the raw stringified column
                    // value the parent-side key derivation matches.
                    let __sn_cast_kw = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::MySql => "CHAR",
                        _ => "TEXT",
                    };

                    // The aggregate expression is selected at runtime
                    // from `kind`. The `column` arg flows untyped into
                    // SQL - identical concern to `#fk` in the count
                    // arm; T9's user-facing Builder surface owns column
                    // validation, the dispatcher doesn't widen the
                    // contract.
                    let __sn_agg_expr: ::std::string::String = match kind {
                        ::suprnova::AggregateKind::Sum => {
                            ::std::format!("SUM({})", column)
                        }
                        ::suprnova::AggregateKind::Avg => {
                            ::std::format!("AVG({})", column)
                        }
                        ::suprnova::AggregateKind::Min => {
                            ::std::format!("MIN({})", column)
                        }
                        ::suprnova::AggregateKind::Max => {
                            ::std::format!("MAX({})", column)
                        }
                    };

                    let __sn_sql = ::std::format!(
                        "SELECT CAST({fk} AS {cast}) AS __sn_fk_key, \
                                {agg} AS __sn_agg \
                           FROM {table} \
                          WHERE {fk} IN ({phs}) \
                          GROUP BY {fk}",
                        fk = #fk,
                        cast = __sn_cast_kw,
                        agg = __sn_agg_expr,
                        table = __sn_source,
                        phs = placeholders.join(", "),
                    );

                    let stmt = ::suprnova::sea_orm::Statement::from_sql_and_values(
                        db_backend,
                        &__sn_sql,
                        binds,
                    );
                    // DATA-033: this raw read names its tables to the render
                    // cache, so a write to any of them invalidates the page.
                    ::suprnova::render_cache::collector::observe_table_read(
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::TABLE,
                    );
                    let rows = __sn_exec.query_all(stmt)
                        .await
                        .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;

                    use ::std::collections::HashMap;
                    // `Option<f64>` so AVG (and SUM) over zero rows -
                    // which manifests as SQL NULL - survives the read
                    // and falls through the Sum|Avg vs Min|Max branch
                    // at distribution time. For HasMany the row map is
                    // only populated for parents with at least one
                    // child row, so empty groups never even appear as
                    // a `Some(_)` here; the missing-key path on the
                    // parent loop handles those.
                    //
                    // The database chooses the type of `__sn_agg`: an
                    // integer on SQLite for SUM of an INTEGER column,
                    // `numeric` on Postgres and `DECIMAL` on MySQL for
                    // a sum or average of integers. The framework reads
                    // whichever arrives; a decoder that read only one of
                    // them stored 0.0 for every parent.
                    let mut by_fk: HashMap<
                        ::std::string::String,
                        ::core::option::Option<f64>,
                    > = HashMap::new();
                    for r in rows.iter() {
                        let key: ::std::string::String = r
                            .try_get::<::std::string::String>("", "__sn_fk_key")
                            .unwrap_or_default();
                        let agg: ::core::option::Option<f64> =
                            ::suprnova::database::column_value::__relation_aggregate(r, "__sn_agg")
                                .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;
                        by_fk.insert(key, agg);
                    }

                    let __sn_agg_key: ::std::string::String =
                        ::suprnova::eloquent::relations::aggregate_cache_key(
                            #name_str, kind, column,
                        );
                    for p in parents.iter_mut() {
                        let key = __sn_parent_key_to_match_cast(
                            ::suprnova::serde_json::to_value(&p.#pk_ident)
                                .unwrap_or(::suprnova::serde_json::Value::Null),
                        );
                        // Missing-key (parent had no child rows) and
                        // present-but-NULL collapse into the same
                        // `None` branch for the per-kind distribution.
                        let agg: ::core::option::Option<f64> = by_fk
                            .get(&key)
                            .copied()
                            .unwrap_or(::core::option::Option::None);
                        match kind {
                            ::suprnova::AggregateKind::Sum
                            | ::suprnova::AggregateKind::Avg => {
                                p.__eager.set_aggregate::<f64>(
                                    &__sn_agg_key,
                                    agg.unwrap_or(0.0),
                                );
                            }
                            ::suprnova::AggregateKind::Min
                            | ::suprnova::AggregateKind::Max => {
                                p.__eager.set_aggregate::<::core::option::Option<f64>>(
                                    &__sn_agg_key,
                                    agg,
                                );
                            }
                        }
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::BelongsToMany => {
            // BelongsToMany aggregate is over the RELATED table's
            // columns (Laravel parity - users typically aggregate over
            // role.weight, not pivot.assigned_at). The dispatcher JOINs
            // the pivot to the related table and groups by the pivot's
            // FK column.
            //
            //   SELECT CAST(p.fk AS TEXT|CHAR) AS __sn_fk_key,
            //          AGG(r.col)              AS __sn_agg
            //     FROM <pivot_table> p
            //     JOIN <related_table> r ON r.id = p.<related_key>
            //    WHERE p.<fk> IN (...)
            //    GROUP BY p.<fk>
            //
            // Sum/Avg → f64 with 0.0 empty default. Min/Max →
            // Option<f64> with None empty default. Matches HasMany's
            // contract.
            let pivot_ty = rel.through.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "BelongsToMany requires a pivot type (parser bug if reached)",
                )
            })?;
            let pivot_fk = pivot_fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&parent_name)));
            let pivot_related = pivot_related_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&last_segment_name(target_ty))));
            // Related-side key column - see `related_key_expr`. When the
            // user declares `related_key = "uuid"` on the relation, the
            // JOIN reads `__sn_r.uuid = __sn_p.{rk}`.
            let related_pk = related_key_expr(rel, target_ty);
            let pivot_table_expr: TokenStream = match pivot_table_override(rel) {
                Some(t) => {
                    let lit = syn::LitStr::new(t, proc_macro2::Span::call_site());
                    quote! { #lit }
                }
                None => quote! {
                    <#pivot_ty as ::suprnova::eloquent::EloquentModel>::TABLE
                },
            };

            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }

                    fn __sn_parent_key_to_match_cast(
                        v: ::suprnova::serde_json::Value,
                    ) -> ::std::string::String {
                        match v {
                            ::suprnova::serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        }
                    }

                    let pk_json_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();

                    // Phase 10C audit-fix AF1 - resolve through ExecutorChoice so
                    // raw eager-load SQL honors any ambient CURRENT_TX. Without
                    // this every leaf would run on `DB::connection()` (the pool)
                    // and miss in-tx state under a `DB::transaction` closure.
                    let __sn_exec = ::suprnova::database::transaction::ExecutorChoice::resolve_read(
                        ::core::option::Option::None,
                        ::core::option::Option::None,
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                    ).await?;
                    let db_backend = __sn_exec.backend();
                    // The related rows as the related model's `query()` reads
                    // them: its soft-delete filter and global scopes apply, so
                    // the count or total covers the rows `with` loads. The
                    // source binds come first.
                    let (__sn_source, __sn_source_binds) =
                        ::suprnova::Builder::<#target_ty>::__relation_source(
                            db_backend,
                            "__sn_r",
                        )?;
                    let __sn_offset = __sn_source_binds.len();

                    let mut placeholders: ::std::vec::Vec<::std::string::String> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    let mut binds: ::std::vec::Vec<::suprnova::sea_orm::Value> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    binds.extend(__sn_source_binds);
                    for (i, v) in pk_json_values.iter().enumerate() {
                        let ph = match db_backend {
                            ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                                ::std::format!("${}", i + 1 + __sn_offset)
                            }
                            _ => ::std::string::String::from("?"),
                        };
                        placeholders.push(ph);
                        binds.push(
                            ::suprnova::eloquent::model::json_value_to_sea_value(v),
                        );
                    }

                    let __sn_cast_kw = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::MySql => "CHAR",
                        _ => "TEXT",
                    };
                    let __sn_pivot = #pivot_table_expr;

                    let __sn_agg_expr: ::std::string::String = match kind {
                        ::suprnova::AggregateKind::Sum => {
                            ::std::format!("SUM(__sn_r.{})", column)
                        }
                        ::suprnova::AggregateKind::Avg => {
                            ::std::format!("AVG(__sn_r.{})", column)
                        }
                        ::suprnova::AggregateKind::Min => {
                            ::std::format!("MIN(__sn_r.{})", column)
                        }
                        ::suprnova::AggregateKind::Max => {
                            ::std::format!("MAX(__sn_r.{})", column)
                        }
                    };

                    let __sn_sql = ::std::format!(
                        "SELECT CAST(__sn_p.{fk} AS {cast}) AS __sn_fk_key, \
                                {agg} AS __sn_agg \
                           FROM {pivot} __sn_p \
                           JOIN {related} ON __sn_r.{related_pk} = __sn_p.{rk} \
                          WHERE __sn_p.{fk} IN ({phs}) \
                          GROUP BY __sn_p.{fk}",
                        fk = #pivot_fk,
                        rk = #pivot_related,
                        related_pk = #related_pk,
                        cast = __sn_cast_kw,
                        agg = __sn_agg_expr,
                        pivot = __sn_pivot,
                        related = __sn_source,
                        phs = placeholders.join(", "),
                    );

                    let stmt = ::suprnova::sea_orm::Statement::from_sql_and_values(
                        db_backend,
                        &__sn_sql,
                        binds,
                    );
                    // DATA-033: this raw read names its tables to the render
                    // cache, so a write to any of them invalidates the page.
                    ::suprnova::render_cache::collector::observe_table_read(__sn_pivot);
                    ::suprnova::render_cache::collector::observe_table_read(
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::TABLE,
                    );
                    let rows = __sn_exec.query_all(stmt)
                        .await
                        .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;

                    use ::std::collections::HashMap;
                    let mut by_fk: HashMap<
                        ::std::string::String,
                        ::core::option::Option<f64>,
                    > = HashMap::new();
                    for r in rows.iter() {
                        let key: ::std::string::String = r
                            .try_get::<::std::string::String>("", "__sn_fk_key")
                            .unwrap_or_default();
                        let agg: ::core::option::Option<f64> =
                            ::suprnova::database::column_value::__relation_aggregate(r, "__sn_agg")
                                .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;
                        by_fk.insert(key, agg);
                    }

                    // Cache key is the wide `<rel>_<kind>_<col>` form
                    // (P1 fix) so `with_sum` + `with_avg` over the
                    // same pivot relation coexist on the same row.
                    let __sn_agg_key: ::std::string::String =
                        ::suprnova::eloquent::relations::aggregate_cache_key(
                            #name_str, kind, column,
                        );
                    for p in parents.iter_mut() {
                        let key = __sn_parent_key_to_match_cast(
                            ::suprnova::serde_json::to_value(&p.#pk_ident)
                                .unwrap_or(::suprnova::serde_json::Value::Null),
                        );
                        let agg: ::core::option::Option<f64> = by_fk
                            .get(&key)
                            .copied()
                            .unwrap_or(::core::option::Option::None);
                        match kind {
                            ::suprnova::AggregateKind::Sum
                            | ::suprnova::AggregateKind::Avg => {
                                p.__eager.set_aggregate::<f64>(
                                    &__sn_agg_key,
                                    agg.unwrap_or(0.0),
                                );
                            }
                            ::suprnova::AggregateKind::Min
                            | ::suprnova::AggregateKind::Max => {
                                p.__eager.set_aggregate::<::core::option::Option<f64>>(
                                    &__sn_agg_key,
                                    agg,
                                );
                            }
                        }
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::HasManyThrough | RelationKindAttr::HasOneThrough => {
            // Through aggregate is over the TARGET (C) table's
            // columns. The dispatcher JOINs C to B and groups by the
            // intermediate's first_key column. Same SQL skeleton as
            // BelongsToMany's aggregate, except the JOIN connects
            // C.{second_key} to B.id (not a pivot table's two FKs).
            //
            //   SELECT CAST(b.<first_key> AS TEXT|CHAR) AS __sn_fk_key,
            //          AGG(c.<col>)                     AS __sn_agg
            //     FROM <C> __sn_c
            //     JOIN <B> __sn_b ON __sn_c.<second_key> = __sn_b.id
            //    WHERE __sn_b.<first_key> IN (...)
            //    GROUP BY __sn_b.<first_key>
            //
            // Sum/Avg → f64 with 0.0 empty default. Min/Max →
            // Option<f64> with None empty default. Matches the
            // HasMany / BelongsToMany contract.
            //
            // Type rebinding: same swap as the eager + count arms -
            // for Through kinds `(rel.target, rel.through)` is
            // `(B, C)`. Shadow the function-scope `target_ty` with
            // the final target `C`.
            let through_ty = &rel.target; // intermediate B
            let target_ty = rel.through.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "HasOneThrough / HasManyThrough require a final target type \
                     (parser bug if reached)",
                )
            })?; // final target C
            let first_key = first_key_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&parent_name)));
            let second_key = second_key_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&last_segment_name(through_ty))));
            // JOIN-target column on B. Defaults to `"id"`; overridable
            // via `second_local_key = "..."` for intermediates with a
            // non-`id` PK.
            let second_local_key = second_local_key_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| "id".to_string());

            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }

                    fn __sn_parent_key_to_match_cast(
                        v: ::suprnova::serde_json::Value,
                    ) -> ::std::string::String {
                        match v {
                            ::suprnova::serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        }
                    }

                    let pk_json_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();

                    // Phase 10C audit-fix AF1 - resolve through ExecutorChoice so
                    // raw eager-load SQL honors any ambient CURRENT_TX. Without
                    // this every leaf would run on `DB::connection()` (the pool)
                    // and miss in-tx state under a `DB::transaction` closure.
                    let __sn_exec = ::suprnova::database::transaction::ExecutorChoice::resolve_read(
                        ::core::option::Option::None,
                        ::core::option::Option::None,
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                    ).await?;
                    let db_backend = __sn_exec.backend();
                    // The related rows as the related model's `query()` reads
                    // them: its soft-delete filter and global scopes apply, so
                    // the count or total covers the rows `with` loads. The
                    // source binds come first.
                    let (__sn_source, __sn_source_binds) =
                        ::suprnova::Builder::<#target_ty>::__relation_source(
                            db_backend,
                            "__sn_c",
                        )?;
                    let __sn_offset = __sn_source_binds.len();

                    let mut placeholders: ::std::vec::Vec<::std::string::String> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    let mut binds: ::std::vec::Vec<::suprnova::sea_orm::Value> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    binds.extend(__sn_source_binds);
                    for (i, v) in pk_json_values.iter().enumerate() {
                        let ph = match db_backend {
                            ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                                ::std::format!("${}", i + 1 + __sn_offset)
                            }
                            _ => ::std::string::String::from("?"),
                        };
                        placeholders.push(ph);
                        binds.push(
                            ::suprnova::eloquent::model::json_value_to_sea_value(v),
                        );
                    }

                    let __sn_cast_kw = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::MySql => "CHAR",
                        _ => "TEXT",
                    };
                    let __sn_b_table = <#through_ty as
                        ::suprnova::eloquent::EloquentModel>::TABLE;

                    let __sn_agg_expr: ::std::string::String = match kind {
                        ::suprnova::AggregateKind::Sum => {
                            ::std::format!("SUM(__sn_c.{})", column)
                        }
                        ::suprnova::AggregateKind::Avg => {
                            ::std::format!("AVG(__sn_c.{})", column)
                        }
                        ::suprnova::AggregateKind::Min => {
                            ::std::format!("MIN(__sn_c.{})", column)
                        }
                        ::suprnova::AggregateKind::Max => {
                            ::std::format!("MAX(__sn_c.{})", column)
                        }
                    };

                    let __sn_sql = ::std::format!(
                        "SELECT CAST(__sn_b.{fk} AS {cast}) AS __sn_fk_key, \
                                {agg} AS __sn_agg \
                           FROM {c_table} \
                           JOIN {b_table} __sn_b \
                             ON __sn_c.{second_key} = __sn_b.{slk} \
                          WHERE __sn_b.{fk} IN ({phs}){b_alive} \
                          GROUP BY __sn_b.{fk}",
                        fk = #first_key,
                        second_key = #second_key,
                        slk = #second_local_key,
                        cast = __sn_cast_kw,
                        agg = __sn_agg_expr,
                        c_table = __sn_source,
                        b_table = __sn_b_table,
                        b_alive = ::suprnova::eloquent::relations::__soft_delete_guard::<#through_ty>("__sn_b"),
                        phs = placeholders.join(", "),
                    );

                    let stmt = ::suprnova::sea_orm::Statement::from_sql_and_values(
                        db_backend,
                        &__sn_sql,
                        binds,
                    );
                    // DATA-033: this raw read names its tables to the render
                    // cache, so a write to any of them invalidates the page.
                    ::suprnova::render_cache::collector::observe_table_read(
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::TABLE,
                    );
                    ::suprnova::render_cache::collector::observe_table_read(__sn_b_table);
                    let rows = __sn_exec.query_all(stmt)
                        .await
                        .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;

                    use ::std::collections::HashMap;
                    let mut by_fk: HashMap<
                        ::std::string::String,
                        ::core::option::Option<f64>,
                    > = HashMap::new();
                    for r in rows.iter() {
                        let key: ::std::string::String = r
                            .try_get::<::std::string::String>("", "__sn_fk_key")
                            .unwrap_or_default();
                        let agg: ::core::option::Option<f64> =
                            ::suprnova::database::column_value::__relation_aggregate(r, "__sn_agg")
                                .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;
                        by_fk.insert(key, agg);
                    }

                    // Cache key is the wide `<rel>_<kind>_<col>` form
                    // (P1 fix) so `with_sum` + `with_avg` etc. coexist
                    // on the same row without overwriting.
                    let __sn_agg_key: ::std::string::String =
                        ::suprnova::eloquent::relations::aggregate_cache_key(
                            #name_str, kind, column,
                        );
                    for p in parents.iter_mut() {
                        let key = __sn_parent_key_to_match_cast(
                            ::suprnova::serde_json::to_value(&p.#pk_ident)
                                .unwrap_or(::suprnova::serde_json::Value::Null),
                        );
                        let agg: ::core::option::Option<f64> = by_fk
                            .get(&key)
                            .copied()
                            .unwrap_or(::core::option::Option::None);
                        match kind {
                            ::suprnova::AggregateKind::Sum
                            | ::suprnova::AggregateKind::Avg => {
                                p.__eager.set_aggregate::<f64>(
                                    &__sn_agg_key,
                                    agg.unwrap_or(0.0),
                                );
                            }
                            ::suprnova::AggregateKind::Min
                            | ::suprnova::AggregateKind::Max => {
                                p.__eager.set_aggregate::<::core::option::Option<f64>>(
                                    &__sn_agg_key,
                                    agg,
                                );
                            }
                        }
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::MorphMany | RelationKindAttr::MorphOne => {
            // Server-side GROUP BY aggregate over the child table.
            // Same SQL skeleton as the HasMany aggregate arm but with
            // the extra `<name>_type = '<morph_type>'` predicate so
            // aggregates of children pointing at OTHER morph families
            // are excluded.
            //
            //   SELECT CAST(<id_col> AS TEXT|CHAR) AS __sn_fk_key,
            //          <AGG>(<col>)                AS __sn_agg
            //     FROM <child_table>
            //    WHERE <id_col> IN (?, ?, ...)
            //      AND <type_col> = ?
            //    GROUP BY <id_col>
            //
            // Sum/Avg → f64 with 0.0 empty default. Min/Max →
            // Option<f64> with None empty default. Matches the
            // HasMany contract.
            //
            // MorphOne aggregates work the same way - the per-parent
            // group is 0-or-1 row by contract; the server-side GROUP
            // BY collapses to the single row's column value (or NULL
            // when no row matches, which falls through the Sum|Avg vs
            // Min|Max branch).
            let morph_name = morph_name_or_default(rel);
            let morph_type_value = morph_type_of(input);
            let id_col = format!("{morph_name}_id");
            let type_col = format!("{morph_name}_type");
            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }

                    fn __sn_parent_key_to_match_cast(
                        v: ::suprnova::serde_json::Value,
                    ) -> ::std::string::String {
                        match v {
                            ::suprnova::serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        }
                    }

                    let pk_json_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();

                    // Phase 10C audit-fix AF1 - resolve through ExecutorChoice so
                    // raw eager-load SQL honors any ambient CURRENT_TX. Without
                    // this every leaf would run on `DB::connection()` (the pool)
                    // and miss in-tx state under a `DB::transaction` closure.
                    let __sn_exec = ::suprnova::database::transaction::ExecutorChoice::resolve_read(
                        ::core::option::Option::None,
                        ::core::option::Option::None,
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                    ).await?;
                    let db_backend = __sn_exec.backend();
                    // The related rows as the related model's `query()` reads
                    // them: its soft-delete filter and global scopes apply, so
                    // the count or total covers the rows `with` loads. The
                    // source binds come first.
                    let (__sn_source, __sn_source_binds) =
                        ::suprnova::Builder::<#target_ty>::__relation_source(
                            db_backend,
                            <#target_ty as ::suprnova::eloquent::EloquentModel>::TABLE,
                        )?;
                    let __sn_offset = __sn_source_binds.len();

                    let mut placeholders: ::std::vec::Vec<::std::string::String> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    let mut binds: ::std::vec::Vec<::suprnova::sea_orm::Value> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len() + 1);
                    binds.extend(__sn_source_binds);
                    for (i, v) in pk_json_values.iter().enumerate() {
                        let ph = match db_backend {
                            ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                                ::std::format!("${}", i + 1 + __sn_offset)
                            }
                            _ => ::std::string::String::from("?"),
                        };
                        placeholders.push(ph);
                        binds.push(
                            ::suprnova::eloquent::model::json_value_to_sea_value(v),
                        );
                    }
                    let type_ph = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                            ::std::format!("${}", pk_json_values.len() + 1 + __sn_offset)
                        }
                        _ => ::std::string::String::from("?"),
                    };
                    binds.push(::suprnova::sea_orm::Value::from(#morph_type_value));

                    let __sn_cast_kw = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::MySql => "CHAR",
                        _ => "TEXT",
                    };

                    let __sn_agg_expr: ::std::string::String = match kind {
                        ::suprnova::AggregateKind::Sum => {
                            ::std::format!("SUM({})", column)
                        }
                        ::suprnova::AggregateKind::Avg => {
                            ::std::format!("AVG({})", column)
                        }
                        ::suprnova::AggregateKind::Min => {
                            ::std::format!("MIN({})", column)
                        }
                        ::suprnova::AggregateKind::Max => {
                            ::std::format!("MAX({})", column)
                        }
                    };

                    let __sn_sql = ::std::format!(
                        "SELECT CAST({id} AS {cast}) AS __sn_fk_key, \
                                {agg} AS __sn_agg \
                           FROM {table} \
                          WHERE {id} IN ({phs}) \
                            AND {type_col} = {type_ph} \
                          GROUP BY {id}",
                        id = #id_col,
                        cast = __sn_cast_kw,
                        agg = __sn_agg_expr,
                        table = __sn_source,
                        type_col = #type_col,
                        type_ph = type_ph,
                        phs = placeholders.join(", "),
                    );

                    let stmt = ::suprnova::sea_orm::Statement::from_sql_and_values(
                        db_backend,
                        &__sn_sql,
                        binds,
                    );
                    // DATA-033: this raw read names its tables to the render
                    // cache, so a write to any of them invalidates the page.
                    ::suprnova::render_cache::collector::observe_table_read(
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::TABLE,
                    );
                    let rows = __sn_exec.query_all(stmt)
                        .await
                        .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;

                    use ::std::collections::HashMap;
                    let mut by_fk: HashMap<
                        ::std::string::String,
                        ::core::option::Option<f64>,
                    > = HashMap::new();
                    for r in rows.iter() {
                        let key: ::std::string::String = r
                            .try_get::<::std::string::String>("", "__sn_fk_key")
                            .unwrap_or_default();
                        let agg: ::core::option::Option<f64> =
                            ::suprnova::database::column_value::__relation_aggregate(r, "__sn_agg")
                                .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;
                        by_fk.insert(key, agg);
                    }

                    // Cache key is the wide `<rel>_<kind>_<col>` form
                    // (P1 fix) so `with_sum` + `with_avg` etc. coexist
                    // on the same row without overwriting.
                    let __sn_agg_key: ::std::string::String =
                        ::suprnova::eloquent::relations::aggregate_cache_key(
                            #name_str, kind, column,
                        );
                    for p in parents.iter_mut() {
                        let key = __sn_parent_key_to_match_cast(
                            ::suprnova::serde_json::to_value(&p.#pk_ident)
                                .unwrap_or(::suprnova::serde_json::Value::Null),
                        );
                        let agg: ::core::option::Option<f64> = by_fk
                            .get(&key)
                            .copied()
                            .unwrap_or(::core::option::Option::None);
                        match kind {
                            ::suprnova::AggregateKind::Sum
                            | ::suprnova::AggregateKind::Avg => {
                                p.__eager.set_aggregate::<f64>(
                                    &__sn_agg_key,
                                    agg.unwrap_or(0.0),
                                );
                            }
                            ::suprnova::AggregateKind::Min
                            | ::suprnova::AggregateKind::Max => {
                                p.__eager.set_aggregate::<::core::option::Option<f64>>(
                                    &__sn_agg_key,
                                    agg,
                                );
                            }
                        }
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::MorphToMany => {
            // Polymorphic m2m aggregate. Same JOIN-the-pivot-to-related
            // shape as BelongsToMany's aggregate arm, with the extra
            // `<name>_type = '<self_morph_type>'` predicate on the
            // pivot side so pivot rows pointing at other morph families
            // are excluded.
            //
            //   SELECT CAST(__sn_p.<id_col> AS TEXT|CHAR) AS __sn_fk_key,
            //          AGG(__sn_r.<column>)              AS __sn_agg
            //     FROM <pivot_table>   __sn_p
            //     JOIN <related_table> __sn_r ON __sn_r.<related_pk>
            //                                   = __sn_p.<pivot_related_key>
            //    WHERE __sn_p.<id_col>  IN (?, ?, ...)
            //      AND __sn_p.<type_col> = ?
            //    GROUP BY __sn_p.<id_col>
            //
            // Sum/Avg → f64 with 0.0 empty default. Min/Max →
            // Option<f64> with None empty default. Matches BelongsToMany.
            let pivot_ty = rel.through.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "MorphToMany requires a pivot type (parser bug if reached)",
                )
            })?;
            let morph_name = morph_name_or_default(rel);
            let parent_morph_type = morph_type_of(input);
            let id_col = format!("{morph_name}_id");
            let type_col = format!("{morph_name}_type");
            let pivot_related = pivot_related_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&last_segment_name(target_ty))));
            let related_pk = related_key_expr(rel, target_ty);
            let pivot_table_expr: TokenStream = match pivot_table_override(rel) {
                Some(t) => {
                    let lit = syn::LitStr::new(t, proc_macro2::Span::call_site());
                    quote! { #lit }
                }
                None => quote! {
                    <#pivot_ty as ::suprnova::eloquent::EloquentModel>::TABLE
                },
            };
            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }

                    fn __sn_parent_key_to_match_cast(
                        v: ::suprnova::serde_json::Value,
                    ) -> ::std::string::String {
                        match v {
                            ::suprnova::serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        }
                    }

                    let pk_json_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();

                    // Phase 10C audit-fix AF1 - resolve through ExecutorChoice so
                    // raw eager-load SQL honors any ambient CURRENT_TX. Without
                    // this every leaf would run on `DB::connection()` (the pool)
                    // and miss in-tx state under a `DB::transaction` closure.
                    let __sn_exec = ::suprnova::database::transaction::ExecutorChoice::resolve_read(
                        ::core::option::Option::None,
                        ::core::option::Option::None,
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                    ).await?;
                    let db_backend = __sn_exec.backend();
                    // The related rows as the related model's `query()` reads
                    // them: its soft-delete filter and global scopes apply, so
                    // the count or total covers the rows `with` loads. The
                    // source binds come first.
                    let (__sn_source, __sn_source_binds) =
                        ::suprnova::Builder::<#target_ty>::__relation_source(
                            db_backend,
                            "__sn_r",
                        )?;
                    let __sn_offset = __sn_source_binds.len();

                    let mut placeholders: ::std::vec::Vec<::std::string::String> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    let mut binds: ::std::vec::Vec<::suprnova::sea_orm::Value> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len() + 1);
                    binds.extend(__sn_source_binds);
                    for (i, v) in pk_json_values.iter().enumerate() {
                        let ph = match db_backend {
                            ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                                ::std::format!("${}", i + 1 + __sn_offset)
                            }
                            _ => ::std::string::String::from("?"),
                        };
                        placeholders.push(ph);
                        binds.push(
                            ::suprnova::eloquent::model::json_value_to_sea_value(v),
                        );
                    }
                    let type_ph = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                            ::std::format!("${}", pk_json_values.len() + 1 + __sn_offset)
                        }
                        _ => ::std::string::String::from("?"),
                    };
                    binds.push(::suprnova::sea_orm::Value::from(#parent_morph_type));

                    let __sn_cast_kw = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::MySql => "CHAR",
                        _ => "TEXT",
                    };
                    let __sn_pivot = #pivot_table_expr;

                    let __sn_agg_expr: ::std::string::String = match kind {
                        ::suprnova::AggregateKind::Sum => {
                            ::std::format!("SUM(__sn_r.{})", column)
                        }
                        ::suprnova::AggregateKind::Avg => {
                            ::std::format!("AVG(__sn_r.{})", column)
                        }
                        ::suprnova::AggregateKind::Min => {
                            ::std::format!("MIN(__sn_r.{})", column)
                        }
                        ::suprnova::AggregateKind::Max => {
                            ::std::format!("MAX(__sn_r.{})", column)
                        }
                    };

                    let __sn_sql = ::std::format!(
                        "SELECT CAST(__sn_p.{id} AS {cast}) AS __sn_fk_key, \
                                {agg} AS __sn_agg \
                           FROM {pivot} __sn_p \
                           JOIN {related} ON __sn_r.{related_pk} = __sn_p.{rk} \
                          WHERE __sn_p.{id} IN ({phs}) \
                            AND __sn_p.{type_col} = {type_ph} \
                          GROUP BY __sn_p.{id}",
                        id = #id_col,
                        rk = #pivot_related,
                        related_pk = #related_pk,
                        cast = __sn_cast_kw,
                        agg = __sn_agg_expr,
                        pivot = __sn_pivot,
                        related = __sn_source,
                        type_col = #type_col,
                        type_ph = type_ph,
                        phs = placeholders.join(", "),
                    );

                    let stmt = ::suprnova::sea_orm::Statement::from_sql_and_values(
                        db_backend,
                        &__sn_sql,
                        binds,
                    );
                    // DATA-033: this raw read names its tables to the render
                    // cache, so a write to any of them invalidates the page.
                    ::suprnova::render_cache::collector::observe_table_read(__sn_pivot);
                    ::suprnova::render_cache::collector::observe_table_read(
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::TABLE,
                    );
                    let rows = __sn_exec.query_all(stmt)
                        .await
                        .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;

                    use ::std::collections::HashMap;
                    let mut by_fk: HashMap<
                        ::std::string::String,
                        ::core::option::Option<f64>,
                    > = HashMap::new();
                    for r in rows.iter() {
                        let key: ::std::string::String = r
                            .try_get::<::std::string::String>("", "__sn_fk_key")
                            .unwrap_or_default();
                        let agg: ::core::option::Option<f64> =
                            ::suprnova::database::column_value::__relation_aggregate(r, "__sn_agg")
                                .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;
                        by_fk.insert(key, agg);
                    }

                    // Cache key is the wide `<rel>_<kind>_<col>` form
                    // (P1 fix) so `with_sum` + `with_avg` etc. coexist
                    // on the same row without overwriting.
                    let __sn_agg_key: ::std::string::String =
                        ::suprnova::eloquent::relations::aggregate_cache_key(
                            #name_str, kind, column,
                        );
                    for p in parents.iter_mut() {
                        let key = __sn_parent_key_to_match_cast(
                            ::suprnova::serde_json::to_value(&p.#pk_ident)
                                .unwrap_or(::suprnova::serde_json::Value::Null),
                        );
                        let agg: ::core::option::Option<f64> = by_fk
                            .get(&key)
                            .copied()
                            .unwrap_or(::core::option::Option::None);
                        match kind {
                            ::suprnova::AggregateKind::Sum
                            | ::suprnova::AggregateKind::Avg => {
                                p.__eager.set_aggregate::<f64>(
                                    &__sn_agg_key,
                                    agg.unwrap_or(0.0),
                                );
                            }
                            ::suprnova::AggregateKind::Min
                            | ::suprnova::AggregateKind::Max => {
                                p.__eager.set_aggregate::<::core::option::Option<f64>>(
                                    &__sn_agg_key,
                                    agg,
                                );
                            }
                        }
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        RelationKindAttr::MorphedByMany => {
            // Inverse polymorphic m2m aggregate. Same JOIN shape as
            // MorphToMany but the grouped column is the pivot's FK to
            // the m2m side (tag_id), and the JOIN target is the morph
            // target's own table (Post / Video). Type predicate uses
            // the explicit `target_morph_type`.
            //
            //   SELECT CAST(__sn_p.<pivot_fk> AS TEXT|CHAR) AS __sn_fk_key,
            //          AGG(__sn_r.<column>)               AS __sn_agg
            //     FROM <pivot_table>   __sn_p
            //     JOIN <related_table> __sn_r ON __sn_r.<related_pk>
            //                                   = __sn_p.<id_col>
            //    WHERE __sn_p.<pivot_fk> IN (?, ?, ...)
            //      AND __sn_p.<type_col> = ?
            //    GROUP BY __sn_p.<pivot_fk>
            let pivot_ty = rel.through.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "MorphedByMany requires a pivot type (parser bug if reached)",
                )
            })?;
            let morph_name = morph_name_or_default(rel);
            let target_morph_type = target_morph_type_override(rel).ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "MorphedByMany requires `target_morph_type = \"...\"` \
                         (parse-time validation should reject this earlier)",
                )
            })?;
            let id_col = format!("{morph_name}_id");
            let type_col = format!("{morph_name}_type");
            let pivot_fk = pivot_fk_override(rel)
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}_id", to_snake(&parent_name)));
            let related_pk = related_key_expr(rel, target_ty);
            let pivot_table_expr: TokenStream = match pivot_table_override(rel) {
                Some(t) => {
                    let lit = syn::LitStr::new(t, proc_macro2::Span::call_site());
                    quote! { #lit }
                }
                None => quote! {
                    <#pivot_ty as ::suprnova::eloquent::EloquentModel>::TABLE
                },
            };
            Ok(Some(quote! {
                #name_str => {
                    if parents.is_empty() { return ::core::result::Result::Ok(()); }

                    fn __sn_parent_key_to_match_cast(
                        v: ::suprnova::serde_json::Value,
                    ) -> ::std::string::String {
                        match v {
                            ::suprnova::serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        }
                    }

                    let pk_json_values: ::std::vec::Vec<::suprnova::serde_json::Value> = parents
                        .iter()
                        .map(|p| ::suprnova::serde_json::to_value(&p.#pk_ident)
                            .unwrap_or(::suprnova::serde_json::Value::Null))
                        .collect();

                    // Phase 10C audit-fix AF1 - resolve through ExecutorChoice so
                    // raw eager-load SQL honors any ambient CURRENT_TX. Without
                    // this every leaf would run on `DB::connection()` (the pool)
                    // and miss in-tx state under a `DB::transaction` closure.
                    let __sn_exec = ::suprnova::database::transaction::ExecutorChoice::resolve_read(
                        ::core::option::Option::None,
                        ::core::option::Option::None,
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                    ).await?;
                    let db_backend = __sn_exec.backend();
                    // The related rows as the related model's `query()` reads
                    // them: its soft-delete filter and global scopes apply, so
                    // the count or total covers the rows `with` loads. The
                    // source binds come first.
                    let (__sn_source, __sn_source_binds) =
                        ::suprnova::Builder::<#target_ty>::__relation_source(
                            db_backend,
                            "__sn_r",
                        )?;
                    let __sn_offset = __sn_source_binds.len();

                    let mut placeholders: ::std::vec::Vec<::std::string::String> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len());
                    let mut binds: ::std::vec::Vec<::suprnova::sea_orm::Value> =
                        ::std::vec::Vec::with_capacity(pk_json_values.len() + 1);
                    binds.extend(__sn_source_binds);
                    for (i, v) in pk_json_values.iter().enumerate() {
                        let ph = match db_backend {
                            ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                                ::std::format!("${}", i + 1 + __sn_offset)
                            }
                            _ => ::std::string::String::from("?"),
                        };
                        placeholders.push(ph);
                        binds.push(
                            ::suprnova::eloquent::model::json_value_to_sea_value(v),
                        );
                    }
                    let type_ph = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::Postgres => {
                            ::std::format!("${}", pk_json_values.len() + 1 + __sn_offset)
                        }
                        _ => ::std::string::String::from("?"),
                    };
                    binds.push(::suprnova::sea_orm::Value::from(#target_morph_type));

                    let __sn_cast_kw = match db_backend {
                        ::suprnova::sea_orm::DatabaseBackend::MySql => "CHAR",
                        _ => "TEXT",
                    };
                    let __sn_pivot = #pivot_table_expr;

                    let __sn_agg_expr: ::std::string::String = match kind {
                        ::suprnova::AggregateKind::Sum => {
                            ::std::format!("SUM(__sn_r.{})", column)
                        }
                        ::suprnova::AggregateKind::Avg => {
                            ::std::format!("AVG(__sn_r.{})", column)
                        }
                        ::suprnova::AggregateKind::Min => {
                            ::std::format!("MIN(__sn_r.{})", column)
                        }
                        ::suprnova::AggregateKind::Max => {
                            ::std::format!("MAX(__sn_r.{})", column)
                        }
                    };

                    let __sn_sql = ::std::format!(
                        "SELECT CAST(__sn_p.{fk} AS {cast}) AS __sn_fk_key, \
                                {agg} AS __sn_agg \
                           FROM {pivot} __sn_p \
                           JOIN {related} ON __sn_r.{related_pk} = __sn_p.{id_col} \
                          WHERE __sn_p.{fk} IN ({phs}) \
                            AND __sn_p.{type_col} = {type_ph} \
                          GROUP BY __sn_p.{fk}",
                        fk = #pivot_fk,
                        id_col = #id_col,
                        related_pk = #related_pk,
                        cast = __sn_cast_kw,
                        agg = __sn_agg_expr,
                        pivot = __sn_pivot,
                        related = __sn_source,
                        type_col = #type_col,
                        type_ph = type_ph,
                        phs = placeholders.join(", "),
                    );

                    let stmt = ::suprnova::sea_orm::Statement::from_sql_and_values(
                        db_backend,
                        &__sn_sql,
                        binds,
                    );
                    // DATA-033: this raw read names its tables to the render
                    // cache, so a write to any of them invalidates the page.
                    ::suprnova::render_cache::collector::observe_table_read(__sn_pivot);
                    ::suprnova::render_cache::collector::observe_table_read(
                        <#target_ty as ::suprnova::eloquent::EloquentModel>::TABLE,
                    );
                    let rows = __sn_exec.query_all(stmt)
                        .await
                        .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;

                    use ::std::collections::HashMap;
                    let mut by_fk: HashMap<
                        ::std::string::String,
                        ::core::option::Option<f64>,
                    > = HashMap::new();
                    for r in rows.iter() {
                        let key: ::std::string::String = r
                            .try_get::<::std::string::String>("", "__sn_fk_key")
                            .unwrap_or_default();
                        let agg: ::core::option::Option<f64> =
                            ::suprnova::database::column_value::__relation_aggregate(r, "__sn_agg")
                                .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;
                        by_fk.insert(key, agg);
                    }

                    // Cache key is the wide `<rel>_<kind>_<col>` form
                    // (P1 fix) so `with_sum` + `with_avg` etc. coexist
                    // on the same row without overwriting.
                    let __sn_agg_key: ::std::string::String =
                        ::suprnova::eloquent::relations::aggregate_cache_key(
                            #name_str, kind, column,
                        );
                    for p in parents.iter_mut() {
                        let key = __sn_parent_key_to_match_cast(
                            ::suprnova::serde_json::to_value(&p.#pk_ident)
                                .unwrap_or(::suprnova::serde_json::Value::Null),
                        );
                        let agg: ::core::option::Option<f64> = by_fk
                            .get(&key)
                            .copied()
                            .unwrap_or(::core::option::Option::None);
                        match kind {
                            ::suprnova::AggregateKind::Sum
                            | ::suprnova::AggregateKind::Avg => {
                                p.__eager.set_aggregate::<f64>(
                                    &__sn_agg_key,
                                    agg.unwrap_or(0.0),
                                );
                            }
                            ::suprnova::AggregateKind::Min
                            | ::suprnova::AggregateKind::Max => {
                                p.__eager.set_aggregate::<::core::option::Option<f64>>(
                                    &__sn_agg_key,
                                    agg,
                                );
                            }
                        }
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
        // MorphTo: same story as `emit_count_arm` - the targets are
        // different tables, so there's no single SQL shape for a
        // polymorphic aggregate. Emit an explicit error rather than
        // falling through to the dispatcher catch-all.
        RelationKindAttr::MorphTo => Ok(Some(quote! {
            #name_str => {
                return ::core::result::Result::Err(
                    ::suprnova::FrameworkError::internal(::std::format!(
                        "with_sum / with_avg / with_min / with_max on the MorphTo relation \
                         `{}` of `{}` is not supported: its targets are different tables; \
                         load it with `with([\"{}\"])` instead",
                        #name_str,
                        #parent_name,
                        #name_str,
                    )),
                );
            }
        })),
    }
}

/// `__recurse_eager_load` arm - T9 ships nested-path resolution.
///
/// Each arm walks the already-loaded child rows in `self.__eager`,
/// peels one segment off `rest`, and recurses into the child type's
/// `__eager_load` (head) plus per-row `__recurse_eager_load` (tail).
///
/// For collection kinds (HasMany / BelongsToMany / Through /
/// MorphMany / MorphToMany / MorphedByMany): the cache holds
/// `Vec<R>`; we take `&mut [R]` via `get_many_mut::<R>(name)` and
/// call `R::__eager_load(rest_head, &mut refs, db, None)`. Then for
/// each child, recurse with the remaining segments.
///
/// For single-value kinds (HasOne / BelongsTo / MorphOne /
/// HasOneThrough): the cache holds `Option<R>`; we take `&mut R` via
/// `get_one_mut::<R>(name)` if `Some`, build a one-element slice, and
/// recurse the same way. `None` means "FK was null, nothing to walk
/// into" - silently return Ok.
///
/// For `MorphTo` the cache holds the per-family enum. The arm matches
/// the variant, which names the concrete target type, and loads the
/// rest of the path through that model's own dispatcher. Every target
/// of the family must declare the next segment (see
/// [`emit_morph_family_check`]).
fn emit_recurse_arm(input: &ModelInput, rel: &RelationDecl) -> Result<Option<TokenStream>> {
    let name_str = rel.name.to_string();
    let target_ty: &syn::Type = match rel.kind {
        // Through kinds: the user-facing target is the FINAL `C`, not
        // the intermediate `B`. Same treatment as `emit_relation_accessors`.
        RelationKindAttr::HasManyThrough | RelationKindAttr::HasOneThrough => {
            rel.through.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "HasOneThrough / HasManyThrough require a final target type \
                     (parser bug if reached)",
                )
            })?
        }
        _ => &rel.target,
    };

    match rel.kind {
        // Collection kinds - `__eager.get_many_mut::<R>(name)` returns
        // `Option<&mut Vec<R>>`. None means "relation wasn't loaded";
        // the orchestrator only calls `__recurse_eager_load` after a
        // successful `__eager_load` on the same name, so the None
        // branch is defensive (returns Ok silently).
        RelationKindAttr::HasMany
        | RelationKindAttr::BelongsToMany
        | RelationKindAttr::HasManyThrough
        | RelationKindAttr::MorphMany
        | RelationKindAttr::MorphToMany
        | RelationKindAttr::MorphedByMany => Ok(Some(quote! {
            #name_str => {
                let children: ::std::option::Option<&mut ::std::vec::Vec<#target_ty>> =
                    self.__eager.get_many_mut::<#target_ty>(#name_str);
                if let ::core::option::Option::Some(children_vec) = children {
                    if children_vec.is_empty() {
                        return ::core::result::Result::Ok(());
                    }
                    let (head, tail) = match rest.split_once('.') {
                        ::core::option::Option::Some((h, t)) => (h, ::core::option::Option::Some(t)),
                        ::core::option::Option::None => (rest, ::core::option::Option::None),
                    };
                    {
                        // Per-row partition of the cached children. When
                        // `missing_only` is true, only children without
                        // `head` cached get bulk-loaded; the rest are
                        // already-loaded and stay untouched. This is the
                        // P3 contract for `Collection::load_missing` -
                        // partition every level of a dotted path, not
                        // just the top one. The borrow scope ends before
                        // the recursive walk below so the parents slice
                        // is free for the per-row recursion.
                        let mut refs: ::std::vec::Vec<&mut #target_ty> = if missing_only {
                            children_vec
                                .iter_mut()
                                .filter(|c| {
                                    !<#target_ty as ::suprnova::EagerLoadDispatch>::has_eager(c, head)
                                })
                                .collect()
                        } else {
                            children_vec.iter_mut().collect()
                        };
                        if !refs.is_empty() {
                            <#target_ty as ::suprnova::EagerLoadDispatch>::eager_load(
                                head,
                                refs.as_mut_slice(),
                                db,
                                ::core::option::Option::None,
                            )
                            .await?;
                        }
                    }
                    if let ::core::option::Option::Some(more) = tail {
                        for c in children_vec.iter_mut() {
                            <#target_ty as ::suprnova::EagerLoadDispatch>::recurse_eager_load(
                                c, head, more, db, missing_only,
                            )
                            .await?;
                        }
                    }
                }
                return ::core::result::Result::Ok(());
            }
        })),

        // Single-value kinds - walk the loaded row through a
        // one-element slice. None means the FK was null / no matching
        // parent; nothing to recurse into, return Ok.
        RelationKindAttr::HasOne
        | RelationKindAttr::BelongsTo
        | RelationKindAttr::HasOneThrough
        | RelationKindAttr::MorphOne => Ok(Some(quote! {
            #name_str => {
                let child: ::std::option::Option<&mut #target_ty> =
                    self.__eager.get_one_mut::<#target_ty>(#name_str);
                if let ::core::option::Option::Some(child_row) = child {
                    let (head, tail) = match rest.split_once('.') {
                        ::core::option::Option::Some((h, t)) => (h, ::core::option::Option::Some(t)),
                        ::core::option::Option::None => (rest, ::core::option::Option::None),
                    };
                    // `missing_only` skips the bulk-load if the
                    // single cached child already has the next
                    // segment populated. Same contract as the
                    // collection-kind arm: load_missing only fills
                    // the missing tail.
                    let already_loaded: bool = missing_only
                        && <#target_ty as ::suprnova::EagerLoadDispatch>::has_eager(child_row, head);
                    if !already_loaded {
                        let mut refs: ::std::vec::Vec<&mut #target_ty> =
                            ::std::vec![child_row];
                        <#target_ty as ::suprnova::EagerLoadDispatch>::eager_load(
                            head,
                            refs.as_mut_slice(),
                            db,
                            ::core::option::Option::None,
                        )
                        .await?;
                    }
                    if let ::core::option::Option::Some(more) = tail {
                        // Re-fetch the mut borrow after the dispatcher
                        // call dropped its slice - the dispatcher only
                        // mutates the `__eager` cache on each row, the
                        // row identity is unchanged.
                        let again: ::std::option::Option<&mut #target_ty> =
                            self.__eager.get_one_mut::<#target_ty>(#name_str);
                        if let ::core::option::Option::Some(c) = again {
                            <#target_ty as ::suprnova::EagerLoadDispatch>::recurse_eager_load(
                                c, head, more, db, missing_only,
                            )
                            .await?;
                        }
                    }
                }
                return ::core::result::Result::Ok(());
            }
        })),

        // MorphTo: the cache holds the per-family enum. The variant
        // names the concrete target, so the rest of the path runs
        // through that model's own dispatcher. `Unknown` has nothing to
        // walk into.
        RelationKindAttr::MorphTo => {
            let enum_ident = morph_enum_ident(rel);
            let targets = morph_targets(rel).ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "MorphTo requires `targets = [...]` (parser bug if reached)",
                )
            })?;
            let variant_idents = morph_variant_idents(targets);
            let family_check = emit_morph_family_check(input, rel, targets);
            let arms: Vec<TokenStream> = targets
                .iter()
                .zip(variant_idents.iter())
                .map(|(ty, variant)| {
                    quote! {
                        #enum_ident::#variant(row) => {
                            // `missing_only` skips the bulk-load when the
                            // row already has the next segment - the same
                            // contract as the single-value arms.
                            let already_loaded: bool = missing_only
                                && <#ty as ::suprnova::EagerLoadDispatch>::has_eager(row, head);
                            if !already_loaded {
                                let mut refs: ::std::vec::Vec<&mut #ty> =
                                    ::std::vec![&mut *row];
                                <#ty as ::suprnova::EagerLoadDispatch>::eager_load(
                                    head,
                                    refs.as_mut_slice(),
                                    db,
                                    ::core::option::Option::None,
                                )
                                .await?;
                            }
                            if let ::core::option::Option::Some(more) = tail {
                                <#ty as ::suprnova::EagerLoadDispatch>::recurse_eager_load(
                                    row, head, more, db, missing_only,
                                )
                                .await?;
                            }
                        }
                    }
                })
                .collect();
            Ok(Some(quote! {
                #name_str => {
                    let (head, tail) = match rest.split_once('.') {
                        ::core::option::Option::Some((h, t)) => (h, ::core::option::Option::Some(t)),
                        ::core::option::Option::None => (rest, ::core::option::Option::None),
                    };
                    #family_check
                    let loaded: ::std::option::Option<&mut #enum_ident> =
                        self.__eager.get_one_mut::<#enum_ident>(#name_str);
                    if let ::core::option::Option::Some(value) = loaded {
                        match value {
                            #( #arms )*
                            #enum_ident::Unknown(..) => {}
                        }
                    }
                    return ::core::result::Result::Ok(());
                }
            }))
        }
    }
}

/// The `Model::__morph_owner` override for a model that declares
/// `MorphTo` relations: one arm per relation, each reading the
/// relation's `<name>_id` and `<name>_type` values and resolving the
/// owner through the relation's fetch helper, so the parent-touch
/// cascade picks the owner exactly as `.get()` and the eager loader do.
/// A value comes from `attrs` when it carries the column and from the
/// row `base` otherwise. Returns an empty stream for a model without
/// `MorphTo` relations, which keeps the trait default.
///
/// Emitted inside the `impl Model` block by `derive_eloquent`, because
/// the cascade that calls it is a trait default in the framework.
pub(super) fn emit_morph_owner_method(input: &ModelInput) -> TokenStream {
    let mut arms: Vec<TokenStream> = Vec::new();
    for rel in input
        .relations
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter(|rel| rel.kind == RelationKindAttr::MorphTo)
    {
        let name_str = rel.name.to_string();
        let morph_name = morph_name_or_default(rel);
        let id_col = format!("{morph_name}_id");
        let type_col = format!("{morph_name}_type");
        let fetch_ident = quote::format_ident!("{}Fetch", morph_enum_ident(rel));
        arms.push(quote! {
            #name_str => {
                let morph_id = column(#id_col).unwrap_or(::suprnova::serde_json::Value::Null);
                let morph_type = match column(#type_col) {
                    ::core::option::Option::Some(::suprnova::serde_json::Value::String(s)) => s,
                    _ => ::std::string::String::new(),
                };
                #fetch_ident::__owner_of(&morph_id, &morph_type)
            }
        });
    }
    if arms.is_empty() {
        return TokenStream::new();
    }
    quote! {
        fn __morph_owner(
            relation: &str,
            base: ::core::option::Option<&Self>,
            attrs: &::suprnova::eloquent::Attrs,
        ) -> ::core::result::Result<
            ::core::option::Option<::suprnova::eloquent::relations::morph::MorphOwner>,
            ::suprnova::FrameworkError,
        > {
            let column = |name: &str| {
                attrs.get(name).cloned().or_else(|| {
                    base.and_then(|row| <Self as ::suprnova::eloquent::Model>::field_value(row, name))
                })
            };
            match relation {
                #(#arms)*
                other => ::core::result::Result::Err(::suprnova::FrameworkError::internal(
                    ::std::format!(
                        "model `{}` has no MorphTo relation `{}`",
                        ::std::any::type_name::<Self>(),
                        other,
                    ),
                )),
            }
        }
    }
}

/// The guard every nested load through a `MorphTo` relation runs
/// first: each declared target must have the relation the path names
/// next (`head`). It is checked for the whole family, not only for the
/// targets the loaded rows happen to hold, so a path that loads today
/// cannot start failing when the first row of another type appears. A
/// target without the relation is an error that names it - never a
/// skipped load, since a path that silently stays unloaded is an N+1
/// that nobody sees.
///
/// The emitted code expects `head` and `rest` in scope.
fn emit_morph_family_check(
    input: &ModelInput,
    rel: &RelationDecl,
    targets: &[syn::Type],
) -> TokenStream {
    let struct_ident = &input.item.ident;
    let name_str = rel.name.to_string();
    let checks: Vec<TokenStream> = targets
        .iter()
        .map(|ty| {
            quote! {
                if ::suprnova::find_relation::<#ty>(head).is_none() {
                    return ::core::result::Result::Err(
                        ::suprnova::FrameworkError::internal(::std::format!(
                            "model `{}` cannot eager load `{}.{}`: `{}` is a target of its \
                             MorphTo relation `{}` and has no relation `{}`",
                            ::core::stringify!(#struct_ident),
                            #name_str,
                            rest,
                            ::std::any::type_name::<#ty>(),
                            #name_str,
                            head,
                        )),
                    );
                }
            }
        })
        .collect();
    quote! { #( #checks )* }
}

/// `__recurse_eager_load_batched` arm - the collection-wide form of
/// [`emit_recurse_arm`]. Instead of walking a single parent's cached
/// children, it gathers EVERY parent's children of the relation into one
/// slice, loads the next path segment with a single IN query, and
/// recurses batched. This keeps a dotted path (`"posts.comments"`) a
/// constant number of queries regardless of parent count; the per-parent
/// form re-issues the next-segment query once per parent (N+1).
fn emit_recurse_batched_arm(input: &ModelInput, rel: &RelationDecl) -> Result<Option<TokenStream>> {
    let name_str = rel.name.to_string();
    let target_ty: &syn::Type = match rel.kind {
        RelationKindAttr::HasManyThrough | RelationKindAttr::HasOneThrough => {
            rel.through.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "HasOneThrough / HasManyThrough require a final target type \
                     (parser bug if reached)",
                )
            })?
        }
        _ => &rel.target,
    };

    // Holding `&mut` to children reached through a slice of `&mut Self`
    // across the whole batch defeats the borrow checker (the double
    // indirection can't escape the gather loop). So we TAKE the children
    // out by value (no borrows held across the gather), load + recurse on
    // the owned flat slice, then put them back in original order. MorphTo
    // takes its per-family enums the same way and splits them by variant
    // (see its arm below).
    //
    // The load + recurse step over the owned `Vec<#target_ty>` is shared;
    // only the take and the put-back differ by cardinality.
    let split_rest = quote! {
        let (head, tail) = match rest.split_once('.') {
            ::core::option::Option::Some((h, t)) => (h, ::core::option::Option::Some(t)),
            ::core::option::Option::None => (rest, ::core::option::Option::None),
        };
    };
    // When no parent holds a child, the rest of the path still walks the
    // target's relations with no rows, so a `MorphTo` further down checks
    // its whole family as it would with rows.
    let walk_without_children = quote! {
        if let ::core::option::Option::Some(more) = tail {
            let mut none: ::std::vec::Vec<&mut #target_ty> = ::std::vec::Vec::new();
            <#target_ty as ::suprnova::EagerLoadDispatch>::recurse_eager_load_batched(
                none.as_mut_slice(),
                head,
                more,
                db,
                missing_only,
            )
            .await?;
        }
    };
    let process_owned = quote! {
        {
            // Bulk-load the next segment ONCE across every gathered child.
            // `missing_only` filters to children still missing `head`.
            let mut refs: ::std::vec::Vec<&mut #target_ty> = if missing_only {
                owned
                    .iter_mut()
                    .filter(|c| !<#target_ty as ::suprnova::EagerLoadDispatch>::has_eager(c, head))
                    .collect()
            } else {
                owned.iter_mut().collect()
            };
            if !refs.is_empty() {
                <#target_ty as ::suprnova::EagerLoadDispatch>::eager_load(
                    head,
                    refs.as_mut_slice(),
                    db,
                    ::core::option::Option::None,
                )
                .await?;
            }
        }
        if let ::core::option::Option::Some(more) = tail {
            let mut refs: ::std::vec::Vec<&mut #target_ty> = owned.iter_mut().collect();
            <#target_ty as ::suprnova::EagerLoadDispatch>::recurse_eager_load_batched(
                refs.as_mut_slice(),
                head,
                more,
                db,
                missing_only,
            )
            .await?;
        }
    };

    match rel.kind {
        RelationKindAttr::HasMany
        | RelationKindAttr::BelongsToMany
        | RelationKindAttr::HasManyThrough
        | RelationKindAttr::MorphMany
        | RelationKindAttr::MorphToMany
        | RelationKindAttr::MorphedByMany => Ok(Some(quote! {
            #name_str => {
                #split_rest
                // Take every parent's children out by value. The guard
                // puts each parent's own children back, in order, when it
                // drops: after the load, and also when the load fails or
                // the caller stops awaiting it.
                let mut __sn_taken = ::suprnova::eloquent::relations::__TakenRows::take(
                    parents,
                    |p: &mut Self| p.__eager.take_many::<#target_ty>(#name_str),
                    |p: &mut Self, children: ::std::vec::Vec<#target_ty>| {
                        p.__eager.set_many(#name_str, children)
                    },
                );
                let owned: &mut ::std::vec::Vec<#target_ty> = __sn_taken.rows();
                if owned.is_empty() {
                    #walk_without_children
                    return ::core::result::Result::Ok(());
                }
                #process_owned
                return ::core::result::Result::Ok(());
            }
        })),

        RelationKindAttr::HasOne
        | RelationKindAttr::BelongsTo
        | RelationKindAttr::HasOneThrough
        | RelationKindAttr::MorphOne => Ok(Some(quote! {
            #name_str => {
                #split_rest
                // Same take-and-guard as the many kinds: the guard puts
                // each parent's child back however the load ends.
                let mut __sn_taken = ::suprnova::eloquent::relations::__TakenRows::take(
                    parents,
                    |p: &mut Self| {
                        p.__eager
                            .take_one::<#target_ty>(#name_str)
                            .map(|child| ::std::vec![child])
                    },
                    |p: &mut Self, mut child: ::std::vec::Vec<#target_ty>| {
                        p.__eager.set_one(#name_str, child.pop())
                    },
                );
                let owned: &mut ::std::vec::Vec<#target_ty> = __sn_taken.rows();
                if owned.is_empty() {
                    #walk_without_children
                    return ::core::result::Result::Ok(());
                }
                #process_owned
                return ::core::result::Result::Ok(());
            }
        })),

        // MorphTo: take every parent's per-family enum out by value,
        // then, for each target type present, gather that type's rows
        // into one slice and run the rest of the path through the
        // target's own dispatcher: one query per (target type, next
        // relation), whatever the parent count. `Unknown` values ride
        // along untouched and go back where they came from.
        RelationKindAttr::MorphTo => {
            let enum_ident = morph_enum_ident(rel);
            let targets = morph_targets(rel).ok_or_else(|| {
                syn::Error::new_spanned(
                    &rel.name,
                    "MorphTo requires `targets = [...]` (parser bug if reached)",
                )
            })?;
            let variant_idents = morph_variant_idents(targets);
            let family_check = emit_morph_family_check(input, rel, targets);
            let per_target: Vec<TokenStream> = targets
                .iter()
                .zip(variant_idents.iter())
                .map(|(ty, variant)| {
                    quote! {
                        {
                            // Bulk-load the next segment ONCE across every
                            // row of this target. `missing_only` filters to
                            // rows still missing `head`.
                            let mut refs: ::std::vec::Vec<&mut #ty> = owned
                                .iter_mut()
                                .filter_map(|v| match v {
                                    #enum_ident::#variant(row) => ::core::option::Option::Some(row),
                                    _ => ::core::option::Option::None,
                                })
                                .filter(|c| {
                                    !missing_only
                                        || !<#ty as ::suprnova::EagerLoadDispatch>::has_eager(c, head)
                                })
                                .collect();
                            if !refs.is_empty() {
                                <#ty as ::suprnova::EagerLoadDispatch>::eager_load(
                                    head,
                                    refs.as_mut_slice(),
                                    db,
                                    ::core::option::Option::None,
                                )
                                .await?;
                            }
                        }
                        if let ::core::option::Option::Some(more) = tail {
                            let mut refs: ::std::vec::Vec<&mut #ty> = owned
                                .iter_mut()
                                .filter_map(|v| match v {
                                    #enum_ident::#variant(row) => ::core::option::Option::Some(row),
                                    _ => ::core::option::Option::None,
                                })
                                .collect();
                            if !refs.is_empty() {
                                <#ty as ::suprnova::EagerLoadDispatch>::recurse_eager_load_batched(
                                    refs.as_mut_slice(),
                                    head,
                                    more,
                                    db,
                                    missing_only,
                                )
                                .await?;
                            }
                        }
                    }
                })
                .collect();
            Ok(Some(quote! {
                #name_str => {
                    let (head, tail) = match rest.split_once('.') {
                        ::core::option::Option::Some((h, t)) => (h, ::core::option::Option::Some(t)),
                        ::core::option::Option::None => (rest, ::core::option::Option::None),
                    };
                    #family_check
                    // The guard puts every value back into the parent it
                    // came from however the load ends.
                    let mut __sn_taken = ::suprnova::eloquent::relations::__TakenRows::take(
                        parents,
                        |p: &mut Self| {
                            p.__eager
                                .take_one::<#enum_ident>(#name_str)
                                .map(|value| ::std::vec![value])
                        },
                        |p: &mut Self, mut value: ::std::vec::Vec<#enum_ident>| {
                            p.__eager.set_one(#name_str, value.pop())
                        },
                    );
                    let owned: &mut ::std::vec::Vec<#enum_ident> = __sn_taken.rows();
                    if owned.is_empty() {
                        return ::core::result::Result::Ok(());
                    }
                    #( #per_target )*
                    return ::core::result::Result::Ok(());
                }
            }))
        }
    }
}

/// Emit `Self::with([...])` - the minimal eager-load entrypoint T2
/// ships so the eager-load test in `eloquent_relations_one_to_one.rs`
/// can run. T9 will expand this with `with_count` / `with_sum`-`max`
/// / `with_where` / nested-path resolution. For T2 we only need the
/// flat list of relation names + a `Builder<Self>` that invokes the
/// per-model `__eager_load` dispatcher for each name at fetch time.
///
/// The wired path:
///
/// 1. `Self::with(["profile"])` returns a `Builder<Self>` with an
///    eager spec list attached.
/// 2. `Builder::get` (on a builder carrying eager specs) issues the
///    base SELECT, calls `M::__eager_load(name, &mut [&mut row, ...], db, None)`
///    for each spec, and returns the rows with their `__eager` cache
///    populated.
fn emit_with_helper(struct_ident: &syn::Ident) -> TokenStream {
    quote! {
        impl #struct_ident {
            #[doc = "Open a `Builder<Self>` that eager-loads the listed relations."]
            #[doc = ""]
            #[doc = "Names can be flat (`\"posts\"`) or dotted (`\"posts.comments\"`)."]
            #[doc = "Dotted paths drive nested-path recursion at fetch time - "]
            #[doc = "`User::with([\"posts.comments\"]).get()` runs three queries"]
            #[doc = "(users, posts, comments) and zero N+1 SELECTs."]
            pub fn with<I, S>(relations: I) -> ::suprnova::Builder<Self>
            where
                I: ::core::iter::IntoIterator<Item = S>,
                S: ::core::convert::Into<::std::string::String>,
            {
                <Self as ::suprnova::eloquent::Model>::query()
                    .with(relations)
            }

            #[doc = "Open a `Builder<Self>` that eager-loads each listed relation's row count."]
            #[doc = ""]
            #[doc = "Reads via the macro-emitted `<rel>_count()` accessor on each row."]
            pub fn with_count<I, S>(relations: I) -> ::suprnova::Builder<Self>
            where
                I: ::core::iter::IntoIterator<Item = S>,
                S: ::core::convert::Into<::std::string::String>,
            {
                <Self as ::suprnova::eloquent::Model>::query()
                    .with_count(relations)
            }

            #[doc = "Open a `Builder<Self>` that eager-loads SUM(col) over a relation."]
            #[doc = ""]
            #[doc = "Reads via `parent.__eager.get_aggregate::<f64>(relation_name)`."]
            pub fn with_sum<S1, S2>(t: (S1, S2)) -> ::suprnova::Builder<Self>
            where
                S1: ::core::convert::Into<::std::string::String>,
                S2: ::core::convert::Into<::std::string::String>,
            {
                <Self as ::suprnova::eloquent::Model>::query()
                    .with_sum(t)
            }

            #[doc = "Open a `Builder<Self>` that eager-loads AVG(col) over a relation."]
            #[doc = ""]
            #[doc = "Reads via `parent.__eager.get_aggregate::<f64>(relation_name)`."]
            pub fn with_avg<S1, S2>(t: (S1, S2)) -> ::suprnova::Builder<Self>
            where
                S1: ::core::convert::Into<::std::string::String>,
                S2: ::core::convert::Into<::std::string::String>,
            {
                <Self as ::suprnova::eloquent::Model>::query()
                    .with_avg(t)
            }

            #[doc = "Open a `Builder<Self>` that eager-loads MIN(col) over a relation."]
            #[doc = ""]
            #[doc = "Reads via `parent.__eager.get_aggregate::<Option<f64>>(relation_name)`."]
            pub fn with_min<S1, S2>(t: (S1, S2)) -> ::suprnova::Builder<Self>
            where
                S1: ::core::convert::Into<::std::string::String>,
                S2: ::core::convert::Into<::std::string::String>,
            {
                <Self as ::suprnova::eloquent::Model>::query()
                    .with_min(t)
            }

            #[doc = "Open a `Builder<Self>` that eager-loads MAX(col) over a relation."]
            #[doc = ""]
            #[doc = "Reads via `parent.__eager.get_aggregate::<Option<f64>>(relation_name)`."]
            pub fn with_max<S1, S2>(t: (S1, S2)) -> ::suprnova::Builder<Self>
            where
                S1: ::core::convert::Into<::std::string::String>,
                S2: ::core::convert::Into<::std::string::String>,
            {
                <Self as ::suprnova::eloquent::Model>::query()
                    .with_max(t)
            }
        }
    }
}

/// Render a [`syn::Type`] back to its compact source form for the
/// inventory's `target_type_name` literal.
///
/// `proc_macro2::TokenStream::to_string()` inserts spaces between
/// every adjacent token pair, so a type written as `Vec<Post>` round
/// trips through `quote!(#ty).to_string()` as `"Vec < Post >"`. Phase
/// 8 admin renders this string in the relation listing UI - the
/// padded form is visually wrong. Stripping every space yields the
/// compact `"Vec<Post>"` / `"Option<i64>"` form users actually wrote.
///
/// This is correct for the common cases (single idents, generic
/// applications, qualified paths). The rare case of a function-typed
/// target (`Box<dyn Fn(i32) -> bool>`) would have its inner spaces
/// stripped too - but relation targets are model structs, not closure
/// types, so the trade-off is fine. If we ever need fancier formatting
/// we can swap this for a `syn::Type` walker.
fn format_target_type(ty: &syn::Type) -> String {
    quote::quote!(#ty).to_string().replace(' ', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_target_type_strips_spaces_around_generics() {
        // Bare ident - pass-through.
        let ty: syn::Type = syn::parse_str("Post").unwrap();
        assert_eq!(format_target_type(&ty), "Post");
    }

    #[test]
    fn format_target_type_vec_target_round_trips_without_spaces() {
        // `Vec<Post>` is the common collection-of-models shape - must
        // never appear as `"Vec < Post >"` in the admin UI.
        let ty: syn::Type = syn::parse_str("Vec<Post>").unwrap();
        assert_eq!(format_target_type(&ty), "Vec<Post>");
    }

    #[test]
    fn format_target_type_option_target_round_trips_without_spaces() {
        // `Option<i64>` is what nullable FK fields would surface as if
        // ever used as a target ident. Same no-padding rule.
        let ty: syn::Type = syn::parse_str("Option<i64>").unwrap();
        assert_eq!(format_target_type(&ty), "Option<i64>");
    }

    #[test]
    fn format_target_type_qualified_path_round_trips_without_spaces() {
        // Fully qualified `crate::models::Post` should keep its colons
        // and lose any `quote!`-inserted padding.
        let ty: syn::Type = syn::parse_str("crate::models::Post").unwrap();
        assert_eq!(format_target_type(&ty), "crate::models::Post");
    }

    #[test]
    fn format_target_type_nested_generics_round_trip_without_spaces() {
        // Nested generic - pivot models that are themselves generic
        // round-trip cleanly.
        let ty: syn::Type = syn::parse_str("Vec<Option<Post>>").unwrap();
        assert_eq!(format_target_type(&ty), "Vec<Option<Post>>");
    }
}
