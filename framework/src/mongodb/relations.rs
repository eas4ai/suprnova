//! Relations between document models, and between documents and SQL
//! models (PAR-185), as `laravel-mongodb`'s relations and its
//! `HybridRelations` give them.
//!
//! A document model declares its relations in `relations = { ... }` on
//! `#[suprnova::document]`, and an SQL model declares its relations to
//! documents in the `relations = { ... }` of `#[suprnova::model]`. Each
//! declaration adds a method of the relation's name that answers one of
//! the types here:
//!
//! | Declared as | On | Answers |
//! |---|---|---|
//! | `HasMany<D>` | a document | [`HasManyDocuments<D>`] |
//! | `HasOne<D>` | a document | [`HasOneDocument<D>`] |
//! | `BelongsTo<D>` | a document | [`BelongsToDocument<D>`] |
//! | `BelongsToMany<D>` | a document | [`BelongsToManyDocuments<Self, D>`] |
//! | `BelongsToModel<M>` | a document | [`BelongsToModel<M>`] |
//! | `HasManyDocuments<D>` | an SQL model | [`HasManyDocuments<D>`] |
//! | `HasOneDocument<D>` | an SQL model | [`HasOneDocument<D>`] |
//!
//! Each relation reads through its own store: a document through its
//! collection, an SQL model through its connection. `with(..)` on either
//! query builder loads a relation for every model the query read with one
//! query per relation.
//!
//! A many-to-many between documents keeps no pivot collection: each side
//! keeps the keys of the other in an array, as `laravel-mongodb` stores
//! it, and [`BelongsToManyDocuments::attach`] writes both arrays.
//!
//! The key a relation compares on one side and the field that holds it on
//! the other must have types that can match: the same type, or the field
//! holding it in an `Option` or as a `Vec` of keys. The macros check each
//! declared relation against [`RelationKey`], so a pair that cannot match
//! fails the build with an error that names both models.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::marker::PhantomData;

use ::bson::{Bson, Document};
use ::mongodb::options::ReturnDocument;
use futures::TryStreamExt;
use sea_orm::{EntityTrait, PrimaryKeyTrait};
use serde::Serialize;

use super::document::{DocumentModel, create_guarded, key_to_bson, model_name, one};
use super::query::{DocumentQuery, storage};
use crate::eloquent::lazy_loading::LazyLoadGuard;
use crate::eloquent::relations::__TakenRows;
use crate::eloquent::{Builder, Collection as Models, EagerLoadDispatch, EloquentModel, Model};
use crate::error::FrameworkError;

// --- Key types ---------------------------------------------------------------

/// Whether a relation can compare a key of type `Self` in the model
/// `Left` with a field of type `Other` in the model `Right`.
///
/// The relation macros require it of every relation they declare, so a
/// relation whose types cannot match fails the build, and the error names
/// both models and both types. It holds when the field has the key's type,
/// holds it in an `Option`, or holds an array of such keys (`Vec`), and
/// when the key is an `Option` of the field's type.
#[diagnostic::on_unimplemented(
    message = "`{Left}` and `{Right}` cannot relate: the key `{Self}` cannot match `{Other}`",
    label = "this relation compares `{Self}` in `{Left}` with `{Other}` in `{Right}`",
    note = "give the field the type of the key it holds: `{Self}`, `Option<{Self}>`, or `Vec<{Self}>` for an array of keys"
)]
pub trait RelationKey<Other, Left, Right> {}

impl<K, Left, Right> RelationKey<K, Left, Right> for K {}
impl<K, Left, Right> RelationKey<Option<K>, Left, Right> for K {}
impl<K, Left, Right> RelationKey<Vec<K>, Left, Right> for K {}
impl<K, Left, Right> RelationKey<K, Left, Right> for Option<K> {}

/// The compile-time check of one relation: the key `left` reads from
/// `Left` must match the field `right` reads from `Right`. The closures
/// are never called; their types carry the check.
///
/// **Not part of the public API.** It is `pub` because the code the
/// relation macros emit calls it.
#[doc(hidden)]
pub fn __relation_keys<Left, Right, A, B, FL, FR>(left: FL, right: FR)
where
    FL: Fn(&Left) -> &A,
    FR: Fn(&Right) -> &B,
    A: RelationKey<B, Left, Right>,
{
    let _ = (left, right);
}

/// [`__relation_keys`] for an SQL model `M`'s primary key, whose type is
/// the type of its primary-key field.
///
/// **Not part of the public API.** It is `pub` because the code the
/// relation macros emit calls it.
#[doc(hidden)]
pub fn __relation_model_key<M, Right, B, FR>(right: FR)
where
    M: EloquentModel,
    FR: Fn(&Right) -> &B,
    M::Key: RelationKey<B, M, Right>,
{
    let _ = right;
}

// --- Values the emitted code reads ----------------------------------------------

