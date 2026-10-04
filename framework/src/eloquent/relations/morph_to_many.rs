//! Polymorphic many-to-many - m2m through a pivot table that
//! distinguishes parent rows by a `*_type` discriminator column.
//!
//! Mirrors Laravel's
//! [polymorphic m2m](https://laravel.com/docs/12.x/eloquent-relationships#many-to-many-polymorphic-relations)
//! semantics: a single pivot table (`taggables`) carries one FK to the
//! shared m2m side (`tag_id`) plus a `<name>_id` / `<name>_type` pair
//! that points at one of several parent morph families (Post, Video).
//!
//! Two flavours:
//!
//! - [`MorphToMany`] lives on the morphable side (`Post.tags()`).
//!   The pivot row matches `<name>_id = parent.id AND
//!   <name>_type = parent_morph_type`.
//! - [`MorphedByMany`] is the inverse, on the m2m side
//!   (`Tag.posts()` / `Tag.videos()`). The pivot row matches
//!   `<pivot_foreign_key> = tag.id AND <name>_type = target_morph_type`.
//!   Filters one specific target morph family at a time so
//!   `tag.posts()` returns only Post-typed taggables and `tag.videos()`
//!   returns only Video-typed taggables, never mixing them in a single
//!   collection.
//!
//! Both share the [`BelongsToMany`](super::BelongsToMany) two-query
//! load strategy (related rows by IN + pivot rows separately), with
//! the extra `*_type` filter layered on every SQL statement that
//! touches the pivot.
//!
//! Default key conventions:
//!
//! - `morph_name` (controls the `<name>_id` + `<name>_type` columns):
//!   the relation name itself (`"taggable"` from
//!   `relations = { taggable: ... }`).
//! - `pivot_table`: `<P as EloquentModel>::TABLE` - the pivot model's
//!   own `#[model(table = "...")]` declaration is the single source of
//!   truth.
//! - `pivot_related_key` (`MorphToMany`'s pivot column → R): `<snake(R)>_id`.
//! - `pivot_foreign_key` (`MorphedByMany`'s pivot column → L=Tag):
//!   `<snake(L)>_id`.
//! - `parent_morph_type` (`MorphToMany`): L's `morph_type = "..."`
//!   attribute, defaulted to `to_snake(struct_name)`.
//! - `target_morph_type` (`MorphedByMany`): R's `morph_type` - passed
//!   explicitly via the relation declaration's `target_morph_type =
//!   "..."` option, since the macro at the L-side declaration site
//!   can't introspect R's `morph_type` attribute (it lives in a
//!   separate `#[suprnova::model]` invocation).
//!
//! Mutators (`MorphToMany` only):
//!
//! - [`attach`](MorphToMany::attach) - INSERT a pivot row with the
//!   `<name>_id` + `<name>_type` + pivot related FK.
//! - [`attach_with`](MorphToMany::attach_with) - INSERT with extra
//!   pivot columns (and timestamps if `with_timestamps()` is set).
//! - [`detach`](MorphToMany::detach) - DELETE matching the parent's id
//!   + type.
//! - [`sync`](MorphToMany::sync) - diff-and-apply, transactional via
//!   `DatabaseConnection::begin()`.
//! - [`sync_without_detaching`](MorphToMany::sync_without_detaching) -
//!   the attach half of `sync`, leaving existing rows untouched.
//!
//! Readers (both flavours):
//!
//! - `.get()` - JOIN R to pivot with the type filter. Two-query
//!   strategy filling `__pivot` per row.
//! - `.first()` - `.get().into_iter().next()`.
//! - `.count()` - `SELECT COUNT(*) FROM pivot WHERE ... AND
//!   <name>_type = ?`.
//! - `where_pivot` and family - constrain the pivot side of a read.
//!   Applies to `.get()` / `.first()` / `.count()` on both flavours;
//!   `MorphToMany`'s mutators refuse to run while a filter is set,
//!   because Suprnova builds its pivot DELETE by hand and a read
//!   predicate silently not narrowing a write is a difference the
//!   caller cannot see. Eager loading (`MmPost::with(["tags"])`) goes
//!   through the macro-emitted `__eager_load` arm, which never
//!   constructs a `MorphToMany` and therefore carries no pivot filter -
//!   use the relation accessor for a filtered read.
//!
//! Eager loading happens through the parent model's `__eager_load`
//! match arm - emitted by `#[suprnova::model]` and exercised by
//! `MmPost::with(["tags"])`. Same per-attachment clone semantics as
//! BelongsToMany (each parent gets its own `__pivot` context on every
//! returned R clone).

use std::marker::PhantomData;
use std::sync::Arc;

use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};

use crate::database::transaction::ExecutorChoice;
use crate::eloquent::EloquentModel;
use crate::eloquent::attrs::Attrs;
use crate::eloquent::builder::{Builder, IntoColumn, IntoVal, WhereTerm};
use crate::eloquent::collection::Collection;
use crate::eloquent::lazy_loading::LazyLoadGuard;
use crate::eloquent::model::{Model, json_value_to_sea_value};
use crate::eloquent::relations::belongs_to_many::{
    PivotExtra, PivotMatch, PivotTarget, bind_pivot_comparison, bind_pivot_extra, bind_pivot_write,
    load_pivot_rows, pivot_extras_through_casts, pivot_key_json, unique_pivot_ids,
};
use crate::eloquent::relations::pivot_filters::{PivotFilters, pivot_filter_methods};
use crate::eloquent::relations::{Relation, RelationKind};
use crate::error::FrameworkError;

/// Boxed builder-rewrite closure for [`MorphToMany::with_trashed`] /
/// [`MorphedByMany::with_trashed`] (and their `only_trashed`
/// siblings). Same closure-erasure trick as
/// [`super::belongs_to::ScopeRewrite`][crate::eloquent::relations::belongs_to].
type ScopeRewrite<R> = Box<dyn FnOnce(Builder<R>) -> Builder<R> + Send>;

