//! Polymorphic relations - `MorphTo` / `MorphOne` / `MorphMany`.
//!
//! `MorphTo` lives on the morph-table side (e.g. `Comment.commentable`):
//! a polymorphic FK column pair (`commentable_id` + `commentable_type`)
//! that points at a row in one of several "parent" tables. Because the
//! parent type varies per row, the macro emits a per-family enum
//! (`CommentableMorph { MorphPost(MorphPost), MorphVideo(MorphVideo),
//! Unknown(String, serde_json::Value) }`) at the declaration site. The
//! runtime [`MorphTo<C>`] struct in this module is only a metadata
//! carrier for the `RelationEntry` inventory + the user-side `pub use`
//! re-export; it does NOT participate in the per-family dispatch (that
//! happens in the macro-generated `<Name>MorphFetch::get` async method).
//!
//! The id of a morph target travels as the JSON value of its primary
//! key, the same form the keyset cursor of
//! [`Builder::chunk_by_id`] carries and the form
//! [`Model::field_value`] returns. Every morph relation binds that value
//! as it is, so a target may have an `i64`, `String`, UUID or ULID key.
//! The targets of one `MorphTo` relation share one key type, and the
//! child's `<name>_id` field has that type: the macro checks both at
//! compile time, through [`MorphTargetsShareKey`] and
//! [`MorphIdColumnHoldsKey`].
//!
//! [`MorphOne`] / [`MorphMany`] live on the parent side. They mirror
//! [`HasOne`](super::HasOne) / [`HasMany`](super::HasMany) but layer
//! the morph-type discriminator on top - the inner [`Builder<R>`] is
//! pre-filtered with both `<morph>_id = <parent_id>` and
//! `<morph>_type = <parent_morph_type>`, so polymorphic children
//! pointing at OTHER families never appear in `.get()` / `.first()` /
//! `.count()` results.
//!
//! Eager-load orchestration lives in the parent model's
//! `__eager_load("<rel>", ...)` match arm; this module's structs only
//! handle the lazy `.first()` / `.get()` / `.count()` path. The
//! parent-side relation method also passes the parent's
//! `morph_type = "..."` attribute value into the morph runtime (so the
//! filter knows which type-string to send).
//!
//! ## Lazy and eager loading of a `MorphTo`
//!
//! The lazy read (`comment.commentable().get()`) finds the target by its
//! key and applies no global scope of the target: a target hidden by a
//! global scope is found, and only a soft-delete target stays scoped,
//! because its `find` is. The eager load (`with(["commentable"])`) runs
//! the target's own query and applies its global scopes, so a target
//! hidden by a scope comes back as the `Unknown` variant. For a target
//! without a global scope the two agree.

use std::any::TypeId;
use std::marker::PhantomData;

use crate::eloquent::EloquentModel;
use crate::eloquent::builder::{Builder, Direction, IntoColumn, IntoVal};
use crate::eloquent::collection::Collection;
use crate::eloquent::lazy_loading::LazyLoadGuard;
use crate::eloquent::model::Model;
use crate::eloquent::relations::{Relation, RelationKind};
use crate::error::FrameworkError;

