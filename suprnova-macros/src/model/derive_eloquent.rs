//! Emits Eloquent trait impls for the user's struct.
//!
//! Task 3 shipped the `EloquentModel` marker - enough to register the
//! type as a Suprnova model and bridge the user's struct name (e.g.
//! `User`) to the inner-module SeaORM `Entity` / `Column` types
//! (`user::Entity`, `user::Column`).
//!
//! Task 4 (this expansion) adds:
//! - `From<#inner::Model> for User` and `From<User> for #inner::Model`
//!   so the trait surface can round-trip between Suprnova and SeaORM
//! - `Default for User` so `replicate` can build a fresh shell
//! - `impl Model for User` with the per-model hook fills
//!   (`primary_key_value`, `apply_attrs_to_active_model`, ...)
//! - `impl ReplicateExt for User` with field-by-field clone
//! - `impl FirstOrCreate for User` with the `from_attrs_unsaved` hook
//!
//! Tasks 6 / 7a-c / 8 / 9 / 10 each extend this file with their slice
//! of the generated impl (fillable, casts, accessors/mutators,
//! timestamps, soft deletes).

use proc_macro2::TokenStream;
use quote::quote;
use syn::Result;

use super::casts;
use super::parse::{ModelInput, RelationKindAttr};
use super::relations;
use super::serialization;