/// Polymorphic m2m from morphable parent `L` to m2m target `R` through
/// polymorphic pivot `P`. Constructed by the macro-emitted relation
/// method (`fn tags(&self) -> MorphToMany<Self, Tag, Taggable>`); user
/// code never calls [`MorphToMany::__new`] directly.
///
/// The wrapper holds the morph + key metadata plus the parent's PK
/// value and morph-type string, all paid up at construction time.
/// Terminal methods (`attach`, `detach`, `sync`, `get`, `first`,
/// `count`) issue the SQL.
pub struct MorphToMany<L, R, P>
where
    L: EloquentModel,
    R: Model,
    R: From<<R::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <R::Entity as sea_orm::EntityTrait>::Model: From<R>
        + sea_orm::IntoActiveModel<<R::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <R::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<R::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
    P: Model + 'static,
    P: From<<P::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <P::Entity as sea_orm::EntityTrait>::Model: From<P>
        + sea_orm::IntoActiveModel<<P::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <P::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<P::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    /// Parent row's local-key value, JSON-encoded. The macro emits
    /// `serde_json::to_value(&self.id)` at the call site, matching
    /// [`BelongsToMany`](super::BelongsToMany)'s storage convention.
    parent_key_value: serde_json::Value,
    /// L's `morph_type = "..."` attribute value - the string the
    /// pivot's `<morph_name>_type` column has to equal for the pivot
    /// row to belong to this parent. Defaults to `to_snake(L)` at the
    /// macro emission site when the attribute isn't declared.
    parent_morph_type: String,
    /// Morph family name. Controls the `<morph_name>_id` and
    /// `<morph_name>_type` column names on the pivot table. Defaults
    /// to the relation name (e.g. `"taggable"` from a relation
    /// declared as `taggable: MorphToMany<...>`); overridable via
    /// `name = "..."` (alias `morph_name = "..."`).
    morph_name: String,
    /// Pivot table name. Defaults to `<P as EloquentModel>::TABLE` -
    /// the pivot's own `#[suprnova::model(table = "...")]` declaration
    /// is the single source of truth. Override via the macro's
    /// `pivot_table = "..."` option.
    pivot_table: String,
    /// Pivot column pointing at the related row (`R`). Default:
    /// `<snake(R)>_id`. Override via `pivot_related_key = "..."`.
    pivot_related_key: String,
    /// Parent table's key column. Default: the parent model's primary
    /// key. Honoured by the [`Relation`] impl + admin introspection.
    parent_key: String,
    /// Related table's key COLUMN used by the JOIN in [`Self::get`].
    /// Default: the related model's primary key. Set via
    /// `.related_pk(...)`.
    related_key: String,
    /// Extra pivot columns to project into `__pivot`. Always includes
    /// the implicit pivot FKs + the morph discriminator pair.
    pivot_columns: Vec<String>,
    /// When true, the attach path stamps `created_at` / `updated_at`
    /// on every pivot row written.
    with_timestamps: bool,
    /// Deferred soft-delete scope rewrite applied to the related-row
    /// query at [`Self::get`] / [`Self::first`] time. Only set by
    /// [`Self::with_trashed`] / [`Self::only_trashed`], both gated
    /// on `R: SoftDeletes`.
    scope_rewrite: Option<ScopeRewrite<R>>,
    /// Pivot-side WHERE terms accumulated by the `where_pivot*` family.
    /// Applied to the pivot scan in [`Self::get`] and to
    /// [`Self::count`]; the mutators refuse to run while it is
    /// non-empty. Empty by default, and an empty set renders no SQL at
    /// all, so an unfiltered relation issues exactly the statements it
    /// issued before pivot filtering existed.
    pivot_filters: PivotFilters,
    /// The lazy-loading check [`Self::get`] runs before its first query
    /// ([`Self::first`] goes through it). The mutators do not run it: a
    /// write is no lazy load. Set by the macro-emitted relation method.
    lazy_load: LazyLoadGuard,
    /// `PhantomData` carries `L`, `R`, `P` so the [`Relation`] impl
    /// can name `type Parent = L` / `type Target = R` without runtime
    /// fields.
    #[allow(clippy::type_complexity)]
    _phantom: PhantomData<fn() -> (L, R, P)>,
}

