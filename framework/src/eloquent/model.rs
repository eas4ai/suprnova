//! Eloquent Model trait - the CRUD lifecycle layer.
//!
//! Implemented by the `#[suprnova::model]` macro on every annotated
//! struct, this trait carries the bulk of the Eloquent API surface:
//! `find` / `find_or_fail` / `find_many` / `all` / `query` /
//! `create` / `save` / `update` / `delete` / `force_delete` /
//! `refresh` / `fresh` / `replicate` / `replicate_except` /
//! `replicate_into` / `increment` / `decrement`. The companion
//! [`FirstOrCreate`] trait carries the first-or-... lookup methods.
//!
//! All trait methods are default-implemented. The macro emits the
//! per-model glue (PK accessor, attribute application, into-active-model
//! conversion) and trivial `impl ::suprnova::eloquent::Model for #struct {}`
//! lines.
//!
//! ## delete vs force_delete
//!
//! T4 ships hard-delete only - both methods call SeaORM's DELETE. T10
//! introduces soft-deletes; once that lands, `delete` honours the
//! `soft_deletes` attribute (sets `deleted_at` instead of removing the
//! row) while `force_delete` always removes the row.

use std::collections::HashMap;
use std::hash::Hash;

use async_trait::async_trait;
use sea_orm::{
    ColumnTrait, EntityTrait, IntoActiveModel, PrimaryKeyToColumn, PrimaryKeyTrait, QueryFilter,
};
// `find_many` calls `<Self::Entity as EntityTrait>::PrimaryKey::iter()`.
// `iter` lives on `IntoEnumIterator`, brought in via `PrimaryKeyTrait`'s
// `Iterable` supertrait. Importing it explicitly so the call resolves
// regardless of supertrait-method-resolution edge cases.
use sea_orm::strum::IntoEnumIterator;
use serde::Serialize;
use serde::de::DeserializeOwned;

// Direct `DB::connection()` calls have been replaced with
// `crate::database::transaction::ExecutorChoice::resolve()` so every
// Model CRUD path honours an active `DB::transaction` scope without
// callers threading a tx handle through every method.
use crate::eloquent::EloquentModel;
use crate::eloquent::attrs::Attrs;
use crate::eloquent::builder::Builder;
use crate::eloquent::collection::Collection;
use crate::eloquent::events::ModelEventHooks;
use crate::eloquent::fillable::Fillable;
use crate::error::FrameworkError;

/// Records a table read and hands the error back unchanged.
///
/// Used on every failure path of a point read, so a read that failed is
/// never recorded as narrower than a whole-table read: the row it would
/// have returned is unknown, and an entry that depended on "no answer" has
/// to be invalidated by anything that could change it.
fn table_read_then<E>(table: &str, error: E) -> E {
    crate::render_cache::collector::observe_table_read(table);
    error
}

/// Whether `key` holds a value its column cannot hold on `backend`: a
/// `u64` above `i64::MAX` on Postgres or SQLite, where the key is a signed
/// `BIGINT`. No row has such a key, so a lookup by it finds nothing without
/// asking, and is never sent: sea-query-sqlx's binders there would panic
/// on the value.
pub(crate) fn key_beyond_signed(
    backend: sea_orm::DbBackend,
    key: &sea_orm::sea_query::ValueTuple,
) -> bool {
    key.iter()
        .any(|value| crate::eloquent::casts::unsigned::beyond_signed(backend, value))
}

/// `UPDATE table SET column = column <operator> by WHERE pk = ?`, the
/// body of [`Model::increment`] and [`Model::decrement`]. The operator is
/// written rather than the amount negated, because `i64::MIN` has no
/// negation.
async fn step_column<M>(
    model: &M,
    column: &str,
    operator: &str,
    by: i64,
) -> Result<(), FrameworkError>
where
    M: Model,
    M: From<<M::Entity as EntityTrait>::Model>,
    <M::Entity as EntityTrait>::Model: From<M>
        + IntoActiveModel<<M::Entity as EntityTrait>::ActiveModel>
        + Serialize
        + Send
        + Sync,
    <M::Entity as EntityTrait>::ActiveModel: Send,
    <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    // Audit HIGH `eloquent` #1 - column is interpolated raw into
    // the SQL string and cannot be parameterised. Validate
    // against the framework's SQL identifier rules before render.
    crate::database::validate_identifier(column)?;
    let table = M::TABLE;
    let pk_name = M::primary_key_name();
    let pk_value = model.primary_key_value_json();
    // T11/T12: route through resolve_write.
    let exec = crate::database::transaction::ExecutorChoice::resolve_write(
        None,
        None,
        M::default_connection_name(),
    )
    .await?;
    let backend = exec.backend();
    // Rendered after the executor resolves, because only it knows the
    // backend - and Postgres rejects `?`, so a hard-coded placeholder
    // made increment/decrement (and every counter built on them) fail
    // outright there.
    let by_ph = crate::database::placeholder::placeholder(backend, 1)?;
    let pk_ph = crate::database::placeholder::placeholder(backend, 2)?;
    let sql = format!(
        "UPDATE {table} SET {column} = {column} {operator} {by_ph} WHERE {pk_name} = {pk_ph}"
    );
    exec.run(sea_orm::Statement::from_sql_and_values(
        backend,
        &sql,
        vec![by.into(), json_value_to_sea_value(&pk_value)],
    ))
    .await
    .map_err(|e| FrameworkError::database(e.to_string()))?;
    crate::render_cache::orm::after_model_write(model).await?;
    Ok(())
}

/// The row state a model's relation cache keeps, when it has a cache.
fn row_state(
    cache: Option<&crate::eloquent::relations::EagerLoadCache>,
) -> Option<&crate::eloquent::changes::RowState> {
    cache.map(crate::eloquent::relations::EagerLoadCache::row_state)
}

/// Record a save on `current`, the model hydrated from the row the write
/// returned, comparing it with `saved`, the model that was saved.
///
/// Runs right after the write and before the `Updated` event, so an
/// `updated` observer reads the record off `current`. `adopt` copies the
/// record onto `saved` as well: `save` borrows the caller's model and
/// cannot hand back `current`, so the caller's model has to carry it.
/// `update` returns `current` and leaves `saved`, the observer's
/// `previous`, as it was. The save ends with
/// [`finish_save`](crate::eloquent::changes::finish_save) on the model the
/// caller keeps, after the `Saved` event and the owner touches, as
/// Laravel's `finishSave` ends with `syncOriginal`.
///
/// `decoded_equal` is the model's [`Model::__decoded_values_equal`], which
/// compares a column whose cast stores a new value on every write by its
/// decoded value.
///
/// The only failure is a stored row that will not serialize to JSON, which
/// a `#[suprnova::model]` row cannot produce; it is still reported rather
/// than recorded wrong.
fn record_save_on(
    saved: Option<&crate::eloquent::relations::EagerLoadCache>,
    current: Option<&crate::eloquent::relations::EagerLoadCache>,
    adopt: bool,
    decoded_equal: crate::eloquent::changes::DecodedEqual,
) -> Result<(), FrameworkError> {
    let saved = row_state(saved);
    let current = row_state(current);
    crate::eloquent::changes::record_save(saved, current, decoded_equal)?;
    if adopt && let (Some(saved), Some(current)) = (saved, current) {
        saved.adopt(current);
    }
    Ok(())
}

/// The Eloquent CRUD lifecycle. Auto-implemented for every
/// `#[suprnova::model]` struct.
///
/// Method semantics mirror Laravel's `Illuminate\Database\Eloquent\Model`
/// where possible. Divergences are flagged in the rustdoc and in
/// `docs/superpowers/specs/phase-10/phase-10a/01-crud.md`.
#[async_trait]
pub trait Model:
    EloquentModel
    + Send
    + Sync
    + Sized
    + Clone
    + Serialize
    + DeserializeOwned
    + ModelEventHooks
    + 'static
