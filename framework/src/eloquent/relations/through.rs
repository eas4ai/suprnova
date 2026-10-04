//! `HasManyThrough` / `HasOneThrough` - two-hop relations.
//!
//! Mirrors Laravel's
//! [`hasManyThrough`](https://laravel.com/docs/12.x/eloquent-relationships#has-many-through)
//! and [`hasOneThrough`](https://laravel.com/docs/12.x/eloquent-relationships#has-one-through)
//! semantics: traverse `A -> B -> C` where `B` is an intermediate
//! model whose FK column points at `A`, and `C` is the final target
//! whose FK column points at `B`.
//!
//! ## Scopes and soft deletes
//!
//! Through relations use raw `INNER JOIN` SQL rather than the
//! `Builder<C>` pipeline, but the target side of the join is `C`'s own
//! scoped query: `C`'s soft-delete filter and its registered global
//! scopes apply, as they apply to `C::query()`, to the eager load, and
//! to `with_count`. The intermediate `B` is filtered by its soft-delete
//! column only, reading [`EloquentModel::SOFT_DELETES_COLUMN`]:
//!
//! - Empty `SOFT_DELETES_COLUMN` (the trait default for models that
//!   don't opt into `#[model(soft_deletes)]`) emits no clause.
//! - Non-empty column appends `AND <table>.<col> IS NULL` to the
//!   `WHERE` clause.
//!
//! This matches Laravel's `hasManyThrough`, whose query is the target
//! model's builder - so its global scopes apply - joined to the
//! intermediate, which is filtered by `deleted_at IS NULL` when it
//! declares `SoftDeletes`.
//!
//! Callers that need to *include* trashed rows in a Through traversal
//! (the inverse of the default - Laravel's `->withTrashed()`) fall
//! back to chaining the two relations explicitly:
//!
//! ```ignore
//! // Instead of `country.posts().get()`, do:
//! let users = country.users().get().await?;
//! let user_ids: Vec<i64> = users.iter().map(|u| u.id).collect();
//! let posts = Post::query().filter_in("user_id", user_ids).get().await?;
//! // Both User and Post scopes apply to their respective query.
//! ```
//!
//! Laravel example: `Country` has many `User`s and `User` has many
//! `Post`s. `Country::posts()` is a HasManyThrough that returns every
//! `Post` belonging to any `User` in that country - a two-hop traversal.
//!
//! Default key conventions:
//!
//! - `first_key` (column on `B` pointing at `A`): `<snake(A)>_id`
//! - `second_key` (column on `C` pointing at `B`): `<snake(B)>_id`
//! - `local_key` (column on `A` matched by `first_key`): `A`'s primary
//!   key
//! - `second_local_key` (column on `B` matched by `second_key`): `B`'s
//!   primary key, as Laravel's `secondLocalKey` defaults to the
//!   intermediate's key name
//!
//! All four customisable via the macro's `first_key = "..."` /
//! `second_key = "..."` / `lk = "..."` / `second_local_key = "..."`
//! options. The chainable [`HasManyThrough::second_local_key`] setter
//! is also available at runtime - useful for tooling that constructs
//! the relation outside the `#[suprnova::model]` declaration.
//!
//! The terminal `.get()` / `.first()` / `.count()` issue a single
//! `INNER JOIN` query - one round trip per call, regardless of fan-out.
//! Eager loading is split across two queries (see the macro-emitted
//! dispatcher arm in `suprnova-macros/src/model/relations.rs`) to keep
//! the SeaORM deserialisation path homogeneous: the framework
//! deserialises `C` rows via the existing `Builder<C>` pipeline rather
//! than a raw JSON-row split.

use std::marker::PhantomData;

use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

use crate::database::transaction::ExecutorChoice;
use crate::eloquent::EloquentModel;
use crate::eloquent::builder::Builder;
use crate::eloquent::collection::Collection;
use crate::eloquent::lazy_loading::LazyLoadGuard;
use crate::eloquent::model::{Model, json_value_to_sea_value};
use crate::eloquent::relations::{Relation, RelationKind};
use crate::error::FrameworkError;