impl<L, R, P> MorphToMany<L, R, P>
where
    L: EloquentModel,
    R: Model,
    R: From<<R::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <R::Entity as sea_orm::EntityTrait>::Model: From<R>
        + sea_orm::IntoActiveModel<<R::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <R::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<R::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
    P: Model + 'static,
    P: From<<P::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <P::Entity as sea_orm::EntityTrait>::Model: From<P>
        + sea_orm::IntoActiveModel<<P::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <P::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<P::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    /// Construct a `MorphToMany`. Invoked by the macro-emitted
    /// relation method; not part of the public API.
    #[doc(hidden)]
    pub fn __new(
        parent_key_value: serde_json::Value,
        parent_morph_type: String,
        morph_name: String,
        pivot_table: String,
        pivot_related_key: String,
    ) -> Self {
        Self {
            parent_key_value,
            parent_morph_type,
            morph_name,
            pivot_table,
            pivot_related_key,
            parent_key: L::PRIMARY_KEY.into(),
            related_key: R::PRIMARY_KEY.into(),
            pivot_columns: Vec::new(),
            with_timestamps: false,
            scope_rewrite: None,
            pivot_filters: PivotFilters::default(),
            lazy_load: LazyLoadGuard::default(),
            _phantom: PhantomData,
        }
    }

    /// Attach the lazy-loading check of the row this relation was read
    /// from. Invoked by the macro-emitted relation method; not part of
    /// the public API.
    #[doc(hidden)]
    pub fn __lazy_load(mut self, guard: LazyLoadGuard) -> Self {
        self.lazy_load = guard;
        self
    }

    /// Declare extra pivot columns to surface on each loaded R via
    /// `r.pivot::<P>()`. Mirrors Laravel's `->withPivot([...])`.
    ///
    /// The morph FK + type columns are always loaded; this option is
    /// for "extras" - `assigned_at`, `notes`, custom payloads.
    pub fn with_pivot<I, S>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.pivot_columns
            .extend(columns.into_iter().map(Into::into));
        self
    }

    /// Touch `created_at` / `updated_at` on every pivot row written
    /// by `attach` / `attach_with` / `sync`. Mirrors Laravel's
    /// `->withTimestamps()`.
    pub fn with_timestamps(mut self) -> Self {
        self.with_timestamps = true;
        self
    }

    /// Override the parent's key column. Only updates the metadata
    /// surface; the runtime parent value was extracted at construction.
    pub fn local_key(mut self, key: impl Into<String>) -> Self {
        self.parent_key = key.into();
        self
    }

    /// Override the related-side key COLUMN used by [`Self::get`]'s
    /// IN-set filter. Defaults to the related model's primary key.
    pub fn related_pk(mut self, key: impl Into<String>) -> Self {
        self.related_key = key.into();
        self
    }

    pivot_filter_methods!(P);

    /// Validate the three SQL identifiers that flow unquoted into every
    /// raw-SQL statement this relation builds. `morph_name` is the root
    /// of the derived `<morph_name>_id` / `<morph_name>_type` column
    /// names; validating it covers both derived names. Called at the
    /// top of every terminal method.
    fn validate_meta(&self) -> Result<(), FrameworkError> {
        crate::database::validate_identifier(&self.morph_name)?;
        crate::database::validate_identifier(&self.pivot_table)?;
        crate::database::validate_identifier(&self.pivot_related_key)?;
        Ok(())
    }

    /// The pivot's `<morph_name>_id` and `<morph_name>_type` columns.
    fn morph_columns(&self) -> (String, String) {
        (
            format!("{}_id", self.morph_name),
            format!("{}_type", self.morph_name),
        )
    }

    /// The pivot as the pivot statements address it, given the columns
    /// [`Self::morph_columns`] names, with the binders that type its
    /// columns.
    fn morph_pivot<'a>(&'a self, id_col: &'a str, type_col: &'a str) -> MorphPivot<'a> {
        MorphPivot {
            target: PivotTarget {
                table: &self.pivot_table,
                foreign_key: id_col,
                related_key: &self.pivot_related_key,
                pivot: <P as EloquentModel>::bind_column,
                parent: (<L as EloquentModel>::bind_column, &self.parent_key),
                related: (<R as EloquentModel>::bind_column, &self.related_key),
            },
            type_col,
            morph_type: &self.parent_morph_type,
        }
    }

    /// Insert a pivot row linking the parent (`<morph_name>_id =
    /// parent.id AND <morph_name>_type = parent_morph_type`) to
    /// `related_id`. Equivalent to `attach_with(related_id,
    /// Attrs::new())`.
    pub async fn attach(
        self,
        related_id: impl Into<serde_json::Value>,
    ) -> Result<(), FrameworkError> {
        self.attach_with(related_id, Attrs::new()).await
    }

    /// Insert a pivot row with extra column values (and timestamps
    /// when `with_timestamps()` is on).
    ///
    /// # Security
    ///
    /// Same contract as
    /// [`BelongsToMany::attach_with`](crate::eloquent::BelongsToMany::attach_with):
    /// keys of `extra` are pivot column names that interpolate **raw**
    /// into the rendered INSERT. Never accept the key names from
    /// untrusted input.
    pub async fn attach_with(
        self,
        related_id: impl Into<serde_json::Value>,
        extra: Attrs,
    ) -> Result<(), FrameworkError> {
        self.pivot_filters.reject_mutation()?;
        self.validate_meta()?;
        let id = related_id.into();
        crate::render_cache::orm::atomic(L::default_connection_name(), || {
            self.attach_with_inner(id, extra)
        })
        .await
    }

    /// The pivot insert and its advance, run under [`Self::attach_with`]'s
    /// atomic wrapper (CACHE-009).
    async fn attach_with_inner(
        self,
        id: serde_json::Value,
        extra: Attrs,
    ) -> Result<(), FrameworkError> {
        // Phase 10C audit-fix AF2 - resolve through ExecutorChoice so the
        // pivot INSERT lands on the ambient transaction when CURRENT_TX
        // is active.
        let exec = ExecutorChoice::resolve_write(None, None, L::default_connection_name()).await?;
        let (id_col, type_col) = self.morph_columns();
        let pivot = self.morph_pivot(&id_col, &type_col);
        let extra =
            pivot_extras_through_casts::<P>(extra, &[&self.pivot_related_key, &id_col, &type_col])?;
        match &exec {
            ExecutorChoice::Tx(t, _) => {
                morph_attach_one(
                    t.as_ref(),
                    &pivot,
                    &self.parent_key_value,
                    &id,
                    extra,
                    self.with_timestamps,
                )
                .await
            }
            ExecutorChoice::Pool(c, _) => {
                morph_attach_one(
                    c.inner(),
                    &pivot,
                    &self.parent_key_value,
                    &id,
                    extra,
                    self.with_timestamps,
                )
                .await
            }
        }?;
        // No explicit-tx override on this relation (only ambient
        // `CURRENT_TX` or the pool), and the pivot row's key is composite,
        // not addressable by `DependencyIdentity::record` - the table
        // identity is correct, matching `BelongsToMany::attach_with`.
        crate::render_cache::orm::after_bulk_write(&self.pivot_table).await
    }

    /// Delete pivot rows linking this parent to `related_id`.
    pub async fn detach(
        self,
        related_id: impl Into<serde_json::Value>,
    ) -> Result<(), FrameworkError> {
        self.pivot_filters.reject_mutation()?;
        self.validate_meta()?;
        let id = related_id.into();
        crate::render_cache::orm::atomic(L::default_connection_name(), || self.detach_inner(id))
            .await
    }

    /// The pivot delete and its advance, run under [`Self::detach`]'s
    /// atomic wrapper (CACHE-009).
    async fn detach_inner(self, id: serde_json::Value) -> Result<(), FrameworkError> {
        // Phase 10C audit-fix AF2 - see attach_with above.
        let exec = ExecutorChoice::resolve_write(None, None, L::default_connection_name()).await?;
        let (id_col, type_col) = self.morph_columns();
        let pivot = self.morph_pivot(&id_col, &type_col);
        match &exec {
            ExecutorChoice::Tx(t, _) => {
                morph_detach_one(t.as_ref(), &pivot, &self.parent_key_value, &id).await
            }
            ExecutorChoice::Pool(c, _) => {
                morph_detach_one(c.inner(), &pivot, &self.parent_key_value, &id).await
            }
        }?;
        crate::render_cache::orm::after_bulk_write(&self.pivot_table).await
    }

    /// Replace the parent's full set of attached relations with the
    /// given IDs. Transactional - partial failure rolls back.
    pub async fn sync<I, V>(self, ids: I) -> Result<(), FrameworkError>
    where
        I: IntoIterator<Item = V>,
        V: Into<serde_json::Value>,
    {
        self.sync_ids(unique_pivot_ids(ids), true).await
    }

    /// Attach each of `ids` the parent does not hold yet, and leave every
    /// pivot row it already has as it is. Mirrors Laravel's
    /// `->syncWithoutDetaching([...])`, with the same contract as
    /// [`BelongsToMany::sync_without_detaching`](crate::eloquent::relations::BelongsToMany::sync_without_detaching):
    /// existing rows keep their extra columns and timestamps, the inserts
    /// run in one transaction, and rows of other morph families are never
    /// read or written.
    pub async fn sync_without_detaching<I, V>(self, ids: I) -> Result<(), FrameworkError>
    where
        I: IntoIterator<Item = V>,
        V: Into<serde_json::Value>,
    {
        self.sync_ids(unique_pivot_ids(ids), false).await
    }

    /// The checks and the atomic wrapper shared by [`Self::sync`] and
    /// [`Self::sync_without_detaching`]. `detaching` says whether rows
    /// missing from `target_ids` are deleted.
    async fn sync_ids(
        self,
        target_ids: Vec<serde_json::Value>,
        detaching: bool,
    ) -> Result<(), FrameworkError> {
        self.pivot_filters.reject_mutation()?;
        self.validate_meta()?;
        crate::render_cache::orm::atomic(L::default_connection_name(), || {
            self.sync_inner(target_ids, detaching)
        })
        .await
    }

    /// The pivot reconciliation and its advance, run under [`Self::sync`]'s
    /// atomic wrapper (CACHE-009): inside the ambient transaction the
    /// writes route through it and the advance joins it. Without
    /// `detaching` nothing is deleted.
    async fn sync_inner(
        self,
        target_ids: Vec<serde_json::Value>,
        detaching: bool,
    ) -> Result<(), FrameworkError> {
        use std::collections::{HashMap, HashSet};

        // Phase 10C audit-fix AF2 - same shape as BelongsToMany::sync -
        // route through ExecutorChoice so the SELECT + inner writes
        // honor CURRENT_TX.
        let exec = ExecutorChoice::resolve_write(None, None, L::default_connection_name()).await?;
        let backend = exec.backend();

        let (id_col, type_col) = self.morph_columns();
        let pivot = self.morph_pivot(&id_col, &type_col);

        // SELECT current pivot rows: keyed by the parent's id + type. A
        // parent id no pivot row can hold has no rows.
        let parent = bind_pivot_comparison(
            backend,
            pivot
                .target
                .typed(&id_col, Some(pivot.target.parent), &self.parent_key_value),
            &self.parent_key_value,
        );
        let rows = match parent {
            Some(parent) => {
                let id_ph = crate::database::placeholder::typed_placeholder(backend, 1, &parent)?;
                let type_ph = match backend {
                    DatabaseBackend::Postgres => "$2".to_string(),
                    _ => "?".to_string(),
                };
                let select_sql = format!(
                    "SELECT {related_key} AS __sn_related FROM {table} \
                      WHERE {id_col} = {id_ph} AND {type_col} = {type_ph}",
                    related_key = self.pivot_related_key,
                    table = self.pivot_table,
                );
                let select_stmt = Statement::from_sql_and_values(
                    backend,
                    &select_sql,
                    vec![parent, sea_orm::Value::from(self.parent_morph_type.clone())],
                );
                exec.query_all(select_stmt)
                    .await
                    .map_err(|e| FrameworkError::database(e.to_string()))?
            }
            None => Vec::new(),
        };

        let mut current_map: HashMap<String, serde_json::Value> = HashMap::new();
        for r in rows.iter() {
            if let Some(v) = pivot_key_json(r, "__sn_related") {
                current_map.insert(v.to_string(), v);
            }
        }
        let current_keys: HashSet<String> = current_map.keys().cloned().collect();

        let target_keys: HashSet<String> = target_ids.iter().map(|v| v.to_string()).collect();

        let mut attach_set: Vec<serde_json::Value> = Vec::new();
        for v in target_ids.into_iter() {
            if !current_keys.contains(&v.to_string()) {
                attach_set.push(v);
            }
        }
        let detach_set: Vec<serde_json::Value> = if detaching {
            current_map
                .into_iter()
                .filter_map(|(k, v)| (!target_keys.contains(&k)).then_some(v))
                .collect()
        } else {
            Vec::new()
        };

        // Atomicity: inherit from CURRENT_TX when active, else open
        // inner SeaORM tx - same precedence as BelongsToMany::sync.
        match &exec {
            ExecutorChoice::Tx(t, _) => {
                for related_id in detach_set.iter() {
                    morph_detach_one(t.as_ref(), &pivot, &self.parent_key_value, related_id)
                        .await?;
                }
                for related_id in attach_set.iter() {
                    morph_attach_one(
                        t.as_ref(),
                        &pivot,
                        &self.parent_key_value,
                        related_id,
                        Vec::new(),
                        self.with_timestamps,
                    )
                    .await?;
                }
            }
            ExecutorChoice::Pool(c, _) => {
                let txn = c
                    .inner()
                    .begin()
                    .await
                    .map_err(|e| FrameworkError::database(e.to_string()))?;
                for related_id in detach_set.iter() {
                    morph_detach_one(&txn, &pivot, &self.parent_key_value, related_id).await?;
                }
                for related_id in attach_set.iter() {
                    morph_attach_one(
                        &txn,
                        &pivot,
                        &self.parent_key_value,
                        related_id,
                        Vec::new(),
                        self.with_timestamps,
                    )
                    .await?;
                }
                txn.commit()
                    .await
                    .map_err(|e| FrameworkError::database(e.to_string()))?;
            }
        }
        if !attach_set.is_empty() || !detach_set.is_empty() {
            crate::render_cache::orm::after_bulk_write(&self.pivot_table).await?;
        }
        Ok(())
    }

    /// Fetch every related row currently attached to this parent
    /// through the polymorphic pivot. Each row carries its pivot
    /// context via `__pivot` (accessible through the macro-emitted
    /// `.pivot::<P>()` accessor).
    ///
    /// Two-query strategy: fetch related rows by IN-set on the pivot's
    /// related-FK values (filtered by the parent's id + type), then
    /// fetch pivot rows separately and zip via `(parent_id, related_id)`.
    ///
    /// Refused without a query when it is a lazy load that
    /// [lazy-loading prevention](crate::eloquent::lazy_loading) catches.
    pub async fn get(self) -> Result<Collection<R>, FrameworkError> {
        self.lazy_load.check()?;
        self.validate_meta()?;
        // Phase 10C audit-fix AF2 - route the pivot-id SELECT through
        // ExecutorChoice so it honors CURRENT_TX. Downstream
        // Model::query() calls already do so via Builder::get.
        let exec = ExecutorChoice::resolve_read(None, None, L::default_connection_name()).await?;
        let backend = exec.backend();

        let id_col = format!("{}_id", self.morph_name);
        let type_col = format!("{}_type", self.morph_name);

        // Fetch the set of related IDs attached to this parent.
        let (id_ph, type_ph) = match backend {
            DatabaseBackend::Postgres => ("$1".to_string(), "$2".to_string()),
            _ => ("?".to_string(), "?".to_string()),
        };
        let mut id_values: Vec<sea_orm::Value> = vec![
            json_value_to_sea_value(&self.parent_key_value),
            sea_orm::Value::from(self.parent_morph_type.clone()),
        ];
        // Two prefix binds (parent id, morph type) before any filter.
        // PostgreSQL placeholders are positional, so the filter
        // renderer has to continue from `$3`.
        let mut id_bind_index: usize = 2;
        let pivot_predicates =
            self.pivot_filters
                .render_and(backend, &mut id_values, &mut id_bind_index)?;
        let id_sql = format!(
            "SELECT {rk} AS __sn_related FROM {table} \
              WHERE {id_col} = {id_ph} AND {type_col} = {type_ph}{pivot_predicates}",
            rk = self.pivot_related_key,
            table = self.pivot_table,
            id_col = id_col,
            type_col = type_col,
            id_ph = id_ph,
            type_ph = type_ph,
            pivot_predicates = pivot_predicates,
        );
        let id_stmt = Statement::from_sql_and_values(backend, &id_sql, id_values);
        let id_rows = exec
            .query_all(id_stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        let mut related_ids: Vec<serde_json::Value> = Vec::with_capacity(id_rows.len());
        for r in id_rows.iter() {
            if let Some(v) = pivot_key_json(r, "__sn_related") {
                related_ids.push(v);
            }
        }
        if related_ids.is_empty() {
            return Ok(Collection::new());
        }

        // Fetch the related rows by IN-set on their PK column. The
        // optional `scope_rewrite` closure (set by `with_trashed` /
        // `only_trashed` when `R: SoftDeletes`) widens or restricts
        // the scope on `R`. Non-soft-delete `R`s have no closure to
        // apply.
        let related_rows: Vec<R> = {
            let mut q = R::query().filter_in(self.related_key.as_str(), related_ids.clone());
            if let Some(rw) = self.scope_rewrite {
                q = rw(q);
            }
            q.get().await?.into_vec()
        };

        // Fetch the pivot rows attached to this parent (filtered by
        // both id and type), from the table the id scan read and under
        // the same predicates - otherwise a filtered read, or a relation
        // that names its own pivot table, could stamp `__pivot` from a
        // row the scan never saw.
        let pivot_rows: Vec<P> = load_pivot_rows::<P>(
            &self.pivot_table,
            L::default_connection_name(),
            vec![
                (
                    id_col.clone(),
                    PivotMatch::Eq(self.parent_key_value.clone()),
                ),
                (
                    type_col.clone(),
                    PivotMatch::Eq(serde_json::Value::String(self.parent_morph_type.clone())),
                ),
            ],
            &self.pivot_filters,
        )
        .await?;

        // Index pivots by related_key value (JSON-string form).
        use std::collections::HashMap;
        let mut by_related: HashMap<String, P> = HashMap::new();
        for p in pivot_rows.into_iter() {
            let p_json = serde_json::to_value(&p).unwrap_or(serde_json::Value::Null);
            let key = p_json
                .get(&self.pivot_related_key)
                .map(|v| v.to_string())
                .unwrap_or_default();
            by_related.insert(key, p);
        }

        // Stamp the pivot context onto each related row via the
        // `EagerLoadDispatch::set_pivot_arc` hook.
        let mut out: Vec<R> = Vec::with_capacity(related_rows.len());
        for r in related_rows.into_iter() {
            let r_json = serde_json::to_value(&r).unwrap_or(serde_json::Value::Null);
            let key = r_json
                .get(&self.related_key)
                .map(|v| v.to_string())
                .unwrap_or_default();
            let mut row = r;
            if let Some(pivot) = by_related.get(&key) {
                row.set_pivot_arc(Some(Arc::new(pivot.clone())));
            }
            out.push(row);
        }
        Ok(Collection::from_vec(out))
    }

    /// Convenience over `get()` - drop everything after the first
    /// related row.
    pub async fn first(self) -> Result<Option<R>, FrameworkError> {
        Ok(self.get().await?.into_vec().into_iter().next())
    }

    /// `SELECT COUNT(*) FROM pivot WHERE <name>_id = ? AND <name>_type = ?`.
    /// Returns `i64` to match [`BelongsToMany::count`](super::BelongsToMany::count).
    pub async fn count(self) -> Result<i64, FrameworkError> {
        self.validate_meta()?;
        // Phase 10C audit-fix AF2 - see attach_with above.
        let exec = ExecutorChoice::resolve_read(None, None, L::default_connection_name()).await?;
        let backend = exec.backend();
        let (id_ph, type_ph) = match backend {
            DatabaseBackend::Postgres => ("$1".to_string(), "$2".to_string()),
            _ => ("?".to_string(), "?".to_string()),
        };
        let id_col = format!("{}_id", self.morph_name);
        let type_col = format!("{}_type", self.morph_name);
        // `parent_morph_type` is cloned rather than moved because the
        // filter renderer needs `&self` afterwards.
        let mut values: Vec<sea_orm::Value> = vec![
            json_value_to_sea_value(&self.parent_key_value),
            sea_orm::Value::from(self.parent_morph_type.clone()),
        ];
        let mut bind_index: usize = 2;
        let pivot_predicates =
            self.pivot_filters
                .render_and(backend, &mut values, &mut bind_index)?;
        let sql = format!(
            "SELECT COUNT(*) AS __sn_count FROM {table} \
              WHERE {id_col} = {id_ph} AND {type_col} = {type_ph}{pivot_predicates}",
            table = self.pivot_table,
            id_col = id_col,
            type_col = type_col,
            id_ph = id_ph,
            type_ph = type_ph,
            pivot_predicates = pivot_predicates,
        );
        let stmt = Statement::from_sql_and_values(backend, &sql, values);
        let row = exec
            .query_one(stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        Ok(row
            .and_then(|r| r.try_get::<i64>("", "__sn_count").ok())
            .unwrap_or(0))
    }
}

/// Soft-delete scope modifiers for `MorphToMany<L, R, P>` when the
/// related (`R`) side is soft-deletable. Same shape as
/// [`BelongsToMany`](super::BelongsToMany)'s equivalent block - the
/// pivot table itself is never filtered for `deleted_at` (pivots are
/// a join artefact), only the related rows. The closure captures the
/// `R: SoftDeletes` bound at construction so [`Self::get`] can call
/// it generic over plain `R: Model`.
impl<L, R, P> MorphToMany<L, R, P>
where
    L: EloquentModel,
    R: Model + crate::eloquent::SoftDeletes,
    R: From<<R::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <R::Entity as sea_orm::EntityTrait>::Model: From<R>
        + sea_orm::IntoActiveModel<<R::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <R::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<R::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
    P: Model + 'static,
    P: From<<P::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <P::Entity as sea_orm::EntityTrait>::Model: From<P>
        + sea_orm::IntoActiveModel<<P::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <P::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<P::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    /// Widen the related-row lookup to include trashed `R` rows.
    pub fn with_trashed(mut self) -> Self {
        self.scope_rewrite = Some(Box::new(|b: Builder<R>| b.with_trashed()));
        self
    }

    /// Restrict the related-row lookup to *only* trashed `R` rows.
    pub fn only_trashed(mut self) -> Self {
        self.scope_rewrite = Some(Box::new(|b: Builder<R>| b.only_trashed()));
        self
    }
}