where
    Self: From<<Self::Entity as EntityTrait>::Model>,
    <Self::Entity as EntityTrait>::Model: From<Self>
        + IntoActiveModel<<Self::Entity as EntityTrait>::ActiveModel>
        + Serialize
        + Send
        + Sync,
    <Self::Entity as EntityTrait>::ActiveModel: Send,
    <<Self::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    /// Primary-key column name. The macro emits the value from the
    /// `primary_key = "..."` attribute (default `"id"`).
    fn primary_key_name() -> &'static str {
        "id"
    }

    /// The table-qualified primary-key column - `"users.id"`.
    ///
    /// Laravel's `getQualifiedKeyName()`. Terminals that project the key
    /// alone emit this rather than the bare column so the statement stays
    /// unambiguous the moment the query grows a join: two joined tables
    /// each carrying an `id` make a bare `id` a hard error on every
    /// backend, and the caller who wrote `.model_keys()` never asked to
    /// think about that.
    fn qualified_key_name() -> String {
        format!("{}.{}", Self::TABLE, Self::primary_key_name())
    }

    /// Per-model mass-assignment guard. The macro's Task 4 emission
    /// returns `Fillable::guarded(vec![PRIMARY_KEY])`; Task 6 wires
    /// `fillable = [...]` / `guarded = [...]` attributes.
    fn fillable_filter() -> Fillable;

    /// Fallible hydration of an inner SeaORM row into this model - the
    /// `?`-propagating analogue of the infallible
    /// `From<<Self::Entity>::Model>` bridge the macro also emits.
    ///
    /// The framework's own read paths (`find`, `find_many`, `all`,
    /// [`Builder::get`](crate::eloquent::Builder), ...) route through
    /// this method so a cast that fails to decode a stored value - a
    /// corrupt column, a deprecated enum variant left in old rows,
    /// schema drift - surfaces as a recoverable [`FrameworkError`]
    /// rather than a panic. That matters off the HTTP path: a queue
    /// worker, the scheduler, or a CLI command has no panic-recovery
    /// middleware to turn a panic into a 500, so an unguarded panic
    /// there tears down the task.
    ///
    /// The infallible `From` impl is retained as an ergonomic escape
    /// hatch (`let u: User = row.into()`); it panics on the same
    /// failure with a field-named diagnostic. The default below
    /// delegates to it so non-`#[suprnova::model]` types that satisfy
    /// the trait bounds still compile; the macro overrides this with
    /// the per-field `Cast::from_storage` form that propagates via `?`.
    fn try_from_storage(row: <Self::Entity as EntityTrait>::Model) -> Result<Self, FrameworkError> {
        Ok(Self::from(row))
    }

    /// Fallible dehydration of this model into its inner SeaORM row -
    /// the `?`-propagating analogue of the infallible
    /// `From<Self> for <Self::Entity>::Model` bridge.
    ///
    /// The framework's write paths (`save`, `update`, `delete`,
    /// `force_delete`, and their `_with_tx` variants) route through
    /// this so a cast that fails to encode a runtime value becomes a
    /// recoverable [`FrameworkError`] instead of a panic. See
    /// [`Self::try_from_storage`] for the off-the-HTTP-path rationale;
    /// the macro overrides this with the per-field `Cast::to_storage`
    /// form.
    fn try_into_storage(self) -> Result<<Self::Entity as EntityTrait>::Model, FrameworkError> {
        Ok(self.into())
    }

    /// Phase 10C T5b - read this row's field by column name and
    /// serialise it to a `serde_json::Value`. Returns `None` when the
    /// column name doesn't match any declared field on the model (and
    /// when the per-field serialisation fails, which the macro's
    /// arms lower to `None`).
    ///
    /// The default returns `None` so non-`#[suprnova::model]` types
    /// that meet the supertrait bounds (rare - almost nothing else
    /// satisfies them) don't break. The macro overrides this with one
    /// match arm per declared column field.
    ///
    /// Powers the string-keyed surface on
    /// [`Collection<M>`](crate::eloquent::Collection) -
    /// `pluck("col")`, `group_by("col")`, `sort_by("col")`,
    /// `where_eq("col", v)`, `sum::<T>("col")`, etc. The macro emission
    /// lives in `suprnova-macros/src/model/serialization.rs`.
    fn field_value(&self, _name: &str) -> ::core::option::Option<serde_json::Value> {
        ::core::option::Option::None
    }

    /// Phase 10C T6 - serialise this row to a JSON object.
    ///
    /// Default implementation serialises the whole struct via
    /// `serde_json::to_value(self)` and explicitly removes the
    /// macro-injected `__eager` / `__pivot` scratch fields. Both
    /// fields carry `#[serde(skip)]` on the struct definition, so the
    /// removal is belt-and-braces - it pins the [Phase 10B P6
    /// contract](../../docs/superpowers/specs/phase-10/phase-10b.md)
    /// (eager-load cache stays out of serialisation) even against a
    /// hypothetical future model with a hand-rolled `Serialize` impl.
    ///
    /// The macro overrides this when the model declares
    /// `hidden = [...]`, `visible = [...]`, or `appends = [...]` on
    /// `#[suprnova::model]`. The override applies those filters in
    /// Laravel order: visible (whitelist) → hidden (denylist) →
    /// appends (accessor injection, runs after filters so appends
    /// always show up even if they share a name with a hidden field).
    fn to_array(&self) -> serde_json::Value {
        let mut v = serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        if let Some(m) = v.as_object_mut() {
            m.remove("__eager");
            m.remove("__pivot");
        }
        v
    }

    /// Phase 10C T6 - serialise this row to a JSON string. Delegates
    /// to [`Self::to_array`] so the same hidden/visible/appends
    /// filters apply when callers reach for the string shape directly.
    fn to_json(&self) -> String {
        serde_json::to_string(&self.to_array()).unwrap_or_default()
    }

    /// Phase 10C T6 - append-accessor dispatcher. The macro overrides
    /// this with a `match` block when `appends = [...]` is non-empty,
    /// dispatching each declared name to the user's
    /// `#[suprnova::accessor]`-tagged method. The default returns
    /// `None` for every name, which keeps the [`Self::to_array`]
    /// override branch a no-op for models that don't declare appends.
    #[doc(hidden)]
    fn __append_accessor(&self, _name: &str) -> ::core::option::Option<serde_json::Value> {
        ::core::option::Option::None
    }

    /// Record that this row came out of a query that returned more than
    /// one row: the mark lazy-loading prevention reads (see
    /// [`crate::eloquent::lazy_loading`]).
    ///
    /// The `#[suprnova::model]` macro overrides it to set the mark in
    /// the row's relation cache. The default does nothing: a model
    /// without that cache has no relation methods to refuse.
    ///
    /// **Not part of the public API.** It is `pub` because the macro
    /// emits the override.
    #[doc(hidden)]
    fn __mark_from_multi_row_query(&mut self) {}

    /// The relation cache the `#[suprnova::model]` macro injects as
    /// `__eager`. Besides relations it keeps the row the instance was read
    /// from, which [`Self::was_changed`] and its siblings read.
    ///
    /// The macro overrides it. The default, for a type without the cache,
    /// keeps no row, so such a type reports no change and no original.
    ///
    /// **Not part of the public API.** It is `pub` because the macro emits
    /// the override.
    #[doc(hidden)]
    fn __eager_cache(&self) -> Option<&crate::eloquent::relations::EagerLoadCache> {
        None
    }

    /// Build this model back from a row the cache kept, so
    /// [`Self::get_original`] can read a value through the model's casts.
    /// `None` when `row` is not this model's stored row.
    ///
    /// The macro overrides it with a downcast to the model's own SeaORM
    /// row, the one type the generic default cannot name.
    ///
    /// **Not part of the public API.** It is `pub` because the macro emits
    /// the override.
    #[doc(hidden)]
    fn __model_from_stored_row(
        row: &(dyn std::any::Any + Send + Sync),
    ) -> Option<Result<Self, FrameworkError>> {
        let _ = row;
        None
    }

    /// Whether `column` holds the same value in `before` and `after`, two
    /// of this model's stored rows whose stored values of `column` differ.
    ///
    /// A save tells what it changed by comparing stored rows, and asks this
    /// for each column whose stored value differs. A column whose cast
    /// stores a new value on every write (see
    /// [`Cast::DETERMINISTIC_STORAGE`](crate::eloquent::casts::Cast::DETERMINISTIC_STORAGE)),
    /// such as an encrypted one, then counts as changed only when its
    /// decoded value did. The macro overrides this for a model with casts,
    /// decoding through them; the default answers `false`, so differing
    /// stored values are a change.
    ///
    /// **Not part of the public API.** It is `pub` because the macro emits
    /// the override.
    #[doc(hidden)]
    fn __decoded_values_equal(
        column: &str,
        before: &(dyn std::any::Any + Send + Sync),
        after: &(dyn std::any::Any + Send + Sync),
    ) -> Result<bool, FrameworkError> {
        let _ = (column, before, after);
        Ok(false)
    }

    /// Mark every row of one query's result when the query returned more
    /// than one row. The one place that rule lives: every read path that
    /// hydrates several rows at once calls it with the rows it hydrated.
    ///
    /// **Not part of the public API.**
    #[doc(hidden)]
    fn __mark_query_result(rows: &mut [Self]) {
        if rows.len() > 1 {
            for row in rows {
                row.__mark_from_multi_row_query();
            }
        }
    }

    /// Look up a row by primary key. `None` if no row matches.
    ///
    /// The trait default uses SeaORM's `find_by_id` directly - no
    /// global scopes apply. Models that declare `#[model(soft_deletes)]`
    /// receive an inherent `find` override emitted by the macro that
    /// routes through [`Self::query`] (which applies the
    /// `deleted_at IS NULL` filter); the inherent shadows the trait
    /// default for the soft-delete path. Callers that need to bypass
    /// the scope use `Self::with_trashed()` instead.
    ///
    /// Dispatches `Retrieving` before the SELECT and `Retrieved`
    /// when a row is hydrated (no dispatch when the row is missing).
    async fn find<K>(id: K) -> Result<Option<Self>, FrameworkError>
    where
        K: Into<<<Self::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType> + Send,
    {
        Self::__dispatch_retrieving()
            .await
            .map_err(|error| table_read_then(Self::TABLE, error))?;
        // T11/T12: route through resolve_read so the read honours any
        // ambient `DB::transaction` closure scope, per-model
        // `connection = "..."` default, and `__read_replica__`
        // auto-routing. No builder-level overrides at this layer -
        // `Model::find` doesn't take a Builder.
        let exec = crate::database::transaction::ExecutorChoice::resolve_read(
            None,
            None,
            Self::default_connection_name(),
        )
        .await
        .map_err(|error| table_read_then(Self::TABLE, error))?;
        // The key is checked before SeaORM binds it, so it is filtered here
        // the way `find_by_id` would filter it. A key no row can hold finds
        // nothing, as a missing row does.
        let key = sea_orm::sea_query::IntoValueTuple::into_value_tuple(id.into());
        if key_beyond_signed(exec.backend(), &key) {
            crate::render_cache::collector::observe_table_read(Self::TABLE);
            return Ok(None);
        }
        let mut select = Self::Entity::find();
        for (column, value) in <Self::Entity as EntityTrait>::PrimaryKey::iter().zip(key) {
            select = select.filter(column.into_column().eq(value));
        }
        let row = exec
            .select_one(select)
            .await
            .map_err(|e| table_read_then(Self::TABLE, FrameworkError::database(e.to_string())))?;
        let hydrated = row
            .map(Self::try_from_storage)
            .transpose()
            .map_err(|error| table_read_then(Self::TABLE, error))?;
        match hydrated {
            // A hydrated row depends on that row and on any write that
            // could have touched it without naming it, and on nothing else:
            // this is what lets a cached page built from one row survive
            // every write to every other row of the table.
            //
            // Guarded on `is_active()` before computing
            // `primary_key_value_json()`/`to_string()`, which would
            // otherwise run on every `find`, including the common request
            // with no collector scope at all.
            Some(ref m) => {
                if crate::render_cache::collector::is_active() {
                    crate::render_cache::collector::observe_record_read_json(
                        Self::TABLE,
                        &m.primary_key_value_json(),
                    );
                    crate::render_cache::collector::observe_unkeyed_write(Self::TABLE);
                }
            }
            // No row: the answer changes when one is inserted, which no
            // record identity can express.
            None => crate::render_cache::collector::observe_table_read(Self::TABLE),
        }
        if let Some(ref m) = hydrated {
            Self::__dispatch_retrieved(m).await?;
        }
        Ok(hydrated)
    }

    /// Look up a row by primary key. Returns `FrameworkError::ModelNotFound`
    /// (HTTP 404) when no row matches.
    async fn find_or_fail<K>(id: K) -> Result<Self, FrameworkError>
    where
        K: Into<<<Self::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType>
            + std::fmt::Debug
            + Copy
            + Send,
    {
        match Self::find(id).await? {
            Some(m) => Ok(m),
            None => Err(FrameworkError::not_found(format!(
                "{} with {} = {:?} not found",
                std::any::type_name::<Self>(),
                Self::primary_key_name(),
                id
            ))),
        }
    }

    /// Fetch every row whose PK is in `ids`. Result preserves the
    /// order of `ids` (not the database's natural order). Unmatched
    /// IDs are silently dropped.
    ///
    /// Dispatches `Retrieving` once before the SELECT and
    /// `Retrieved` once per hydrated row.
    async fn find_many<I, K>(ids: I) -> Result<Vec<Self>, FrameworkError>
    where
        I: IntoIterator<Item = K> + Send,
        I::IntoIter: Send,
        K: Into<<<Self::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType>
            + Clone
            + Send,
        <<Self::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
            Hash + Eq + Clone,
    {
        let id_vec: Vec<_> = ids.into_iter().map(|k| k.into()).collect();
        if id_vec.is_empty() {
            return Ok(Vec::new());
        }
        let requested = id_vec.len();
        Self::__dispatch_retrieving()
            .await
            .map_err(|error| table_read_then(Self::TABLE, error))?;
        let pk = <Self::Entity as EntityTrait>::PrimaryKey::iter()
            .next()
            .expect("model has at least one primary-key column");
        // T11/T12: route through resolve_read.
        let exec = crate::database::transaction::ExecutorChoice::resolve_read(
            None,
            None,
            Self::default_connection_name(),
        )
        .await
        .map_err(|error| table_read_then(Self::TABLE, error))?;
        // An id no row can hold is not sent; it is skipped like an id
        // that matches no row.
        let held: Vec<_> = id_vec
            .iter()
            .filter(|id| {
                let key = sea_orm::sea_query::IntoValueTuple::into_value_tuple((*id).clone());
                !key_beyond_signed(exec.backend(), &key)
            })
            .cloned()
            .collect();
        let rows = if held.is_empty() {
            Vec::new()
        } else {
            exec.select_all(Self::Entity::find().filter(pk.into_column().is_in(held)))
                .await
                .map_err(|e| {
                    table_read_then(Self::TABLE, FrameworkError::database(e.to_string()))
                })?
        };

        let mut by_id: HashMap<_, _> = rows
            .into_iter()
            .map(|row| {
                let model = Self::try_from_storage(row)?;
                Ok((model.primary_key_value(), model))
            })
            .collect::<Result<HashMap<_, _>, FrameworkError>>()
            .map_err(|error| table_read_then(Self::TABLE, error))?;
        let mut ordered: Vec<Self> = id_vec
            .into_iter()
            .filter_map(|id| by_id.remove(&id))
            .collect();
        Self::__mark_query_result(&mut ordered);
        if crate::render_cache::collector::is_active() {
            for row in &ordered {
                crate::render_cache::collector::observe_record_read_json(
                    Self::TABLE,
                    &row.primary_key_value_json(),
                );
            }
            if !ordered.is_empty() {
                crate::render_cache::collector::observe_unkeyed_write(Self::TABLE);
            }
            // Fewer rows than ids asked for: an insert of a missing id
            // changes the answer, and so does a duplicate id in the request
            // (the second copy finds the row already taken), which this
            // treats as a miss. Over-observing is the safe direction.
            if ordered.len() != requested {
                crate::render_cache::collector::observe_table_read(Self::TABLE);
            }
        }
        for row in &ordered {
            Self::__dispatch_retrieved(row).await?;
        }
        Ok(ordered)
    }

    /// Fetch every row in the table.
    ///
    /// Dispatches `Retrieving` once before the SELECT and
    /// `Retrieved` once per hydrated row.
    ///
    /// Returns a [`Collection<Self>`](crate::eloquent::Collection) so
    /// the result composes with the model-aware string-keyed surface
    /// (`pluck("col")`, `group_by("col")`, `sum::<T>("col")`, ...). The
    /// inner `Vec` is reachable via `.into_vec()` for call sites that
    /// need explicit `Vec` semantics; slice-shape access (`.iter()`,
    /// `.len()`, indexing, `for row in &collection`) works directly
    /// via `Deref<Target = [Self]>`.
    async fn all() -> Result<Collection<Self>, FrameworkError> {
        crate::render_cache::collector::observe_table_read(Self::TABLE);
        Self::__dispatch_retrieving().await?;
        // T11/T12: route through resolve_read.
        let exec = crate::database::transaction::ExecutorChoice::resolve_read(
            None,
            None,
            Self::default_connection_name(),
        )
        .await?;
        let rows = exec
            .select_all(Self::Entity::find())
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        let mut out: Vec<Self> = rows
            .into_iter()
            .map(Self::try_from_storage)
            .collect::<Result<Vec<_>, _>>()?;
        Self::__mark_query_result(&mut out);
        for row in &out {
            Self::__dispatch_retrieved(row).await?;
        }
        Ok(Collection::from_vec(out))
    }

    /// Start a new builder against this model. The model's soft-delete
    /// filter and its registered global scopes apply to every query the
    /// builder runs, reads and mass writes alike; callers opt out
    /// per-type with [`Builder::without_global_scope::<S>`] or
    /// all-at-once with [`Builder::without_global_scopes`], anywhere in
    /// the chain.
    ///
    /// The scopes are folded in when the query runs, not here. A scope
    /// that reads per-request state, such as the current tenant, reads
    /// it at that moment.
    ///
    /// The scope registry is keyed by `TypeId::of::<Self>()`. The
    /// `Model: 'static` supertrait bound makes that lookup well-defined
    /// for every concrete `#[suprnova::model]` struct.
    ///
    /// [`Builder::without_global_scope::<S>`]: crate::eloquent::Builder::without_global_scope
    /// [`Builder::without_global_scopes`]: crate::eloquent::Builder::without_global_scopes
    fn query() -> Builder<Self> {
        Builder::__scoped()
    }

    /// Mass-create a row from the given attributes. Attributes are
    /// filtered through [`Self::fillable_filter`] before the SeaORM
    /// ActiveModel is built.
    ///
    /// ## Lifecycle events (Phase 10C T1)
    ///
    /// Dispatched in this order:
    ///
    /// 1. `Creating { attrs }` - cancellable
    /// 2. `Saving { attrs, is_creating: true }` - cancellable
    /// 3. *INSERT lands*
    /// 4. `Created { model }`
    /// 5. `Saved { model }`
    ///
    /// A listener that cancels at (1) or (2) aborts the operation
    /// with `FrameworkError::bad_request(reason)`; the INSERT never
    /// runs. Listeners on (1) / (2) may mutate the in-flight `Attrs`
    /// through the `Arc<tokio::sync::Mutex<Attrs>>` they receive.
    async fn create(attrs: Attrs) -> Result<Self, FrameworkError> {
        let filtered = Self::fillable_filter().apply_checked(attrs)?;
        // Wrap the filtered attrs in an Arc<Mutex<_>> so cancellable
        // listeners (Creating, Saving) can mutate the in-flight
        // values before the INSERT runs.
        let shared = std::sync::Arc::new(tokio::sync::Mutex::new(filtered));
        Self::__dispatch_creating(shared.clone()).await?;
        Self::__dispatch_saving(shared.clone(), true).await?;

        // Read the (possibly mutated) attrs back out of the mutex
        // before consuming them to build the ActiveModel. The
        // Arc<Mutex<_>> handle is dropped once we leave this scope.
        let final_attrs = shared.lock().await.clone();
        let touch_plan = Self::__plan_touches(None, &final_attrs)?;
        let am = Self::active_model_from_attrs(final_attrs)?;
        // T11/T12: route through resolve_write - insert lands in the
        // active transaction when called inside `DB::transaction`,
        // honours per-model `connection = "..."`, and skips
        // `__read_replica__` (writes always go to primary unless the
        // model explicitly opts elsewhere).
        let row =
            crate::render_cache::orm::atomic(Self::default_connection_name(), || async move {
                let exec = crate::database::transaction::ExecutorChoice::resolve_write(
                    None,
                    None,
                    Self::default_connection_name(),
                )
                .await?;
                let inserted = exec
                    .insert_active(am)
                    .await
                    .map_err(|e| FrameworkError::database(e.to_string()))?;
                let row = Self::try_from_storage(inserted)?;
                crate::render_cache::orm::after_model_write(&row).await?;
                Ok(row)
            })
            .await?;

        // Nothing was loaded before the insert: no original until it returns.
        crate::eloquent::changes::begin_insert(row_state(row.__eager_cache()));
        Self::__dispatch_created(&row).await?;
        Self::__dispatch_saved(&row).await?;
        row.__touch_planned(&touch_plan).await?;
        crate::eloquent::changes::finish_save(row_state(row.__eager_cache()));
        Ok(row)
    }

    /// Insert this fully built row and fire the lifecycle events
    /// [`Self::create`] fires, in the same order. This is the path a
    /// factory takes, so an observer sees a factory's rows exactly as it
    /// sees the application's.
    ///
    /// The row's values are the ones its builder produced, so the
    /// fillable filter does not apply to them. A `Creating` or `Saving`
    /// listener sees the row as attributes; whatever it changes there is
    /// written over the built values, and a listener that cancels aborts
    /// the insert.
    ///
    /// `database_assigns_key` says whether the primary key is left for
    /// the database to assign, as an auto-increment key is, or written as
    /// the row carries it.
    ///
    /// **Not part of the public API.** It is `pub` because the
    /// `#[suprnova::model]` macro's `Persistable` impl calls it.
    #[doc(hidden)]
    async fn __insert_built(self, database_assigns_key: bool) -> Result<Self, FrameworkError> {
        use sea_orm::{ActiveModelTrait, Iterable, PrimaryKeyToColumn};

        let built = serde_json::to_value(&self).map_err(|e| {
            FrameworkError::internal(format!("factory insert: serialize the built row: {e}"))
        })?;
        let built = Attrs::from(built);
        let shared = std::sync::Arc::new(tokio::sync::Mutex::new(built.clone()));
        Self::__dispatch_creating(shared.clone()).await?;
        Self::__dispatch_saving(shared.clone(), true).await?;

        // Only what a listener changed goes over the built values. The
        // attributes are the row as it serializes, which leaves out a
        // hidden column and carries the placeholder key; writing them
        // all back would lose the one and set the other.
        let after = shared.lock().await.clone();
        let mut changed = Attrs::new();
        for (key, value) in after.iter() {
            if built.get(key) != Some(value) {
                changed.insert(key, value.clone());
            }
        }

        let touch_plan = Self::__plan_touches(Some(&self), &changed)?;
        let mut am = self.into_active_model_for_update()?;
        if database_assigns_key {
            for key in <<Self::Entity as EntityTrait>::PrimaryKey as Iterable>::iter() {
                am.not_set(key.into_column());
            }
        }
        Self::apply_attrs_to_active_model(&mut am, changed)?;

        let row =
            crate::render_cache::orm::atomic(Self::default_connection_name(), || async move {
                let exec = crate::database::transaction::ExecutorChoice::resolve_write(
                    None,
                    None,
                    Self::default_connection_name(),
                )
                .await?;
                let inserted = exec
                    .insert_active(am)
                    .await
                    .map_err(|e| FrameworkError::database(e.to_string()))?;
                let row = Self::try_from_storage(inserted)?;
                crate::render_cache::orm::after_model_write(&row).await?;
                Ok(row)
            })
            .await?;

        // Nothing was loaded before the insert: no original until it returns.
        crate::eloquent::changes::begin_insert(row_state(row.__eager_cache()));
        Self::__dispatch_created(&row).await?;
        Self::__dispatch_saved(&row).await?;
        row.__touch_planned(&touch_plan).await?;
        crate::eloquent::changes::finish_save(row_state(row.__eager_cache()));
        Ok(row)
    }

    /// Persist any field changes on this row. The full row is sent to
    /// the database - T4 doesn't track per-field dirty state.
    ///
    /// ## Lifecycle events (Phase 10C T1)
    ///
    /// 1. `Updating { previous, attrs }` - cancellable
    /// 2. `Saving { attrs, is_creating: false }` - cancellable
    /// 3. *UPDATE lands*
    /// 4. `Updated { previous, current }`
    /// 5. `Saved { model: current }`
    ///
    /// The `previous` snapshot is `self` at call time; `current` is
    /// the row as the database has it after the UPDATE. A listener
    /// that cancels at (1) or (2) aborts with
    /// `FrameworkError::bad_request(reason)`.
    async fn save(&self) -> Result<(), FrameworkError> {
        // Serialize the in-memory model to an Attrs map so listeners
        // see the "what's about to be written" payload through the
        // same Arc<Mutex<Attrs>> shape they see on create.
        let attrs_value = serde_json::to_value(self).map_err(|e| {
            FrameworkError::internal(format!("save: serialize self for Saving event: {e}"))
        })?;
        let attrs = Attrs::from(attrs_value);
        let shared = std::sync::Arc::new(tokio::sync::Mutex::new(attrs));

        Self::__dispatch_updating(self, shared.clone()).await?;
        Self::__dispatch_saving(shared.clone(), false).await?;

        // Audit HIGH `eloquent` #2 - read the (possibly listener-
        // mutated) attrs back from the shared map and overlay onto
        // the ActiveModel. The earlier code built the ActiveModel
        // straight from `self.clone()` and silently dropped any
        // listener mutations to the Updating / Saving payload.
        let final_attrs = shared.lock().await.clone();
        let touch_plan = Self::__plan_touches(Some(self), &final_attrs)?;
        let mut am = self.clone().into_active_model_for_update()?;
        Self::apply_attrs_to_active_model(&mut am, final_attrs)?;
        // T11/T12: route through resolve_write.
        let current =
            crate::render_cache::orm::atomic(Self::default_connection_name(), || async move {
                let exec = crate::database::transaction::ExecutorChoice::resolve_write(
                    None,
                    None,
                    Self::default_connection_name(),
                )
                .await?;
                let updated = exec
                    .update_active(am)
                    .await
                    .map_err(|e| FrameworkError::database(e.to_string()))?;
                let current = Self::try_from_storage(updated)?;
                crate::render_cache::orm::after_model_write(&current).await?;
                Ok(current)
            })
            .await?;

        record_save_on(
            self.__eager_cache(),
            current.__eager_cache(),
            true,
            Self::__decoded_values_equal,
        )?;
        Self::__dispatch_updated(self, &current).await?;
        Self::__dispatch_saved(&current).await?;
        current.__touch_planned(&touch_plan).await?;
        crate::eloquent::changes::finish_save(row_state(self.__eager_cache()));
        Ok(())
    }

    /// Apply a partial attribute set and persist. Attributes are
    /// filtered through [`Self::fillable_filter`] first.
    ///
    /// ## Lifecycle events (Phase 10C T1)
    ///
    /// Same event sequence as [`Self::save`] - `Updating` /
    /// `Saving { is_creating: false }` before the UPDATE, then
    /// `Updated` / `Saved` after.
    async fn update(self, attrs: Attrs) -> Result<Self, FrameworkError> {
        let previous = self.clone();
        let filtered = Self::fillable_filter().apply_checked(attrs)?;
        let shared = std::sync::Arc::new(tokio::sync::Mutex::new(filtered));

        Self::__dispatch_updating(&previous, shared.clone()).await?;
        Self::__dispatch_saving(shared.clone(), false).await?;

        let final_attrs = shared.lock().await.clone();
        let touch_plan = Self::__plan_touches(Some(&self), &final_attrs)?;
        let row = self.try_into_storage()?;
        let mut am = row.into_active_model();
        Self::apply_attrs_to_active_model(&mut am, final_attrs)?;
        // T11/T12: route through resolve_write.
        let current =
            crate::render_cache::orm::atomic(Self::default_connection_name(), || async move {
                let exec = crate::database::transaction::ExecutorChoice::resolve_write(
                    None,
                    None,
                    Self::default_connection_name(),
                )
                .await?;
                let updated = exec
                    .update_active(am)
                    .await
                    .map_err(|e| FrameworkError::database(e.to_string()))?;
                let current = Self::try_from_storage(updated)?;
                crate::render_cache::orm::after_model_write(&current).await?;
                Ok(current)
            })
            .await?;

        record_save_on(
            previous.__eager_cache(),
            current.__eager_cache(),
            false,
            Self::__decoded_values_equal,
        )?;
        Self::__dispatch_updated(&previous, &current).await?;
        Self::__dispatch_saved(&current).await?;
        current.__touch_planned(&touch_plan).await?;
        crate::eloquent::changes::finish_save(row_state(current.__eager_cache()));
        Ok(current)
    }

    /// Whether the last save that changed something changed `attribute`.
    /// Laravel's `$model->wasChanged('attribute')`.
    ///
    /// A column counts as changed when the value the database stores
    /// after the save differs from the value the instance held before it.
    /// Writing the same value back is no change; a column a `Saving`
    /// listener or the timestamps rewrote is one. A column whose cast
    /// stores a new value on every write, such as an encrypted one, is
    /// compared by its decoded value, so it counts as changed only when
    /// that value changed. A save that changed
    /// nothing leaves the previous save's record in place, as Laravel's
    /// does. `false` before any save that changed something, after an
    /// insert, and for an attribute the model does not have.
    ///
    /// [`Self::save`], [`Self::update`], [`Self::save_with_tx`] and
    /// [`Self::update_with_tx`] record the change on the model the caller
    /// holds afterwards and on the model the `updated` and `saved`
    /// observers receive, which can still read the value from before the
    /// save, so an observer can audit a change:
    ///
    /// ```ignore
    /// async fn updated(&self, _previous: &User, user: &User) -> Result<(), FrameworkError> {
    ///     if user.was_changed("is_admin") {
    ///         let before = user.get_raw_original("is_admin");
    ///         tracing::info!(user_id = user.id, ?before, after = user.is_admin, "admin flag changed");
    ///     }
    ///     Ok(())
    /// }
    /// ```
    fn was_changed(&self, attribute: &str) -> bool {
        crate::eloquent::changes::was_changed_any(row_state(self.__eager_cache()), &[attribute])
    }

    /// Whether the last save changed any of `attributes`. Laravel's
    /// `$model->wasChanged([...])`.
    ///
    /// An empty slice asks whether the save changed anything at all, which
    /// is Laravel's `wasChanged()` with no argument. See
    /// [`Self::was_changed`] for what counts as a change.
    fn was_changed_any(&self, attributes: &[&str]) -> bool {
        crate::eloquent::changes::was_changed_any(row_state(self.__eager_cache()), attributes)
    }

    /// The attributes the last save that changed something changed, each
    /// with the value it stored. Laravel's `$model->getChanges()`.
    ///
    /// The values are in stored form, before casts, as Laravel's are: an
    /// `AsBool` column reads `1`, not `true`. A later save that changes
    /// something replaces the record; a save that changed nothing, or that
    /// failed, leaves it in place. Empty before any save that changed
    /// something, and after an insert.
    fn get_changes(&self) -> Attrs {
        crate::eloquent::changes::changes(row_state(self.__eager_cache()))
    }

    /// The original value of `attribute`, read through the model's casts,
    /// as [`Self::field_value`] reads the current one. Laravel's
    /// `$model->getOriginal('attribute')`.
    ///
    /// The original is the row as the instance last read or saved it:
    /// changing a field in memory does not change it. While a save's
    /// `updated` and `saved` observers run, it is still the row the save
    /// started from, so an observer reads the value before the save; once
    /// the save returns, it is the saved row, as Laravel's `finishSave`
    /// syncs it. `Ok(None)` when the instance was never read from the
    /// database, such as a model built with `Default` and not saved yet,
    /// inside the `created` and `saved` observers of an insert, which had
    /// nothing loaded before it, and for an attribute the model does not
    /// have.
    ///
    /// # Errors
    ///
    /// When the kept value no longer decodes through the model's cast, for
    /// example an encrypted column whose key has left the key ring.
    fn get_original(&self, attribute: &str) -> Result<Option<serde_json::Value>, FrameworkError> {
        let Some(row) = crate::eloquent::changes::original_row(row_state(self.__eager_cache()))
        else {
            return Ok(None);
        };
        match Self::__model_from_stored_row(row.as_any()) {
            Some(original) => Ok(original?.field_value(attribute)),
            None => Err(FrameworkError::internal(format!(
                "get_original: the row kept for `{}` is not that model's row",
                std::any::type_name::<Self>(),
            ))),
        }
    }

    /// The original value of `attribute`, as stored: no cast applied.
    /// Laravel's `$model->getRawOriginal('attribute')`.
    ///
    /// The same value as [`Self::get_original`] in the form the column
    /// holds it, so an `AsBool` column reads `0` or `1` and an encrypted
    /// column reads its ciphertext. `None` when the instance was never read
    /// from the database, inside the `created` and `saved` observers of an
    /// insert, and for an attribute the model does not have.
    fn get_raw_original(&self, attribute: &str) -> Option<serde_json::Value> {
        crate::eloquent::changes::raw_original(row_state(self.__eager_cache()), attribute)
    }

    /// Delete this row. The trait default performs a hard DELETE.
    /// Models annotated `#[suprnova::model(soft_deletes)]` get an
    /// inherent override that flips this to an UPDATE SET deleted_at
    /// (see `suprnova-macros/src/model/derive_eloquent.rs`).
    ///
    /// ## Lifecycle events (Phase 10C T1)
    ///
    /// 1. `Deleting { model, is_force: false }` - cancellable
    /// 2. *DELETE lands*
    /// 3. `Deleted { model, is_force: false }`
    ///
    /// Soft-delete models override the inherent `delete` to also
    /// dispatch `Trashed { model }` after step 2.
    async fn delete(self) -> Result<(), FrameworkError> {
        Self::__dispatch_deleting(&self, false).await?;
        let touch_plan = Self::__plan_touches(Some(&self), &Attrs::new())?;

        let snapshot = self.clone();
        let row = self.try_into_storage()?;
        let am = row.into_active_model();
        // T11/T12: route through resolve_write.
        crate::render_cache::orm::atomic(Self::default_connection_name(), || async {
            let exec = crate::database::transaction::ExecutorChoice::resolve_write(
                None,
                None,
                Self::default_connection_name(),
            )
            .await?;
            exec.delete_active(am)
                .await
                .map_err(|e| FrameworkError::database(e.to_string()))?;
            crate::render_cache::orm::after_model_write(&snapshot).await
        })
        .await?;

        Self::__dispatch_deleted(&snapshot, false).await?;
        snapshot.__touch_planned(&touch_plan).await?;
        Ok(())
    }

    /// Hard-delete this row, bypassing any soft-delete override. For
    /// non-soft-delete models this is identical to `delete`. Models
    /// annotated `#[suprnova::model(soft_deletes)]` get an inherent
    /// override that ALSO fires `ForceDeleting` / `ForceDeleted` and
    /// `Deleting { is_force: true }` / `Deleted { is_force: true }`
    /// (Trashed is NOT fired - the row is gone, not tombstoned).
    async fn force_delete(self) -> Result<(), FrameworkError> {
        Self::__dispatch_deleting(&self, true).await?;
        Self::__dispatch_force_deleting(&self).await?;
        let touch_plan = Self::__plan_touches(Some(&self), &Attrs::new())?;

        let snapshot = self.clone();
        let row = self.try_into_storage()?;
        let am = row.into_active_model();
        // T11/T12: route through resolve_write.
        crate::render_cache::orm::atomic(Self::default_connection_name(), || async {
            let exec = crate::database::transaction::ExecutorChoice::resolve_write(
                None,
                None,
                Self::default_connection_name(),
            )
            .await?;
            exec.delete_active(am)
                .await
                .map_err(|e| FrameworkError::database(e.to_string()))?;
            crate::render_cache::orm::after_model_write(&snapshot).await
        })
        .await?;

        Self::__dispatch_force_deleted(&snapshot).await?;
        Self::__dispatch_deleted(&snapshot, true).await?;
        snapshot.__touch_planned(&touch_plan).await?;
        Ok(())
    }

    /// Bump `updated_at` on every parent named by
    /// `#[model(touches = [...])]`. Laravel's `Model::touchOwners()`.
    ///
    /// Called automatically after a successful `create`, `save`,
    /// `update`, `delete`, and `force_delete`; call it by hand only
    /// when you wrote the child row through a path the framework
    /// doesn't own.
    ///
    /// Routes through
    /// [`ExecutorChoice::resolve_write`](crate::database::transaction::ExecutorChoice::resolve_write),
    /// so inside a `DB::transaction` closure the touch joins the
    /// caller's transaction and a rollback reverts it.
    async fn touch_owners(&self) -> Result<(), FrameworkError> {
        let plan = Self::__plan_touches(Some(self), &Attrs::new())?;
        self.__touch_planned(&plan).await
    }

    /// [`Self::touch_owners`] pinned to an explicit transaction handle.
    /// The `*_with_tx` shims bypass the `CURRENT_TX` task-local by
    /// design, so they can't rely on picking the transaction up
    /// ambiently. This is also the only `*_with_tx` function in the
    /// framework that executes SQL and, before fix2, advanced no
    /// generation for it - the same defect shape ruling R47 fixed for the
    /// model `_with_tx` shims, and the same fix: `__touch_owners_via`
    /// takes the handle explicitly and routes each touch's advance
    /// through it, rather than through the ambient task-local it
    /// deliberately bypasses.
    async fn touch_owners_with_tx(
        &self,
        tx: &crate::database::Transaction,
    ) -> Result<(), FrameworkError> {
        let plan = Self::__plan_touches(Some(self), &Attrs::new())?;
        self.__touch_planned_with_tx(tx, &plan).await
    }

    /// Resolve the owners that a write of a row will touch, after the
    /// pre-write listeners (`Creating`, `Saving`, `Updating`,
    /// `Deleting`) have run and before the statement runs. Every name in
    /// `#[model(touches = [...])]` is resolved here, so one name that
    /// cannot be resolved stops the write before its statement runs,
    /// before a post-write event (`Created`, `Saved`, `Updated`,
    /// `Deleted`) is dispatched, and before any owner is touched. The
    /// pre-write listeners have run by then, as they have for a write
    /// that a listener cancels.
    ///
    /// Only a `MorphTo` name can fail: its owner model is chosen by the
    /// row's `<name>_type` column, and a value that names none of the
    /// relation's targets is an error. A `BelongsTo` owner is read from
    /// the row after the write.
    ///
    /// The values come from `attrs` first and from `base` for a column
    /// that `attrs` does not carry, which is the row as the statement
    /// will write it: `create` passes the final attributes and no row,
    /// `save`, `update` and the factory insert pass the row and the
    /// attributes the listeners left, and a delete passes the row alone.
    /// A listener that rewrites a `<name>_type` or `<name>_id` column is
    /// therefore followed.
    ///
    /// Empty when `TOUCHES` is empty or touching is disabled for the
    /// running task.
    ///
    /// **Not part of the public API.** It is `pub` because the macro's
    /// soft-delete `delete` and `force_delete` call it.
    #[doc(hidden)]
    fn __plan_touches(base: Option<&Self>, attrs: &Attrs) -> Result<TouchPlan, FrameworkError> {
        let mut plan = TouchPlan::default();
        if Self::TOUCHES.is_empty() || crate::eloquent::touches_disabled() {
            return Ok(plan);
        }
        for relation in Self::TOUCHES {
            let Some(entry) = crate::eloquent::find_relation::<Self>(relation) else {
                return Err(unregistered_touch(Self::TABLE, relation));
            };
            if entry.kind == crate::eloquent::RelationKind::MorphTo {
                let owner = Self::__morph_owner(relation, base, attrs)?;
                plan.morph_owners.push((*relation, owner));
            }
        }
        Ok(plan)
    }

    /// Touch the owners of this row, `plan` being what
    /// [`Self::__plan_touches`] resolved before the write. The row is
    /// the one the write left, so a `BelongsTo` owner is read from it
    /// and a `MorphTo` owner is taken from `plan`, not resolved again.
    ///
    /// **Not part of the public API.** It is `pub` because the macro's
    /// soft-delete `delete` and `force_delete` call it.
    #[doc(hidden)]
    async fn __touch_planned(&self, plan: &TouchPlan) -> Result<(), FrameworkError> {
        if Self::TOUCHES.is_empty() {
            return Ok(());
        }
        crate::render_cache::orm::atomic(Self::default_connection_name(), || async {
            let exec = crate::database::transaction::ExecutorChoice::resolve_write(
                None,
                None,
                Self::default_connection_name(),
            )
            .await?;
            self.__touch_owners_via(&exec, None, plan).await
        })
        .await
    }

    /// [`Self::__touch_planned`] pinned to an explicit transaction
    /// handle, as [`Self::touch_owners_with_tx`] is for
    /// [`Self::touch_owners`].
    ///
    /// **Not part of the public API.**
    #[doc(hidden)]
    async fn __touch_planned_with_tx(
        &self,
        tx: &crate::database::Transaction,
        plan: &TouchPlan,
    ) -> Result<(), FrameworkError> {
        if Self::TOUCHES.is_empty() {
            return Ok(());
        }
        let exec = crate::database::transaction::ExecutorChoice::from_tx(tx);
        self.__touch_owners_via(&exec, Some(&tx.handle()), plan)
            .await
    }

    /// The parent-touch cascade itself: one
    /// `UPDATE <owner> SET <updated_at> = ? WHERE <key> = ?` per
    /// declared touch relation.
    ///
    /// Type-erased on purpose - the owner is reached through its
    /// [`RelationEntry`](crate::eloquent::RelationEntry), never
    /// hydrated. That makes the cascade one level deep: Laravel
    /// recurses to grandparents by loading the parent model, and we
    /// trade that for not issuing a SELECT per touch.
    ///
    /// A `BelongsTo` owner's table and columns are in its entry. A
    /// `MorphTo` owner's model varies from row to row, so it is found
    /// from the row's `<name>_type` and `<name>_id` columns through the
    /// morph registry by [`Self::__morph_owner`], before the statement
    /// runs, and arrives here in `plan`. A null `<name>_id` touches nothing.
    ///
    /// `#[model(touches = [...])]` exists precisely to bust a parent's
    /// cached representation when a child write should invalidate it too;
    /// each successful touch here advances the target table's and the
    /// touched row's generations (`render_cache::orm::after_row_write`),
    /// using the same foreign-key value already resolved for the `SET`
    /// binding as the record identity's key - no second lookup needed.
    /// `tx_handle` is `Some` only from [`Self::touch_owners_with_tx`],
    /// whose ambient-task-local bypass means the advance must go through
    /// the same explicit handle the row write did; see that method's
    /// documentation.
    ///
    /// # Security
    ///
    /// Every interpolated identifier comes from a macro-emitted
    /// `&'static str`, never from request data. They are still run
    /// through [`crate::database::validate_identifier`] before
    /// rendering, on the same principle as [`Self::increment`]: the
    /// SQL-identifier trust boundary is checked at the render site,
    /// not assumed upstream.
    #[doc(hidden)]
    async fn __touch_owners_via(
        &self,
        exec: &crate::database::transaction::ExecutorChoice,
        tx_handle: Option<&crate::database::transaction::TxHandle>,
        plan: &TouchPlan,
    ) -> Result<(), FrameworkError> {
        if Self::TOUCHES.is_empty() || crate::eloquent::touches_disabled() {
            return Ok(());
        }
        let now = crate::clock::now();

        for relation in Self::TOUCHES {
            let Some(entry) = crate::eloquent::find_relation::<Self>(relation) else {
                // The macro rejects this at expansion time; reaching it
                // means the registry and the const disagree.
                return Err(unregistered_touch(Self::TABLE, relation));
            };
            if entry.kind == crate::eloquent::RelationKind::MorphTo {
                // No owner in the plan: the row's `<name>_id` column is
                // null, so there is no owner to identify.
                let Some(owner) = plan.owner(relation) else {
                    continue;
                };
                // As for a `BelongsTo` owner, one whose model disclaims
                // timestamps is skipped: not an error, not a write.
                if owner.updated_at_column.is_empty()
                    || crate::eloquent::touches_ignored_for(owner.type_id)
                {
                    continue;
                }
                touch_owner_row(
                    exec,
                    tx_handle,
                    OwnerRow {
                        table: owner.table,
                        updated_at: (owner.updated_at_storage)(&now)?,
                        updated_at_column: owner.updated_at_column,
                        key_column: owner.key_column,
                        soft_deletes_column: owner.soft_deletes_column,
                        key: &owner.key,
                    },
                )
                .await?;
                continue;
            }
            // laravel/framework#61073 - an owner whose model disclaims
            // timestamps is skipped. Not an error, not a write.
            if entry.related_updated_at_column.is_empty() {
                continue;
            }
            if crate::eloquent::touches_ignored_for((entry.target_type)()) {
                continue;
            }
            // No FK value on this row means no owner to identify.
            let Some(key) = self.field_value(entry.foreign_key) else {
                continue;
            };
            if key.is_null() {
                continue;
            }
            touch_owner_row(
                exec,
                tx_handle,
                OwnerRow {
                    table: entry.target_table,
                    updated_at: (entry.related_updated_at_storage)(&now)?,
                    updated_at_column: entry.related_updated_at_column,
                    key_column: entry.parent_key,
                    soft_deletes_column: entry.related_soft_deletes_column,
                    key: &key,
                },
            )
            .await?;
        }
        Ok(())
    }

    /// The owner row the `MorphTo` relation named `relation` points at,
    /// for the parent-touch cascade of `#[model(touches = [...])]`.
    /// `None` when the relation's `<name>_id` value is null. A
    /// `<name>_type` value that names none of the relation's targets is
    /// an error, and it is raised by [`Self::__plan_touches`] before the
    /// statement runs, so nothing is written for such a row.
    ///
    /// `attrs` is read first and `base` for a column `attrs` does not
    /// carry, as [`Self::__plan_touches`] describes.
    ///
    /// The `#[suprnova::model]` macro overrides this for a model that
    /// declares `MorphTo` relations, choosing the target through the
    /// relation's fetch helper, as `.get()` and the eager loader do. The
    /// default is for a model without one and reports the name as
    /// unknown.
    ///
    /// **Not part of the public API.** It is `pub` because the macro
    /// emits the override.
    #[doc(hidden)]
    fn __morph_owner(
        relation: &str,
        base: Option<&Self>,
        attrs: &Attrs,
    ) -> Result<Option<crate::eloquent::relations::morph::MorphOwner>, FrameworkError> {
        let _ = (base, attrs);
        Err(FrameworkError::internal(format!(
            "model `{}` has no MorphTo relation `{relation}`",
            std::any::type_name::<Self>(),
        )))
    }

    // ---- Phase 10C T11 - manual-transaction shims --------------------
    //
    // `DB::begin_transaction()` returns a `Transaction` handle and
    // does NOT install the [`CURRENT_TX`] task-local; callers must
    // opt every operation into the transaction explicitly. These
    // `_with_tx` methods are the per-Model entry points; pair them
    // with `Builder::with_tx(&tx)` for read paths.
    //
    // The shims route through `ExecutorChoice::from_tx(tx)` which
    // bypasses CURRENT_TX consultation entirely - the explicit
    // handle is authoritative. Lifecycle events still fire in the
    // same order as the non-tx variant.

    /// Persist this row's in-memory state through `tx`. Same lifecycle
    /// event sequence as [`Self::save`] (`Updating` → `Saving` →
    /// UPDATE → `Updated` → `Saved`). Used with
    /// [`DB::begin_transaction`](crate::DB::begin_transaction) when the
    /// closure form doesn't fit the caller's control flow.
    async fn save_with_tx(&self, tx: &crate::database::Transaction) -> Result<(), FrameworkError> {
        let attrs_value = serde_json::to_value(self).map_err(|e| {
            FrameworkError::internal(format!(
                "save_with_tx: serialize self for Saving event: {e}"
            ))
        })?;
        let attrs = Attrs::from(attrs_value);
        let shared = std::sync::Arc::new(tokio::sync::Mutex::new(attrs));

        Self::__dispatch_updating(self, shared.clone()).await?;
        Self::__dispatch_saving(shared.clone(), false).await?;

        // Audit HIGH `eloquent` #2 - match `save()`'s lifecycle: read
        // the listener-mutated attrs back and apply them to the
        // ActiveModel before the UPDATE fires.
        let final_attrs = shared.lock().await.clone();
        let touch_plan = Self::__plan_touches(Some(self), &final_attrs)?;
        let mut am = self.clone().into_active_model_for_update()?;
        Self::apply_attrs_to_active_model(&mut am, final_attrs)?;
        let exec = crate::database::transaction::ExecutorChoice::from_tx(tx);
        let updated = exec
            .update_active(am)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        let current = Self::try_from_storage(updated)?;
        crate::render_cache::orm::after_model_write_with_tx(tx, &current).await?;

        record_save_on(
            self.__eager_cache(),
            current.__eager_cache(),
            true,
            Self::__decoded_values_equal,
        )?;
        Self::__dispatch_updated(self, &current).await?;
        Self::__dispatch_saved(&current).await?;
        current.__touch_planned_with_tx(tx, &touch_plan).await?;
        crate::eloquent::changes::finish_save(row_state(self.__eager_cache()));
        Ok(())
    }

    /// Apply `attrs` to this row through `tx`. Mirrors
    /// [`Self::update`] event-for-event but pins the SQL to the
    /// supplied transaction. Returns the updated row.
    async fn update_with_tx(
        self,
        tx: &crate::database::Transaction,
        attrs: Attrs,
    ) -> Result<Self, FrameworkError> {
        let previous = self.clone();
        let filtered = Self::fillable_filter().apply_checked(attrs)?;
        let shared = std::sync::Arc::new(tokio::sync::Mutex::new(filtered));

        Self::__dispatch_updating(&previous, shared.clone()).await?;
        Self::__dispatch_saving(shared.clone(), false).await?;

        let final_attrs = shared.lock().await.clone();
        let touch_plan = Self::__plan_touches(Some(&self), &final_attrs)?;
        let row = self.try_into_storage()?;
        let mut am = row.into_active_model();
        Self::apply_attrs_to_active_model(&mut am, final_attrs)?;
        let exec = crate::database::transaction::ExecutorChoice::from_tx(tx);
        let updated = exec
            .update_active(am)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        let current = Self::try_from_storage(updated)?;
        crate::render_cache::orm::after_model_write_with_tx(tx, &current).await?;

        record_save_on(
            previous.__eager_cache(),
            current.__eager_cache(),
            false,
            Self::__decoded_values_equal,
        )?;
        Self::__dispatch_updated(&previous, &current).await?;
        Self::__dispatch_saved(&current).await?;
        current.__touch_planned_with_tx(tx, &touch_plan).await?;
        crate::eloquent::changes::finish_save(row_state(current.__eager_cache()));
        Ok(current)
    }

    /// Delete this row through `tx`. Soft-delete models override the
    /// inherent `delete` to apply tombstone semantics on the trait
    /// path, but this trait-level shim performs a hard DELETE
    /// regardless. Use [`Self::force_delete_with_tx`] for symmetry
    /// when you want the operation to read as "definitely remove" at
    /// the call site.
    async fn delete_with_tx(self, tx: &crate::database::Transaction) -> Result<(), FrameworkError> {
        Self::__dispatch_deleting(&self, false).await?;
        let touch_plan = Self::__plan_touches(Some(&self), &Attrs::new())?;

        let snapshot = self.clone();
        let row = self.try_into_storage()?;
        let am = row.into_active_model();
        let exec = crate::database::transaction::ExecutorChoice::from_tx(tx);
        exec.delete_active(am)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        crate::render_cache::orm::after_model_write_with_tx(tx, &snapshot).await?;

        Self::__dispatch_deleted(&snapshot, false).await?;
        snapshot.__touch_planned_with_tx(tx, &touch_plan).await?;
        Ok(())
    }

    /// Create a row through `tx`. Phase 10C audit-fix AF5 closes the
    /// manual-transaction shim inventory - every CRUD entry point on
    /// [`Self`] except `create` previously had a `*_with_tx`
    /// counterpart, so a user inside [`DB::begin_transaction`](crate::database::DB::begin_transaction) who
    /// wanted to `create` had to fall back to building an
    /// `ActiveModel` by hand and reaching for raw SeaORM. This shim
    /// mirrors [`Self::create`] event-for-event (`Creating` → `Saving`
    /// → INSERT → `Created` → `Saved`) but pins the INSERT to `tx`.
    async fn create_with_tx(
        tx: &crate::database::Transaction,
        attrs: Attrs,
    ) -> Result<Self, FrameworkError> {
        let filtered = Self::fillable_filter().apply_checked(attrs)?;
        let shared = std::sync::Arc::new(tokio::sync::Mutex::new(filtered));
        Self::__dispatch_creating(shared.clone()).await?;
        Self::__dispatch_saving(shared.clone(), true).await?;

        let final_attrs = shared.lock().await.clone();
        let touch_plan = Self::__plan_touches(None, &final_attrs)?;
        let am = Self::active_model_from_attrs(final_attrs)?;
        let exec = crate::database::transaction::ExecutorChoice::from_tx(tx);
        let inserted = exec
            .insert_active(am)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        let row = Self::try_from_storage(inserted)?;
        crate::render_cache::orm::after_model_write_with_tx(tx, &row).await?;

        // Nothing was loaded before the insert: no original until it returns.
        crate::eloquent::changes::begin_insert(row_state(row.__eager_cache()));
        Self::__dispatch_created(&row).await?;
        Self::__dispatch_saved(&row).await?;
        row.__touch_planned_with_tx(tx, &touch_plan).await?;
        crate::eloquent::changes::finish_save(row_state(row.__eager_cache()));
        Ok(row)
    }

    /// Force-delete this row through `tx`. Mirrors
    /// [`Self::force_delete`] event-for-event but pins the DELETE to
    /// the supplied transaction.
    async fn force_delete_with_tx(
        self,
        tx: &crate::database::Transaction,
    ) -> Result<(), FrameworkError> {
        Self::__dispatch_deleting(&self, true).await?;
        Self::__dispatch_force_deleting(&self).await?;
        let touch_plan = Self::__plan_touches(Some(&self), &Attrs::new())?;

        let snapshot = self.clone();
        let row = self.try_into_storage()?;
        let am = row.into_active_model();
        let exec = crate::database::transaction::ExecutorChoice::from_tx(tx);
        exec.delete_active(am)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        crate::render_cache::orm::after_model_write_with_tx(tx, &snapshot).await?;

        Self::__dispatch_force_deleted(&snapshot).await?;
        Self::__dispatch_deleted(&snapshot, true).await?;
        snapshot.__touch_planned_with_tx(tx, &touch_plan).await?;
        Ok(())
    }

    /// Reload this row from the database, mutating self in place. The
    /// PK is preserved; every other column reflects the latest state.
    async fn refresh(&mut self) -> Result<(), FrameworkError> {
        let pk = self.primary_key_value();
        let fresh = Self::find(pk)
            .await?
            .ok_or_else(|| FrameworkError::not_found("refresh: row no longer exists"))?;
        *self = fresh;
        Ok(())
    }

    /// Reload this row from the database under an exclusive row lock,
    /// mutating self in place. The locked read and the reload are one
    /// statement, so there is no window between "I have the current
    /// values" and "I hold the lock".
    ///
    /// Mirrors Laravel's `refreshForUpdate()` -> `newQueryWithoutScopes()`.
    /// The reload runs on a fresh, unscoped [`Builder`], never through
    /// [`Self::query`]: every registered global scope AND the
    /// `#[model(soft_deletes)]` filter are bypassed, the same way the
    /// macro-emitted `with_trashed()` / `only_trashed()` build their
    /// own unscoped builder rather than undoing `query()`'s scope. A
    /// trashed row reloads too, with `deleted_at` coming back set -
    /// this is a lookup by primary key under a lock, and scoping a
    /// by-key reload is exactly what would hand admin tooling and
    /// cross-tenant callers a false not-found. A row that no longer
    /// exists returns [`FrameworkError::not_found`] rather than
    /// leaving `self` stale.
    ///
    /// The lock only holds for the length of a transaction - call this
    /// inside `DB::transaction(...)`, otherwise the lock releases the
    /// moment the statement finishes and buys you nothing.
    ///
    /// **SQLite emits no lock clause.** SQLite has no row-level
    /// locking, so the reload happens without one and the framework
    /// logs a single `warn!` per process on the
    /// `suprnova::eloquent::lock` target. The method is kept on that
    /// backend so cross-backend code compiles unchanged.
    ///
    /// The two extra bounds are the ones
    /// [`Builder::first`](crate::eloquent::Builder::first) needs and
    /// this trait's own where clause does not carry - the same gap
    /// [`FirstOrCreate`] documents. Every `#[suprnova::model]` struct
    /// satisfies them, so call sites never name them.
    async fn refresh_for_update(&mut self) -> Result<(), FrameworkError>
    where
        Self: crate::eloquent::EagerLoadDispatch,
        <Self::Entity as EntityTrait>::Model: sea_orm::FromQueryResult,
    {
        let fresh = Builder::<Self>::new()
            .where_key(self.primary_key_value_json())
            .lock_for_update()
            .first()
            .await?
            .ok_or_else(|| FrameworkError::not_found("refresh_for_update: row no longer exists"))?;
        *self = fresh;
        Ok(())
    }

    /// Return a freshly-fetched copy of this row without mutating
    /// `self`. `None` if the row was deleted in the interim.
    async fn fresh(&self) -> Result<Option<Self>, FrameworkError> {
        Self::find(self.primary_key_value()).await
    }

    /// Eager-load the named relations onto this row after the fact.
    /// Laravel's `$model->load(...)`.
    ///
    /// Runs the loader of
    /// [`Collection::load`](crate::eloquent::Collection::load) with this
    /// row as a one-row slice, so the relations land in this row's own
    /// relation cache, and a dotted name (`"comments.author"`), the
    /// eager-load dispatcher and the connection routing behave exactly
    /// as they do for a collection. Read the result with the
    /// macro-emitted `<relation>_loaded()` accessors, which do not query
    /// again. Inside a `DB::transaction` closure the reads go through the
    /// transaction.
    ///
    /// A relation that is already loaded is loaded again, as in Laravel;
    /// [`Self::load_missing`] skips it instead.
    ///
    /// A trait method rather than a macro-emitted inherent one, so code
    /// that holds a model through an `M: Model` bound can call it.
    ///
    /// ## Example
    ///
    /// ```ignore
    /// let mut post = Post::find_or_fail(id).await?;
    /// post.load(["comments.author"]).await?;
    /// println!("{} comments", post.comments_loaded().len());
    /// ```
    async fn load<I, S>(&mut self, relations: I) -> Result<(), FrameworkError>
    where
        Self: crate::eloquent::EagerLoadDispatch,
        I: IntoIterator<Item = S> + Send,
        S: Into<String> + Send,
    {
        crate::eloquent::collection::load_relations::<Self, I, S>(
            std::slice::from_mut(self),
            relations,
        )
        .await
    }

    /// Eager-load the named relations onto this row, skipping any this
    /// row already has loaded. Laravel's `$model->loadMissing(...)`.
    ///
    /// Runs the loader of
    /// [`Collection::load_missing`](crate::eloquent::Collection::load_missing)
    /// with this row as a one-row slice. A relation already in this
    /// row's cache runs no query. A dotted name is checked at every
    /// level: with `comments` loaded, `load_missing(["comments.author"])`
    /// loads only the authors the cached comments lack.
    ///
    /// ## Example
    ///
    /// ```ignore
    /// let mut post = Post::find_or_fail(id).await?;
    /// post.load(["comments"]).await?;
    /// post.load_missing(["comments", "tags"]).await?; // queries `tags` only
    /// ```
    async fn load_missing<I, S>(&mut self, relations: I) -> Result<(), FrameworkError>
    where
        Self: crate::eloquent::EagerLoadDispatch,
        I: IntoIterator<Item = S> + Send,
        S: Into<String> + Send,
    {
        crate::eloquent::collection::load_missing_relations::<Self, I, S>(
            std::slice::from_mut(self),
            relations,
        )
        .await
    }

    /// Build an unsaved clone with the PK reset and any auto-managed
    /// columns cleared. Caller saves explicitly.
    ///
    /// ## Relation state
    ///
    /// Eager-loaded relations and pivot context are preserved on the
    /// replica (Laravel parity: `clone $user` retains `$user->posts`).
    /// The macro-emitted `replicate_with` copies the relations of the
    /// source's `__eager` cache - each cell carries a clone trampoline
    /// so the replica's loaded rows are independent of the source's -
    /// and `Arc`-clones the pivot slot. The copy leaves out the mark of
    /// a multi-row query: the replica is built in the process, and
    /// [lazy-loading prevention](crate::eloquent::lazy_loading) never
    /// refuses a relation read on such a model.
    /// Use [`Self::replicate_except`] if a specific relation should
    /// be dropped on the replica (column names only; relation cache
    /// keys are out of scope for the `except` filter).
    ///
    /// ## Lifecycle events
    ///
    /// Fires `Replicating { source, replica }` AFTER the in-memory
    /// clone is constructed and BEFORE this method returns. The
    /// `replica` field is an `Arc<tokio::sync::Mutex<Self>>` so
    /// listeners can mutate the replica (clear timestamps, reset
    /// flags, append a `(copy)` prefix to the title, etc.) before
    /// the caller sees it.
    async fn replicate(&self) -> Result<Self, FrameworkError>
    where
        Self: ReplicateExt,
    {
        // ReplicateExt::replicate_with takes Vec<String>; match the
        // element type. `Vec::<&str>::new()` would compile-error here
        // even though the vec is empty.
        let copy = self.replicate_with(Vec::<String>::new());
        let shared = std::sync::Arc::new(tokio::sync::Mutex::new(copy));
        Self::__dispatch_replicating(self, shared.clone()).await?;
        Ok(shared.lock().await.clone())
    }

    /// Like [`Self::replicate`] but also clears every column whose
    /// name appears in `except`.
    ///
    /// Fires `Replicating` with the same `Arc<Mutex<Self>>` contract
    /// as [`Self::replicate`].
    async fn replicate_except<I, S>(&self, except: I) -> Result<Self, FrameworkError>
    where
        I: IntoIterator<Item = S> + Send,
        S: AsRef<str> + Send,
        Self: ReplicateExt,
    {
        let copy = self.replicate_with(
            except
                .into_iter()
                .map(|s| s.as_ref().to_string())
                .collect::<Vec<_>>(),
        );
        let shared = std::sync::Arc::new(tokio::sync::Mutex::new(copy));
        Self::__dispatch_replicating(self, shared.clone()).await?;
        Ok(shared.lock().await.clone())
    }

    /// Replicate this row into a different model type. Suprnova
    /// divergence from Laravel - Laravel can't do this because PHP
    /// has no static types. The transfer goes via JSON: `self` is
    /// serialised to a `serde_json::Value`, then deserialised into
    /// `T`. After deserialisation, the target's PK is reset to its
    /// `Default::default()` so the replica is genuinely unsaved.
    ///
    /// ## Field-shape contract
    ///
    /// `T` must accept every field `Self` serialises. Concretely:
    /// fields present on `T` but absent from `Self` must be
    /// `Option<_>` or annotated `#[serde(default)]`; otherwise serde
    /// will fail the round-trip with a "missing field" error. Fields
    /// present on `Self` but absent from `T` are silently dropped.
    /// For the same-shape case (e.g. `User` -> `UserDraft` where both
    /// carry the same columns), no extra annotations are needed.
    ///
    /// ## No `Replicating` event for cross-type replication
    ///
    /// `Replicating` is per-source-type (the event struct holds an
    /// `Arc<Mutex<Self>>`). For cross-type replication the source's
    /// `Replicating` listener would receive an `Arc<Mutex<Self>>`,
    /// not `Arc<Mutex<T>>` - which can't mutate the cross-type
    /// replica that's about to be returned. We deliberately skip the
    /// dispatch: callers wanting per-T setup should run it on the
    /// returned `T` value before calling `T::save`. Inside `T::save`
    /// the normal `Saving` / `Created` chain still fires.
    async fn replicate_into<T>(&self) -> Result<T, FrameworkError>
    where
        T: Model + DeserializeOwned + Serialize,
        T: From<<T::Entity as EntityTrait>::Model>,
        <T::Entity as EntityTrait>::Model: From<T>
            + IntoActiveModel<<T::Entity as EntityTrait>::ActiveModel>
            + Serialize
            + Send
            + Sync,
        <T::Entity as EntityTrait>::ActiveModel: Send,
        <<T::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
            Send + Into<sea_orm::Value>,
    {
        let json = serde_json::to_value(self)
            .map_err(|e| FrameworkError::internal(format!("replicate_into serialize: {e}")))?;
        // The PK from `self` will land in `T`'s same-named PK field
        // during deserialisation. We don't strip it from the JSON
        // (that would break round-tripping when `T`'s PK field isn't
        // `Option`/`#[serde(default)]`); instead, we reset the PK on
        // the replica immediately after deserialisation so the result
        // is genuinely unsaved.
        let mut replica: T = serde_json::from_value(json)
            .map_err(|e| FrameworkError::internal(format!("replicate_into deserialize: {e}")))?;
        replica.reset_primary_key();
        Ok(replica)
    }

    /// Atomic `UPDATE table SET col = col + by WHERE pk = ?`. Safe
    /// against concurrent updates - no read-modify-write race.
    ///
    /// # Security
    ///
    /// `column` is interpolated as a SQL identifier (not a bound
    /// parameter - SQL doesn't allow that). The call validates
    /// `column` via [`crate::database::validate_identifier`] before
    /// rendering, so attacker-controlled strings are rejected at the
    /// I/O boundary with [`FrameworkError`]. Same contract as
    /// Laravel's `Model::increment($column, $by)`.
    async fn increment(&self, column: &str, by: i64) -> Result<(), FrameworkError> {
        step_column(self, column, "+", by).await
    }

    /// Atomic `UPDATE table SET col = col - by WHERE pk = ?`. The
    /// subtraction is written into the SQL rather than `-by` added, so
    /// every `i64` amount works, `i64::MIN` included. Same identifier
    /// validation as [`Self::increment`].
    async fn decrement(&self, column: &str, by: i64) -> Result<(), FrameworkError> {
        step_column(self, column, "-", by).await
    }

    // ---- Static destroy / is / is_not (Laravel parity) -----------------

    /// Static mass-delete by primary key set. Laravel's
    /// `Model::destroy([1,2,3])` analogue. Returns the count of rows
    /// actually removed.
    ///
    /// Per-row lifecycle events fire - internally this hydrates each
    /// matching row via [`Self::find`] and calls `.delete()` on it.
    /// That preserves the soft-delete inherent override behaviour and
    /// dispatches `Deleting` / `Deleted` for each row.
    async fn destroy<I, K>(ids: I) -> Result<u64, FrameworkError>
    where
        I: IntoIterator<Item = K> + Send,
        I::IntoIter: Send,
        K: Into<<<Self::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType> + Send,
    {
        let mut removed: u64 = 0;
        for id in ids {
            if let Some(row) = Self::find(id).await? {
                row.delete().await?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Static force-mass-delete by primary key set. Mirrors
    /// `Model::forceDestroy([1,2,3])`. Bypasses soft-delete tombstone
    /// semantics - every matched row is physically removed.
    async fn force_destroy<I, K>(ids: I) -> Result<u64, FrameworkError>
    where
        I: IntoIterator<Item = K> + Send,
        I::IntoIter: Send,
        K: Into<<<Self::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType> + Send,
    {
        let mut removed: u64 = 0;
        for id in ids {
            if let Some(row) = Self::find(id).await? {
                row.force_delete().await?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Whether two model rows refer to the same database row - same
    /// `type_name` AND same primary-key value. Mirrors Laravel's
    /// `Model::is($other)` (which compares both class + key).
    fn is(&self, other: &Self) -> bool {
        self.primary_key_value_json() == other.primary_key_value_json()
    }

    /// Negation of [`Self::is`]. Mirrors `Model::isNot($other)`.
    fn is_not(&self, other: &Self) -> bool {
        !self.is(other)
    }

    /// Filtered serialisation - emit a JSON object minus the named
    /// columns. Suprnova's Rust-native equivalent of Laravel's
    /// per-instance `$model->makeHidden($cols)`. The default doesn't
    /// carry a runtime attribute bag, so the hide list is supplied
    /// directly at the call site.
    ///
    /// ```ignore
    /// return Json::ok(user.to_array_except(&["password_hash", "remember_token"]));
    /// ```
    fn to_array_except(&self, columns: &[&str]) -> serde_json::Value {
        let mut v = self.to_array();
        if let Some(map) = v.as_object_mut() {
            for col in columns {
                map.remove(*col);
            }
        }
        v
    }

    /// Filtered serialisation - emit a JSON object containing ONLY
    /// the named columns. Suprnova's Rust-native equivalent of
    /// Laravel's `$model->makeVisible($cols)` invoked alongside a
    /// reset.
    fn to_array_only(&self, columns: &[&str]) -> serde_json::Value {
        let full = self.to_array();
        let mut out = serde_json::Map::new();
        if let Some(map) = full.as_object() {
            for col in columns {
                if let Some(v) = map.get(*col) {
                    out.insert((*col).to_string(), v.clone());
                }
            }
        }
        serde_json::Value::Object(out)
    }

    /// Quiet variant of [`Self::save`] - runs the UPDATE inside a
    /// [`crate::seed::without_events`] scope so no model lifecycle
    /// events fire. Mirrors Laravel's `Model::saveQuietly`.
    async fn save_quietly(&self) -> Result<(), FrameworkError> {
        crate::seed::without_events(async { self.save().await }).await
    }

    /// Quiet variant of [`Self::update`] - runs the UPDATE inside a
    /// [`crate::seed::without_events`] scope.
    async fn update_quietly(self, attrs: Attrs) -> Result<Self, FrameworkError> {
        crate::seed::without_events(async { self.update(attrs).await }).await
    }

    /// Quiet variant of [`Self::delete`].
    async fn delete_quietly(self) -> Result<(), FrameworkError> {
        crate::seed::without_events(async { self.delete().await }).await
    }

    /// Quiet variant of [`Self::force_delete`].
    async fn force_delete_quietly(self) -> Result<(), FrameworkError> {
        crate::seed::without_events(async { self.force_delete().await }).await
    }

    /// Variant of [`Self::update`] that returns
    /// `FrameworkError::not_found` when the row no longer exists.
    /// Mirrors Laravel's `Model::updateOrFail`.
    ///
    /// The find + UPDATE pair runs inside a single transaction so a
    /// concurrent DELETE cannot slip between the existence check and
    /// the write. When the caller is already inside a
    /// [`DB::transaction`](crate::DB::transaction) closure the
    /// ambient transaction is reused (opening a fresh `DB::transaction`
    /// would be rejected as nested); otherwise one is opened for the
    /// duration of the call.
    ///
    /// SeaORM's `ActiveModelTrait::update` already surfaces a
    /// missing-row UPDATE as [`sea_orm::DbErr::RecordNotUpdated`] /
    /// [`sea_orm::DbErr::RecordNotFound`]; both are translated to
    /// `FrameworkError::not_found` here so a TOCTOU loss looks the
    /// same to the caller as a stale-handle pre-flight failure (HTTP
    /// 404 rather than a generic 500).
    async fn update_or_fail(self, attrs: Attrs) -> Result<Self, FrameworkError> {
        if crate::database::after_commit::in_transaction() {
            // Already inside `DB::transaction` - the surrounding
            // closure owns atomicity. Run the UPDATE through the
            // ambient tx and translate SeaORM's missing-row signals
            // (`RecordNotUpdated` on the WHERE miss,
            // `RecordNotFound` on the post-UPDATE re-fetch miss the
            // non-RETURNING backends use) into 404 instead of the
            // generic 500 a `DbErr::Database(...)` would map to.
            return self.update(attrs).await.map_err(|e| {
                if is_record_missing(&e) {
                    FrameworkError::not_found("update_or_fail: row no longer exists")
                } else {
                    e
                }
            });
        }

        // No ambient transaction - open one so a concurrent DELETE
        // can't slip between the pre-flight existence check and the
        // write. Inside the closure the UPDATE acquires the write
        // lock atomically with no separate read step, eliminating
        // the TOCTOU window the old implementation had between
        // `Self::find(pk)` and `Self::update(attrs)`.
        crate::database::DB::transaction(|tx| {
            Box::pin(async move {
                self.update_with_tx(tx, attrs).await.map_err(|e| {
                    if is_record_missing(&e) {
                        FrameworkError::not_found("update_or_fail: row no longer exists")
                    } else {
                        e
                    }
                })
            })
        })
        .await
    }

    /// Variant of [`Self::delete`] that returns
    /// `FrameworkError::not_found` when the row no longer exists.
    /// Mirrors Laravel's `Model::deleteOrFail`.
    ///
    /// Atomicity follows the same shape as
    /// [`Self::update_or_fail`] - the existence check and the DELETE
    /// share a transaction (ambient when one is in scope, otherwise a
    /// freshly opened one). SeaORM's `ActiveModelTrait::delete`
    /// returns `Ok` even when the WHERE clause matched zero rows, so
    /// we additionally inspect [`sea_orm::DeleteResult::rows_affected`]
    /// after the DELETE lands and surface `0` as 404 - matching what
    /// the caller would have seen had the pre-flight observed the
    /// missing row.
    async fn delete_or_fail(self) -> Result<(), FrameworkError> {
        if crate::database::after_commit::in_transaction() {
            return delete_one_or_fail::<Self>(self, None).await;
        }

        // No ambient transaction - wrap the DELETE in one so the
        // row-existence check and the write are atomic. Unlike
        // `Self::delete`, this helper inspects
        // `DeleteResult::rows_affected` and surfaces `0` as 404
        // instead of letting it look like a successful no-op.
        crate::database::DB::transaction(|tx| {
            Box::pin(async move { delete_one_or_fail::<Self>(self, Some(tx)).await })
        })
        .await
    }

    // --- Hooks the macro-generated impl fills in ---

    /// Return this row's primary-key value (typed, for SeaORM lookups).
    fn primary_key_value(
        &self,
    ) -> <<Self::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType;

    /// Return this row's primary-key value as JSON (for `increment` /
    /// `decrement` SQL binding without exposing the typed PK to the
    /// trait surface).
    fn primary_key_value_json(&self) -> serde_json::Value;

    /// Reset the PK to its `Default::default()` value. Used by
    /// `replicate_into` to ensure the replica is unsaved.
    fn reset_primary_key(&mut self);

    /// Build a fresh SeaORM `ActiveModel` from `attrs`. The PK is
    /// left as `NotSet` so `auto_increment` kicks in.
    fn active_model_from_attrs(
        attrs: Attrs,
    ) -> Result<<Self::Entity as EntityTrait>::ActiveModel, FrameworkError>;

    /// Apply `attrs` to an existing `ActiveModel`. Used by `update`
    /// to overlay partial changes onto the full row before SeaORM
    /// fires the UPDATE.
    fn apply_attrs_to_active_model(
        am: &mut <Self::Entity as EntityTrait>::ActiveModel,
        attrs: Attrs,
    ) -> Result<(), FrameworkError>;

    /// Materialise `self` into an `ActiveModel` for `save`. The PK is
    /// marked as Unchanged (so it acts as the WHERE clause) and every
    /// other column as Set.
    fn into_active_model_for_update(
        self,
    ) -> Result<<Self::Entity as EntityTrait>::ActiveModel, FrameworkError>;
}

/// The owners of the `MorphTo` relations named in a model's
/// `#[model(touches = [...])]`, found from a row before it is written
/// and used by the touch that follows the write. Built by
/// [`Model::__plan_touches`].
///
/// **Not part of the public API.** It is `pub` because the `Model`
/// trait methods that carry it are.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct TouchPlan {
    /// One entry per `MorphTo` name of `TOUCHES`: the owner, or `None`
    /// when the row's `<name>_id` value is null.
    morph_owners: Vec<(
        &'static str,
        Option<crate::eloquent::relations::morph::MorphOwner>,
    )>,
}

impl TouchPlan {
    /// The resolved owner of the `MorphTo` relation `relation`, `None`
    /// when the relation has no owner for the row.
    fn owner(&self, relation: &str) -> Option<&crate::eloquent::relations::morph::MorphOwner> {
        self.morph_owners
            .iter()
            .find(|(name, _)| *name == relation)
            .and_then(|(_, owner)| owner.as_ref())
    }
}

/// The error for a name in `touches` that the relation registry does
/// not know. The macro rejects such a name at expansion time, so this is
/// reached only when the registry and the `TOUCHES` const disagree.
fn unregistered_touch(table: &str, relation: &str) -> FrameworkError {
    FrameworkError::internal(format!(
        "`{table}` declares touches = [\"{relation}\"] but no relation named \
         `{relation}` is registered for it",
    ))
}

/// One owner row the parent-touch cascade writes: where it lives, the
/// columns the `UPDATE` names, and its key. A `BelongsTo` owner fills
/// it from its [`RelationEntry`](crate::eloquent::RelationEntry), a
/// `MorphTo` owner from the model its child's `<name>_type` names.
struct OwnerRow<'a> {
    table: &'a str,
    /// The time, as the owner's `updated_at` cast stores it.
    updated_at: sea_orm::Value,
    updated_at_column: &'a str,
    key_column: &'a str,
    soft_deletes_column: &'a str,
    key: &'a serde_json::Value,
}

/// One parent touch: `UPDATE <table> SET <updated_at> = ? WHERE <key> = ?`
/// through `exec`, then the render-cache advance for the touched row.
/// `tx_handle` is the explicit transaction of
/// [`Model::touch_owners_with_tx`], whose advance cannot rely on the
/// ambient task-local.
///
/// Every identifier is checked with
/// [`crate::database::validate_identifier`] before it is rendered, on
/// the principle [`Model::__touch_owners_via`] documents.
async fn touch_owner_row(
    exec: &crate::database::transaction::ExecutorChoice,
    tx_handle: Option<&crate::database::transaction::TxHandle>,
    owner: OwnerRow<'_>,
) -> Result<(), FrameworkError> {
    let backend = exec.backend();
    crate::database::validate_identifier(owner.table)?;
    crate::database::validate_identifier(owner.updated_at_column)?;
    crate::database::validate_identifier(owner.key_column)?;

    let set_ph = crate::database::placeholder::placeholder(backend, 1)?;
    let key_ph = crate::database::placeholder::placeholder(backend, 2)?;
    let mut sql = format!(
        "UPDATE {} SET {} = {set_ph} WHERE {} = {key_ph}",
        owner.table, owner.updated_at_column, owner.key_column,
    );
    // Agree with the owner's own default scope: a trashed
    // parent isn't a parent. Mirrors Laravel, where
    // `Relation::touch` runs through the relation query and so
    // inherits the related model's soft-delete scope.
    if !owner.soft_deletes_column.is_empty() {
        crate::database::validate_identifier(owner.soft_deletes_column)?;
        sql.push_str(&format!(" AND {} IS NULL", owner.soft_deletes_column));
    }

    exec.run(sea_orm::Statement::from_sql_and_values(
        backend,
        &sql,
        vec![owner.updated_at, json_value_to_sea_value(owner.key)],
    ))
    .await
    .map_err(|e| FrameworkError::database(e.to_string()))?;
    match tx_handle {
        Some(handle) => {
            crate::render_cache::orm::after_row_write_with_handle(handle, owner.table, owner.key)
                .await
        }
        None => crate::render_cache::orm::after_row_write(owner.table, owner.key).await,
    }
}

/// Cross-cutting helper called from `increment` / `decrement` and
/// from the Builder stub. Centralised here so both call sites stay
/// in sync.
pub fn json_value_to_sea_value(v: &serde_json::Value) -> sea_orm::Value {
    use sea_orm::Value;
    match v {
        serde_json::Value::String(s) => Value::String(Some(s.clone())),
        serde_json::Value::Bool(b) => Value::Bool(Some(*b)),
        serde_json::Value::Number(n) if n.is_i64() => Value::BigInt(Some(n.as_i64().unwrap())),
        serde_json::Value::Number(n) if n.is_u64() => {
            // SeaORM has no unsigned 64-bit type that maps cleanly here;
            // fall back to i64 if it fits, else to string.
            n.as_u64()
                .and_then(|u| i64::try_from(u).ok())
                .map(|i| Value::BigInt(Some(i)))
                .unwrap_or_else(|| Value::String(Some(n.to_string())))
        }
        serde_json::Value::Number(n) if n.is_f64() => Value::Double(Some(n.as_f64().unwrap())),
        serde_json::Value::Null => Value::String(None),
        _ => Value::String(Some(v.to_string())),
    }
}

/// Inverse of [`json_value_to_sea_value`] - best-effort conversion of
/// the variants commonly used as cursor / PK boundaries
/// (`Int`/`BigInt`/`Float`/`Double`/`String`/`Uuid`/`Bool`) into a
/// JSON form the Builder's `filter_op` chain can rebind through its
/// own placeholder pipeline.
///
/// Unsigned integers fall back to a stringified form when they don't
/// fit `i64`; rarely-used variants (bytes, decimals, chrono types)
/// stringify too. The full SeaORM `Value` ↔ JSON round-trip is
/// handled by the cursor wire codec in `pagination/cursor.rs`; this
/// helper exists only for the page-fetch boundary, where the value
/// will be rebound by SQLx within microseconds of being JSON-ified.
pub fn sea_value_to_json_loose(v: &sea_orm::Value) -> serde_json::Value {
    use sea_orm::Value;
    use serde_json::Value as J;
    match v {
        Value::Bool(Some(b)) => J::from(*b),
        Value::TinyInt(Some(i)) => J::from(*i),
        Value::SmallInt(Some(i)) => J::from(*i),
        Value::Int(Some(i)) => J::from(*i),
        Value::BigInt(Some(i)) => J::from(*i),
        Value::TinyUnsigned(Some(i)) => J::from(*i),
        Value::SmallUnsigned(Some(i)) => J::from(*i),
        Value::Unsigned(Some(i)) => J::from(*i),
        Value::BigUnsigned(Some(i)) => i64::try_from(*i)
            .map(J::from)
            .unwrap_or_else(|_| J::String(i.to_string())),
        Value::Float(Some(f)) => J::from(*f as f64),
        Value::Double(Some(f)) => J::from(*f),
        Value::String(Some(s)) => J::String(s.clone()),
        Value::Char(Some(c)) => J::String(c.to_string()),
        Value::Uuid(Some(u)) => J::String(u.to_string()),
        // Datetimes / decimals stringify - they round-trip back through
        // `json_value_to_sea_value` as Value::String, which the dialect
        // adapter then re-binds via SQL string coercion. Sufficient for
        // a cursor boundary comparison since the underlying column
        // already accepts string-shaped binds for these types.
        Value::ChronoDate(Some(d)) => J::String(d.to_string()),
        Value::ChronoTime(Some(t)) => J::String(t.to_string()),
        Value::ChronoDateTime(Some(dt)) => J::String(dt.to_string()),
        Value::ChronoDateTimeUtc(Some(dt)) => {
            J::String(dt.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))
        }
        Value::ChronoDateTimeLocal(Some(dt)) => {
            J::String(dt.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))
        }
        Value::ChronoDateTimeWithTimeZone(Some(dt)) => {
            J::String(dt.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true))
        }
        Value::Decimal(Some(d)) => J::String(d.to_string()),
        Value::BigDecimal(Some(d)) => J::String(d.to_string()),
        // Null variants (Some=None) or unsupported variants - emit
        // JSON null so the rebind lands on `WHERE col > NULL`. SQL's
        // three-valued logic treats that as "no rows match", which is
        // a safer-than-silent-mismatch failure mode.
        _ => J::Null,
    }
}