/// One-to-many morph relation from parent `L` to children `R`. The
/// parent declares this as `MorphMany<Child> { name = "..." }`, e.g.
/// `comments: MorphMany<Comment> { name = "commentable" }` on `Post`
/// + `Video`.
///
/// Constructed by the macro-emitted relation method
/// (`fn comments(&self) -> MorphMany<Self, Comment>`); user code never
/// calls [`MorphMany::__new`] directly.
///
/// The wrapper carries the parent PK value, the morph-name (which
/// controls the `<name>_id` + `<name>_type` column names), the
/// parent's morph-type string, and a pre-filtered [`Builder<R>`]
/// targeting the child table. Chaining `filter` / `order_by` / `limit`
/// forwards onto that builder.
pub struct MorphMany<L, R>
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
{
    /// Parent row's PK value, JSON-encoded. Same reasoning as
    /// [`HasMany`](super::HasMany) - JSON is the
    /// [`Builder::filter`] storage shape, and converting once at
    /// construction keeps the wrapper's chainable surface free of
    /// `T: IntoVal` bounds.
    ///
    /// Stored on the struct (rather than consumed at construction)
    /// because future overrides like `.parent_key(...)` would need to
    /// rebuild the inner builder, and admin tooling reading the
    /// [`Relation`] surface needs to surface this value alongside the
    /// morph metadata.
    #[allow(dead_code)]
    parent_key_value: serde_json::Value,
    /// Morph family name - e.g. `"commentable"`. Controls both the
    /// `<name>_id` and `<name>_type` column names on the child table.
    /// Read by the [`Relation`] impl + the eager-load dispatcher.
    morph_name: String,
    /// What the PARENT registers as in the child's `<name>_type`
    /// column. Defaults to `to_snake(struct_name)` at the macro
    /// emission site; can be overridden per-struct via
    /// `#[model(morph_type = "...")]`. Stored for the [`Relation`]
    /// impl + admin introspection; consumed once at construction
    /// (cloned into the inner builder's WHERE clause).
    #[allow(dead_code)]
    morph_type_value: String,
    /// Pre-filtered builder against the child table - both
    /// `<name>_id = <parent_id>` AND `<name>_type = <morph_type>`
    /// applied at construction.
    inner: Builder<R>,
    /// The lazy-loading check [`Self::first`] and [`Self::get`] run
    /// before their query. Set by the macro-emitted relation method.
    lazy_load: LazyLoadGuard,
    /// PhantomData carries the parent type so the [`Relation`] impl
    /// can name `type Parent = L` without a runtime field.
    _phantom: PhantomData<fn() -> L>,
}

