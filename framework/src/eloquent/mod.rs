//! Eloquent - Laravel-shape API layered over SeaORM.
//!
//! See `manual/eloquent.md` for the user guide.
//!
//! Phase 10A ships the foundation: `#[suprnova::model]` macro,
//! `Model` trait (CRUD lifecycle), `Builder<M>` (dual-API where
//! surface), Fillable/Guarded, the 21 built-in casts, accessors and
//! mutators, auto-managed timestamps, and soft deletes + Prunable.
//! Phase 10B adds relationships; Phase 10C adds collections /
//! pagination / observers / transactions / multi-connection.

pub mod attrs;
pub mod builder;
pub mod casts;
pub(crate) mod changes;
pub mod collection;
pub mod console;
pub mod events;
pub mod fillable;
pub mod lazy;
pub mod lazy_loading;
pub mod model;
pub mod observers;
pub mod prunable;
pub mod registry;
pub mod relations;
pub mod scopes;
pub mod soft_deletes;
pub mod timestamps;
pub mod unique_id;

pub use attrs::Attrs;
pub use builder::{Builder, Direction, IntoColumn, IntoVal};
pub use casts::{
    AsArray, AsArrayObject, AsBool, AsCollection, AsDate, AsDateTime, AsDecimal, AsEncrypted,
    AsEncryptedArray, AsEncryptedCollection, AsEncryptedObject, AsEnum, AsFloat, AsHashed,
    AsImmutableDate, AsImmutableDateTime, AsInt, AsJson, AsNaiveDateTime, AsNativeDateTime,
    AsNativeJson, AsObject, AsOptionalArray, AsOptionalArrayObject, AsOptionalCollection,
    AsOptionalDateTime, AsOptionalJson, AsOptionalNaiveDateTime, AsOptionalNativeDateTime,
    AsOptionalNativeJson, AsOptionalObject, AsOptionalU64, AsString, AsTimestamp, AsU64, Cast,
    DynCast, IntoDynCast, StoredU64,
};
pub use collection::Collection;
pub use fillable::{
    Fillable, prevent_silently_discarding_attributes, preventing_silently_discarding_attributes,
    unguarded,
};
pub use lazy::LazyCollection;
pub use lazy_loading::{
    LazyLoadingViolation, clear_lazy_loading_violation_handler, handle_lazy_loading_violation,
    prevent_lazy_loading, preventing_lazy_loading,
};
pub use model::{FirstOrCreate, Model, ReplicateExt};
pub use prunable::{
    MassPrunable, Prunable, PrunerEntry, PrunerFn, prune_all, prune_all_dry, prune_one, pruners,
};
pub use registry::{ModelEntry, find_model_by_table, models};
pub use relations::{
    AggregateKind, BelongsTo, BelongsToMany, EagerLoadCache, EagerLoadDispatch, HasMany,
    HasManyThrough, HasOne, HasOneThrough, MorphMany, MorphOne, MorphTo, MorphToMany,
    MorphTypeEntry, MorphedByMany, Relation, RelationEntry, RelationKind, aggregate_cache_key,
    find_morph_type, find_morph_type_by_id, find_relation, morph_types, relations, relations_of,
    touch_column,
};
pub use scopes::{GlobalScope, ScopeRegistry};
pub use soft_deletes::SoftDeletes;
pub use timestamps::{
    Touchable, touches_disabled, touches_ignored_for, without_touching, without_touching_on,
};
pub use unique_id::{HasUniqueId, UniqueIdKind};

/// Holds when `Self` and `K` are one type. The `#[suprnova::model]` macro
/// asserts it between the primary-key field's type and a `key_type`
/// attribute spelled otherwise, so a `key_type` that disagrees with the
/// field fails the build, naming both. Not part of the public API.
#[doc(hidden)]
#[diagnostic::on_unimplemented(
    message = "`key_type = \"{K}\"` disagrees with the primary-key field's type `{Self}`",
    label = "`key_type` names `{K}`, the primary-key field is `{Self}`",
    note = "remove `key_type`: the model takes the key type from the primary-key field"
)]
pub trait KeyTypeMatches<K> {}