/// Macro-generated hook used by [`Model::replicate`] and
/// [`Model::replicate_except`].
pub trait ReplicateExt: Sized {
    /// Build a clone of `self` with the PK reset and every column
    /// named in `except` cleared to its `Default::default()`.
    fn replicate_with(&self, except: Vec<String>) -> Self;
}

/// First-or-... lookup helpers. Split into a separate trait so the
/// macro can emit a one-line `from_attrs_unsaved` hook per model
/// without bloating the [`Model`] surface.
///
/// The bounds duplicate `Model`'s where clause because Rust's trait
/// elaboration doesn't transitively propagate associated-type bounds
/// from a supertrait's where clause to a subtrait's method bodies.
/// Without these, `Self::query()` inside `first_or_create` fails to
/// type-check against the same constraints `Model::query()` is
/// declared with.
#[async_trait]
pub trait FirstOrCreate: Model + Send + Sync
where
    Self: From<<Self::Entity as EntityTrait>::Model> + crate::eloquent::EagerLoadDispatch,
    <Self::Entity as EntityTrait>::Model: From<Self>
        + IntoActiveModel<<Self::Entity as EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + Serialize
        + Send
        + Sync,
    <Self::Entity as EntityTrait>::ActiveModel: Send,
    <<Self::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    /// Look up a row by `lookup`. If found, return it. Otherwise
    /// create one with `lookup` merged with `extras` and return that.
    async fn first_or_create(lookup: Attrs, extras: Attrs) -> Result<Self, FrameworkError> {
        let existing = Self::query().filter_attrs(&lookup).first().await?;
        if let Some(found) = existing {
            return Ok(found);
        }
        Self::create(lookup.merge(extras)).await
    }

    /// Look up a row by `lookup`. If found, apply `updates` to it and
    /// return. Otherwise create one with `lookup` merged with `updates`
    /// and return that.
    async fn update_or_create(lookup: Attrs, updates: Attrs) -> Result<Self, FrameworkError> {
        let existing = Self::query().filter_attrs(&lookup).first().await?;
        if let Some(found) = existing {
            return found.update(updates).await;
        }
        Self::create(lookup.merge(updates)).await
    }

    /// Look up a row by `lookup`. If found, return it. Otherwise build
    /// an unsaved in-memory instance from `lookup` and return that.
    async fn first_or_new(lookup: Attrs) -> Result<Self, FrameworkError> {
        match Self::query().filter_attrs(&lookup).first().await? {
            Some(found) => Ok(found),
            None => Self::from_attrs_unsaved(lookup),
        }
    }

    /// Look up a row by `lookup`. If found, return it. Otherwise run
    /// `fallback` and return whatever it produces.
    async fn first_or<F, Fut>(lookup: Attrs, fallback: F) -> Result<Self, FrameworkError>
    where
        F: FnOnce() -> Fut + Send,
        Fut: std::future::Future<Output = Result<Self, FrameworkError>> + Send,
    {
        match Self::query().filter_attrs(&lookup).first().await? {
            Some(found) => Ok(found),
            None => fallback().await,
        }
    }

    /// Laravel's `findOr($id, $callback)` - look up by PK; when no row
    /// matches, run `fallback` and return its result.
    async fn find_or<K, F, Fut>(id: K, fallback: F) -> Result<Self, FrameworkError>
    where
        K: Into<<<Self::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType> + Send,
        F: FnOnce() -> Fut + Send,
        Fut: std::future::Future<Output = Result<Self, FrameworkError>> + Send,
    {
        match Self::find(id).await? {
            Some(found) => Ok(found),
            None => fallback().await,
        }
    }

    /// Laravel's `findOrNew($id)` - look up by PK; when no row matches,
    /// build an unsaved in-memory instance from `defaults`. The
    /// defaults map seeds the new instance the same way
    /// [`Self::first_or_new`] does.
    async fn find_or_new<K>(id: K, defaults: Attrs) -> Result<Self, FrameworkError>
    where
        K: Into<<<Self::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType> + Send,
    {
        match Self::find(id).await? {
            Some(found) => Ok(found),
            None => Self::from_attrs_unsaved(defaults),
        }
    }

    /// Laravel's `createOrFirst` - race-safe insert. Try to create the
    /// row; if it conflicts on a unique constraint, return the
    /// existing row. The conflict detection narrows on
    /// `FrameworkError::Database` only: validation failures,
    /// listener cancellations, or any other non-DB error variant
    /// propagate to the caller unchanged. If the re-query finds no
    /// row after a DB error, surface the original DB error (the
    /// "conflict + lookup hits nothing" combination is almost
    /// certainly a serialization / connection failure rather than
    /// a real uniqueness conflict).
    async fn create_or_first(lookup: Attrs, extras: Attrs) -> Result<Self, FrameworkError> {
        let attrs = lookup.clone().merge(extras);
        match Self::create(attrs).await {
            Ok(row) => Ok(row),
            Err(err @ FrameworkError::Database(_)) => {
                match Self::query().filter_attrs(&lookup).first().await? {
                    Some(found) => Ok(found),
                    None => Err(err),
                }
            }
            Err(other) => Err(other),
        }
    }

    /// Build an unsaved in-memory instance from `attrs`. Used by
    /// `first_or_new` / `find_or_new`. The macro fills this in.
    fn from_attrs_unsaved(attrs: Attrs) -> Result<Self, FrameworkError>;
}