/// The key of `model`, as stored under `_id`.
///
/// **Not part of the public API.** It is `pub` because the code the
/// relation macros emit calls it.
#[doc(hidden)]
pub fn __document_key<M: DocumentModel>(model: &M) -> Result<Bson, FrameworkError> {
    key_to_bson::<M>(model.key())
}

/// `value` when it holds a key, `None` when it is null: a relation reads
/// nothing for a missing key.
///
/// **Not part of the public API.** It is `pub` because the code the
/// relation macros emit calls it.
#[doc(hidden)]
pub fn __present(value: Result<Bson, FrameworkError>) -> Result<Option<Bson>, FrameworkError> {
    match value? {
        Bson::Null | Bson::Undefined => Ok(None),
        other => Ok(Some(other)),
    }
}

/// The JSON form of `value`, the field `field` of `model`, as an SQL
/// query binds it; `None` when it is null.
///
/// **Not part of the public API.** It is `pub` because the code the
/// relation macros emit calls it.
#[doc(hidden)]
pub fn __present_json<T: Serialize>(
    value: &T,
    model: &str,
    field: &str,
) -> Result<Option<serde_json::Value>, FrameworkError> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::Null) => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(error) => Err(FrameworkError::internal(format!(
            "`{model}::{field}` has no JSON form for a relation to compare: {error}"
        ))),
    }
}

/// The first name of the relation path `path` and the rest after its
/// first dot.
///
/// **Not part of the public API.** It is `pub` because the code the
/// relation macros emit calls it.
#[doc(hidden)]
pub fn __split_relation_path(path: &str) -> (&str, Option<&str>) {
    match path.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (path, None),
    }
}

/// The error for a relation path whose first name `M` does not declare.
///
/// **Not part of the public API.** It is `pub` because the code the
/// relation macros emit calls it.
#[doc(hidden)]
pub fn __unknown_relation<M: DocumentModel>(path: &str) -> FrameworkError {
    let (head, _) = __split_relation_path(path);
    let declared = if M::RELATIONS.is_empty() {
        "it declares none".to_owned()
    } else {
        format!("it declares {}", M::RELATIONS.join(", "))
    };
    FrameworkError::internal(format!(
        "`{}` has no relation `{head}` to load; {declared}",
        model_name::<M>()
    ))
}

// --- Lookups -----------------------------------------------------------------

/// The largest magnitude below which every integer has an exact `f64`.
const EXACT_DOUBLE: f64 = 9_007_199_254_740_992.0;

/// The text an eager load groups a key by. Numbers compare by value, as
/// MongoDB compares them, so an `Int32`, an `Int64` and a whole `Double`
/// of one value meet.
fn key_text(key: &Bson) -> String {
    match key {
        Bson::Int32(number) => format!("{:?}", Bson::Int64(i64::from(*number))),
        Bson::Double(number) if number.fract() == 0.0 && number.abs() < EXACT_DOUBLE => {
            // Exact: the magnitude is below 2^53 and the value is whole.
            format!("{:?}", Bson::Int64(*number as i64))
        }
        other => format!("{other:?}"),
    }
}

/// The documents of `C` whose `field` holds `key`. A missing key matches
/// nothing, as an empty `$in`, so a relation of a model without a key
/// never answers documents whose field is null.
fn lookup<C: DocumentModel>(field: &str, key: &Result<Option<Bson>, String>) -> DocumentQuery<C> {
    match key {
        Ok(Some(key)) => C::query().where_(field, "=", key.clone()),
        Ok(None) => C::query().where_in(field, Vec::<Bson>::new()),
        Err(message) => DocumentQuery::new().fail(message.clone()),
    }
}

/// The key a relation compares, `None` when it is null, or the message
/// of the error that kept it from being read.
fn read_key(key: Result<Bson, FrameworkError>) -> Result<Option<Bson>, String> {
    __present(key).map_err(|error| error.message().to_owned())
}

/// The error a write answers when the model `M` whose key it needs has
/// none.
fn keyless<M>(call: &str) -> FrameworkError {
    FrameworkError::internal(format!(
        "{call}: the {} has no key to relate by",
        model_name::<M>()
    ))
}

/// Create a document of `C` whose `field` holds `key`, as a relation's
/// `create` does: the relation's key wins over `attrs`.
async fn create_related<C: DocumentModel>(
    field: &'static str,
    key: &Result<Option<Bson>, String>,
    attrs: Document,
) -> Result<C, FrameworkError> {
    let key = match key {
        Ok(Some(key)) => key.clone(),
        Ok(None) => {
            return Err(FrameworkError::internal(format!(
                "create: the parent has no key for `{}::{field}` to hold",
                model_name::<C>()
            )));
        }
        Err(message) => return Err(FrameworkError::internal(message.clone())),
    };
    create_guarded::<C>(attrs, one(field, key), "create").await
}