impl<L, R, P> Relation for MorphToMany<L, R, P>
where
    L: EloquentModel,
    R: Model,
    R: From<<R::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <R::Entity as sea_orm::EntityTrait>::Model: From<R>
        + sea_orm::IntoActiveModel<<R::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <R::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<R::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
    P: Model,
    P: From<<P::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <P::Entity as sea_orm::EntityTrait>::Model: From<P>
        + sea_orm::IntoActiveModel<<P::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <P::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<P::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    type Parent = L;
    type Target = R;
    const KIND: RelationKind = RelationKind::MorphToMany;

    fn parent_key(&self) -> &str {
        &self.parent_key
    }

    fn foreign_key(&self) -> &str {
        // Surface the morph-name root - the actual pivot columns are
        // `<morph_name>_id` (FK) and `<morph_name>_type`
        // (discriminator). Admin tooling reading the
        // [`RelationEntry`](super::RelationEntry) surfaces this.
        &self.morph_name
    }
}

// ---- MorphedByMany ------------------------------------------------------

/// Inverse polymorphic m2m - from the m2m side `L` (e.g. `Tag`) to one
/// specific morph target family `R` (e.g. `Post` or `Video`) through
/// polymorphic pivot `P`. Constructed by the macro-emitted relation
/// method; user code never calls [`MorphedByMany::__new`] directly.
///
/// Each declaration filters one target morph family - so `Tag.posts()`
/// returns only Post-typed taggables and `Tag.videos()` returns only
/// Video-typed taggables. The target's morph-type string is declared
/// explicitly on the relation via `target_morph_type = "..."` because
/// the macro at L's expansion site can't introspect R's `morph_type`
/// attribute (it lives in a separate `#[suprnova::model]` invocation).
pub struct MorphedByMany<L, R, P>
where
    L: EloquentModel,
    R: Model,
    R: From<<R::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <R::Entity as sea_orm::EntityTrait>::Model: From<R>
        + sea_orm::IntoActiveModel<<R::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <R::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<R::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
    P: Model + 'static,
    P: From<<P::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <P::Entity as sea_orm::EntityTrait>::Model: From<P>
        + sea_orm::IntoActiveModel<<P::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <P::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<P::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    /// The m2m side row's key value, JSON-encoded.
    tag_key_value: serde_json::Value,
    /// R's `morph_type` value - the string the pivot's
    /// `<morph_name>_type` column has to equal for the pivot row to
    /// point at this morph target family. Declared via the relation's
    /// `target_morph_type = "..."` option.
    target_morph_type: String,
    /// Morph family name - controls the `<morph_name>_id` /
    /// `<morph_name>_type` column names on the pivot.
    morph_name: String,
    /// Pivot table name.
    pivot_table: String,
    /// Pivot column pointing at the m2m side (`L=Tag`). Default
    /// `<snake(L)>_id`.
    pivot_foreign_key: String,
    /// Related-side key COLUMN used by the JOIN. Default: the related
    /// model's primary key.
    related_key: String,
    /// Tag-side key column. Default: the tag model's primary key.
    /// Honoured by the [`Relation`] impl.
    parent_key: String,
    /// Deferred soft-delete scope rewrite applied to the related-row
    /// query at [`Self::get`] time. See [`MorphToMany::scope_rewrite`]
    /// for the matching closure-erasure pattern.
    scope_rewrite: Option<ScopeRewrite<R>>,
    /// Pivot-side WHERE terms accumulated by the `where_pivot*` family.
    /// Applied to the pivot query in [`Self::get`] and to
    /// [`Self::count`]. `MorphedByMany` is read-only, so there is no
    /// mutator to refuse. Empty by default, and an empty set renders no
    /// SQL at all.
    pivot_filters: PivotFilters,
    /// The lazy-loading check [`Self::get`] runs before its first query
    /// ([`Self::first`] goes through it). Set by the macro-emitted
    /// relation method.
    lazy_load: LazyLoadGuard,
    /// `PhantomData` carries `L`, `R`, `P` so the [`Relation`] impl
    /// can name `type Parent = L` / `type Target = R`.
    #[allow(clippy::type_complexity)]
    _phantom: PhantomData<fn() -> (L, R, P)>,
}