/// Return `true` when `err` came from SeaORM signalling the WHERE
/// clause matched no rows (`RecordNotUpdated`) or the post-UPDATE
/// re-fetch found nothing (`RecordNotFound`). Used by `update_or_fail`
/// to translate a TOCTOU loss into HTTP 404 instead of a generic 500.
///
/// `FrameworkError::From<DbErr>` flattens the variant to its
/// `Display` form, so the match is by message prefix - both messages
/// are stable across SeaORM 1.x and unique enough to avoid
/// collisions with user error text.
fn is_record_missing(err: &FrameworkError) -> bool {
    if let FrameworkError::Database(msg) = err {
        msg.starts_with("None of the records are updated")
            || msg.starts_with("RecordNotFound Error")
    } else {
        false
    }
}

/// Execute a hard DELETE for `model` and assert that exactly one row
/// was removed. Shared between `delete_or_fail` and its
/// ambient-transaction branch so both paths fire the same lifecycle
/// events (`Deleting` → DELETE → `Deleted`) and the same
/// `rows_affected == 1` check.
///
/// When `tx` is `Some` the DELETE is pinned to the supplied
/// transaction (the `delete_or_fail` no-ambient-tx path uses this
/// after opening its own `DB::transaction`). When `tx` is `None` the
/// DELETE flows through whatever the ambient `CURRENT_TX` /
/// per-model default connection routing resolves to - which inside
/// `delete_or_fail`'s ambient-tx branch is the surrounding closure
/// transaction.
async fn delete_one_or_fail<M>(
    model: M,
    tx: Option<&crate::database::Transaction>,
) -> Result<(), FrameworkError>
where
    M: Model,
    M: From<<M::Entity as EntityTrait>::Model>,
    <M::Entity as EntityTrait>::Model: From<M>
        + IntoActiveModel<<M::Entity as EntityTrait>::ActiveModel>
        + Serialize
        + Send
        + Sync,
    <M::Entity as EntityTrait>::ActiveModel: Send,
    <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    M::__dispatch_deleting(&model, false).await?;

    let snapshot = model.clone();
    let row = model.try_into_storage()?;
    let am = row.into_active_model();
    // CACHE-009: with an explicit handle the write and its advance already
    // share that transaction, so the write runs as is; without one,
    // `atomic` opens the transaction they share.
    let snapshot_ref = &snapshot;
    let write = || async move {
        let exec = match tx {
            Some(t) => crate::database::transaction::ExecutorChoice::from_tx(t),
            None => {
                crate::database::transaction::ExecutorChoice::resolve_write(
                    None,
                    None,
                    M::default_connection_name(),
                )
                .await?
            }
        };
        let result = exec
            .delete_active(am)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        if result.rows_affected == 0 {
            return Err(FrameworkError::not_found(
                "delete_or_fail: row no longer exists",
            ));
        }
        match tx {
            Some(t) => crate::render_cache::orm::after_model_write_with_tx(t, snapshot_ref).await?,
            None => crate::render_cache::orm::after_model_write(snapshot_ref).await?,
        }
        Ok(())
    };
    match tx {
        Some(_) => write().await?,
        None => crate::render_cache::orm::atomic(M::default_connection_name(), write).await?,
    }

    M::__dispatch_deleted(&snapshot, false).await?;
    Ok(())
}