// --- Has many and has one ----------------------------------------------------------

/// The documents of `C` whose foreign key holds a parent's key: the
/// relation `HasMany<C>` declares on a document and `HasManyDocuments<C>`
/// on an SQL model, as Laravel's `hasMany`.
///
/// ```rust,ignore
/// let posts = user.posts().get().await?;
/// let recent = user.posts().query().order_by("created_at", Direction::Desc).take(5).get().await?;
/// let post = user.posts().create(doc! { "title": "Hello" }).await?;
/// ```
pub struct HasManyDocuments<C> {
    foreign_key: &'static str,
    parent_key: Result<Option<Bson>, String>,
    lazy_load: LazyLoadGuard,
    child: PhantomData<fn() -> C>,
}

impl<C> fmt::Debug for HasManyDocuments<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HasManyDocuments")
            .field("child", &model_name::<C>())
            .field("foreign_key", &self.foreign_key)
            .field("parent_key", &self.parent_key)
            .finish()
    }
}

impl<C: DocumentModel> HasManyDocuments<C> {
    /// The relation of the parent whose key is `parent_key`, over the
    /// field `foreign_key` of `C`.
    ///
    /// **Not part of the public API.** The relation macros call it.
    #[doc(hidden)]
    pub fn __new(foreign_key: &'static str, parent_key: Result<Bson, FrameworkError>) -> Self {
        Self {
            foreign_key,
            parent_key: read_key(parent_key),
            lazy_load: LazyLoadGuard::default(),
            child: PhantomData,
        }
    }

    /// Attach the lazy-loading check of the SQL model this relation was
    /// read from.
    ///
    /// **Not part of the public API.** The relation macros call it.
    #[doc(hidden)]
    pub fn __lazy_load(mut self, guard: LazyLoadGuard) -> Self {
        self.lazy_load = guard;
        self
    }

    /// The field of `C` that holds the parent's key.
    pub fn foreign_key(&self) -> &str {
        self.foreign_key
    }

    /// The query for the related documents, to add conditions to before
    /// it runs. A parent without a key matches no document.
    pub fn query(&self) -> DocumentQuery<C> {
        lookup::<C>(self.foreign_key, &self.parent_key)
    }

    /// Every related document.
    ///
    /// # Errors
    ///
    /// As [`DocumentQuery::get`], and when the parent is an SQL model
    /// whose lazy loading is prevented.
    pub async fn get(&self) -> Result<Models<C>, FrameworkError> {
        self.lazy_load.check()?;
        self.query().get().await
    }

    /// The first related document, or `None`.
    ///
    /// # Errors
    ///
    /// As [`Self::get`].
    pub async fn first(&self) -> Result<Option<C>, FrameworkError> {
        self.lazy_load.check()?;
        self.query().first().await
    }

    /// The number of related documents. It loads no model, so lazy
    /// loading prevention does not apply.
    ///
    /// # Errors
    ///
    /// As [`DocumentQuery::count`].
    pub async fn count(&self) -> Result<u64, FrameworkError> {
        self.query().count().await
    }

    /// Whether any document is related.
    ///
    /// # Errors
    ///
    /// As [`DocumentQuery::exists`].
    pub async fn exists(&self) -> Result<bool, FrameworkError> {
        self.query().exists().await
    }

    /// Create a document of `C` from `attrs` with the foreign key set to
    /// the parent's key, as Laravel's `$user->posts()->create(...)`. The
    /// guard applies to `attrs`; the foreign key is written whatever the
    /// guard or `attrs` say.
    ///
    /// # Errors
    ///
    /// When the parent has no key, and as [`DocumentModel::create`].
    pub async fn create(&self, attrs: Document) -> Result<C, FrameworkError> {
        create_related::<C>(self.foreign_key, &self.parent_key, attrs).await
    }
}

/// The document of `C` whose foreign key holds a parent's key: the
/// relation `HasOne<C>` declares on a document and `HasOneDocument<C>`
/// on an SQL model, as Laravel's `hasOne`. When several documents hold
/// the key, the first the server answers is the one.
pub struct HasOneDocument<C> {
    many: HasManyDocuments<C>,
}

impl<C> fmt::Debug for HasOneDocument<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HasOneDocument")
            .field("child", &model_name::<C>())
            .field("foreign_key", &self.many.foreign_key)
            .field("parent_key", &self.many.parent_key)
            .finish()
    }
}

impl<C: DocumentModel> HasOneDocument<C> {
    /// The relation of the parent whose key is `parent_key`, over the
    /// field `foreign_key` of `C`.
    ///
    /// **Not part of the public API.** The relation macros call it.
    #[doc(hidden)]
    pub fn __new(foreign_key: &'static str, parent_key: Result<Bson, FrameworkError>) -> Self {
        Self {
            many: HasManyDocuments::__new(foreign_key, parent_key),
        }
    }