impl<L, R, P> MorphedByMany<L, R, P>
where
    L: EloquentModel,
    R: Model,
    R: From<<R::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <R::Entity as sea_orm::EntityTrait>::Model: From<R>
        + sea_orm::IntoActiveModel<<R::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <R::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<R::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
    P: Model + 'static,
    P: From<<P::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <P::Entity as sea_orm::EntityTrait>::Model: From<P>
        + sea_orm::IntoActiveModel<<P::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <P::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<P::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    /// Construct a `MorphedByMany`. Invoked by the macro-emitted
    /// relation method.
    #[doc(hidden)]
    pub fn __new(
        tag_key_value: serde_json::Value,
        target_morph_type: String,
        morph_name: String,
        pivot_table: String,
        pivot_foreign_key: String,
    ) -> Self {
        Self {
            tag_key_value,
            target_morph_type,
            morph_name,
            pivot_table,
            pivot_foreign_key,
            related_key: R::PRIMARY_KEY.into(),
            parent_key: L::PRIMARY_KEY.into(),
            scope_rewrite: None,
            pivot_filters: PivotFilters::default(),
            lazy_load: LazyLoadGuard::default(),
            _phantom: PhantomData,
        }
    }

    /// Attach the lazy-loading check of the row this relation was read
    /// from. Invoked by the macro-emitted relation method; not part of
    /// the public API.
    #[doc(hidden)]
    pub fn __lazy_load(mut self, guard: LazyLoadGuard) -> Self {
        self.lazy_load = guard;
        self
    }

    /// Override the related-side key column used by the JOIN in
    /// [`Self::get`]. Default: the related model's primary key.
    pub fn related_pk(mut self, key: impl Into<String>) -> Self {
        self.related_key = key.into();
        self
    }

    /// Override the tag-side (parent's) key column. Default: the tag
    /// model's primary key.
    pub fn local_key(mut self, key: impl Into<String>) -> Self {
        self.parent_key = key.into();
        self
    }

    pivot_filter_methods!(P);

    /// Validate the SQL identifiers that flow into raw-SQL statements.
    /// `morph_name` is the root of the derived column names; validating
    /// it covers both `<morph_name>_id` and `<morph_name>_type`.
    fn validate_meta(&self) -> Result<(), FrameworkError> {
        crate::database::validate_identifier(&self.morph_name)?;
        crate::database::validate_identifier(&self.pivot_table)?;
        crate::database::validate_identifier(&self.pivot_foreign_key)?;
        Ok(())
    }

    /// Fetch every R row attached to this Tag via the polymorphic
    /// pivot, filtered to the declared target morph family. Each
    /// returned R carries its pivot context via `__pivot` (accessible
    /// through the macro-emitted `.pivot::<P>()` accessor), matching
    /// the symmetric `MorphToMany::get()` contract.
    ///
    /// Two-query strategy. Query 1: SELECT pivot rows where the
    /// tag-side FK matches this Tag and the `<morph_name>_type` column
    /// matches the declared target morph type. Query 2: SELECT R rows
    /// by IN-set on those `<morph_name>_id` values. Zip via the pivot's
    /// `<morph_name>_id` column to stamp `__pivot` per R.
    ///
    /// Refused without a query when it is a lazy load that
    /// [lazy-loading prevention](crate::eloquent::lazy_loading) catches.
    pub async fn get(self) -> Result<Collection<R>, FrameworkError> {
        self.lazy_load.check()?;
        let id_col = format!("{}_id", self.morph_name);
        let type_col = format!("{}_type", self.morph_name);

        // Query 1: pivot rows, full row (we need the morph-id column
        // to zip + the rest for `__pivot` context), from the relation's
        // own pivot table - the one `count` reads - hydrated through P's
        // casts.
        let pivot_rows: Vec<P> = load_pivot_rows::<P>(
            &self.pivot_table,
            L::default_connection_name(),
            vec![
                (
                    self.pivot_foreign_key.clone(),
                    PivotMatch::Eq(self.tag_key_value.clone()),
                ),
                (
                    type_col.clone(),
                    PivotMatch::Eq(serde_json::Value::String(self.target_morph_type.clone())),
                ),
            ],
            &self.pivot_filters,
        )
        .await?;
        if pivot_rows.is_empty() {
            return Ok(Collection::new());
        }

        // Pull morph-target IDs out of the pivot rows.
        let mut target_ids: Vec<serde_json::Value> = Vec::with_capacity(pivot_rows.len());
        let mut seen_target: std::collections::HashSet<String> = std::collections::HashSet::new();
        for pv in pivot_rows.iter() {
            let pj = serde_json::to_value(pv).unwrap_or(serde_json::Value::Null);
            if let Some(v) = pj.get(&id_col) {
                let s = v.to_string();
                if seen_target.insert(s) {
                    target_ids.push(v.clone());
                }
            }
        }

        // Query 2: target rows by PK IN-set. The scope_rewrite hook
        // (set by `with_trashed` / `only_trashed` for soft-delete R)
        // runs against the inner builder before `.get()`.
        let target_rows: Vec<R> = {
            let mut q = R::query().filter_in(self.related_key.as_str(), target_ids);
            if let Some(rw) = self.scope_rewrite {
                q = rw(q);
            }
            q.get().await?.into_vec()
        };

        // Index pivots by `<morph_name>_id` (JSON-string form).
        use std::collections::HashMap;
        let mut by_target: HashMap<String, P> = HashMap::new();
        for pv in pivot_rows.into_iter() {
            let pj = serde_json::to_value(&pv).unwrap_or(serde_json::Value::Null);
            let key = pj.get(&id_col).map(|v| v.to_string()).unwrap_or_default();
            by_target.insert(key, pv);
        }

        // Stamp the pivot context onto each target row via the
        // `EagerLoadDispatch::set_pivot_arc` hook.
        let mut out: Vec<R> = Vec::with_capacity(target_rows.len());
        for r in target_rows.into_iter() {
            let r_json = serde_json::to_value(&r).unwrap_or(serde_json::Value::Null);
            let key = r_json
                .get(&self.related_key)
                .map(|v| v.to_string())
                .unwrap_or_default();
            let mut row = r;
            if let Some(pivot) = by_target.get(&key) {
                row.set_pivot_arc(Some(Arc::new(pivot.clone())));
            }
            out.push(row);
        }
        Ok(Collection::from_vec(out))
    }

    /// Convenience over `get()` - drop everything after the first row.
    pub async fn first(self) -> Result<Option<R>, FrameworkError> {
        Ok(self.get().await?.into_vec().into_iter().next())
    }

    /// `SELECT COUNT(*) FROM pivot WHERE pfk = ? AND <name>_type = ?`.
    pub async fn count(self) -> Result<i64, FrameworkError> {
        self.validate_meta()?;
        // Phase 10C audit-fix AF2 - route the count through
        // ExecutorChoice so it honors CURRENT_TX.
        let exec = ExecutorChoice::resolve_read(None, None, L::default_connection_name()).await?;
        let backend = exec.backend();
        let (id_ph, type_ph) = match backend {
            DatabaseBackend::Postgres => ("$1".to_string(), "$2".to_string()),
            _ => ("?".to_string(), "?".to_string()),
        };
        let type_col = format!("{}_type", self.morph_name);
        // `target_morph_type` is cloned rather than moved because the
        // filter renderer needs `&self` afterwards.
        let mut values: Vec<sea_orm::Value> = vec![
            json_value_to_sea_value(&self.tag_key_value),
            sea_orm::Value::from(self.target_morph_type.clone()),
        ];
        let mut bind_index: usize = 2;
        let pivot_predicates =
            self.pivot_filters
                .render_and(backend, &mut values, &mut bind_index)?;
        let sql = format!(
            "SELECT COUNT(*) AS __sn_count FROM {table} \
              WHERE {pfk} = {id_ph} AND {type_col} = {type_ph}{pivot_predicates}",
            table = self.pivot_table,
            pfk = self.pivot_foreign_key,
            type_col = type_col,
            id_ph = id_ph,
            type_ph = type_ph,
            pivot_predicates = pivot_predicates,
        );
        let stmt = Statement::from_sql_and_values(backend, &sql, values);
        let row = exec
            .query_one(stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        Ok(row
            .and_then(|r| r.try_get::<i64>("", "__sn_count").ok())
            .unwrap_or(0))
    }
}