/// Two-hop one-to-many relation from parent `A` through intermediate
/// `B` to target `C`. Constructed by the macro-emitted relation method
/// (`fn posts(&self) -> HasManyThrough<Self, User, Post>`); user code
/// never calls [`HasManyThrough::__new`] directly.
///
/// The wrapper carries the key metadata + the parent's local-key
/// value, all paid up at construction time. Terminal methods
/// (`get`, `first`, `count`) issue the SQL.
pub struct HasManyThrough<A, B, C>
where
    A: EloquentModel,
    B: EloquentModel,
    C: Model,
    C: From<<C::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <C::Entity as sea_orm::EntityTrait>::Model: From<C>
        + sea_orm::IntoActiveModel<<C::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <C::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<C::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    /// Parent row's local-key value, JSON-encoded. Matches the rest of
    /// the relation surface - `HasMany`, `HasOne`, `BelongsToMany` all
    /// store the parent key as `serde_json::Value` so the runtime path
    /// stays homogeneous regardless of the PK shape (`i64`, `String`,
    /// `Uuid`-via-string). The conversion to `sea_orm::Value` happens
    /// at the SQL binding boundary via [`json_value_to_sea_value`].
    parent_key_value: serde_json::Value,
    /// Column on `B` pointing at `A`. Default: `<snake(A)>_id`.
    first_key: String,
    /// Column on `C` pointing at `B`. Default: `<snake(B)>_id`.
    second_key: String,
    /// Column on `A` matched by `first_key`. Default: `A`'s primary
    /// key. Only affects the [`Relation::parent_key`] metadata - the
    /// runtime value was already extracted at construction.
    local_key: String,
    /// Column on `B` matched by `second_key`. Default: `B`'s primary
    /// key. Drives the `INNER JOIN ... ON C.{second_key} =
    /// B.{second_local_key}` predicate.
    second_local_key: String,
    /// The lazy-loading check [`Self::get`] runs before its query
    /// ([`Self::first`] goes through it). Set by the macro-emitted
    /// relation method.
    lazy_load: LazyLoadGuard,
    /// PhantomData carries `A`, `B`, `C` so the [`Relation`] impl can
    /// name `type Parent = A` / `type Target = C` without runtime
    /// fields. `fn() -> (A, B, C)` keeps the type covariant +
    /// `Send + Sync` regardless of the parameters.
    #[allow(clippy::type_complexity)]
    _phantom: PhantomData<fn() -> (A, B, C)>,
}