    /// Attach the lazy-loading check of the SQL model this relation was
    /// read from.
    ///
    /// **Not part of the public API.** The relation macros call it.
    #[doc(hidden)]
    pub fn __lazy_load(self, guard: LazyLoadGuard) -> Self {
        Self {
            many: self.many.__lazy_load(guard),
        }
    }

    /// The field of `C` that holds the parent's key.
    pub fn foreign_key(&self) -> &str {
        self.many.foreign_key()
    }

    /// The query for the related document, to add conditions to before
    /// it runs.
    pub fn query(&self) -> DocumentQuery<C> {
        self.many.query()
    }

    /// The related document, or `None`.
    ///
    /// # Errors
    ///
    /// As [`HasManyDocuments::first`].
    pub async fn get(&self) -> Result<Option<C>, FrameworkError> {
        self.many.first().await
    }

    /// Whether a document is related.
    ///
    /// # Errors
    ///
    /// As [`DocumentQuery::exists`].
    pub async fn exists(&self) -> Result<bool, FrameworkError> {
        self.many.exists().await
    }

    /// Create the related document from `attrs` with the foreign key set
    /// to the parent's key; see [`HasManyDocuments::create`]. A document
    /// already related stays as it is.
    ///
    /// # Errors
    ///
    /// As [`HasManyDocuments::create`].
    pub async fn create(&self, attrs: Document) -> Result<C, FrameworkError> {
        self.many.create(attrs).await
    }
}

// --- Belongs to -----------------------------------------------------------------------

/// The document of `P` a document's foreign key names: the relation
/// `BelongsTo<P>` declares, as Laravel's `belongsTo`.
pub struct BelongsToDocument<P> {
    foreign_key: &'static str,
    owner_key: &'static str,
    value: Result<Option<Bson>, String>,
    owner: PhantomData<fn() -> P>,
}

impl<P> fmt::Debug for BelongsToDocument<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BelongsToDocument")
            .field("owner", &model_name::<P>())
            .field("foreign_key", &self.foreign_key)
            .field("owner_key", &self.owner_key)
            .field("value", &self.value)
            .finish()
    }
}

impl<P: DocumentModel> BelongsToDocument<P> {
    /// The relation whose foreign key `foreign_key` holds `value`, the
    /// field `owner_key` of `P`.
    ///
    /// **Not part of the public API.** The relation macros call it.
    #[doc(hidden)]
    pub fn __new(
        foreign_key: &'static str,
        owner_key: &'static str,
        value: Result<Bson, FrameworkError>,
    ) -> Self {
        Self {
            foreign_key,
            owner_key,
            value: read_key(value),
            owner: PhantomData,
        }
    }

    /// The field of this document that names the owner.
    pub fn foreign_key(&self) -> &str {
        self.foreign_key
    }

    /// The field of `P` the foreign key holds: its key unless the relation
    /// names another.
    pub fn owner_key(&self) -> &str {
        self.owner_key
    }

    /// The query for the owner. A document whose foreign key is null
    /// matches no owner.
    pub fn query(&self) -> DocumentQuery<P> {
        lookup::<P>(self.owner_key, &self.value)
    }

    /// The owner, or `None`. A null foreign key answers `None` without a
    /// query.
    ///
    /// # Errors
    ///
    /// As [`DocumentQuery::first`].
    pub async fn get(&self) -> Result<Option<P>, FrameworkError> {
        if matches!(self.value, Ok(None)) {
            return Ok(None);
        }
        self.query().first().await
    }
}

/// The SQL model a document's foreign key names: the relation
/// `BelongsToModel<M>` declares, read through the model's own
/// connection, as a `laravel-mongodb` model's `belongsTo` of an SQL
/// model.
pub struct BelongsToModel<M> {
    foreign_key: &'static str,
    owner_key: &'static str,
    value: Result<Option<serde_json::Value>, String>,
    owner: PhantomData<fn() -> M>,
}

impl<M> fmt::Debug for BelongsToModel<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BelongsToModel")
            .field("owner", &model_name::<M>())
            .field("foreign_key", &self.foreign_key)
            .field("owner_key", &self.owner_key)
            .field("value", &self.value)
            .finish()
    }
}