impl<T> KeyTypeMatches<T> for T {}

/// The values a mass soft delete binds. Built by the `#[suprnova::model]`
/// macro; not part of the public API.
#[doc(hidden)]
pub struct SoftDeleteStamp {
    /// The tombstone, in the soft-delete column's storage form.
    pub deleted_at: sea_orm::Value,
    /// "Now" in the `updated_at` column's storage form, when the model
    /// manages timestamps.
    pub updated_at: Option<sea_orm::Value>,
}

/// Marker trait emitted by `#[suprnova::model]`. Indicates the struct
/// is a Suprnova-managed model.
///
/// This trait grows across Phase 10A tasks (T3 / T4 / T6 / T7a / ...);
/// the stable shape locks at T11 closeout.
pub trait EloquentModel: Sized {
    /// SeaORM entity backing this model (`<inner_mod>::Entity`).
    type Entity: crate::EntityTrait;
    /// Column enum for this model (`<inner_mod>::Column`).
    type Column;
    /// The Rust type of this model's primary key: the primary-key field's
    /// declared type, which `#[model(key_type = "...")]` may only repeat.
    ///
    /// Declared on the trait rather than derived from the SeaORM entity
    /// so a terminal that projects the key can name the type the
    /// *user's* struct uses, not the storage type a cast may have
    /// rewritten underneath it.
    ///
    /// The two bounds are the two directions a key is read: straight
    /// out of a SQL row by
    /// [`Builder::model_keys`](crate::eloquent::Builder::model_keys)
    /// (`TryGetable`), and out of an already-hydrated row's JSON field
    /// value by
    /// [`Collection::model_keys`](crate::eloquent::Collection::model_keys)
    /// (`DeserializeOwned`).
    type Key: crate::TryGetable + serde::de::DeserializeOwned + Send + Sync + 'static;
    /// Database table name (`#[model(table = "...")]`).
    const TABLE: &'static str;
    /// Primary-key column name. The macro emits the value from the
    /// `primary_key = "..."` attribute (default `"id"`). Mirrors
    /// [`crate::eloquent::Model::primary_key_name`] but as a `const`
    /// so it can be read by `inventory::submit!` initialisers - the
    /// has/where-has engine pulls each relation's target PK from here
    /// at link time to render the correct pivot join.
    const PRIMARY_KEY: &'static str = "id";
    /// Soft-delete column on this model. `""` when the model does NOT
    /// opt into `#[model(soft_deletes)]`; otherwise the model's
    /// configured `deleted_at` column name. Read by the has/where-has
    /// engine to auto-apply the related model's soft-delete scope to
    /// EXISTS subqueries (a parent with only soft-deleted children
    /// must NOT match `has("children")`).
    const SOFT_DELETES_COLUMN: &'static str = "";

    /// What a mass soft delete writes for "now", each value in its
    /// column's own storage form: the tombstone for
    /// [`Self::SOFT_DELETES_COLUMN`], and the value for
    /// [`Self::UPDATED_AT_COLUMN`] when the model manages timestamps.
    /// `None` on a model without soft deletes. The macro emits it from
    /// the columns' casts, so the mass form writes what a row's own
    /// `delete()` and `save()` write.
    #[doc(hidden)]
    fn __soft_delete_stamp() -> Result<Option<SoftDeleteStamp>, crate::FrameworkError> {
        Ok(None)
    }