impl<L, R> MorphMany<L, R>
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
{
    /// Construct a `MorphMany` from the parent row's PK value + morph
    /// name + the parent's morph-type string. Invoked by the macro-
    /// emitted relation method; not part of the public API.
    ///
    /// `morph_name` controls the `<name>_id` + `<name>_type` columns.
    /// `morph_type_value` is the string the parent expects to see in
    /// the child's `<name>_type` column.
    #[doc(hidden)]
    pub fn __new(
        parent_key_value: serde_json::Value,
        morph_name: String,
        morph_type_value: String,
    ) -> Self {
        let id_col = format!("{morph_name}_id");
        let type_col = format!("{morph_name}_type");
        // Builder::filter() takes anything `IntoVal`, which is the same
        // JSON-shaped path HasMany / HasOne use - we wrap the
        // type-string in `serde_json::Value::String` so the inner
        // WhereTerm storage stays homogeneous with the rest of the
        // dual-API.
        let type_val = serde_json::Value::String(morph_type_value.clone());
        let inner = R::query()
            .filter(id_col.as_str(), parent_key_value.clone())
            .filter(type_col.as_str(), type_val);
        Self {
            parent_key_value,
            morph_name,
            morph_type_value,
            inner,
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

    /// Chainable `WHERE col = val` on the inner builder. Same shape
    /// as [`Builder::filter`]; the dual-API alias [`Self::db_where`]
    /// forwards here.
    pub fn filter(mut self, col: impl IntoColumn, val: impl IntoVal) -> Self {
        self.inner = self.inner.filter(col, val);
        self
    }

    /// Laravel-shape alias for [`Self::filter`].
    pub fn db_where(self, col: impl IntoColumn, val: impl IntoVal) -> Self {
        self.filter(col, val)
    }

    /// Chainable `ORDER BY col <dir>` on the inner builder.
    pub fn order_by(mut self, col: impl IntoColumn, dir: Direction) -> Self {
        self.inner = self.inner.order_by(col, dir);
        self
    }

    /// `ORDER BY created_at DESC` - Laravel-shape sugar. Only resolves
    /// against children that declare a `created_at` column.
    pub fn latest(self) -> Self {
        self.order_by("created_at", Direction::Desc)
    }

    /// `ORDER BY created_at ASC` - Laravel-shape sugar.
    pub fn oldest(self) -> Self {
        self.order_by("created_at", Direction::Asc)
    }

    /// `LIMIT n` on the inner builder.
    pub fn limit(mut self, n: u64) -> Self {
        self.inner = self.inner.limit(n);
        self
    }

    /// Laravel-shape alias for [`Self::limit`].
    pub fn take(self, n: u64) -> Self {
        self.limit(n)
    }

    /// Execute and return the first matching child row.
    ///
    /// Refused without a query when it is a lazy load that
    /// [lazy-loading prevention](crate::eloquent::lazy_loading) catches.
    pub async fn first(self) -> Result<Option<R>, FrameworkError> {
        self.lazy_load.check()?;
        self.inner.first().await
    }

    /// Execute and return every matching child row.
    ///
    /// Returns a [`Collection<R>`](crate::eloquent::Collection); see
    /// [`HasMany::get`](super::HasMany::get) for return-type
    /// rationale. Checked for lazy loading as [`Self::first`] is.
    pub async fn get(self) -> Result<Collection<R>, FrameworkError> {
        self.lazy_load.check()?;
        self.inner.get().await
    }

    /// Count children. Returns `i64` to match the inner
    /// [`Builder::count`] surface. Server-side aggregation through the
    /// builder - no client-side row buffering.
    pub async fn count(self) -> Result<i64, FrameworkError> {
        self.inner.count().await
    }
}

/// Soft-delete forwarding for `MorphMany<L, R>` when `R: SoftDeletes`.
/// Mirrors [`HasMany`](super::HasMany)'s equivalent block - both wrap
/// an inner `Builder<R>` and need only one-liner forwarding to the
/// underlying `Builder::with_trashed` / `only_trashed`.
impl<L, R> MorphMany<L, R>
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
{
    /// Widen the relation to include trashed children.
    pub fn with_trashed(mut self) -> Self {
        self.inner = self.inner.with_trashed();
        self
    }

    /// Restrict the relation to *only* trashed children.
    pub fn only_trashed(mut self) -> Self {
        self.inner = self.inner.only_trashed();
        self
    }
}

impl<L, R> Relation for MorphMany<L, R>
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
{
    type Parent = L;
    type Target = R;
    const KIND: RelationKind = RelationKind::MorphMany;

    fn parent_key(&self) -> &str {
        // Polymorphic relations always join the parent's PK (`id`)
        // against the child's `<name>_id` column. The macro doesn't
        // currently expose a parent-key override on morph relations
        // (the morph runtime keys are baked into the column-name
        // construction in `__new`); if a non-`id` parent PK is needed
        // the parent model declares it via `primary_key = "..."` and
        // the macro reads that when populating the inner builder, not
        // through this accessor.
        "id"
    }

    fn foreign_key(&self) -> &str {
        // Surface the morph name as the "foreign key" name - admin
        // tooling reading the [`RelationEntry`](super::RelationEntry)
        // surfaces this as the child-side column root; the actual
        // column is `<morph_name>_id`. The morph-type discriminator
        // (`<morph_name>_type`) is implicit.
        &self.morph_name
    }
}

/// Single-row morph relation from parent `L` to child `R`. Same shape
/// as [`MorphMany`] internally - pre-filtered builder with both
/// `<name>_id` and `<name>_type` predicates applied - but the public
/// surface returns `Option<R>` from `.first()` rather than a Vec from
/// `.get()`.
///
/// Constructed by the macro-emitted relation method
/// (`fn profile_image(&self) -> MorphOne<Self, Image>`); user code
/// never calls [`MorphOne::__new`] directly.
pub struct MorphOne<L, R>
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
{
    inner: MorphMany<L, R>,
}

impl<L, R> MorphOne<L, R>
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
{
    /// Construct a `MorphOne`. Invoked by the macro-emitted relation
    /// method.
    #[doc(hidden)]
    pub fn __new(
        parent_key_value: serde_json::Value,
        morph_name: String,
        morph_type_value: String,
    ) -> Self {
        Self {
            inner: MorphMany::__new(parent_key_value, morph_name, morph_type_value),
        }
    }

    /// Attach the lazy-loading check of the row this relation was read
    /// from. Held by the inner relation, whose read [`Self::first`] goes
    /// through. Invoked by the macro-emitted relation method; not part
    /// of the public API.
    #[doc(hidden)]
    pub fn __lazy_load(mut self, guard: LazyLoadGuard) -> Self {
        self.inner = self.inner.__lazy_load(guard);
        self
    }

    /// Chainable `WHERE col = val` on the inner builder.
    pub fn filter(mut self, col: impl IntoColumn, val: impl IntoVal) -> Self {
        self.inner = self.inner.filter(col, val);
        self
    }

    /// Laravel-shape alias for [`Self::filter`].
    pub fn db_where(self, col: impl IntoColumn, val: impl IntoVal) -> Self {
        self.filter(col, val)
    }

    /// Chainable `ORDER BY col <dir>` on the inner builder.
    pub fn order_by(mut self, col: impl IntoColumn, dir: Direction) -> Self {
        self.inner = self.inner.order_by(col, dir);
        self
    }

    /// Execute and return the single matching child row (if any).
    pub async fn first(self) -> Result<Option<R>, FrameworkError> {
        self.inner.first().await
    }
}

/// Soft-delete forwarding for `MorphOne<L, R>` when `R: SoftDeletes`.
/// Same forwarding pattern as the [`MorphMany`] block above, applied
/// through the inner `MorphMany`'s own `with_trashed` / `only_trashed`.
impl<L, R> MorphOne<L, R>
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
{
    /// Widen the relation to include the trashed child if it exists.
    pub fn with_trashed(mut self) -> Self {
        self.inner = self.inner.with_trashed();
        self
    }

    /// Restrict the relation to a trashed child.
    pub fn only_trashed(mut self) -> Self {
        self.inner = self.inner.only_trashed();
        self
    }
}

impl<L, R> Relation for MorphOne<L, R>
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
{
    type Parent = L;
    type Target = R;
    const KIND: RelationKind = RelationKind::MorphOne;

    fn parent_key(&self) -> &str {
        self.inner.parent_key()
    }

    fn foreign_key(&self) -> &str {
        self.inner.foreign_key()
    }
}

/// Inverse-side morph relation. Lives on the morph-table side
/// (e.g. `Comment.commentable`); the user declares it as
/// `MorphTo { name = "commentable", targets = [Post, Video] }`.
///
/// **Metadata only.** Unlike [`HasOne`](super::HasOne) /
/// [`BelongsTo`](super::BelongsTo), the user's call site DOES NOT
/// receive a `MorphTo<C>` instance directly. Instead the macro emits
/// a per-family enum (`CommentableMorph`) and a fetch helper
/// (`CommentableMorphFetch`) at the declaration site, and the
/// `comment.commentable()` method returns the fetch helper. The
/// `MorphTo<C>` struct exists so:
///
/// 1. The [`RelationEntry`](super::RelationEntry) inventory submission
///    can name a concrete type (the per-family enum is local to the
///    declaration site and isn't reachable from the inventory entry).
/// 2. The framework re-exports a symbol users can name for advanced
///    cases (custom relation impls, third-party integrations).
/// 3. The seal contract through `Relation` stays uniform - every
///    declared relation has an impl.
///
/// The per-family enum dispatch in `<Name>MorphFetch::get()` calls
/// `Target::find(id)` for each branch directly; it does not flow
/// through this struct.
///
/// # Key types
///
/// The id is the JSON value of the target's primary key, as the
/// child's `<name>_id` column holds it: a number for an `i64` key, a
/// string for a `String`, UUID or ULID key. It is bound as it is, so
/// one morph family works for any of those key types. The fetch helper
/// turns it back into the target's typed key before it calls `find`.
///
/// Every target of one `MorphTo` relation has the same key type, and
/// the child's `<name>_id` field has that type too (or `Option` of it
/// for a nullable morph). One column cannot hold two kinds of key, so
/// the macro refuses a family that mixes them at compile time; the
/// error names the two models (see [`MorphTargetsShareKey`] and
/// [`MorphIdColumnHoldsKey`]).
///
/// The check covers the `MorphTo` side only. A parent's [`MorphMany`]
/// or [`MorphOne`] binds its own key against the child's `<name>_id`
/// column, and its declaration cannot see the child's field types: the
/// child is a separate `#[model]` expansion, it need not declare a
/// `MorphTo` (a child may hold just the two columns), and the model
/// macro emits no per-column type trait a parent could assert against.
/// A parent whose key type differs from the child's `<name>_id` type
/// therefore compiles; the lookup then finds nothing on SQLite and
/// fails with a type error on PostgreSQL. Keep the child's `<name>_id`
/// field at the key type of every parent that owns it.
pub struct MorphTo<C>
where
    C: EloquentModel,
{
    /// FK value on the child row (`<name>_id` column): the JSON value
    /// of the target's primary key, `Null` when the column is null.
    pub morph_id: serde_json::Value,
    /// Type-string on the child row (`<name>_type` column).
    pub morph_type: String,
    _phantom: PhantomData<fn() -> C>,
}

impl<C> MorphTo<C>
where
    C: EloquentModel,
{
    /// Construct a `MorphTo` metadata carrier. Invoked by macro-
    /// emitted code only - user code uses the per-family fetch helper
    /// instead of touching this struct directly.
    #[doc(hidden)]
    pub fn __new(morph_id: serde_json::Value, morph_type: String) -> Self {
        Self {
            morph_id,
            morph_type,
            _phantom: PhantomData,
        }
    }
}

impl<C> Relation for MorphTo<C>
where
    C: EloquentModel,
{
    type Parent = C;
    /// `MorphTo` doesn't have a single concrete target - the per-family
    /// enum at the declaration site stands in. The unit type signals
    /// "look at the macro-generated per-family enum, not a single
    /// target" to admin tooling and the eager-load dispatcher.
    type Target = ();
    const KIND: RelationKind = RelationKind::MorphTo;

    fn parent_key(&self) -> &str {
        "id"
    }

    fn foreign_key(&self) -> &str {
        ""
    }
}

/// The owner row a `MorphTo` relation of a child row points at, as the
/// parent-touch cascade of `#[model(touches = [...])]` writes it: the
/// owner model's table and columns, and the key the child's
/// `<name>_id` column holds.
///
/// Built by the fetch helper the `#[suprnova::model]` macro emits for a
/// `MorphTo` relation, from the target that the child's `<name>_type`
/// column names, and read by
/// [`Model::touch_owners`](crate::eloquent::Model::touch_owners). The
/// owner is never loaded: the touch is one `UPDATE` by key, as it is
/// for a `BelongsTo` owner.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct MorphOwner {
    /// The owner model's `TypeId`, for the per-type
    /// `without_touching_on` scope.
    pub(crate) type_id: TypeId,
    /// The owner model's table.
    pub(crate) table: &'static str,
    /// The owner model's primary-key column.
    pub(crate) key_column: &'static str,
    /// The owner model's `updated_at` column, `""` when the model
    /// manages no timestamps and the touch skips it.
    pub(crate) updated_at_column: &'static str,
    /// The owner model's soft-delete column, `""` when it has none. A
    /// trashed owner is not touched.
    pub(crate) soft_deletes_column: &'static str,
    /// How the owner's `updated_at` cast stores the time.
    pub(crate) updated_at_storage: super::TouchStorage,
    /// The owner's key, the JSON value of the child's `<name>_id`
    /// column, bound as it is.
    pub(crate) key: serde_json::Value,
}

impl MorphOwner {
    /// The owner row of model `T` whose primary key is `key`. Called by
    /// the macro-emitted fetch helper once the child's `<name>_type`
    /// column has named `T`.
    pub fn of<T>(key: serde_json::Value) -> Self
    where
        T: EloquentModel + 'static,
    {
        Self {
            type_id: TypeId::of::<T>(),
            table: T::TABLE,
            key_column: T::PRIMARY_KEY,
            updated_at_column: super::touch_column(T::HAS_TIMESTAMPS, T::UPDATED_AT_COLUMN),
            soft_deletes_column: T::SOFT_DELETES_COLUMN,
            updated_at_storage: T::updated_at_storage,
            key,
        }
    }
}

/// Proof that two targets of one `MorphTo` relation have keys of one
/// type. `Self` is the key type of the target `First`, `Other` the key
/// type of the target `Second`.
///
/// The child's `<name>_id` column holds the key of whichever target a
/// row points at, so an `i64` column cannot also hold a `String` key.
/// The `#[suprnova::model]` macro asks for this bound between the first
/// target of a `MorphTo` relation and each other target, and the only
/// implementation is the one where `Self` and `Other` are the same
/// type. A family that mixes key types is therefore a compile error,
/// and the error names the two models.
///
/// `Relation` is the per-family enum the relation emits
/// (`CommentableMorph` for a relation named `commentable`). It is there
/// only so the error can name the relation.
///
/// A family whose targets have an `i64` key and a `String` key does not
/// compile:
///
/// ```compile_fail
/// use suprnova::model;
///
/// #[model(table = "numbered_posts")]
/// pub struct NumberedPost {
///     pub id: i64,
/// }
///
/// #[model(table = "coded_videos", key_type = "String", auto_increment = false)]
/// pub struct CodedVideo {
///     pub id: String,
/// }
///
/// #[model(table = "notes", relations = {
///     subject: MorphTo { targets = [NumberedPost, CodedVideo] },
/// })]
/// pub struct Note {
///     pub id: i64,
///     pub subject_id: i64,
///     pub subject_type: String,
/// }
///
/// fn main() {}
/// ```
///
/// The same family compiles once both targets have `i64` keys:
///
/// ```
/// use suprnova::model;
///
/// #[model(table = "numbered_posts")]
/// pub struct NumberedPost {
///     pub id: i64,
/// }
///
/// #[model(table = "numbered_videos")]
/// pub struct NumberedVideo {
///     pub id: i64,
/// }
///
/// #[model(table = "notes", relations = {
///     subject: MorphTo { targets = [NumberedPost, NumberedVideo] },
/// })]
/// pub struct Note {
///     pub id: i64,
///     pub subject_id: i64,
///     pub subject_type: String,
/// }
///
/// fn main() {}
/// ```
#[diagnostic::on_unimplemented(
    message = "the `MorphTo` relation behind `{Relation}` mixes key types: its target `{First}` \
               has a `{Self}` key and its target `{Second}` has a `{Other}` key",
    label = "every target of one `MorphTo` relation needs the same key type",
    note = "the child's `<name>_id` column holds the key of any target, so it can hold only one \
            kind of key"
)]
pub trait MorphTargetsShareKey<Other, First, Second, Relation> {}