impl<M> BelongsToModel<M>
where
    M: Model
        + From<<M::Entity as EntityTrait>::Model>
        + serde::de::DeserializeOwned
        + EagerLoadDispatch,
    <M::Entity as EntityTrait>::Model: From<M>
        + sea_orm::IntoActiveModel<<M::Entity as EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + Serialize
        + Send
        + Sync,
    <M::Entity as EntityTrait>::ActiveModel: Send,
    <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    /// The relation whose foreign key `foreign_key` holds `value`, the
    /// column `owner_key` of `M`.
    ///
    /// **Not part of the public API.** The relation macros call it.
    #[doc(hidden)]
    pub fn __new(
        foreign_key: &'static str,
        owner_key: &'static str,
        value: Result<Option<serde_json::Value>, FrameworkError>,
    ) -> Self {
        Self {
            foreign_key,
            owner_key,
            value: value.map_err(|error| error.message().to_owned()),
            owner: PhantomData,
        }
    }

    /// The field of this document that names the SQL model.
    pub fn foreign_key(&self) -> &str {
        self.foreign_key
    }

    /// The column of `M` the foreign key holds: its primary key unless the
    /// relation names another.
    pub fn owner_key(&self) -> &str {
        self.owner_key
    }

    /// The SQL query for the owner, on the model's connection. A document
    /// whose foreign key is null matches no row.
    pub fn query(&self) -> Builder<M> {
        match &self.value {
            Ok(Some(value)) => M::query().filter(self.owner_key, value.clone()),
            Ok(None) => M::query().filter_in(self.owner_key, Vec::<serde_json::Value>::new()),
            Err(message) => M::query().failing(message.clone()),
        }
    }

    /// The owner, or `None`. A null foreign key answers `None` without a
    /// query.
    ///
    /// # Errors
    ///
    /// As [`Builder::first`].
    pub async fn get(&self) -> Result<Option<M>, FrameworkError> {
        if matches!(self.value, Ok(None)) {
            return Ok(None);
        }
        self.query().first().await
    }
}

// --- Belongs to many ---------------------------------------------------------------------

/// The documents of `R` related to a document of `P` many-to-many, with
/// each side keeping the other's keys in an array, as `laravel-mongodb`'s
/// `belongsToMany` stores it: the relation `BelongsToMany<R>` declares.
///
/// The related documents are the ones whose array (the foreign pivot key,
/// `<p>_ids` unless named) holds the parent's key. [`Self::attach`],
/// [`Self::detach`] and [`Self::sync`] write both arrays: the related
/// documents' and the parent's own (the related pivot key, `<r>_ids`
/// unless named), so the relation reads the same from either side.
///
/// ```rust,ignore
/// let user = user.roles().attach([editor.id, admin.id]).await?;
/// let roles = user.roles().get().await?;
/// let users = editor.users().get().await?;
/// ```
pub struct BelongsToManyDocuments<P, R> {
    parent_key: Result<Option<Bson>, String>,
    foreign_pivot_key: &'static str,
    related_pivot_key: &'static str,
    models: PhantomData<fn() -> (P, R)>,
}

impl<P, R> fmt::Debug for BelongsToManyDocuments<P, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BelongsToManyDocuments")
            .field("parent", &model_name::<P>())
            .field("related", &model_name::<R>())
            .field("parent_key", &self.parent_key)
            .field("foreign_pivot_key", &self.foreign_pivot_key)
            .field("related_pivot_key", &self.related_pivot_key)
            .finish()
    }
}

impl<P: DocumentModel, R: DocumentModel> BelongsToManyDocuments<P, R> {
    /// The relation of the parent whose key is `parent_key`: the array
    /// `foreign_pivot_key` of `R` holds the parent keys, and the array
    /// `related_pivot_key` of `P` holds the related keys.
    ///
    /// **Not part of the public API.** The relation macros call it.
    #[doc(hidden)]
    pub fn __new(
        parent_key: Result<Bson, FrameworkError>,
        foreign_pivot_key: &'static str,
        related_pivot_key: &'static str,
    ) -> Self {
        Self {
            parent_key: read_key(parent_key),
            foreign_pivot_key,
            related_pivot_key,
            models: PhantomData,
        }
    }

    /// The array of `R` that holds the keys of its `P` documents.
    pub fn foreign_pivot_key(&self) -> &str {
        self.foreign_pivot_key
    }

    /// The array of `P` that holds the keys of its `R` documents.
    pub fn related_pivot_key(&self) -> &str {
        self.related_pivot_key
    }

    /// The query for the related documents, to add conditions to before
    /// it runs.
    pub fn query(&self) -> DocumentQuery<R> {
        lookup::<R>(self.foreign_pivot_key, &self.parent_key)
    }

    /// Every related document.
    ///
    /// # Errors
    ///
    /// As [`DocumentQuery::get`].
    pub async fn get(&self) -> Result<Models<R>, FrameworkError> {
        self.query().get().await
    }

    /// The first related document, or `None`.
    ///
    /// # Errors
    ///
    /// As [`DocumentQuery::first`].
    pub async fn first(&self) -> Result<Option<R>, FrameworkError> {
        self.query().first().await
    }

    /// The number of related documents.
    ///
    /// # Errors
    ///
    /// As [`DocumentQuery::count`].
    pub async fn count(&self) -> Result<u64, FrameworkError> {
        self.query().count().await
    }