impl<A, B, C> HasManyThrough<A, B, C>
where
    A: EloquentModel,
    B: EloquentModel,
    C: Model,
    C: From<<C::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <C::Entity as sea_orm::EntityTrait>::Model: From<C>
        + sea_orm::IntoActiveModel<<C::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <C::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<C::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    /// Construct a `HasManyThrough`. Invoked by the macro-emitted
    /// relation method; not part of the public API.
    ///
    /// `parent_key_value` is the JSON-serialised parent PK
    /// (e.g. `serde_json::to_value(&self.id)`). `first_key` and
    /// `second_key` come from macro-resolved defaults or user-supplied
    /// `first_key = "..."` / `second_key = "..."` options.
    #[doc(hidden)]
    pub fn __new(
        parent_key_value: serde_json::Value,
        first_key: String,
        second_key: String,
    ) -> Self {
        Self {
            parent_key_value,
            first_key,
            second_key,
            local_key: A::PRIMARY_KEY.into(),
            second_local_key: B::PRIMARY_KEY.into(),
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

    /// Override the column on `B` pointing at `A`.
    pub fn first_key(mut self, key: impl Into<String>) -> Self {
        self.first_key = key.into();
        self
    }

    /// Override the column on `C` pointing at `B`.
    pub fn second_key(mut self, key: impl Into<String>) -> Self {
        self.second_key = key.into();
        self
    }

    /// Override the column on `A` matched by `first_key`. Only updates
    /// metadata - the runtime parent value was extracted at
    /// construction. Mirrors the [`crate::eloquent::HasMany::local_key`]
    /// setter shape.
    pub fn local_key(mut self, key: impl Into<String>) -> Self {
        self.local_key = key.into();
        self
    }

    /// Override the column on `B` matched by `second_key`. Drives the
    /// `INNER JOIN ... ON C.{second_key} = B.{second_local_key}`
    /// predicate.
    pub fn second_local_key(mut self, key: impl Into<String>) -> Self {
        self.second_local_key = key.into();
        self
    }

    /// Validate the three SQL identifiers that flow unquoted into the
    /// JOIN SQL rendered by `render_join_sql`.
    /// Called at the top of every terminal method.
    fn validate_meta(&self) -> Result<(), FrameworkError> {
        crate::database::validate_identifier(&self.first_key)?;
        crate::database::validate_identifier(&self.second_key)?;
        crate::database::validate_identifier(&self.second_local_key)?;
        Ok(())
    }

    /// Fetch every `C` row reachable from this parent through `B`.
    ///
    /// Issues a single `INNER JOIN` query:
    ///
    /// ```sql
    /// SELECT c.*
    ///   FROM <C's scoped rows> c
    ///  INNER JOIN <B> b
    ///     ON c.<second_key> = b.<second_local_key>
    ///  WHERE b.<first_key> = ?
    ///    AND b.<B::SOFT_DELETES_COLUMN> IS NULL  -- if B opts in
    /// ```
    ///
    /// `C`'s scoped rows are `C::query()`'s: its soft-delete filter and
    /// its global scopes apply, as they do to the eager load.
    ///
    /// Backend-aware placeholders (`?` for sqlite / mysql, `$1` for
    /// postgres) match the rest of the framework's raw-SQL paths.
    /// Routes through
    /// [`ExecutorChoice::resolve_read`](crate::database::transaction::ExecutorChoice::resolve_read)
    /// so the query honours an ambient `CURRENT_TX`, the final
    /// target's `#[model(connection = "...")]` default, and the
    /// read-replica auto-routing chain.
    ///
    /// Refused without a query when it is a lazy load that
    /// [lazy-loading prevention](crate::eloquent::lazy_loading) catches.
    pub async fn get(self) -> Result<Collection<C>, FrameworkError> {
        self.lazy_load.check()?;
        self.validate_meta()?;
        let exec = ExecutorChoice::resolve_read(
            None,
            None,
            <C as EloquentModel>::default_connection_name(),
        )
        .await?;
        let backend = exec.backend();
        let (sql, values) = self.render_join_sql(backend, "SELECT {c}.*")?;
        let stmt = Statement::from_sql_and_values(backend, sql, values);
        use sea_orm::EntityTrait;
        let find = <C as EloquentModel>::Entity::find().from_raw_sql(stmt);
        let rows = match &exec {
            ExecutorChoice::Tx(t, _) => find.all(t.as_ref()).await,
            ExecutorChoice::Pool(c, _) => find.all(c.inner()).await,
        }
        .map_err(|e| FrameworkError::database(e.to_string()))?;
        let mut rows: Vec<C> = rows.into_iter().map(C::from).collect();
        // The JOIN hydrates here rather than through `Builder::get`, so
        // it sets the mark of a multi-row query itself.
        C::__mark_query_result(&mut rows);
        Ok(Collection::from_vec(rows))
    }

    /// Convenience over `.get()` - drop everything after the first row.
    pub async fn first(self) -> Result<Option<C>, FrameworkError> {
        Ok(self.get().await?.into_vec().into_iter().next())
    }

    /// `SELECT COUNT(*) FROM <C> INNER JOIN <B> ... WHERE B.<first_key> = ?`.
    /// Returns `i64` to match [`crate::eloquent::HasMany::count`] and
    /// [`crate::eloquent::BelongsToMany::count`]. Routes through
    /// [`ExecutorChoice::resolve_read`](crate::database::transaction::ExecutorChoice::resolve_read)
    /// on the same terms as [`Self::get`], over the same rows: `C`'s
    /// scopes and soft-delete filter, and `B`'s soft-delete filter.
    pub async fn count(self) -> Result<i64, FrameworkError> {
        self.validate_meta()?;
        let exec = ExecutorChoice::resolve_read(
            None,
            None,
            <C as EloquentModel>::default_connection_name(),
        )
        .await?;
        let backend = exec.backend();
        let (sql, values) = self.render_join_sql(backend, "SELECT COUNT(*) AS __sn_count")?;
        let stmt = Statement::from_sql_and_values(backend, sql, values);
        let row = match &exec {
            ExecutorChoice::Tx(t, _) => t.query_one_raw(stmt).await,
            ExecutorChoice::Pool(c, _) => c.inner().query_one_raw(stmt).await,
        }
        .map_err(|e| FrameworkError::database(e.to_string()))?;
        Ok(row
            .and_then(|r| r.try_get::<i64>("", "__sn_count").ok())
            .unwrap_or(0))
    }

    /// Render the JOIN statement both terminals run, `projection`
    /// first, and the values it binds.
    ///
    /// ```sql
    /// <projection>
    ///   FROM <C's scoped rows> c
    ///  INNER JOIN <B> b
    ///     ON c.<second_key> = b.<second_local_key>
    ///  WHERE b.<first_key> = ?
    ///    AND b.<B::SOFT_DELETES_COLUMN> IS NULL  -- if B opts in
    /// ```
    ///
    /// `C`'s side is `C`'s own scoped query, aliased with its table
    /// name, so its soft-delete filter and global scopes apply and its
    /// binds come first. `{c}` in `projection` names that alias.
    fn render_join_sql(
        &self,
        backend: DatabaseBackend,
        projection: &str,
    ) -> Result<(String, Vec<sea_orm::Value>), FrameworkError> {
        let b_table = <B as EloquentModel>::TABLE;
        let c_table = <C as EloquentModel>::TABLE;
        let (c_source, mut values) = Builder::<C>::__relation_source(backend, c_table)?;
        values.push(json_value_to_sea_value(&self.parent_key_value));
        let ph = crate::database::__macro_support::placeholder(backend, values.len())?;
        let mut sql = format!(
            "{projection} FROM {c_source} INNER JOIN {b} \
             ON {c}.{second_key} = {b}.{second_local_key} \
             WHERE {b}.{first_key} = {ph}",
            projection = projection.replace("{c}", c_table),
            c = c_table,
            b = b_table,
            second_key = self.second_key,
            second_local_key = self.second_local_key,
            first_key = self.first_key,
        );
        sql.push_str(&crate::eloquent::relations::__soft_delete_guard::<B>(
            b_table,
        ));
        Ok((sql, values))
    }
}

impl<A, B, C> Relation for HasManyThrough<A, B, C>
where
    A: EloquentModel,
    B: EloquentModel,
    C: Model,
    C: From<<C::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <C::Entity as sea_orm::EntityTrait>::Model: From<C>
        + sea_orm::IntoActiveModel<<C::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <C::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<C::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    type Parent = A;
    type Target = C;
    const KIND: RelationKind = RelationKind::HasManyThrough;

    fn parent_key(&self) -> &str {
        &self.local_key
    }

    fn foreign_key(&self) -> &str {
        &self.first_key
    }
}