/// Soft-delete scope modifiers for `MorphedByMany<L, R, P>` when the
/// related (`R`) side is soft-deletable. The pivot table itself is
/// never filtered for `deleted_at` (pivots are a join artefact);
/// only the related-row IN-set query gets the rewrite. Same
/// closure-erasure pattern as the
/// [`MorphToMany`] block above.
impl<L, R, P> MorphedByMany<L, R, P>
where
    L: EloquentModel,
    R: Model + crate::eloquent::SoftDeletes,
    R: From<<R::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <R::Entity as sea_orm::EntityTrait>::Model: From<R>
        + sea_orm::IntoActiveModel<<R::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <R::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<R::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
    P: Model + 'static,
    P: From<<P::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <P::Entity as sea_orm::EntityTrait>::Model: From<P>
        + sea_orm::IntoActiveModel<<P::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <P::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<P::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    /// Widen the related-row lookup to include trashed `R` rows.
    pub fn with_trashed(mut self) -> Self {
        self.scope_rewrite = Some(Box::new(|b: Builder<R>| b.with_trashed()));
        self
    }

    /// Restrict the related-row lookup to *only* trashed `R` rows.
    pub fn only_trashed(mut self) -> Self {
        self.scope_rewrite = Some(Box::new(|b: Builder<R>| b.only_trashed()));
        self
    }
}