    /// Whether any document is related.
    ///
    /// # Errors
    ///
    /// As [`DocumentQuery::exists`].
    pub async fn exists(&self) -> Result<bool, FrameworkError> {
        self.query().exists().await
    }

    /// Relate the documents of `R` whose keys are `ids`: add `ids` to the
    /// parent's array with `$addToSet`, then the parent's key to each
    /// related document's array, and answer the parent as the server holds
    /// it after. A key already related stays once. Without a transaction
    /// the writes are separate; each is idempotent, so attaching again
    /// completes an attach that failed between them.
    ///
    /// # Errors
    ///
    /// When the parent has no key or its document no longer exists (before
    /// any related document is written), when a key has no BSON form, and
    /// when a write fails.
    pub async fn attach<I>(&self, ids: I) -> Result<P, FrameworkError>
    where
        I: IntoIterator<Item = R::Key>,
    {
        let parent = self.require_parent("attach")?;
        let keys = related_keys::<R, I>(ids)?;
        let stored = self
            .write_parent(
                &parent,
                one(
                    "$addToSet",
                    one(self.related_pivot_key, one("$each", keys.clone())),
                ),
                "attach",
            )
            .await?;
        if !keys.is_empty() {
            R::collection()?
                .update_many(
                    one("_id", one("$in", keys)),
                    one("$addToSet", one(self.foreign_pivot_key, parent)),
                )
                .await?;
        }
        Ok(stored)
    }

    /// Unrelate the documents of `R` whose keys are `ids`, from both
    /// sides, and answer the parent as the server holds it after.
    ///
    /// # Errors
    ///
    /// As [`Self::attach`].
    pub async fn detach<I>(&self, ids: I) -> Result<P, FrameworkError>
    where
        I: IntoIterator<Item = R::Key>,
    {
        let parent = self.require_parent("detach")?;
        let keys = related_keys::<R, I>(ids)?;
        let stored = self
            .write_parent(
                &parent,
                one("$pullAll", one(self.related_pivot_key, keys.clone())),
                "detach",
            )
            .await?;
        if !keys.is_empty() {
            R::collection()?
                .update_many(
                    one("_id", one("$in", keys)),
                    one("$pull", one(self.foreign_pivot_key, parent)),
                )
                .await?;
        }
        Ok(stored)
    }

    /// Unrelate every related document, from both sides, as Laravel's
    /// `detach()` without ids, and answer the parent as stored after.
    ///
    /// # Errors
    ///
    /// As [`Self::attach`].
    pub async fn detach_all(&self) -> Result<P, FrameworkError> {
        let parent = self.require_parent("detach_all")?;
        let stored = self
            .write_parent(
                &parent,
                one("$set", one(self.related_pivot_key, Bson::Array(Vec::new()))),
                "detach_all",
            )
            .await?;
        R::collection()?
            .update_many(
                one(self.foreign_pivot_key, parent.clone()),
                one("$pull", one(self.foreign_pivot_key, parent)),
            )
            .await?;
        Ok(stored)
    }

    /// Make the documents of `R` whose keys are `ids` the related ones,
    /// from both sides: the parent's array holds `ids` in their order,
    /// every other document lets the parent go, and these name it. Answers
    /// the parent as stored after.
    ///
    /// # Errors
    ///
    /// As [`Self::attach`].
    pub async fn sync<I>(&self, ids: I) -> Result<P, FrameworkError>
    where
        I: IntoIterator<Item = R::Key>,
    {
        let parent = self.require_parent("sync")?;
        let keys = related_keys::<R, I>(ids)?;
        let stored = self
            .write_parent(
                &parent,
                one("$set", one(self.related_pivot_key, keys.clone())),
                "sync",
            )
            .await?;
        let collection = R::collection()?;
        let mut released = one(self.foreign_pivot_key, parent.clone());
        released.insert("_id", one("$nin", keys.clone()));
        collection
            .update_many(
                released,
                one("$pull", one(self.foreign_pivot_key, parent.clone())),
            )
            .await?;
        if !keys.is_empty() {
            collection
                .update_many(
                    one("_id", one("$in", keys)),
                    one("$addToSet", one(self.foreign_pivot_key, parent)),
                )
                .await?;
        }
        Ok(stored)
    }

    fn require_parent(&self, call: &str) -> Result<Bson, FrameworkError> {
        match &self.parent_key {
            Ok(Some(key)) => Ok(key.clone()),
            Ok(None) => Err(keyless::<P>(call)),
            Err(message) => Err(FrameworkError::internal(message.clone())),
        }
    }

    /// Apply `update` to the parent's document and answer it as stored
    /// after.
    async fn write_parent(
        &self,
        parent: &Bson,
        update: Document,
        call: &str,
    ) -> Result<P, FrameworkError> {
        let stored = P::collection()?
            .find_one_and_update(one("_id", parent.clone()), update)
            .return_document(ReturnDocument::After)
            .await?
            .ok_or_else(|| gone::<P>(call))?;
        P::from_document(stored)
    }
}