/// Two-hop one-to-one relation. Same key mechanics as
/// [`HasManyThrough`] but the terminal methods return at most one row.
///
/// Internally delegates to [`HasManyThrough`] and takes the first row;
/// the wrapper exists so the [`Relation::KIND`] constant resolves to
/// [`RelationKind::HasOneThrough`] for admin tooling and dispatchers
/// that branch on kind.
pub struct HasOneThrough<A, B, C>
where
    A: EloquentModel,
    B: EloquentModel,
    C: Model,
    C: From<<C::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <C::Entity as sea_orm::EntityTrait>::Model: From<C>
        + sea_orm::IntoActiveModel<<C::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <C::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<C::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    inner: HasManyThrough<A, B, C>,
}

impl<A, B, C> HasOneThrough<A, B, C>
where
    A: EloquentModel,
    B: EloquentModel,
    C: Model,
    C: From<<C::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <C::Entity as sea_orm::EntityTrait>::Model: From<C>
        + sea_orm::IntoActiveModel<<C::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <C::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<C::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    /// Construct a `HasOneThrough`. Invoked by the macro-emitted
    /// relation method; not part of the public API.
    #[doc(hidden)]
    pub fn __new(
        parent_key_value: serde_json::Value,
        first_key: String,
        second_key: String,
    ) -> Self {
        Self {
            inner: HasManyThrough::__new(parent_key_value, first_key, second_key),
        }
    }

    /// Attach the lazy-loading check of the row this relation was read
    /// from. Held by the inner relation, whose read both [`Self::first`]
    /// and [`Self::get`] go through. Invoked by the macro-emitted
    /// relation method; not part of the public API.
    #[doc(hidden)]
    pub fn __lazy_load(mut self, guard: LazyLoadGuard) -> Self {
        self.inner = self.inner.__lazy_load(guard);
        self
    }

    /// Override the column on `B` pointing at `A`.
    pub fn first_key(mut self, key: impl Into<String>) -> Self {
        self.inner = self.inner.first_key(key);
        self
    }

    /// Override the column on `C` pointing at `B`.
    pub fn second_key(mut self, key: impl Into<String>) -> Self {
        self.inner = self.inner.second_key(key);
        self
    }

    /// Override the column on `A` matched by `first_key`.
    pub fn local_key(mut self, key: impl Into<String>) -> Self {
        self.inner = self.inner.local_key(key);
        self
    }

    /// Override the column on `B` matched by `second_key`.
    pub fn second_local_key(mut self, key: impl Into<String>) -> Self {
        self.inner = self.inner.second_local_key(key);
        self
    }

    /// Fetch the first matching `C` row reachable from this parent.
    ///
    /// Equivalent to `.get()` for HasOne semantics - at most one row.
    pub async fn first(self) -> Result<Option<C>, FrameworkError> {
        self.inner.first().await
    }

    /// Fetch the matching `C` row (HasOne semantics - at most one row).
    /// Returns `None` when no `C` row is reachable.
    pub async fn get(self) -> Result<Option<C>, FrameworkError> {
        Ok(self.inner.get().await?.into_vec().into_iter().next())
    }
}

impl<A, B, C> Relation for HasOneThrough<A, B, C>
where
    A: EloquentModel,
    B: EloquentModel,
    C: Model,
    C: From<<C::Entity as sea_orm::EntityTrait>::Model>
        + serde::Serialize
        + serde::de::DeserializeOwned
        + crate::eloquent::EagerLoadDispatch,
    <C::Entity as sea_orm::EntityTrait>::Model: From<C>
        + sea_orm::IntoActiveModel<<C::Entity as sea_orm::EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + serde::Serialize
        + Send
        + Sync,
    <C::Entity as sea_orm::EntityTrait>::ActiveModel: Send,
    <<C::Entity as sea_orm::EntityTrait>::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    type Parent = A;
    type Target = C;
    const KIND: RelationKind = RelationKind::HasOneThrough;

    fn parent_key(&self) -> &str {
        self.inner.parent_key()
    }

    fn foreign_key(&self) -> &str {
        self.inner.foreign_key()
    }
}