impl<L, R, P> Relation for MorphedByMany<L, R, P>
where
    L: EloquentModel,
    R: Model,
    R: From<<R::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <R::Entity as sea_orm::EntityTrait>::Model: From<R>
        + sea_orm::IntoActiveModel<<R::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <R::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<R::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
    P: Model,
    P: From<<P::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <P::Entity as sea_orm::EntityTrait>::Model: From<P>
        + sea_orm::IntoActiveModel<<P::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <P::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<P::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    type Parent = L;
    type Target = R;
    const KIND: RelationKind = RelationKind::MorphedByMany;

    fn parent_key(&self) -> &str {
        &self.parent_key
    }

    fn foreign_key(&self) -> &str {
        &self.pivot_foreign_key
    }
}

// ---- Internal helpers ----------------------------------------------------

/// A polymorphic pivot as its statements address it: the pivot target,
/// whose `foreign_key` is the `<morph_name>_id` column, plus the
/// `<morph_name>_type` column and the parent's morph type it must equal.
struct MorphPivot<'a> {
    /// The pivot table, its key columns and their binders.
    target: PivotTarget<'a>,
    /// The `<morph_name>_type` column.
    type_col: &'a str,
    /// The string the type column holds for this parent.
    morph_type: &'a str,
}

/// INSERT path shared by `attach` / `attach_with` / `sync`. Same
/// connection-or-transaction abstraction as BelongsToMany, and the same
/// binding: each id and extra binds by the type its column has, and a
/// `u64` a signed column cannot hold is refused before anything is sent.
async fn morph_attach_one<C: ConnectionTrait>(
    conn: &C,
    pivot: &MorphPivot<'_>,
    parent_id: &serde_json::Value,
    related_id: &serde_json::Value,
    extra: Vec<PivotExtra>,
    with_timestamps: bool,
) -> Result<(), FrameworkError> {
    let backend = conn.get_database_backend();
    let target = &pivot.target;
    let pivot_table = target.table;
    let pivot_related_key = target.related_key;
    let id_col = target.foreign_key;
    let type_col = pivot.type_col;

    let mut columns: Vec<String> = vec![
        pivot_related_key.to_string(),
        id_col.to_string(),
        type_col.to_string(),
    ];
    // `None` represents an explicit JSON null from pivot extras. Framework-
    // managed IDs, morph type, and timestamps remain bound values.
    let mut values: Vec<Option<sea_orm::Value>> = vec![
        Some(
            bind_pivot_write(
                conn,
                pivot_table,
                pivot_related_key,
                target.typed(pivot_related_key, Some(target.related), related_id),
                related_id,
            )
            .await?,
        ),
        Some(
            bind_pivot_write(
                conn,
                pivot_table,
                id_col,
                target.typed(id_col, Some(target.parent), parent_id),
                parent_id,
            )
            .await?,
        ),
        Some(sea_orm::Value::from(pivot.morph_type.to_string())),
    ];
    // Extras arrive encoded through the pivot model's casts, their
    // names checked and the framework-written columns dropped - see
    // `belongs_to_many::pivot_extras_through_casts`.
    for (column, value) in extra {
        let bound = bind_pivot_extra(conn, pivot_table, &column, value).await?;
        columns.push(column);
        values.push(bound);
    }
    if with_timestamps {
        let now = crate::clock::now();
        if !columns.iter().any(|c| c == "created_at") {
            columns.push("created_at".to_string());
            values.push(Some(sea_orm::Value::ChronoDateTimeUtc(Some(now))));
        }
        if !columns.iter().any(|c| c == "updated_at") {
            columns.push("updated_at".to_string());
            values.push(Some(sea_orm::Value::ChronoDateTimeUtc(Some(now))));
        }
    }

    // PostgreSQL cannot infer a non-text target type from the text-typed null
    // produced by `json_value_to_sea_value`. Render explicit null extras as a
    // constant and number only values that are actually bound.
    let mut bound_values = Vec::with_capacity(values.len());
    let mut bind_position = 0;
    let value_expressions: Vec<String> = values
        .into_iter()
        .map(|value| match value {
            Some(value) => {
                bind_position += 1;
                let ph = crate::database::placeholder::typed_placeholder(
                    backend,
                    bind_position,
                    &value,
                )?;
                bound_values.push(value);
                Ok(ph)
            }
            None => Ok("NULL".to_string()),
        })
        .collect::<Result<_, FrameworkError>>()?;

    let sql = format!(
        "INSERT INTO {table} ({cols}) VALUES ({phs})",
        table = pivot_table,
        cols = columns.join(", "),
        phs = value_expressions.join(", "),
    );
    let stmt = Statement::from_sql_and_values(backend, &sql, bound_values);
    conn.execute_raw(stmt)
        .await
        .map_err(|e| FrameworkError::database(e.to_string()))?;
    Ok(())
}