/// The stored keys of `ids`, each once, in their order.
fn related_keys<R: DocumentModel, I: IntoIterator<Item = R::Key>>(
    ids: I,
) -> Result<Vec<Bson>, FrameworkError> {
    let mut seen = HashSet::new();
    let mut keys = Vec::new();
    for id in ids {
        let key = key_to_bson::<R>(&id)?;
        if seen.insert(key_text(&key)) {
            keys.push(key);
        }
    }
    Ok(keys)
}

fn gone<M>(call: &str) -> FrameworkError {
    FrameworkError::not_found(format!(
        "{call}: the document of {} no longer exists",
        model_name::<M>()
    ))
}

// --- Eager loads --------------------------------------------------------------------

/// Hand each of `parents` its share of `rows`, the related models each
/// beside the key text it is grouped by: a parent whose key is shared
/// with another gets a copy, the last one the rows themselves.
fn distribute<C: Clone>(keys: &[Option<String>], rows: HashMap<String, Vec<C>>) -> Vec<Vec<C>> {
    let mut rows = rows;
    let mut remaining: HashMap<&str, usize> = HashMap::new();
    for key in keys.iter().flatten() {
        *remaining.entry(key.as_str()).or_default() += 1;
    }
    keys.iter()
        .map(|key| {
            let Some(key) = key else {
                return Vec::new();
            };
            let left = remaining.entry(key.as_str()).or_default();
            *left = left.saturating_sub(1);
            if *left == 0 {
                rows.remove(key).unwrap_or_default()
            } else {
                rows.get(key).cloned().unwrap_or_default()
            }
        })
        .collect()
}

/// Load, with one query, the documents of `C` whose `field` holds one of
/// `keys` (one per parent, `None` for a parent without a key), and answer
/// each parent's documents in the order of `keys`. A field that holds an
/// array of keys relates its document to every parent whose key it holds.
async fn documents_by_key<C: DocumentModel>(
    field: &str,
    keys: &[Option<Bson>],
) -> Result<Vec<Vec<C>>, FrameworkError> {
    let texts: Vec<Option<String>> = keys.iter().map(|key| key.as_ref().map(key_text)).collect();
    let mut seen = HashSet::new();
    let distinct: Vec<Bson> = keys
        .iter()
        .flatten()
        .filter(|key| seen.insert(key_text(key)))
        .cloned()
        .collect();
    if distinct.is_empty() {
        return Ok(keys.iter().map(|_| Vec::new()).collect());
    }
    let rows = C::query()
        .where_in(field, distinct)
        .get_keyed(field)
        .await?;
    let mut grouped: HashMap<String, Vec<C>> = HashMap::new();
    for (value, row) in rows {
        match value {
            Bson::Array(items) => {
                let mut held = HashSet::new();
                for item in items {
                    let text = key_text(&item);
                    if seen.contains(&text) && held.insert(text.clone()) {
                        grouped.entry(text).or_default().push(row.clone());
                    }
                }
            }
            value => grouped.entry(key_text(&value)).or_default().push(row),
        }
    }
    Ok(distribute(&texts, grouped))
}

/// Load a relation of `models` from the collection of `C` with one
/// query: the documents whose `field` holds one of `keys` (one per model),
/// each model's share handed to `store`.
///
/// **Not part of the public API.** It is `pub` because the eager loads the
/// relation macros emit call it.
#[doc(hidden)]
pub async fn __load_documents<P, C: DocumentModel>(
    models: &mut [&mut P],
    keys: Vec<Option<Bson>>,
    field: &str,
    store: fn(&mut P, Vec<C>),
) -> Result<(), FrameworkError> {
    let groups = documents_by_key::<C>(field, &keys).await?;
    for (model, group) in models.iter_mut().zip(groups) {
        store(model, group);
    }
    Ok(())
}