impl<Key, First, Second, Relation> MorphTargetsShareKey<Key, First, Second, Relation> for Key {}

/// Proof that the child's `<name>_id` field can hold the key of the
/// targets of its `MorphTo` relation. `Self` is the field's type
/// (the `T` of an `Option<T>` field, for a nullable morph), `Key` the
/// key type of the target `Target`.
///
/// The field's JSON value is what the relation binds against the
/// target's primary key, and what the fetch helper turns back into the
/// target's typed key, so the two types must be the same. The
/// `#[suprnova::model]` macro asks for this bound against the first
/// target; [`MorphTargetsShareKey`] ties the other targets to it.
///
/// `Child` is the model that declares the relation and `Relation` the
/// per-family enum it emits; both are there only so the error can name
/// them.
///
/// A `String` id field over targets with an `i64` key does not compile:
///
/// ```compile_fail
/// use suprnova::model;
///
/// #[model(table = "numbered_posts")]
/// pub struct NumberedPost {
///     pub id: i64,
/// }
///
/// #[model(table = "numbered_videos")]
/// pub struct NumberedVideo {
///     pub id: i64,
/// }
///
/// #[model(table = "notes", relations = {
///     subject: MorphTo { targets = [NumberedPost, NumberedVideo] },
/// })]
/// pub struct Note {
///     pub id: i64,
///     pub subject_id: String,
///     pub subject_type: String,
/// }
///
/// fn main() {}
/// ```
///
/// The same declaration compiles once the id field has the key type of
/// the targets:
///
/// ```
/// use suprnova::model;
///
/// #[model(table = "numbered_posts")]
/// pub struct NumberedPost {
///     pub id: i64,
/// }
///
/// #[model(table = "numbered_videos")]
/// pub struct NumberedVideo {
///     pub id: i64,
/// }
///
/// #[model(table = "notes", relations = {
///     subject: MorphTo { targets = [NumberedPost, NumberedVideo] },
/// })]
/// pub struct Note {
///     pub id: i64,
///     pub subject_id: i64,
///     pub subject_type: String,
/// }
///
/// fn main() {}
/// ```
#[diagnostic::on_unimplemented(
    message = "the `MorphTo` relation behind `{Relation}` keeps its id in a `{Self}` field of \
               `{Child}`, but its target `{Target}` has a `{Key}` key",
    label = "the `<name>_id` field needs the key type of the relation's targets"
)]
pub trait MorphIdColumnHoldsKey<Key, Child, Target, Relation> {}

impl<Key, Child, Target, Relation> MorphIdColumnHoldsKey<Key, Child, Target, Relation> for Key {}

/// The check behind [`MorphTargetsShareKey`]. The `#[suprnova::model]`
/// macro evaluates it in a `const` item, once for each target of a
/// `MorphTo` relation after the first. Not part of the public API.
#[doc(hidden)]
pub const fn assert_morph_targets_share_key<Key, Other, First, Second, Relation>()
where
    Key: MorphTargetsShareKey<Other, First, Second, Relation>,
{
}

/// The check behind [`MorphIdColumnHoldsKey`]. The `#[suprnova::model]`
/// macro evaluates it in a `const` item, once for each `MorphTo`
/// relation. Not part of the public API.
#[doc(hidden)]
pub const fn assert_morph_id_column_holds_key<Column, Key, Child, Target, Relation>()
where
    Column: MorphIdColumnHoldsKey<Key, Child, Target, Relation>,
{
}