/// DELETE path shared by `detach` / `sync`. Filters by all three of
/// `pivot_related_key = related_id`, `<morph_name>_id = parent_id`,
/// and `<morph_name>_type = parent_morph_type`. An id no pivot row can
/// hold deletes nothing, and is not sent.
async fn morph_detach_one<C: ConnectionTrait>(
    conn: &C,
    pivot: &MorphPivot<'_>,
    parent_id: &serde_json::Value,
    related_id: &serde_json::Value,
) -> Result<(), FrameworkError> {
    let backend = conn.get_database_backend();
    let target = &pivot.target;
    let related = bind_pivot_comparison(
        backend,
        target.typed(target.related_key, Some(target.related), related_id),
        related_id,
    );
    let parent = bind_pivot_comparison(
        backend,
        target.typed(target.foreign_key, Some(target.parent), parent_id),
        parent_id,
    );
    let (Some(related), Some(parent)) = (related, parent) else {
        return Ok(());
    };
    let ph1 = crate::database::placeholder::typed_placeholder(backend, 1, &related)?;
    let ph2 = crate::database::placeholder::typed_placeholder(backend, 2, &parent)?;
    let ph3 = match backend {
        DatabaseBackend::Postgres => "$3".to_string(),
        _ => "?".to_string(),
    };
    let sql = format!(
        "DELETE FROM {table} WHERE {rk} = {ph1} AND {id_col} = {ph2} AND {type_col} = {ph3}",
        table = target.table,
        rk = target.related_key,
        id_col = target.foreign_key,
        type_col = pivot.type_col,
    );
    let stmt = Statement::from_sql_and_values(
        backend,
        &sql,
        vec![
            related,
            parent,
            sea_orm::Value::from(pivot.morph_type.to_string()),
        ],
    );
    conn.execute_raw(stmt)
        .await
        .map_err(|e| FrameworkError::database(e.to_string()))?;
    Ok(())
}