pub fn emit(input: &ModelInput) -> Result<TokenStream> {
    let struct_ident = &input.item.ident;
    let module_name = input.module_name();
    // The literal table string from the macro attribute - used in the
    // `const TABLE` initialiser further down. SeaORM 1.1's
    // `EntityName::table_name` isn't `const fn`, so we can't call it
    // here; the parser captured the same value the runtime would
    // return.
    let table = &input.table;

    // `touches` names relations, and both attributes are parsed from
    // the same `#[model(...)]` invocation - so a name that doesn't
    // resolve, or resolves to a kind the cascade can't write, is a
    // build error rather than a surprise on the first save.
    for name in &input.touches {
        match input
            .relations
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .find(|r| r.name == name.as_str())
        {
            Some(rel)
                if matches!(
                    rel.kind,
                    RelationKindAttr::BelongsTo | RelationKindAttr::MorphTo
                ) => {}
            Some(rel) => {
                return Err(syn::Error::new_spanned(
                    &rel.name,
                    format!(
                        "`touches = [\"{name}\"]` needs a `BelongsTo` or `MorphTo` relation - \
                         `{name}` is declared as {:?}. Only the owning side of a relation can \
                         be touched.",
                        rel.kind,
                    ),
                ));
            }
            None => {
                return Err(syn::Error::new_spanned(
                    struct_ident,
                    format!(
                        "`touches = [\"{name}\"]` names a relation that isn't declared. Add \
                         `{name}: BelongsTo<Parent>` (or a `MorphTo`) to this model's \
                         `relations = {{ ... }}`.",
                    ),
                ));
            }
        }
    }

    let pk_name = &input.primary_key;
    let pk_ident = quote::format_ident!("{pk_name}");

    // Phase 10C T12 - emit a `default_connection_name` override when
    // the user declared `#[model(connection = "name")]`. The trait
    // default returns `None` (i.e. "fall through to the routing
    // chain"); the override returns `Some(<literal>)` so the
    // `ExecutorChoice::resolve_{read,write}` steps 4 picks it up.
    //
    // `__primary__` is legal here - it pins reads to the default
    // pool even when a `__read_replica__` is registered. Any other
    // literal goes through `DB::named(name)` at resolve time.
    let default_connection_impl = match input.connection.as_deref() {
        Some(name) => quote! {
            fn default_connection_name() -> ::core::option::Option<&'static str> {
                ::core::option::Option::Some(#name)
            }
        },
        None => quote! {},
    };

    // T6 - wire the parsed `fillable = [...]` / `guarded = [...]`
    // attributes into the per-model `fillable_filter()` impl. Mutual
    // exclusion was already enforced at parse time (parse.rs:67-72), so
    // the `(Some, Some)` arm here is unreachable.
    let fillable_impl = match (&input.fillable, &input.guarded) {
        (Some(list), None) => {
            let lits = list.iter().map(|s| quote! { #s });
            quote! {
                fn fillable_filter() -> ::suprnova::eloquent::Fillable {
                    ::suprnova::eloquent::Fillable::fillable(::std::vec![#(#lits),*])
                }
            }
        }
        (None, Some(list)) => {
            let lits = list.iter().map(|s| quote! { #s });
            quote! {
                fn fillable_filter() -> ::suprnova::eloquent::Fillable {
                    ::suprnova::eloquent::Fillable::guarded(::std::vec![#(#lits),*])
                }
            }
        }
        (None, None) => {
            // Default: guard the macro-parsed PK name so models with
            // `primary_key = "uid"` still have their PK protected from
            // mass assignment.
            //
            // Exception: `#[model(unique_id = "...")]` models opt the
            // PK back into the fillable surface. The id is a generated
            // string and Laravel allows caller-supplied overrides for
            // `HasUuids` / `HasUlids` models - Suprnova matches that.
            if input.unique_id.is_some() {
                quote! {
                    fn fillable_filter() -> ::suprnova::eloquent::Fillable {
                        ::suprnova::eloquent::Fillable::allow_all()
                    }
                }
            } else {
                quote! {
                    fn fillable_filter() -> ::suprnova::eloquent::Fillable {
                        ::suprnova::eloquent::Fillable::guarded(::std::vec![#pk_name])
                    }
                }
            }
        }
        (Some(_), Some(_)) => unreachable!(
            "fillable / guarded mutual exclusion validated at parse time (parse.rs:67-72)"
        ),
    };

    let fields = match &input.item.fields {
        syn::Fields::Named(named) => &named.named,
        _ => unreachable!("validated in derive_seaorm"),
    };

    // Phase 10B T1 - exclude the auto-injected `__eager` / `__pivot`
    // fields from every per-column code path. They're runtime scratch
    // state, not database columns, and the inner SeaORM Model doesn't
    // have them - so `From<Model> for User`, `From<User> for Model`,
    // `apply_attrs_to_active_model`, `replicate_with`, and `fill` all
    // need to step around them. The two reverse paths
    // (`From<Model> for User`, `Default for User`, `replicate_with`)
    // that *construct* a `User` value initialise the two fields via
    // `Default::default()` - `EagerLoadCache::default()` returns the
    // empty cache and `Option::None` is the pivot default.
    let field_idents: Vec<_> = fields
        .iter()
        .map(|f| f.ident.as_ref().expect("named").clone())
        .filter(|i| {
            let s = i.to_string();
            s != "__eager" && s != "__pivot"
        })
        .collect();
    let field_strs: Vec<String> = field_idents.iter().map(|i| i.to_string()).collect();

    // Phase 10C T5b - emit `field_value(&self, name) -> Option<Value>`
    // off the same filtered field-ident slice that drives every other
    // per-column code path. The result is one match-arm per column
    // calling `serde_json::to_value(&self.<field>)`; unknown names
    // return `None`. `Collection<M>`'s string-keyed methods route
    // through this accessor.
    let field_value_method = serialization::emit_field_value(&field_idents);

    // `__morph_owner(relation, base, attrs)`: resolves the owner row a
    // `MorphTo` relation of a row points at, for the parent-touch
    // cascade `Model::__plan_touches` prepares before the statement. Empty
    // for a model without `MorphTo` relations, which keeps the trait
    // default.
    let morph_owner_method = relations::emit_morph_owner_method(input);

    // Every Self { ... } constructor that materialises a user struct
    // without a `self` in scope (`Default`, and the two hydration
    // constructors below) must initialise the auto-injected `__eager`
    // / `__pivot` slots so the struct literal stays exhaustive.
    // `EagerLoadCache::default()` returns the empty cache;
    // `Option::<...>::None` is the pivot default. `Default` builds a model
    // nothing has read, so empty defaults are the correct shape for it.
    let relations_fields_init = quote! {
        __eager: ::core::default::Default::default(),
        __pivot: ::core::default::Default::default(),
    };

    // The two hydration constructors (`From<inner::Model>` and
    // `try_from_storage`) build a model from a row the database returned,
    // so its cache keeps a copy of that row: the base `Model::save`
    // compares against to tell what changed, and what
    // `Model::get_original` reads. The copy is taken before the field
    // arms move the row's fields out, under a name no column can collide
    // with.
    let loaded_relations_fields_init = quote! {
        __eager: ::suprnova::EagerLoadCache::__with_loaded_row(__suprnova_loaded_row),
        __pivot: ::core::default::Default::default(),
    };

    // `replicate_with` is the lone Self { ... } site that DOES have a
    // `self` in scope and must preserve relation state. Laravel's
    // `Model::replicate` carries the source's loaded relations onto
    // the replica (`clone $user` preserves `$user->posts`), and our
    // `EagerLoadCache` docstring explicitly promises the same parity.
    // The copy leaves out the cache's mark of a multi-row query: the
    // replica is a model built in the process, which lazy-loading
    // prevention never refuses. The pivot context is a cheap `Arc`
    // clone and follows the same parity rule.
    let replicate_relations_init = quote! {
        __eager: self.__eager.__clone_for_replica(),
        __pivot: self.__pivot.clone(),
    };

    // T7a - route per-field through `casts::apply_arm`. Fields with a
    // declared cast (in `input.casts`) decode JSON → Runtime →
    // `Cast::to_storage` → ActiveModel; uncast fields use the same
    // direct `serde_json::from_value` shape as before T7a.
    //
    // T8 - fields listed in `mutators = [...]` take precedence over
    // the cast/direct apply paths. The mutator arm builds a scratch
    // `Self::default()`, calls `scratch.set_<field>(val)?`, then
    // serialises the transformed value back into JSON and feeds it
    // into the same cast/direct apply logic. This means a field can
    // declare both a mutator and a cast; the mutator transforms the
    // runtime value, the cast handles the storage shape.
    let mutators_list = &input.mutators;
    let apply_arms = field_idents
        .iter()
        .zip(field_strs.iter())
        .map(|(ident, name)| {
            if mutators_list.iter().any(|m| m == name) {
                casts::mutator_apply_arm(name, ident, input.cast_for_field(name))
            } else {
                casts::apply_arm(name, ident, input.cast_for_field(name))
            }
        });

    let replicate_arms = field_idents
        .iter()
        .zip(field_strs.iter())
        .map(|(ident, name)| {
            let is_pk = name == pk_name;
            // Mirror Laravel's `replicate()` semantics: reset the PK
            // AND the auto-timestamps so the next save fills them with
            // NOW rather than carrying the source row's timing forward.
            // Without this reset a replicated row claims to predate its
            // actual existence.
            let is_auto_timestamp =
                input.timestamps && (name == &input.created_at || name == &input.updated_at);
            if is_pk || is_auto_timestamp {
                quote! { #ident: ::core::default::Default::default() }
            } else {
                quote! {
                    #ident: if except.iter().any(|e| e == #name) {
                        ::core::default::Default::default()
                    } else {
                        self.#ident.clone()
                    }
                }
            }
        });

    // T8 - when the field name is in `mutators = [...]`, route
    // through `s.set_<field>(value)?` so unsaved-instance builders
    // (`first_or_new`) apply the same transformation as `create` /
    // `update`. Non-mutator fields take the direct
    // `serde_json::from_value` apply.
    //
    // P2-10: that direct apply used to end in `.unwrap_or_default()`,
    // so a value of the wrong type became the field's `Default` and the
    // call returned `Ok`. `first_or_new({"age": "abc"})` produced a row
    // with `age = 0` and no indication anything had gone wrong - the
    // same silent-wrong-data class as P2-09(a)'s dropped eager loads,
    // and reachable wherever attrs are built from request input. The
    // enclosing `from_attrs_unsaved` already returns
    // `Result<_, FrameworkError>` and the mutator arm beside this one
    // already uses `?`, so surfacing the error costs no signature
    // change at all.
    let from_attrs_unsaved_arms =
        field_idents
            .iter()
            .zip(field_strs.iter())
            .map(|(ident, name)| {
                if mutators_list.iter().any(|m| m == name) {
                    let setter = quote::format_ident!("set_{}", ident);
                    quote! {
                        #name => {
                            s.#setter(v.clone())?;
                        }
                    }
                } else {
                    quote! {
                        #name => {
                            s.#ident = ::suprnova::serde_json::from_value(v.clone())
                                .map_err(|__decode_err| ::suprnova::FrameworkError::validation(
                                    #name,
                                    ::std::format!(
                                        "could not decode the supplied value: {}",
                                        __decode_err
                                    ),
                                ))?;
                        }
                    }
                }
            });

    // For `save()`, we need every non-PK field marked as Set so SeaORM
    // emits a real UPDATE statement. The SeaORM-derived
    // `IntoActiveModel<ActiveModel> for Model` produces all-Unchanged
    // values (see DeriveActiveModel's `impl From<Model> for ActiveModel`
    // at sea-orm-macros 1.1.20/src/derives/active_model.rs:92), which
    // makes the resulting UPDATE a no-op. So we build the ActiveModel
    // explicitly: PK as Unchanged (it's the WHERE clause), every other
    // field as Set.
    //
    // T7a - cast fields route through `Cast::to_storage` here so the
    // inner Model's Storage-typed field matches what `derive_seaorm`
    // emitted. Without this the ActiveModel write would type-check
    // against the user's Runtime type and miscompile.
    let active_model_for_update_arms =
        field_idents
            .iter()
            .zip(field_strs.iter())
            .map(|(ident, name)| {
                let is_pk = name == pk_name;
                casts::active_model_update_stmt(ident, is_pk, input.cast_for_field(name))
            });

    // T7a - read-direction arm for `From<inner::Model> for UserStruct`.
    // Cast fields call `Cast::from_storage` to inflate the storage
    // shape back into the runtime type.
    let from_storage_arms = field_idents
        .iter()
        .zip(field_strs.iter())
        .map(|(ident, name)| casts::from_storage_arm(ident, input.cast_for_field(name)));

    // T7a - write-direction arm for `From<UserStruct> for inner::Model`.
    // Cast fields call `Cast::to_storage` to flatten the runtime
    // shape back into the storage type the inner Model expects.
    let to_storage_arms = field_idents
        .iter()
        .zip(field_strs.iter())
        .map(|(ident, name)| casts::to_storage_arm(ident, input.cast_for_field(name)));

    // Fallible analogues of the read/write arms above. These feed
    // `Model::try_from_storage` / `try_into_storage`, the `?`-propagating
    // hydration/dehydration the framework's own CRUD hot paths route
    // through. The infallible `From` impls below keep using the
    // panicking arms as the documented escape hatch.
    let try_from_storage_arms = field_idents
        .iter()
        .zip(field_strs.iter())
        .map(|(ident, name)| casts::try_from_storage_arm(ident, input.cast_for_field(name)));

    let try_to_storage_arms = field_idents
        .iter()
        .zip(field_strs.iter())
        .map(|(ident, name)| casts::try_to_storage_arm(ident, input.cast_for_field(name)));

    // `Model::__decoded_values_equal`: a save compares stored rows to tell
    // what it changed, and a column whose cast stores a new value on every
    // write, an encrypted one, would always differ. Such a column is
    // compared by its decoded value instead. Each cast's own
    // `DETERMINISTIC_STORAGE` says which casts those are, so the arm of a
    // deterministic cast is never taken. A model without casts keeps the
    // trait default.
    let decoded_equal_arms: Vec<TokenStream> = field_idents
        .iter()
        .zip(field_strs.iter())
        .filter_map(|(ident, name)| {
            let cast_ty = input.cast_for_field(name)?;
            Some(quote! {
                #name if !<#cast_ty as ::suprnova::eloquent::casts::Cast>::DETERMINISTIC_STORAGE => {
                    let __before_value =
                        <#cast_ty as ::suprnova::eloquent::casts::Cast>::from_storage(&before.#ident)?;
                    let __after_value =
                        <#cast_ty as ::suprnova::eloquent::casts::Cast>::from_storage(&after.#ident)?;
                    let __encode = |__value| {
                        ::suprnova::serde_json::to_value(__value).map_err(|__error| {
                            ::suprnova::FrameworkError::internal(::std::format!(
                                "could not compare the values of `{}` after a save: {}",
                                #name,
                                __error,
                            ))
                        })
                    };
                    ::core::result::Result::Ok(
                        __encode(&__before_value)? == __encode(&__after_value)?,
                    )
                }
            })
        })
        .collect();
    let decoded_equal_method = if decoded_equal_arms.is_empty() {
        quote! {}
    } else {
        quote! {
            fn __decoded_values_equal(
                column: &str,
                before: &(dyn ::std::any::Any + ::core::marker::Send + ::core::marker::Sync),
                after: &(dyn ::std::any::Any + ::core::marker::Send + ::core::marker::Sync),
            ) -> ::core::result::Result<bool, ::suprnova::FrameworkError> {
                let (::core::option::Option::Some(before), ::core::option::Option::Some(after)) = (
                    before.downcast_ref::<#module_name::Model>(),
                    after.downcast_ref::<#module_name::Model>(),
                ) else {
                    return ::core::result::Result::Ok(false);
                };
                match column {
                    #(#decoded_equal_arms)*
                    _ => ::core::result::Result::Ok(false),
                }
            }
        }
    };

    // Phase 10C T6 - emit the `to_array` + `__append_accessor`
    // overrides on the `Model` trait when the model declares any of
    // `hidden = [...]` / `visible = [...]` / `appends = [...]`. When
    // none of those attributes are declared, the emitters return empty
    // token streams and the trait defaults win (which strip the
    // auto-injected `__eager` / `__pivot` keys but apply no filtering).
    //
    // Moving filter emission to the trait (instead of an inherent
    // `to_json` on the user struct) means `Collection<M>::to_array`,
    // resource responses, and any other generic Model consumer routes
    // through the same hidden/visible/appends pipeline.
    let visible_slice_opt: Option<&[String]> = input.visible.as_deref();
    let to_array_override =
        serialization::emit_to_array_override(&input.hidden, visible_slice_opt, &input.appends);
    let append_accessor_dispatch = serialization::emit_append_accessor_dispatch(&input.appends);

    // T8 - `fill` body arms. Mutator-routed fields call
    // `self.set_<field>(value.clone())?`; non-mutator fields
    // direct-assign via `serde_json::from_value(...)`. We skip the
    // duplicate match-pattern hazard by partitioning the lists.
    let fill_mutator_arms = mutators_list.iter().map(|name| {
        let setter = quote::format_ident!("set_{}", name);
        quote! {
            #name => { self.#setter(v.clone())?; }
        }
    });
    let fill_direct_arms =
        field_idents
            .iter()
            .zip(field_strs.iter())
            .filter_map(|(ident, name)| {
                if mutators_list.iter().any(|m| m == name) {
                    // Mutator-listed: skip - its arm is emitted above. Emitting
                    // both arms would produce duplicate match patterns and a
                    // hard rustc error.
                    None
                } else {
                    // P2-10: see `from_attrs_unsaved_arms` above. `fill` already
                    // returns `Result<(), FrameworkError>`, so a malformed value
                    // reports which field it was rather than silently becoming
                    // that field's `Default`.
                    Some(quote! {
                        #name => {
                            self.#ident = ::suprnova::serde_json::from_value(v.clone())
                                .map_err(|__decode_err| ::suprnova::FrameworkError::validation(
                                    #name,
                                    ::std::format!(
                                        "could not decode the supplied value: {}",
                                        __decode_err
                                    ),
                                ))?;
                        }
                    })
                }
            });

    // T9 - auto-managed timestamps + `Touchable` impl + `touches`
    // marker. The macro auto-detects timestamps in `parse.rs` (both
    // columns present → enabled, neither → disabled, partial →
    // compile_error) and auto-injects `AsDateTime` casts for the
    // timestamp columns. Here we wire the runtime injections:
    //
    // - `apply_attrs_to_active_model`: AFTER the for-loop, always
    //   overwrite `updated_at` with NOW so both `create()` and
    //   `update(attrs)` bump it. Then set `created_at` to NOW only if
    //   it's still `NotSet` (create path). On `update(attrs)` it's
    //   `Unchanged(old)` from `into_active_model()`, which the check
    //   correctly leaves alone.
    //
    // - `into_active_model_for_update`: AFTER the per-field arms,
    //   overwrite `updated_at` with NOW so `save()` bumps it. The
    //   arms have already populated `created_at` with the in-memory
    //   value (Set), which we keep.
    //
    // - `impl Touchable for #struct_ident`: builds a minimal
    //   ActiveModel with PK Unchanged + updated_at Set(now), runs an
    //   UPDATE. No other column changes.
    //
    // - `TOUCHES` const: stores the parsed `touches = [...]` list
    //   even on models without timestamps. Read by `Model::touch_owners`
    //   (framework/src/eloquent/model.rs), which runs one
    //   `UPDATE <owner> SET <updated_at> = NOW() WHERE <key> = ?` per
    //   declared touch relation after create/update/delete - the cascade
    //   is real, not a no-op.
    let timestamps_enabled = input.timestamps;
    let updated_at_col = &input.updated_at;
    let created_col_ident = quote::format_ident!("{}", input.created_at);
    let updated_col_ident = quote::format_ident!("{}", input.updated_at);
    // The key of a touch's `UPDATE ... WHERE`, stored through its cast
    // when it has one (a `u64` key).
    let touch_key = casts::active_model_update_stmt(&pk_ident, true, input.cast_for_field(pk_name));
    let created_at_cast = input
        .cast_for_field(&input.created_at)
        .cloned()
        .unwrap_or_else(|| syn::parse_quote!(::suprnova::AsDateTime));
    let updated_at_cast = input
        .cast_for_field(&input.updated_at)
        .cloned()
        .unwrap_or_else(|| syn::parse_quote!(::suprnova::AsDateTime));
    // A timestamp field declared `Option<DateTime<Utc>>` stores "now" as
    // `Some(now)` through its optional cast; `stamp` writes the value each
    // timestamp write hands the cast.
    let created_optional = input.is_optional_datetime(&input.created_at);
    let updated_optional = input.is_optional_datetime(&input.updated_at);
    let stamp = |now: TokenStream, optional: bool| {
        if optional {
            quote! { &::core::option::Option::Some(#now) }
        } else {
            quote! { &#now }
        }
    };
    let updated_now = stamp(quote! { __suprnova_now }, updated_optional);
    let created_now = stamp(quote! { __suprnova_now }, created_optional);
    let updated_touch_now = stamp(quote! { now }, updated_optional);
    let updated_storage_now = stamp(quote! { *now }, updated_optional);

    // ---- unique_id PK generation (HasUuids / HasUlids analogue) -------
    //
    // When `#[model(unique_id = "uuid" | "uuid_v4" | "ulid")]` is set,
    // the macro emits two artifacts:
    //
    //   1. `impl HasUniqueId for #struct_ident` - exposes the kind
    //      and the per-model generator override hook.
    //   2. A pre-INSERT inject block (`unique_id_inject_apply`) that
    //      checks whether the user supplied the PK; if not, the
    //      generated ID is set as a fresh string into the PK column.
    let unique_id_kind_token = input.unique_id.as_deref().map(|s| match s {
        "uuid" | "uuid_v7" => quote! { ::suprnova::eloquent::UniqueIdKind::UuidV7 },
        "uuid_v4" => quote! { ::suprnova::eloquent::UniqueIdKind::UuidV4 },
        "ulid" => quote! { ::suprnova::eloquent::UniqueIdKind::Ulid },
        _ => quote! { ::suprnova::eloquent::UniqueIdKind::UuidV7 }, // parser validates
    });
    let unique_id_impl = if let Some(kind) = &unique_id_kind_token {
        quote! {
            impl ::suprnova::eloquent::HasUniqueId for #struct_ident {
                const UNIQUE_ID_KIND: ::suprnova::eloquent::UniqueIdKind = #kind;
            }
        }
    } else {
        quote! {}
    };
    let unique_id_inject_apply = if unique_id_kind_token.is_some() {
        quote! {
            // If the caller did NOT supply the PK column (NotSet on
            // the ActiveModel), generate a fresh string ID and stamp
            // it in. Mirrors Laravel's HasUuids/HasUlids `creating`
            // hook behaviour exactly: caller-supplied IDs win,
            // generator fills the gap.
            if matches!(
                am.#pk_ident,
                ::suprnova::sea_orm::ActiveValue::NotSet
            ) {
                am.#pk_ident = ::suprnova::sea_orm::Set(
                    <Self as ::suprnova::eloquent::HasUniqueId>::new_unique_id().into()
                );
            }
        }
    } else {
        quote! {}
    };

    let timestamp_inject_apply = if timestamps_enabled {
        quote! {
            // Always bump updated_at - covers create() and
            // update(attrs) in one place.
            let __suprnova_now = ::suprnova::clock::now();
            am.#updated_col_ident = ::suprnova::sea_orm::Set(
                <#updated_at_cast as ::suprnova::eloquent::casts::Cast>::to_storage(
                    #updated_now,
                )?,
            );
            // Set created_at only on first save (NotSet); update()
            // builds the AM from a row, so created_at is
            // Unchanged(old) and the check correctly leaves it alone.
            if matches!(
                am.#created_col_ident,
                ::suprnova::sea_orm::ActiveValue::NotSet
            ) {
                am.#created_col_ident = ::suprnova::sea_orm::Set(
                    <#created_at_cast as ::suprnova::eloquent::casts::Cast>::to_storage(
                        #created_now,
                    )?,
                );
            }
        }
    } else {
        quote! {}
    };

    // An insert stamps the timestamps its builder left at their default,
    // as Laravel's insert stamps those the caller did not set: a
    // replica's (which `replicate` resets) and a factory row's that its
    // definition does not name. A value the builder set, a backdated
    // factory row's, stays.
    let persist_stamp = if timestamps_enabled {
        let created_value = if created_optional {
            quote! { ::core::option::Option::Some(__suprnova_now) }
        } else {
            quote! { __suprnova_now }
        };
        let updated_value = if updated_optional {
            quote! { ::core::option::Option::Some(__suprnova_now) }
        } else {
            quote! { __suprnova_now }
        };
        quote! {
            fn __suprnova_is_default<T>(value: &T) -> bool
            where
                T: ::core::default::Default + ::core::cmp::PartialEq,
            {
                *value == T::default()
            }
            let __suprnova_now = ::suprnova::clock::now();
            if __suprnova_is_default(&self.#created_col_ident) {
                self.#created_col_ident = #created_value;
            }
            if __suprnova_is_default(&self.#updated_col_ident) {
                self.#updated_col_ident = #updated_value;
            }
        }
    } else {
        quote! {}
    };

    let timestamp_inject_save = if timestamps_enabled {
        quote! {
            // save() rebuilds the AM from `self`; the arms have
            // already populated updated_at with the in-memory value.
            // Overwrite with NOW so save() bumps the column even when
            // the caller didn't touch it.
            let __suprnova_now = ::suprnova::clock::now();
            am.#updated_col_ident = ::suprnova::sea_orm::Set(
                <#updated_at_cast as ::suprnova::eloquent::casts::Cast>::to_storage(
                    #updated_now,
                )?,
            );
        }
    } else {
        quote! {}
    };

    // An owner's `updated_at` is written by the child's touch cascade, not
    // by the owner's own save, so the owner tells the cascade how its cast
    // stores the time. Without timestamps the trait default stands: the
    // cascade skips such an owner before it would ask.
    let updated_at_storage_impl = if timestamps_enabled {
        quote! {
            fn updated_at_storage(
                now: &::suprnova::chrono::DateTime<::suprnova::chrono::Utc>,
            ) -> ::core::result::Result<::suprnova::sea_orm::Value, ::suprnova::FrameworkError> {
                ::core::result::Result::Ok(::suprnova::sea_orm::Value::from(
                    <#updated_at_cast as ::suprnova::eloquent::casts::Cast>::to_storage(
                        #updated_storage_now,
                    )?,
                ))
            }
        }
    } else {
        quote! {}
    };

    // Comparisons in a query bind through the column's cast, so a native
    // date-time column is compared with a native parameter. Casts that
    // store text or numbers answer `None` and the value binds as it is.
    // The key binds through its cast too: a `u64` key above `i64::MAX`
    // then binds as an unsigned number rather than as text, which
    // Postgres refuses to compare with its `BIGINT`.
    let mut cast_arms: Vec<TokenStream> = input
        .casts
        .iter()
        .map(|(ident, ty)| {
            let name = ident.to_string();
            quote! {
                #name => <#ty as ::suprnova::eloquent::casts::Cast>::bind_json(value),
            }
        })
        .collect();
    // An integer field without a cast binds a `u64` above `i64::MAX` as the
    // unsigned number it is, so the builder settles a comparison with it
    // (no such column holds it) and refuses to write it, where as text
    // Postgres refused the comparison and SQLite stored a rounded real.
    cast_arms.extend(fields.iter().filter_map(|field| {
        let ident = field.ident.as_ref()?;
        let has_cast = input.casts.iter().any(|(cast, _)| cast == ident);
        if has_cast || !super::parse::is_integer_below_u64(&field.ty) {
            return None;
        }
        let name = ident.to_string();
        Some(quote! {
            #name => ::suprnova::eloquent::casts::__bind_integer(value),
        })
    }));
    let bind_column_impl = if cast_arms.is_empty() {
        quote! {}
    } else {
        quote! {
            fn bind_column(
                column: &str,
                value: &::suprnova::serde_json::Value,
            ) -> ::core::option::Option<::suprnova::sea_orm::Value> {
                let column = match column.rsplit_once('.') {
                    ::core::option::Option::Some((prefix, name))
                        if prefix == <Self as ::suprnova::eloquent::EloquentModel>::TABLE =>
                    {
                        name
                    }
                    ::core::option::Option::Some(_) => return ::core::option::Option::None,
                    ::core::option::Option::None => column,
                };
                match column {
                    #(#cast_arms)*
                    _ => ::core::option::Option::None,
                }
            }
        }
    };

    let touchable_impl = if timestamps_enabled {
        quote! {
            #[::suprnova::__async_trait::async_trait]
            impl ::suprnova::eloquent::Touchable for #struct_ident {
                async fn touch(&self) -> ::core::result::Result<(), ::suprnova::FrameworkError> {
                    // Honour the `without_touching` scope: when the
                    // task-local flag is on, touch() is a no-op.
                    // Mirrors Laravel's `Model::withoutTouching`. The
                    // per-type `without_touching_on::<Self, _, _>` scope
                    // is the same silence, narrowed to this type - it's
                    // exactly Laravel's `withoutTouchingOn([static::class])`.
                    if ::suprnova::eloquent::touches_disabled()
                        || ::suprnova::eloquent::touches_ignored_for(
                            ::std::any::TypeId::of::<Self>(),
                        )
                    {
                        return ::core::result::Result::Ok(());
                    }
                    let now = ::suprnova::clock::now();
                    let mut am = <<#module_name::Entity as ::suprnova::EntityTrait>::ActiveModel
                        as ::core::default::Default>::default();
                    #touch_key
                    am.#updated_col_ident = ::suprnova::sea_orm::Set(
                        <#updated_at_cast as ::suprnova::eloquent::casts::Cast>::to_storage(
                            #updated_touch_now,
                        )?,
                    );
                    // Route through `resolve_write` so the timestamp
                    // bump honours the full write-side precedence chain -
                    // tx override → ambient CURRENT_TX → per-model
                    // `#[model(connection = ".")]` → primary. The bare
                    // `resolve()` only consults CURRENT_TX, silently
                    // ignoring per-model connection routing.
                    let exec = ::suprnova::database::transaction::ExecutorChoice::resolve_write(
                        ::core::option::Option::None,
                        ::core::option::Option::None,
                        <Self as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                    )
                    .await?;
                    exec.update_active(am)
                        .await
                        .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;
                    ::suprnova::render_cache::orm::after_model_write(self).await?;
                    ::core::result::Result::Ok(())
                }
            }
        }
    } else {
        quote! {}
    };

    let touches_const = if input.touches.is_empty() {
        quote! {}
    } else {
        let touches_lits = input.touches.iter().map(|t| quote! { #t });
        quote! {
            const TOUCHES: &'static [&'static str] = &[ #(#touches_lits),* ];
        }
    };

    // T10 - soft deletes. When `#[model(soft_deletes)]` is set:
    //
    // - `Model::query()` needs no override: the builder it returns folds
    //   the soft-delete filter in when the query runs, reading the column
    //   from `SOFT_DELETES_COLUMN`. `with_trashed()` / `only_trashed()`
    //   start the same builder with the `"soft_deletes"` opt-out set, so
    //   the registered global scopes still apply to them.
    // - `impl SoftDeletes for #struct` exposes the column name + the
    //   `is_trashed()` accessor.
    // - Inherent `delete(self)` / `restore(self)` / `force_delete(self)`
    //   / `trashed(&self)` / `with_trashed()` / `only_trashed()` swap in
    //   the tombstone semantics. The lifecycle methods take `self` by
    //   value to match `Model::delete(self)`'s signature - Rust's
    //   method-resolution prefers an inherent method over a trait
    //   default only when they share auto-ref level, so an inherent
    //   `delete(&self)` override would silently lose to the trait's
    //   `delete(self)`.
    let soft_deletes_enabled = input.soft_deletes;
    // Whether a factory insert leaves the primary key for the database.
    let auto_increment = input.auto_increment;
    let soft_delete_col = &input.soft_deletes_column;
    let soft_delete_col_ident = quote::format_ident!("{}", soft_delete_col);
    let soft_delete_cast = input
        .cast_for_field(soft_delete_col)
        .cloned()
        .unwrap_or_else(|| syn::parse_quote!(::suprnova::AsOptionalDateTime));
    let key_type = &input.key_type;

    // A key with a cast, a `u64` key, is read through the cast's storage
    // type: SeaORM decodes a `u64` on MySQL only.
    let decode_key_impl = match input.cast_for_field(pk_name) {
        Some(cast_ty) => quote! {
            fn __decode_key(
                row: &::suprnova::sea_orm::QueryResult,
                column: &str,
            ) -> ::core::result::Result<Self::Key, ::suprnova::sea_orm::DbErr> {
                let stored = row.try_get::<
                    <#cast_ty as ::suprnova::eloquent::casts::Cast>::Storage,
                >("", column)?;
                <#cast_ty as ::suprnova::eloquent::casts::Cast>::from_storage(&stored)
                    .map_err(|error| ::suprnova::sea_orm::DbErr::Type(error.to_string()))
            }
        },
        None => quote! {},
    };

    // The soft-delete filter is folded in by the builder when the query
    // runs, so the trait's own `query()` is already right for this model.
    let query_override = quote! {};

    // Trait-level `find` override for soft-delete models. The macro
    // also emits an inherent `find` (for ergonomic concrete-receiver
    // calls below), but Rust's method resolution picks the inherent
    // only when the receiver is a concrete type - generic dispatch
    // (`M::find(id)` with `M: Model`) walks the trait table and hits
    // the unscoped default, exposing trashed rows. Pinning the trait
    // method here closes that gap so route binding (`RouteParam<M>`),
    // global scopes, and any other generic Eloquent caller all honour
    // the soft-delete filter.
    let find_trait_override = if soft_deletes_enabled {
        quote! {
            async fn find<K>(
                id: K,
            ) -> ::core::result::Result<::core::option::Option<Self>, ::suprnova::FrameworkError>
            where
                K: ::core::convert::Into<
                    <<Self::Entity as ::suprnova::EntityTrait>::PrimaryKey
                        as ::suprnova::PrimaryKeyTrait>::ValueType,
                > + ::core::marker::Send,
            {
                let pk_value: <<Self::Entity as ::suprnova::EntityTrait>::PrimaryKey
                    as ::suprnova::PrimaryKeyTrait>::ValueType = id.into();
                <Self as ::suprnova::eloquent::Model>::query()
                    .filter(
                        <Self as ::suprnova::eloquent::Model>::primary_key_name(),
                        pk_value,
                    )
                    .first()
                    .await
            }
        }
    } else {
        quote! {}
    };

    // Trait-level `delete` override for soft-delete models, for the same
    // reason as `find` above: generic code (`Model::destroy`,
    // `delete_quietly`, a `M::delete` in app code) dispatches through
    // the trait, where the default is a hard DELETE that removes the
    // row for good. The override hands the call to the inherent
    // tombstone `delete` below; a path call on the concrete type names
    // the inherent method, which takes precedence over the trait's.
    let delete_trait_override = if soft_deletes_enabled {
        quote! {
            async fn delete(
                self,
            ) -> ::core::result::Result<(), ::suprnova::FrameworkError> {
                #struct_ident::delete(self).await
            }
        }
    } else {
        quote! {}
    };

    // Seed builder for the static entry points below. It folds the
    // soft-delete filter and the registered global scopes in when the
    // query runs, honouring whatever opt-out the entry point set.
    let t4_fresh_builder = quote! {
        ::suprnova::Builder::<Self>::__scoped()
    };

    let soft_deletes_impl = if soft_deletes_enabled {
        quote! {
            impl ::suprnova::eloquent::SoftDeletes for #struct_ident {
                fn deleted_at_column() -> &'static str { #soft_delete_col }
                fn is_trashed(&self) -> bool { self.#soft_delete_col_ident.is_some() }
            }

            impl #struct_ident {
                /// Soft-delete: `UPDATE table SET deleted_at = NOW()
                /// WHERE pk = ?` instead of DELETE, which also sets
                /// `updated_at` when the model manages timestamps, as
                /// Laravel's soft delete does. Takes `self` by
                /// value to override `Model::delete(self)` cleanly -
                /// a `&self` inherent override would lose to the
                /// trait default through auto-ref resolution.
                ///
                /// ## Lifecycle events (Phase 10C T1)
                ///
                /// 1. `Deleting { is_force: false }` - cancellable
                /// 2. *UPDATE deleted_at lands*
                /// 3. `Trashed`
                /// 4. `Deleted { is_force: false }`
                ///
                /// Cancellation at step 1 aborts the UPDATE - the row
                /// stays alive. The `Trashed` event distinguishes
                /// soft-deletes from hard-deletes for listeners that
                /// only care about the tombstone case.
                pub async fn delete(self) -> ::core::result::Result<(), ::suprnova::FrameworkError> {
                    <Self as ::suprnova::eloquent::events::ModelEventHooks>::__dispatch_deleting(&self, false).await?;
                    let touch_plan = <Self as ::suprnova::eloquent::Model>::__plan_touches(
                        ::core::option::Option::Some(&self),
                        &::suprnova::eloquent::Attrs::new(),
                    )?;

                    // The tombstone, and "now" in `updated_at` when the
                    // model manages timestamps - Laravel's
                    // `runSoftDelete` - the same stamp `delete_all` and
                    // `delete_or_fail` write.
                    let stamp = <Self as ::suprnova::eloquent::EloquentModel>::__soft_delete_stamp()?
                        .ok_or_else(|| ::suprnova::FrameworkError::internal(
                            "a soft-delete model has no soft-delete stamp",
                        ))?;
                    let table = <Self as ::suprnova::eloquent::EloquentModel>::TABLE;
                    let pk_name = <Self as ::suprnova::eloquent::Model>::primary_key_name();
                    // Route through `resolve_write` so the tombstone
                    // UPDATE honours the full write-side precedence chain -
                    // tx override → ambient CURRENT_TX → per-model
                    // `#[model(connection = ".")]` → primary - matching
                    // `restore()`. The bare `resolve()` only consults
                    // CURRENT_TX, silently ignoring per-model routing.
                    // The tombstone UPDATE and its generation advance share one
                    // transaction, so a future dropped between them rolls the
                    // tombstone back instead of leaving it committed ahead of
                    // the pages that read the row.
                    ::suprnova::database::__macro_support::atomic_write(
                        <Self as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                        || async {
                            let exec = ::suprnova::database::transaction::ExecutorChoice::resolve_write(
                                ::core::option::Option::None,
                                ::core::option::Option::None,
                                <Self as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                            )
                            .await?;
                            let backend = exec.backend();
                            let mut values: ::std::vec::Vec<::suprnova::sea_orm::Value> =
                                ::std::vec![::core::clone::Clone::clone(&stamp.deleted_at)];
                            let mut sql = ::std::format!(
                                "UPDATE {table} SET {} = {}",
                                #soft_delete_col,
                                ::suprnova::database::__macro_support::placeholder(backend, values.len())?,
                            );
                            if let ::core::option::Option::Some(updated_at) = &stamp.updated_at {
                                values.push(::core::clone::Clone::clone(updated_at));
                                sql.push_str(&::std::format!(
                                    ", {} = {}",
                                    <Self as ::suprnova::eloquent::EloquentModel>::UPDATED_AT_COLUMN,
                                    ::suprnova::database::__macro_support::placeholder(backend, values.len())?,
                                ));
                            }
                            values.push(::suprnova::eloquent::model::json_value_to_sea_value(
                                &<Self as ::suprnova::eloquent::Model>::primary_key_value_json(&self),
                            ));
                            sql.push_str(&::std::format!(
                                " WHERE {pk_name} = {}",
                                ::suprnova::database::__macro_support::placeholder(backend, values.len())?,
                            ));
                            exec.run(
                                ::suprnova::sea_orm::Statement::from_sql_and_values(backend, &sql, values),
                            )
                            .await
                            .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;
                            ::suprnova::render_cache::orm::after_model_write(&self).await
                        },
                    )
                    .await?;

                    <Self as ::suprnova::eloquent::events::ModelEventHooks>::__dispatch_trashed(&self).await?;
                    <Self as ::suprnova::eloquent::events::ModelEventHooks>::__dispatch_deleted(&self, false).await?;
                    <Self as ::suprnova::eloquent::Model>::__touch_planned(&self, &touch_plan).await?;
                    ::core::result::Result::Ok(())
                }

                /// Restore from soft delete: `UPDATE table SET
                /// deleted_at = NULL WHERE pk = ?`. Takes `self` for
                /// signature parity with the other lifecycle methods.
                ///
                /// ## Lifecycle events (Phase 10C T1)
                ///
                /// 1. `Restoring` - cancellable
                /// 2. *UPDATE deleted_at = NULL lands*
                /// 3. `Restored`
                ///
                /// Cancellation at step 1 aborts the UPDATE - the row
                /// stays trashed with `deleted_at` intact.
                pub async fn restore(self) -> ::core::result::Result<(), ::suprnova::FrameworkError> {
                    <Self as ::suprnova::eloquent::events::ModelEventHooks>::__dispatch_restoring(&self).await?;

                    let table = <Self as ::suprnova::eloquent::EloquentModel>::TABLE;
                    let pk_name = <Self as ::suprnova::eloquent::Model>::primary_key_name();
                    // Route through `resolve_write` (not `resolve`) so
                    // restores honour the full write-side precedence
                    // chain - tx override → ambient CURRENT_TX → builder
                    // `on(name)` → per-model `#[model(connection = ".")]`
                    // → primary. The bare `resolve()` would only consult
                    // CURRENT_TX and fall back to `DB::connection()`,
                    // silently ignoring per-model connection routing.
                    // The UPDATE and its generation advance share one
                    // transaction, as the soft delete's do.
                    ::suprnova::database::__macro_support::atomic_write(
                        <Self as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                        || async {
                            let exec = ::suprnova::database::transaction::ExecutorChoice::resolve_write(
                                ::core::option::Option::None,
                                ::core::option::Option::None,
                                <Self as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                            )
                            .await?;
                            let backend = exec.backend();
                            let pk_placeholder =
                                ::suprnova::database::__macro_support::placeholder(backend, 1)?;
                            let sql = ::std::format!(
                                "UPDATE {table} SET {} = NULL WHERE {pk_name} = {pk_placeholder}",
                                #soft_delete_col,
                            );
                            exec.run(
                                ::suprnova::sea_orm::Statement::from_sql_and_values(
                                    backend,
                                    &sql,
                                    ::std::vec![
                                        ::suprnova::eloquent::model::json_value_to_sea_value(
                                            &<Self as ::suprnova::eloquent::Model>::primary_key_value_json(&self),
                                        ),
                                    ],
                                ),
                            )
                            .await
                            .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;
                            ::suprnova::render_cache::orm::after_model_write(&self).await
                        },
                    )
                    .await?;

                    <Self as ::suprnova::eloquent::events::ModelEventHooks>::__dispatch_restored(&self).await?;
                    ::core::result::Result::Ok(())
                }

                /// Cheap accessor: `deleted_at IS NOT NULL` at row
                /// materialisation time. Does not touch the database.
                pub fn trashed(&self) -> bool {
                    self.#soft_delete_col_ident.is_some()
                }

                /// View including soft-deleted rows. Only the
                /// soft-delete filter is lifted: every registered
                /// global scope still applies.
                pub fn with_trashed() -> ::suprnova::Builder<Self> {
                    ::suprnova::Builder::<Self>::__scoped()
                        .__disable_named_scope("soft_deletes")
                }

                /// View showing only soft-deleted rows. Every
                /// registered global scope still applies.
                pub fn only_trashed() -> ::suprnova::Builder<Self> {
                    ::suprnova::Builder::<Self>::__scoped().only_trashed()
                }

                /// Hard-delete this row, bypassing the soft-delete
                /// override. Runs an actual `DELETE FROM table WHERE
                /// pk = ?` regardless of whether the row was already
                /// trashed.
                ///
                /// ## Lifecycle events (Phase 10C T1)
                ///
                /// 1. `Deleting { is_force: true }` - cancellable
                /// 2. `ForceDeleting`
                /// 3. *DELETE FROM lands*
                /// 4. `ForceDeleted`
                /// 5. `Deleted { is_force: true }`
                ///
                /// `Trashed` is NOT fired - the row is gone, not
                /// tombstoned. Listeners on `Deleted` can branch on
                /// `is_force` to disambiguate from the soft-delete
                /// path.
                pub async fn force_delete(self) -> ::core::result::Result<(), ::suprnova::FrameworkError> {
                    <Self as ::suprnova::eloquent::events::ModelEventHooks>::__dispatch_deleting(&self, true).await?;
                    <Self as ::suprnova::eloquent::events::ModelEventHooks>::__dispatch_force_deleting(&self).await?;
                    let touch_plan = <Self as ::suprnova::eloquent::Model>::__plan_touches(
                        ::core::option::Option::Some(&self),
                        &::suprnova::eloquent::Attrs::new(),
                    )?;

                    let snapshot = ::core::clone::Clone::clone(&self);
                    let row: <<Self as ::suprnova::eloquent::EloquentModel>::Entity as ::suprnova::sea_orm::EntityTrait>::Model = self.into();
                    let am = <_ as ::suprnova::sea_orm::IntoActiveModel<_>>::into_active_model(row);
                    // Route through `resolve_write` so the hard DELETE
                    // honours the full write-side precedence chain - tx
                    // override → ambient CURRENT_TX → per-model
                    // `#[model(connection = ".")]` → primary - matching
                    // `restore()`. The bare `resolve()` only consults
                    // CURRENT_TX, silently ignoring per-model routing.
                    // The DELETE and its generation advance share one
                    // transaction, as the soft delete's do.
                    ::suprnova::database::__macro_support::atomic_write(
                        <Self as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                        || async {
                            let exec = ::suprnova::database::transaction::ExecutorChoice::resolve_write(
                                ::core::option::Option::None,
                                ::core::option::Option::None,
                                <Self as ::suprnova::eloquent::EloquentModel>::default_connection_name(),
                            )
                            .await?;
                            exec.delete_active(am)
                                .await
                                .map_err(|e| ::suprnova::FrameworkError::database(e.to_string()))?;
                            ::suprnova::render_cache::orm::after_model_write(&snapshot).await
                        },
                    )
                    .await?;

                    <Self as ::suprnova::eloquent::events::ModelEventHooks>::__dispatch_force_deleted(&snapshot).await?;
                    <Self as ::suprnova::eloquent::events::ModelEventHooks>::__dispatch_deleted(&snapshot, true).await?;
                    <Self as ::suprnova::eloquent::Model>::__touch_planned(&snapshot, &touch_plan).await?;
                    ::core::result::Result::Ok(())
                }

                /// Look up a row by primary key, honouring the
                /// soft-delete scope. Inherent override of the trait
                /// default - non-soft-delete models still use the
                /// SeaORM `find_by_id` path. Bypass the scope via
                /// `Self::with_trashed().filter(Self::primary_key_name(), id).first()`.
                pub async fn find(
                    id: #key_type,
                ) -> ::core::result::Result<::core::option::Option<Self>, ::suprnova::FrameworkError> {
                    <Self as ::suprnova::eloquent::Model>::query()
                        .filter(<Self as ::suprnova::eloquent::Model>::primary_key_name(), id)
                        .first()
                        .await
                }

                /// Look up a row by primary key, honouring the
                /// soft-delete scope. Returns
                /// `FrameworkError::ModelNotFound` (HTTP 404) when no
                /// row matches (or when the row is trashed).
                pub async fn find_or_fail(
                    id: #key_type,
                ) -> ::core::result::Result<Self, ::suprnova::FrameworkError>
                where
                    #key_type: ::std::fmt::Debug + ::core::marker::Copy,
                {
                    match <Self>::find(id).await? {
                        ::core::option::Option::Some(m) => ::core::result::Result::Ok(m),
                        ::core::option::Option::None => ::core::result::Result::Err(
                            ::suprnova::FrameworkError::not_found(::std::format!(
                                "{} with {} = {:?} not found",
                                ::core::any::type_name::<Self>(),
                                <Self as ::suprnova::eloquent::Model>::primary_key_name(),
                                id,
                            )),
                        ),
                    }
                }

                /// Fetch every alive row (skips trashed). Inherent
                /// override of the trait default - matches the trait
                /// `all` return type ([`Collection<Self>`](::suprnova::eloquent::Collection)).
                pub async fn all() -> ::core::result::Result<
                    ::suprnova::eloquent::Collection<Self>,
                    ::suprnova::FrameworkError,
                > {
                    <Self as ::suprnova::eloquent::Model>::query().get().await
                }

                /// Fetch every alive row whose PK is in `ids` (skips
                /// trashed). Result order is the database's natural
                /// order - does not preserve `ids` order. Use
                /// `Self::with_trashed().where_in(pk, ids).get()` to
                /// include trashed rows.
                ///
                /// Returns `Vec<Self>` (not `Collection<Self>`) to match
                /// the trait's `find_many` shape - it's a PK-set
                /// lookup, not a generic query, so the Collection
                /// surface is unnecessary.
                pub async fn find_many<I>(
                    ids: I,
                ) -> ::core::result::Result<::std::vec::Vec<Self>, ::suprnova::FrameworkError>
                where
                    I: ::core::iter::IntoIterator<Item = #key_type> + ::core::marker::Send,
                {
                    ::core::result::Result::Ok(
                        <Self as ::suprnova::eloquent::Model>::query()
                            .where_in(<Self as ::suprnova::eloquent::Model>::primary_key_name(), ids)
                            .get()
                            .await?
                            .into_vec(),
                    )
                }
            }
        }
    } else {
        quote! {}
    };

    // Soft-delete column for the `const SOFT_DELETES_COLUMN` initialiser
    // on `impl EloquentModel`. Empty when the model didn't opt into
    // `#[model(soft_deletes)]` - the has/where-has engine treats `""` as
    // "do not auto-apply the deleted_at IS NULL filter".
    let soft_deletes_column_const: &str = if input.soft_deletes {
        &input.soft_deletes_column
    } else {
        ""
    };
    // What a mass soft delete writes: "now" through the column's own cast,
    // the same value the inherent `delete()` binds.
    let soft_delete_stamp = if input.soft_deletes {
        let updated_at_stamp = if timestamps_enabled {
            quote! {
                ::core::option::Option::Some(::suprnova::sea_orm::Value::from(
                    <#updated_at_cast as ::suprnova::eloquent::casts::Cast>::to_storage(
                        #updated_touch_now,
                    )?,
                ))
            }
        } else {
            quote! { ::core::option::Option::None }
        };
        quote! {
            fn __soft_delete_stamp() -> ::core::result::Result<
                ::core::option::Option<::suprnova::eloquent::SoftDeleteStamp>,
                ::suprnova::FrameworkError,
            > {
                let now = ::suprnova::clock::now();
                let deleted_at = ::suprnova::sea_orm::Value::from(
                    <#soft_delete_cast as ::suprnova::eloquent::casts::Cast>::to_storage(
                        &::core::option::Option::Some(now),
                    )?,
                );
                ::core::result::Result::Ok(::core::option::Option::Some(
                    ::suprnova::eloquent::SoftDeleteStamp {
                        deleted_at,
                        updated_at: #updated_at_stamp,
                    },
                ))
            }
        }
    } else {
        quote! {}
    };

    Ok(quote! {
        impl ::suprnova::eloquent::EloquentModel for #struct_ident {
            type Entity = #module_name::Entity;
            type Column = #module_name::Column;
            // The declared primary-key Rust type. `key_type` is the
            // parsed `#[model(key_type = "...")]` attribute, defaulted
            // to `i64` at parse time - the same type the inherent
            // `find` / `find_many` signatures already use, so a model
            // whose key compiles there compiles here.
            type Key = #key_type;
            // Literal table string captured at parse time - see T3
            // for why this is the literal rather than a SeaORM call.
            const TABLE: &'static str = #table;
            // The macro-parsed primary key name. Mirrors the
            // `primary_key_name()` method on `Model` but as a `const`,
            // so the has/where-has engine can read it through
            // `inventory::submit!` initialisers (which require const
            // evaluation).
            const PRIMARY_KEY: &'static str = #pk_name;
            // Soft-delete column when `#[model(soft_deletes)]` is set;
            // empty string otherwise. The existence engine consults
            // this through the `RelationEntry` table - the related
            // model's PK and soft-delete column are baked into each
            // relation's inventory entry at link time.
            const SOFT_DELETES_COLUMN: &'static str = #soft_deletes_column_const;
            #soft_delete_stamp

            // Read by `Model::touch_owners`, which is a trait default -
            // so this has to be a trait const, not an inherent one, or
            // the generic body can't see it.
            #touches_const
            // The #61073 gate: `HAS_TIMESTAMPS` says whether this model
            // manages the column at all, `UPDATED_AT_COLUMN` says which
            // column that is. A relation entry pointing here collapses
            // the pair via `touch_column`.
            const HAS_TIMESTAMPS: bool = #timestamps_enabled;
            const UPDATED_AT_COLUMN: &'static str = #updated_at_col;
            #updated_at_storage_impl
            #bind_column_impl
            #decode_key_impl

            // Per-model default connection override. Lives on
            // `EloquentModel` (not the heavier `Model` trait) so
            // generic relation impls that only carry the lightweight
            // marker bound can still consult it without dragging in
            // the full CRUD bound chain. Empty when the model didn't
            // declare `#[model(connection = "...")]`.
            #default_connection_impl
        }

        // Bridge the user struct <-> SeaORM Model row. The inner
        // module's `Model` is what SeaORM returns from queries; the
        // user struct is what their code names. Cast fields route
        // through `Cast::from_storage` / `Cast::to_storage` so the
        // user's Runtime type matches even when the inner Model
        // stores a different Storage type (e.g. INTEGER for bool).
        impl ::core::convert::From<#module_name::Model> for #struct_ident {
            fn from(row: #module_name::Model) -> Self {
                let __suprnova_loaded_row = ::core::clone::Clone::clone(&row);
                Self {
                    #( #from_storage_arms, )*
                    #loaded_relations_fields_init
                }
            }
        }

        impl ::core::convert::From<#struct_ident> for #module_name::Model {
            fn from(s: #struct_ident) -> Self {
                Self { #( #to_storage_arms ),* }
            }
        }

        // `Default` lets `ReplicateExt::replicate_with` build a fresh
        // shell when clearing fields. Field types must individually
        // impl `Default`; we don't try to be clever here.
        impl ::core::default::Default for #struct_ident {
            fn default() -> Self {
                Self {
                    #( #field_idents: ::core::default::Default::default(), )*
                    #relations_fields_init
                }
            }
        }

        #[::suprnova::__async_trait::async_trait]
        impl ::suprnova::eloquent::Model for #struct_ident {
            fn primary_key_name() -> &'static str { #pk_name }


            #fillable_impl

            #query_override

            #find_trait_override

            #delete_trait_override

            #field_value_method

            #morph_owner_method

            #to_array_override

            #append_accessor_dispatch

            // Fallible hydration / dehydration. The framework's CRUD
            // paths call these so a cast that fails to
            // decode a stored value (corrupt column, schema drift) or
            // encode a runtime value surfaces a recoverable
            // `FrameworkError` instead of a panic. That matters off the
            // HTTP path: queue workers, the scheduler, and CLI commands
            // have no panic-recovery middleware. The infallible `From`
            // impls above remain as the ergonomic escape hatch.
            fn try_from_storage(
                row: <Self::Entity as ::suprnova::EntityTrait>::Model,
            ) -> ::core::result::Result<Self, ::suprnova::FrameworkError> {
                let __suprnova_loaded_row = ::core::clone::Clone::clone(&row);
                ::core::result::Result::Ok(Self {
                    #( #try_from_storage_arms, )*
                    #loaded_relations_fields_init
                })
            }

            fn try_into_storage(
                self,
            ) -> ::core::result::Result<
                <Self::Entity as ::suprnova::EntityTrait>::Model,
                ::suprnova::FrameworkError,
            > {
                ::core::result::Result::Ok(#module_name::Model {
                    #( #try_to_storage_arms ),*
                })
            }

            fn primary_key_value(
                &self,
            ) -> <<Self::Entity as ::suprnova::EntityTrait>::PrimaryKey as ::suprnova::PrimaryKeyTrait>::ValueType {
                self.#pk_ident.clone().into()
            }

            fn primary_key_value_json(&self) -> ::suprnova::serde_json::Value {
                ::suprnova::serde_json::to_value(&self.#pk_ident)
                    .unwrap_or(::suprnova::serde_json::Value::Null)
            }

            fn reset_primary_key(&mut self) {
                self.#pk_ident = ::core::default::Default::default();
            }

            // The mark lazy-loading prevention reads lives in the
            // row's relation cache, which only the macro can name.
            fn __mark_from_multi_row_query(&mut self) {
                self.__eager.__mark_from_multi_row_query();
            }

            // `Model::was_changed` and its siblings are trait defaults, so
            // they reach the row the cache keeps through this override
            // rather than through the field, which only the macro can name.
            fn __eager_cache(
                &self,
            ) -> ::core::option::Option<&::suprnova::EagerLoadCache> {
                ::core::option::Option::Some(&self.__eager)
            }

            // `Model::get_original` reads a value through the casts, so it
            // rebuilds the model from the kept row. The row is type-erased
            // in the cache; only here is its type nameable.
            fn __model_from_stored_row(
                row: &(dyn ::std::any::Any + ::core::marker::Send + ::core::marker::Sync),
            ) -> ::core::option::Option<
                ::core::result::Result<Self, ::suprnova::FrameworkError>,
            > {
                row.downcast_ref::<#module_name::Model>().map(|row| {
                    <Self as ::suprnova::eloquent::Model>::try_from_storage(
                        ::core::clone::Clone::clone(row),
                    )
                })
            }

            #decoded_equal_method

            fn active_model_from_attrs(
                attrs: ::suprnova::eloquent::Attrs,
            ) -> ::core::result::Result<
                <Self::Entity as ::suprnova::EntityTrait>::ActiveModel,
                ::suprnova::FrameworkError,
            > {
                let mut am = <<Self::Entity as ::suprnova::EntityTrait>::ActiveModel as ::core::default::Default>::default();
                <Self as ::suprnova::eloquent::Model>::apply_attrs_to_active_model(&mut am, attrs)?;
                ::core::result::Result::Ok(am)
            }

            fn apply_attrs_to_active_model(
                am: &mut <Self::Entity as ::suprnova::EntityTrait>::ActiveModel,
                attrs: ::suprnova::eloquent::Attrs,
            ) -> ::core::result::Result<(), ::suprnova::FrameworkError> {
                for (name, val) in attrs.iter() {
                    match name {
                        #(#apply_arms,)*
                        other => {
                            return ::core::result::Result::Err(
                                ::suprnova::FrameworkError::validation(
                                    other,
                                    ::std::format!(
                                        "unknown column `{other}` on {}",
                                        ::core::stringify!(#struct_ident),
                                    ),
                                ),
                            );
                        }
                    }
                }
                #timestamp_inject_apply
                #unique_id_inject_apply
                ::core::result::Result::Ok(())
            }

            fn into_active_model_for_update(
                self,
            ) -> ::core::result::Result<
                <Self::Entity as ::suprnova::EntityTrait>::ActiveModel,
                ::suprnova::FrameworkError,
            > {
                let mut am = <<Self::Entity as ::suprnova::EntityTrait>::ActiveModel
                    as ::core::default::Default>::default();
                #( #active_model_for_update_arms )*
                #timestamp_inject_save
                ::core::result::Result::Ok(am)
            }
        }

        #touchable_impl
        #soft_deletes_impl
        #unique_id_impl

        // Persistable bridge for the Eloquent-facing struct.
        //
        // The framework's blanket `impl<M, E> Persistable for M where
        // M: ModelTrait<Entity = E> + IntoActiveModel<...>` covers
        // SeaORM Models - but the user-facing `#[suprnova::model]`
        // struct (e.g. `User`) is NOT itself a `ModelTrait`. Adding a
        // second blanket impl gated on `EloquentModel` would conflict
        // with the existing one under Rust's coherence rules (the
        // compiler can't prove `ModelTrait` and `EloquentModel` are
        // disjoint), so we emit a per-struct impl here instead.
        //
        // The bridge piggybacks on the From<...> impls emitted above:
        //   1. Convert `self` to the inner SeaORM Model (storage shape).
        //   2. Hand off to `persist_via_seaorm` - which knows how to
        //      flip the PK to NotSet so the database assigns the id.
        //   3. Convert the post-insert inner Model back to the
        //      Eloquent-facing struct (running `Cast::from_storage`
        //      again, so any storage→runtime translation re-runs
        //      against the canonicalised row).
        //
        // The net effect: factories returning the Eloquent struct
        // (`User`, `Post`) work with `.create()` / `.create_many()`
        // the same way factories returning the inner Model do.
        // Runtime values (`active: true`, `Utc::now()`) flow in;
        // canonicalised runtime values flow out.
        #[::suprnova::__async_trait::async_trait]
        impl ::suprnova::Persistable for #struct_ident {
            async fn persist(mut self) -> ::core::result::Result<Self, ::suprnova::FrameworkError> {
                #persist_stamp
                // The same insert `Model::create` runs: `Creating` and
                // `Saving` before it, `Created` and `Saved` after it, the
                // write-side connection routing, the render cache's
                // bookkeeping. A factory's rows reach every observer the
                // application's rows reach.
                <Self as ::suprnova::eloquent::Model>::__insert_built(self, #auto_increment).await
            }
        }

        impl ::suprnova::eloquent::ReplicateExt for #struct_ident {
            fn replicate_with(&self, except: ::std::vec::Vec<::std::string::String>) -> Self {
                Self {
                    #( #replicate_arms, )*
                    #replicate_relations_init
                }
            }
        }

        #[::suprnova::__async_trait::async_trait]
        impl ::suprnova::eloquent::FirstOrCreate for #struct_ident {
            fn from_attrs_unsaved(
                attrs: ::suprnova::eloquent::Attrs,
            ) -> ::core::result::Result<Self, ::suprnova::FrameworkError> {
                let mut s = <Self as ::core::default::Default>::default();
                for (name, v) in attrs.iter() {
                    match name {
                        #(#from_attrs_unsaved_arms,)*
                        _ => {}
                    }
                }
                ::core::result::Result::Ok(s)
            }
        }

        // Phase 10C T6 - `to_json` / `to_array` moved off the inherent
        // surface and onto the `Model` trait. The trait defaults strip
        // `__eager` / `__pivot`; the per-model overrides (emitted
        // above when `hidden` / `visible` / `appends` is non-empty)
        // apply the filter pipeline + accessor injection. This block
        // keeps the inherent `fill` method which is intrinsically
        // per-model (it dispatches into `set_<field>` mutators by name)
        // and doesn't have a sensible trait-default shape.
        impl #struct_ident {
            /// Apply `attrs` to `self`, routing through any matching
            /// mutator (`set_<field>(value)`) before falling back to
            /// direct field assignment. Unknown columns are silently
            /// skipped to match Laravel's `$model->fill($attrs)`
            /// semantics.
            ///
            /// Honours mass-assignment: the `fillable` / `guarded`
            /// filter declared on `#[model]` runs first, dropping any
            /// columns the caller isn't allowed to set. To bypass the
            /// filter, wrap the call in
            /// [`::suprnova::eloquent::unguarded`] - the same
            /// task-local escape hatch `Model::create` / `update`
            /// honour.
            pub fn fill(
                &mut self,
                attrs: ::suprnova::eloquent::Attrs,
            ) -> ::core::result::Result<(), ::suprnova::FrameworkError> {
                let attrs = <Self as ::suprnova::eloquent::Model>::fillable_filter()
                    .apply(attrs);
                for (k, v) in attrs.iter() {
                    match k {
                        #(#fill_mutator_arms,)*
                        #(#fill_direct_arms,)*
                        _ => {} // unknown column - silently skip (Laravel parity)
                    }
                }
                ::core::result::Result::Ok(())
            }
        }

        // Global-scope opt-out static helpers. Each is the chained
        // form started from the model: the builder folds scopes in when
        // the query runs and honours the opt-out set here. The
        // soft-delete filter is a separate opt-out and stays.
        impl #struct_ident {
            /// Phase 10C T4 - start a query that bypasses one global
            /// scope by type. Other registered scopes still apply.
            /// Soft-delete filter (when `#[model(soft_deletes)]`) is
            /// preserved.
            ///
            /// ## Example
            ///
            /// ```ignore
            /// // TenantScope normally applies to every User read; this
            /// // call drops it for one query.
            /// let everyone = User::without_global_scope::<TenantScope>()
            ///     .get()
            ///     .await?;
            /// ```
            pub fn without_global_scope<__Scope: 'static>() -> ::suprnova::Builder<Self> {
                #t4_fresh_builder
                    .without_global_scope::<__Scope>()
            }

            /// Phase 10C T4 - start a query that bypasses every
            /// registered global scope. Soft-delete filter (when
            /// `#[model(soft_deletes)]`) is preserved - soft-deletes
            /// don't route through the typed registry.
            ///
            /// ## Example
            ///
            /// ```ignore
            /// // Admin tooling reads every row regardless of scoping.
            /// let everything = User::without_global_scopes()
            ///     .get()
            ///     .await?;
            /// ```
            pub fn without_global_scopes() -> ::suprnova::Builder<Self> {
                #t4_fresh_builder
                    .without_global_scopes()
            }
        }

        // Phase 10C T12 - per-model connection-routing entry points.
        // The trait-level [`Builder::on`] / [`Builder::on_write_connection`]
        // already exist; these inherent shortcuts let users write
        // `User::on("analytics")` instead of
        // `User::query().on("analytics")`, matching the Laravel shape.
        impl #struct_ident {
            /// Phase 10C T12 - start a query routed through the named
            /// connection. Equivalent to
            /// `<Self as Model>::query().on(name)`. Inside a
            /// `DB::transaction` closure the override is silently
            /// ignored - every op runs through the tx connection.
            pub fn on(name: impl ::core::convert::Into<::std::string::String>) -> ::suprnova::Builder<Self> {
                <Self as ::suprnova::eloquent::Model>::query().on(name)
            }

            /// Phase 10C T12 - start a query routed through the
            /// primary pool, even when `__read_replica__` is
            /// registered. Use for read-your-writes scenarios where
            /// the replica might not have caught up.
            pub fn on_write_connection() -> ::suprnova::Builder<Self> {
                <Self as ::suprnova::eloquent::Model>::query().on_write_connection()
            }
        }

        // Static-style shortcuts on the user's struct. Each delegates to
        // `<Self as Model>::query()` so the dual-API surface stays in
        // one place. These are the Laravel-shape entry points that
        // make `User::filter("email", ...)` and `User::count()` read
        // the way users expect from PHP.
        impl #struct_ident {
            /// `SELECT COUNT(*) FROM table`.
            pub async fn count() -> ::core::result::Result<i64, ::suprnova::FrameworkError> {
                <Self as ::suprnova::eloquent::Model>::query().count().await
            }

            /// `SELECT COALESCE(SUM(col), 0) FROM table`.
            pub async fn sum<T>(
                col: impl ::suprnova::eloquent::builder::IntoColumn,
            ) -> ::core::result::Result<T, ::suprnova::FrameworkError>
            where
                T: ::suprnova::ColumnValue + ::core::default::Default,
            {
                <Self as ::suprnova::eloquent::Model>::query().sum(col).await
            }

            /// `SELECT COALESCE(AVG(col), 0) FROM table`, as an `f64` or a
            /// `rust_decimal::Decimal`.
            pub async fn avg<T>(
                col: impl ::suprnova::eloquent::builder::IntoColumn,
            ) -> ::core::result::Result<T, ::suprnova::FrameworkError>
            where
                T: ::suprnova::AvgValue,
            {
                <Self as ::suprnova::eloquent::Model>::query().avg(col).await
            }

            /// `SELECT MIN(col) FROM table`.
            pub async fn min<T>(
                col: impl ::suprnova::eloquent::builder::IntoColumn,
            ) -> ::core::result::Result<::core::option::Option<T>, ::suprnova::FrameworkError>
            where
                T: ::suprnova::ColumnValue,
            {
                <Self as ::suprnova::eloquent::Model>::query().min(col).await
            }

            /// `SELECT MAX(col) FROM table`.
            pub async fn max<T>(
                col: impl ::suprnova::eloquent::builder::IntoColumn,
            ) -> ::core::result::Result<::core::option::Option<T>, ::suprnova::FrameworkError>
            where
                T: ::suprnova::ColumnValue,
            {
                <Self as ::suprnova::eloquent::Model>::query().max(col).await
            }

            /// `SELECT col FROM table` - returns one column from every row.
            pub async fn pluck<T>(
                col: impl ::suprnova::eloquent::builder::IntoColumn,
            ) -> ::core::result::Result<::std::vec::Vec<T>, ::suprnova::FrameworkError>
            where
                T: ::suprnova::ColumnValue,
            {
                <Self as ::suprnova::eloquent::Model>::query().pluck(col).await
            }

            /// `SELECT key_col, val_col FROM table` - returns a `HashMap`
            /// keyed by `key_col`, valued by `val_col`.
            pub async fn pluck_keyed<K, V>(
                key_col: impl ::suprnova::eloquent::builder::IntoColumn,
                val_col: impl ::suprnova::eloquent::builder::IntoColumn,
            ) -> ::core::result::Result<
                ::std::collections::HashMap<K, V>,
                ::suprnova::FrameworkError,
            >
            where
                K: ::suprnova::ColumnValue + ::core::cmp::Eq + ::std::hash::Hash,
                V: ::suprnova::ColumnValue,
            {
                <Self as ::suprnova::eloquent::Model>::query()
                    .pluck_keyed(key_col, val_col)
                    .await
            }

            /// Static-style `filter` - opens a `Builder<Self>` with
            /// one equality WHERE clause already attached.
            #[doc(alias = "db_where")]
            pub fn filter(
                col: impl ::suprnova::eloquent::builder::IntoColumn,
                val: impl ::suprnova::eloquent::builder::IntoVal,
            ) -> ::suprnova::Builder<Self> {
                <Self as ::suprnova::eloquent::Model>::query().filter(col, val)
            }

            /// Laravel-shape alias for [`Self::filter`].
            #[doc(alias = "filter")]
            pub fn db_where(
                col: impl ::suprnova::eloquent::builder::IntoColumn,
                val: impl ::suprnova::eloquent::builder::IntoVal,
            ) -> ::suprnova::Builder<Self> {
                <Self as ::suprnova::eloquent::Model>::query().db_where(col, val)
            }

            /// Static-style `where_in`.
            pub fn where_in<V, I>(
                col: impl ::suprnova::eloquent::builder::IntoColumn,
                vals: I,
            ) -> ::suprnova::Builder<Self>
            where
                I: ::core::iter::IntoIterator<Item = V>,
                V: ::suprnova::eloquent::builder::IntoVal,
            {
                <Self as ::suprnova::eloquent::Model>::query().where_in(col, vals)
            }

            /// Static-style `where_like`.
            pub fn where_like(
                col: impl ::suprnova::eloquent::builder::IntoColumn,
                pattern: impl ::core::convert::Into<::std::string::String>,
            ) -> ::suprnova::Builder<Self> {
                <Self as ::suprnova::eloquent::Model>::query().where_like(col, pattern)
            }

            /// Static-style `latest` - `ORDER BY created_at DESC`. Models
            /// without a `created_at` column will fail at the SQL layer;
            /// timestamp surface lands in T9.
            pub fn latest() -> ::suprnova::Builder<Self> {
                <Self as ::suprnova::eloquent::Model>::query().order_by_desc("created_at")
            }

            /// Static-style `oldest` - `ORDER BY created_at ASC`.
            pub fn oldest() -> ::suprnova::Builder<Self> {
                <Self as ::suprnova::eloquent::Model>::query().order_by_asc("created_at")
            }
        }
    })
}