/// Load a `BelongsToModel` relation of `models` with one SQL query on
/// `M`'s connection: the rows whose `column` holds one of `keys` (one per
/// model), each model's row handed to `store`.
///
/// **Not part of the public API.** It is `pub` because the eager loads the
/// relation macros emit call it.
#[doc(hidden)]
pub async fn __load_models<P, M>(
    models: &mut [&mut P],
    keys: Vec<Option<serde_json::Value>>,
    column: &str,
    store: fn(&mut P, Option<M>),
) -> Result<(), FrameworkError>
where
    M: Model
        + From<<M::Entity as EntityTrait>::Model>
        + serde::de::DeserializeOwned
        + EagerLoadDispatch,
    <M::Entity as EntityTrait>::Model: From<M>
        + sea_orm::IntoActiveModel<<M::Entity as EntityTrait>::ActiveModel>
        + sea_orm::FromQueryResult
        + Serialize
        + Send
        + Sync,
    <M::Entity as EntityTrait>::ActiveModel: Send,
    <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    let texts: Vec<Option<String>> = keys
        .iter()
        .map(|key| key.as_ref().map(serde_json::Value::to_string))
        .collect();
    let mut seen = HashSet::new();
    let distinct: Vec<serde_json::Value> = keys
        .iter()
        .flatten()
        .filter(|key| seen.insert(key.to_string()))
        .cloned()
        .collect();
    let mut grouped: HashMap<String, Vec<M>> = HashMap::new();
    if !distinct.is_empty() {
        let rows = M::query().filter_in(column, distinct).get().await?;
        for row in rows.into_vec() {
            let key = crate::eloquent::relations::eager_row_column(
                &row,
                Model::field_value(&row, column),
                column,
            );
            if let Some(key) = key {
                grouped.entry(key.to_string()).or_default().push(row);
            }
        }
    }
    for (model, owner) in models.iter_mut().zip(distribute(&texts, grouped)) {
        store(model, owner.into_iter().next());
    }
    Ok(())
}

/// Count, with one aggregation, the documents of `C` whose `field` holds
/// each of `keys` (one per model), as `with_count` reads a relation to
/// documents.
///
/// **Not part of the public API.** It is `pub` because the code the
/// relation macros emit calls it.
#[doc(hidden)]
pub async fn __count_documents<C: DocumentModel>(
    field: &str,
    keys: &[Option<Bson>],
) -> Result<Vec<u64>, FrameworkError> {
    let mut seen = HashSet::new();
    let distinct: Vec<Bson> = keys
        .iter()
        .flatten()
        .filter(|key| seen.insert(key_text(key)))
        .cloned()
        .collect();
    let mut counts: HashMap<String, u64> = HashMap::new();
    if !distinct.is_empty() {
        let mut stages = C::query().where_in(field, distinct).to_pipeline()?;
        let mut group = one("_id", format!("${}", storage::<C>(field)));
        group.insert("count", one("$sum", 1));
        stages.push(one("$group", group));
        let groups: Vec<Document> = C::collection()?
            .aggregate(stages)
            .await?
            .try_collect()
            .await?;
        for group in groups {
            let count = match group.get("count") {
                Some(Bson::Int32(count)) => u64::try_from(*count).unwrap_or(0),
                Some(Bson::Int64(count)) => u64::try_from(*count).unwrap_or(0),
                _ => 0,
            };
            let key = group.get("_id").cloned().unwrap_or(Bson::Null);
            counts.insert(key_text(&key), count);
        }
    }
    Ok(keys
        .iter()
        .map(|key| {
            key.as_ref()
                .and_then(|key| counts.get(&key_text(key)).copied())
                .unwrap_or(0)
        })
        .collect())
}

/// Load the relation path `rest` on the documents of `C` a relation of
/// `parents` loaded: take them out of every parent with `take`, load the
/// path across all of them with one query per relation, and put each
/// parent's back with `put_back`, however the load ends. With
/// `missing_only`, only the documents that lack the path's first relation
/// load it.
///
/// **Not part of the public API.** It is `pub` because the nested eager
/// loads the relation macros emit call it.
#[doc(hidden)]
pub async fn __load_nested_documents<P, C: DocumentModel>(
    parents: &mut [&mut P],
    take: fn(&mut P) -> Option<Vec<C>>,
    put_back: fn(&mut P, Vec<C>),
    rest: &str,
    missing_only: bool,
) -> Result<(), FrameworkError> {
    let (head, _) = __split_relation_path(rest);
    let mut taken = __TakenRows::take(parents, take, put_back);
    let mut loading: Vec<&mut C> = taken
        .rows()
        .iter_mut()
        .filter(|row| !missing_only || !row.relation_loaded(head))
        .collect();
    if loading.is_empty() {
        return Ok(());
    }
    C::__eager_load(rest, &mut loading).await
}

/// Load the relation path `rest` on the SQL models of `M` a
/// `BelongsToModel` relation of `parents` loaded, through the SQL eager
/// loader, and put each parent's model back however the load ends.
///
/// **Not part of the public API.** It is `pub` because the nested eager
/// loads the relation macros emit call it.
#[doc(hidden)]
pub async fn __load_nested_models<P, M>(
    parents: &mut [&mut P],
    take: fn(&mut P) -> Option<Vec<M>>,
    put_back: fn(&mut P, Vec<M>),
    rest: &str,
) -> Result<(), FrameworkError>
where
    M: EagerLoadDispatch + crate::eloquent::EloquentModel + Send + Sync,
{
    let mut taken = __TakenRows::take(parents, take, put_back);
    crate::eloquent::collection::load_relations::<M, _, _>(taken.rows(), [rest]).await
}