    /// Names of the `BelongsTo` and `MorphTo` relations whose parent
    /// row gets its `updated_at` bumped after this model is created,
    /// saved, updated, or deleted. Populated by
    /// `#[model(touches = [...])]`. A `MorphTo` owner is the row its
    /// `<name>_type` and `<name>_id` columns name. A `<name>_type` that
    /// names none of the relation's targets is an error of the write:
    /// the owners are resolved once the pre-write listeners have run
    /// and before the statement, so it returns `Err` with no statement
    /// run and no post-write event dispatched. A pre-write listener that
    /// rewrites `<name>_type` or `<name>_id` decides the owner.
    ///
    /// Read by [`crate::eloquent::Model::touch_owners`], which is a
    /// trait default - so the list has to live on a trait too, or the
    /// generic body couldn't see it.
    const TOUCHES: &'static [&'static str] = &[];

    /// Whether this model manages `created_at` / `updated_at`. `false`
    /// when the struct carries neither column, or the user wrote
    /// `#[model(timestamps = false)]`.
    ///
    /// Separate from [`Self::UPDATED_AT_COLUMN`] because the two facts
    /// genuinely differ: a model can name a custom `updated_at` column
    /// and still opt out of managing it. The parent-touch cascade
    /// consults this to skip an opted-out owner rather than writing a
    /// column the owner disclaims - Laravel's `isIgnoringTouch` gained
    /// the same check in 13.25.
    const HAS_TIMESTAMPS: bool = false;

    /// The `updated_at` column name, honouring
    /// `#[model(updated_at = "...")]`. Meaningful only when
    /// [`Self::HAS_TIMESTAMPS`] is `true`.
    const UPDATED_AT_COLUMN: &'static str = "updated_at";

    /// The parameter a query binds when it compares `column` with `value`,
    /// through the column's cast ([`casts::Cast::bind_json`]). `None` binds
    /// the value as it is. A column may carry this model's table
    /// (`posts.created_at`); one qualified with another table answers
    /// `None`, since this model's casts say nothing about that table. The
    /// macro overrides this for a model with cast fields; the query
    /// builder asks it for every comparison.
    #[doc(hidden)]
    fn bind_column(_column: &str, _value: &serde_json::Value) -> Option<sea_orm::Value> {
        None
    }

    /// Reads this model's key out of `row`, from the column `column`.
    ///
    /// The default decodes [`Self::Key`] straight from the row. The macro
    /// overrides it when the key field has a cast, a `u64` key among them:
    /// SeaORM decodes a `u64` on MySQL only, so the key is read through the
    /// cast's storage type and converted.
    #[doc(hidden)]
    fn __decode_key(row: &sea_orm::QueryResult, column: &str) -> Result<Self::Key, sea_orm::DbErr> {
        row.try_get::<Self::Key>("", column)
    }

    /// The value this model's `updated_at` column stores for `now`,
    /// through the column's cast. The parent-touch cascade writes an
    /// owner's column with it, so an owner with a native date-time
    /// column receives a native value and not the text the default
    /// cast stores, which Postgres refuses for such a column.
    ///
    /// The default is that RFC 3339 text; the macro overrides it with the
    /// declared cast of the `updated_at` field.
    #[doc(hidden)]
    fn updated_at_storage(
        now: &chrono::DateTime<chrono::Utc>,
    ) -> Result<sea_orm::Value, crate::FrameworkError> {
        Ok(sea_orm::Value::String(Some(now.to_rfc3339())))
    }

    /// The per-model default connection name. Returns `None` for
    /// models that don't declare `#[model(connection = "...")]`; the
    /// macro overrides this when the attribute is set, returning
    /// `Some(<literal>)`.
    ///
    /// Consulted by
    /// [`crate::database::transaction::ExecutorChoice::resolve_read`]
    /// / [`resolve_write`](crate::database::transaction::ExecutorChoice::resolve_write)
    /// as step 4 of the routing chain - after the per-builder
    /// `on(name)` override but before `__read_replica__` auto-routing.
    /// `Some("__primary__")` short-circuits to
    /// [`crate::DB::connection`] without consulting the registry; any
    /// other name routes through
    /// [`crate::database::ConnectionRegistry::get`].
    ///
    /// Lives on `EloquentModel` (not the heavier
    /// [`crate::eloquent::Model`] trait) so generic relation impls
    /// that only need the lightweight marker bound can still consult
    /// it without dragging in the full CRUD bound chain.
    fn default_connection_name() -> ::core::option::Option<&'static str> {
        ::core::option::Option::None
    }
}
