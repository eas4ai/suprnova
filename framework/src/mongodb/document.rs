//! Document models: the trait `#[suprnova::document]` implements, the keys
//! and casts their fields use, and the relations over embedded documents.
//!
//! A document model is a struct stored in one collection. The macro writes
//! the parts that depend on the struct (its fields, how each is written and
//! read, its guard and its serialized fields); [`DocumentModel`] supplies
//! the Eloquent calls on top: `create`, `find`, `update`, `delete`, the
//! soft-delete calls and the array operators.
//!
//! Two names for one document's values appear here. The *stored document*
//! is what the collection holds: the key under `_id`. The *attributes* are
//! what the application passes and the cancellable events carry: the key
//! under its field name, `id` unless the model names another.

use std::fmt::Debug;
use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::Arc;

use ::bson::oid::ObjectId;
use ::bson::{Bson, Decimal128, Document};
use ::mongodb::Collection;
use ::mongodb::options::ReturnDocument;
use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use serde::{Serialize, Serializer};
use tokio::sync::Mutex;

use super::config::DEFAULT_MONGO_CONNECTION;
use super::events::{self, DocumentObserver, SharedAttributes};
use super::facade::Mongo;
use super::query::DocumentQuery;
use crate::eloquent::{Attrs, Collection as Models, Fillable};
use crate::error::FrameworkError;

/// The type of a document model's key, which the collection stores as
/// `_id`.
///
/// Implemented for [`ObjectId`], the default key, which a model generates
/// when it is made, and for `String`, `i64` and `i32`, which the
/// application gives. Implement it for another key type to use that type.
pub trait DocumentKey:
    Serialize + DeserializeOwned + Clone + Debug + Send + Sync + 'static
{
    /// A new key for a document made without one, or `None` when the
    /// attributes must give it.
    fn generate() -> Option<Self> {
        None
    }

    /// The key a route parameter names, or `None` when the parameter is no
    /// key of this type. A route that answers `None` binds nothing, and the
    /// router answers 404, without a query.
    fn parse_route_key(value: &str) -> Option<Self>;

    /// The route parameter that names this key.
    fn to_route_key(&self) -> String;
}

impl DocumentKey for ObjectId {
    fn generate() -> Option<Self> {
        Some(ObjectId::new())
    }

    fn parse_route_key(value: &str) -> Option<Self> {
        ObjectId::parse_str(value).ok()
    }

    fn to_route_key(&self) -> String {
        self.to_hex()
    }
}

impl DocumentKey for String {
    fn parse_route_key(value: &str) -> Option<Self> {
        Some(value.to_owned())
    }

    fn to_route_key(&self) -> String {
        self.clone()
    }
}

impl DocumentKey for i64 {
    fn parse_route_key(value: &str) -> Option<Self> {
        value.parse().ok()
    }

    fn to_route_key(&self) -> String {
        self.to_string()
    }
}

impl DocumentKey for i32 {
    fn parse_route_key(value: &str) -> Option<Self> {
        value.parse().ok()
    }

    fn to_route_key(&self) -> String {
        self.to_string()
    }
}

/// How a field of type `T` is written to BSON and read back, where serde's
/// own form is not the BSON type for it. `casts = { field = Cast }` on the
/// model selects one; the macro selects [`AsBsonDateTime`] and
/// [`AsDecimal128`] by the field's type.
pub trait DocumentCast<T> {
    /// The BSON value stored for `value`.
    ///
    /// # Errors
    ///
    /// When the value has no BSON form.
    fn to_bson(value: &T) -> Result<Bson, FrameworkError>;

    /// The field's value from the stored `value`.
    ///
    /// # Errors
    ///
    /// When the stored value is not one the cast reads.
    fn from_bson(value: Bson) -> Result<T, FrameworkError>;
}

/// Stores a `chrono::DateTime<Utc>` as a BSON datetime, which MongoDB
/// sorts, compares and indexes as a date, where serde would store a
/// string. The macro selects it for every `DateTime<Utc>` field and
/// `Option` of one.
///
/// A BSON datetime holds milliseconds, so a finer time reads back
/// truncated to the millisecond. Reading also accepts an RFC 3339 string,
/// which is how a date arrives in attributes built with `doc!`.
pub struct AsBsonDateTime;

impl DocumentCast<DateTime<Utc>> for AsBsonDateTime {
    fn to_bson(value: &DateTime<Utc>) -> Result<Bson, FrameworkError> {
        Ok(Bson::DateTime(::bson::DateTime::from_millis(
            value.timestamp_millis(),
        )))
    }

    fn from_bson(value: Bson) -> Result<DateTime<Utc>, FrameworkError> {
        match value {
            Bson::DateTime(moment) => {
                DateTime::<Utc>::from_timestamp_millis(moment.timestamp_millis()).ok_or_else(|| {
                    FrameworkError::internal(format!(
                        "the BSON datetime {} is outside the dates chrono represents",
                        moment.timestamp_millis()
                    ))
                })
            }
            Bson::String(text) => DateTime::parse_from_rfc3339(&text)
                .map(|moment| moment.with_timezone(&Utc))
                .map_err(|error| {
                    FrameworkError::internal(format!("`{text}` is no RFC 3339 date-time: {error}"))
                }),
            other => Err(FrameworkError::internal(format!(
                "a {:?} value is no date-time",
                other.element_type()
            ))),
        }
    }
}

impl DocumentCast<Option<DateTime<Utc>>> for AsBsonDateTime {
    fn to_bson(value: &Option<DateTime<Utc>>) -> Result<Bson, FrameworkError> {
        match value {
            Some(moment) => <Self as DocumentCast<DateTime<Utc>>>::to_bson(moment),
            None => Ok(Bson::Null),
        }
    }

    fn from_bson(value: Bson) -> Result<Option<DateTime<Utc>>, FrameworkError> {
        match value {
            Bson::Null | Bson::Undefined => Ok(None),
            other => <Self as DocumentCast<DateTime<Utc>>>::from_bson(other).map(Some),
        }
    }
}

/// Stores a `rust_decimal::Decimal` as a BSON `Decimal128`, which MongoDB
/// adds and compares exactly, where serde would store a string. The macro
/// selects it for every `Decimal` field and `Option` of one.
///
/// Reading also accepts a string, an integer and a double, the forms a
/// decimal arrives in through `doc!` or from another writer.
pub struct AsDecimal128;

fn decimal_from_text(text: &str) -> Result<Decimal, FrameworkError> {
    Decimal::from_str(text)
        .or_else(|_| Decimal::from_scientific(text))
        .map_err(|error| FrameworkError::internal(format!("`{text}` is no decimal: {error}")))
}

impl DocumentCast<Decimal> for AsDecimal128 {
    fn to_bson(value: &Decimal) -> Result<Bson, FrameworkError> {
        Decimal128::from_str(&value.to_string())
            .map(Bson::Decimal128)
            .map_err(|error| {
                FrameworkError::internal(format!("{value} has no Decimal128 form: {error}"))
            })
    }

    fn from_bson(value: Bson) -> Result<Decimal, FrameworkError> {
        match value {
            Bson::Decimal128(decimal) => decimal_from_text(&decimal.to_string()),
            Bson::String(text) => decimal_from_text(&text),
            Bson::Int32(number) => Ok(Decimal::from(number)),
            Bson::Int64(number) => Ok(Decimal::from(number)),
            Bson::Double(number) => Decimal::try_from(number).map_err(|error| {
                FrameworkError::internal(format!("{number} is no decimal: {error}"))
            }),
            other => Err(FrameworkError::internal(format!(
                "a {:?} value is no decimal",
                other.element_type()
            ))),
        }
    }
}

impl DocumentCast<Option<Decimal>> for AsDecimal128 {
    fn to_bson(value: &Option<Decimal>) -> Result<Bson, FrameworkError> {
        match value {
            Some(decimal) => <Self as DocumentCast<Decimal>>::to_bson(decimal),
            None => Ok(Bson::Null),
        }
    }

    fn from_bson(value: Bson) -> Result<Option<Decimal>, FrameworkError> {
        match value {
            Bson::Null | Bson::Undefined => Ok(None),
            other => <Self as DocumentCast<Decimal>>::from_bson(other).map(Some),
        }
    }
}

/// A document model: a struct stored in one MongoDB collection, with the
/// Eloquent calls. `#[suprnova::document]` implements it; the constants
/// and the four required methods are what the macro writes, and every
/// other method is provided.
///
/// The calls that write fire the model events of
/// [`events`](super::events), in Laravel's order: `Saving`, then
/// `Creating` or `Updating`, the write, then `Created` or `Updated`, then
/// `Saved`. A cancellable listener that cancels stops the call before it
/// writes.
#[async_trait]
pub trait DocumentModel: Sized + Clone + Debug + Send + Sync + 'static {
    /// The key's type: the type of the field stored as `_id`.
    type Key: DocumentKey;

    /// The collection the documents are stored in.
    const COLLECTION: &'static str;
    /// The named MongoDB connection, or `None` for the default one.
    const CONNECTION: Option<&'static str>;
    /// The field stored as `_id`.
    const KEY_FIELD: &'static str;
    /// Every field, the key included, by its Rust name.
    const FIELDS: &'static [&'static str];
    /// The managed `created_at` and `updated_at` fields, or `None` when
    /// the model manages no timestamps.
    const TIMESTAMPS: Option<(&'static str, &'static str)>;
    /// The soft-delete field, or `None` when the model deletes documents.
    const SOFT_DELETES: Option<&'static str>;
    /// The fields serialization leaves out.
    const HIDDEN: &'static [&'static str];
    /// The only fields serialization writes, when set.
    const VISIBLE: Option<&'static [&'static str]>;
    /// The relations the model declares in `relations = { ... }`, by
    /// name, in declaration order. `with(..)` refuses a name outside it
    /// before it sends any query.
    const RELATIONS: &'static [&'static str] = &[];

    /// The document's key.
    fn key(&self) -> &Self::Key;

    /// The mass-assignment guard `create`, `update`, `make` and `fill`
    /// apply, from `fillable` or `guarded`.
    fn fillable() -> Fillable;

    /// The model from a stored document: `_id` fills the key, a missing
    /// `Option` reads as `None` and a missing `Vec` as empty.
    ///
    /// # Errors
    ///
    /// When a field is missing that has no empty value, or a stored value
    /// does not read as its field's type; the error names the field.
    fn from_document(document: Document) -> Result<Self, FrameworkError>;

    /// The stored document: every field, the key as `_id`, through the
    /// fields' casts.
    ///
    /// # Errors
    ///
    /// When a field's value has no BSON form.
    fn to_document(&self) -> Result<Document, FrameworkError>;

    /// The model's collection, on its connection, as raw documents.
    ///
    /// # Errors
    ///
    /// When the connection is not registered; see
    /// [`Mongo::connection`](super::Mongo::connection).
    fn collection() -> Result<Collection<Document>, FrameworkError> {
        let name = Self::CONNECTION.unwrap_or(DEFAULT_MONGO_CONNECTION);
        Ok(Mongo::connection_named(name)?.collection(Self::COLLECTION))
    }

    /// A query over the model's documents, leaving trashed ones out when
    /// the model soft deletes.
    fn query() -> DocumentQuery<Self> {
        DocumentQuery::new()
    }

    /// A query that includes trashed documents.
    fn with_trashed() -> DocumentQuery<Self> {
        DocumentQuery::new().with_trashed()
    }

    /// A query over trashed documents only.
    fn only_trashed() -> DocumentQuery<Self> {
        DocumentQuery::new().only_trashed()
    }

    /// The model's attributes: its stored document with the key under its
    /// field name, as the cancellable events carry them.
    ///
    /// # Errors
    ///
    /// As [`Self::to_document`].
    fn attributes(&self) -> Result<Document, FrameworkError> {
        Ok(storage_to_attributes::<Self>(self.to_document()?))
    }

    /// A model from `attrs` that is not stored, as Laravel's `make`: the
    /// guard applies, and an `ObjectId` key is generated when `attrs`
    /// gives none.
    ///
    /// # Errors
    ///
    /// When `attrs` names a field the model does not have, gives a value
    /// of the wrong type, or lacks a key that cannot be generated, and as
    /// the guard's strict mode
    /// ([`prevent_silently_discarding_attributes`](crate::eloquent::fillable::prevent_silently_discarding_attributes)).
    fn make(attrs: Document) -> Result<Self, FrameworkError> {
        let mut attributes = guard::<Self>(attrs, "make")?;
        ensure_key::<Self>(&mut attributes, "make")?;
        from_attributes::<Self>(attributes, "make")
    }

    /// Set the fields `attrs` names, through the guard, without writing.
    /// Save the model to store them.
    ///
    /// # Errors
    ///
    /// As [`Self::make`], and when `attrs` changes the key.
    fn fill(&mut self, attrs: Document) -> Result<(), FrameworkError> {
        let attrs = guard::<Self>(attrs, "fill")?;
        refuse_key_change(self, &attrs, "fill")?;
        let mut merged = self.attributes()?;
        merged.extend(attrs);
        *self = from_attributes::<Self>(merged, "fill")?;
        Ok(())
    }

    /// Insert a document from `attrs` and answer it, as Laravel's
    /// `create`. The guard applies; the timestamps are set unless `attrs`
    /// gives them; an `ObjectId` key is generated unless `attrs` gives one.
    ///
    /// # Errors
    ///
    /// As [`Self::make`]; when a `Saving` or `Creating` listener cancels;
    /// and when the insert fails, a duplicate key included.
    async fn create(attrs: Document) -> Result<Self, FrameworkError> {
        create_guarded::<Self>(attrs, Document::new(), "create").await
    }

    /// The document whose key is `id`, or `None`. A trashed document is
    /// `None`; [`Self::with_trashed`] finds it.
    ///
    /// # Errors
    ///
    /// When the query fails or the stored document does not read as the
    /// model.
    async fn find<K>(id: K) -> Result<Option<Self>, FrameworkError>
    where
        K: Into<Self::Key> + Send,
    {
        let key = key_to_bson::<Self>(&id.into())?;
        Self::query().where_stored_key(key).first().await
    }

    /// The document whose key is `id`, or a 404 error naming the model.
    ///
    /// # Errors
    ///
    /// As [`Self::find`], and `FrameworkError::ModelNotFound` when there is
    /// no such document.
    async fn find_or_fail<K>(id: K) -> Result<Self, FrameworkError>
    where
        K: Into<Self::Key> + Send,
    {
        let key: Self::Key = id.into();
        match Self::find(key.clone()).await? {
            Some(model) => Ok(model),
            None => Err(FrameworkError::not_found(format!(
                "{} with {} = {:?} not found",
                model_name::<Self>(),
                Self::KEY_FIELD,
                key
            ))),
        }
    }

    /// Every document, trashed ones left out.
    ///
    /// # Errors
    ///
    /// As [`Self::find`].
    async fn all() -> Result<Models<Self>, FrameworkError> {
        Self::query().get().await
    }

    /// Store the model: update its document when one has its key, else
    /// insert it. An update writes every field and sets `updated_at`; an
    /// insert sets both timestamps.
    ///
    /// # Errors
    ///
    /// When a listener cancels, or a read or the write fails.
    async fn save(&mut self) -> Result<(), FrameworkError> {
        let key = key_to_bson::<Self>(self.key())?;
        let stored = Self::collection()?.find_one(one("_id", key)).await?;
        match stored {
            Some(stored) => {
                let previous = Self::from_document(stored)?;
                let mut changes = self.attributes()?;
                changes.remove(Self::KEY_FIELD);
                if let Some((_, updated_at)) = Self::TIMESTAMPS {
                    changes.insert(updated_at, now());
                }
                *self = persist_update(previous, self, changes, "save").await?;
            }
            None => {
                let mut attributes = self.attributes()?;
                if let Some((created_at, updated_at)) = Self::TIMESTAMPS {
                    let now = now();
                    attributes.insert(updated_at, now.clone());
                    if matches!(attributes.get(created_at), None | Some(Bson::Null)) {
                        attributes.insert(created_at, now);
                    }
                }
                let shared: SharedAttributes = Arc::new(Mutex::new(attributes));
                events::saving::<Self>(&shared, true).await?;
                events::creating::<Self>(&shared).await?;
                let attributes = shared.lock().await.clone();
                let model = from_attributes::<Self>(attributes, "save")?;
                Self::collection()?.insert_one(model.to_document()?).await?;
                events::created(&model).await?;
                events::saved(&model).await?;
                *self = model;
            }
        }
        Ok(())
    }

    /// Set the fields `attrs` names and store them with `$set`, leaving
    /// the document's other fields as they are, and answer the document as
    /// the server holds it after. The guard applies, and `updated_at` is
    /// set unless `attrs` gives it.
    ///
    /// # Errors
    ///
    /// As [`Self::make`]; when `attrs` changes the key; when a listener
    /// cancels; and when the document no longer exists.
    async fn update(self, attrs: Document) -> Result<Self, FrameworkError> {
        let mut changes = guard::<Self>(attrs, "update")?;
        refuse_key_change(&self, &changes, "update")?;
        changes.remove(Self::KEY_FIELD);
        if let Some((_, updated_at)) = Self::TIMESTAMPS
            && !changes.contains_key(updated_at)
        {
            changes.insert(updated_at, now());
        }
        persist_update(self.clone(), &self, changes, "update").await
    }

    /// Delete the document. A soft-deleting model sets its soft-delete
    /// field instead and keeps the document, which queries then leave out.
    ///
    /// # Errors
    ///
    /// When a `Deleting` listener cancels, the write fails, or the
    /// document no longer exists.
    async fn delete(self) -> Result<(), FrameworkError> {
        events::deleting(&self, false).await?;
        let key = key_to_bson::<Self>(self.key())?;
        match Self::SOFT_DELETES {
            Some(column) => {
                let now = now();
                let mut set = one(column, now.clone());
                if let Some((_, updated_at)) = Self::TIMESTAMPS {
                    set.insert(updated_at, now);
                }
                let stored = Self::collection()?
                    .find_one_and_update(one("_id", key), one("$set", set))
                    .return_document(ReturnDocument::After)
                    .await?
                    .ok_or_else(|| gone::<Self>("delete"))?;
                let trashed = Self::from_document(stored)?;
                events::trashed(&trashed).await?;
                events::deleted(&trashed, false).await?;
            }
            None => {
                let result = Self::collection()?.delete_one(one("_id", key)).await?;
                if result.deleted_count == 0 {
                    return Err(gone::<Self>("delete"));
                }
                events::deleted(&self, false).await?;
            }
        }
        Ok(())
    }

    /// Remove the document, trashed or not. On a model without soft
    /// deletes it is [`Self::delete`] with `is_force` set in the events.
    ///
    /// # Errors
    ///
    /// When a `Deleting` listener cancels, the write fails, or the
    /// document no longer exists.
    async fn force_delete(self) -> Result<(), FrameworkError> {
        events::deleting(&self, true).await?;
        events::force_deleting(&self).await?;
        let key = key_to_bson::<Self>(self.key())?;
        let result = Self::collection()?.delete_one(one("_id", key)).await?;
        if result.deleted_count == 0 {
            return Err(gone::<Self>("force_delete"));
        }
        events::force_deleted(&self).await?;
        events::deleted(&self, true).await?;
        Ok(())
    }

    /// Clear the soft-delete field and answer the restored document. As in
    /// Laravel, the restore is a save: `Restoring`, then the update's
    /// events, then `Restored`.
    ///
    /// # Errors
    ///
    /// When the model does not soft delete (the error names `restore`),
    /// when a listener cancels, or when the document no longer exists.
    async fn restore(self) -> Result<Self, FrameworkError> {
        let Some(column) = Self::SOFT_DELETES else {
            return Err(FrameworkError::internal(format!(
                "restore: `{}` does not soft delete; add `soft_deletes` to its \
                 #[document] attribute",
                model_name::<Self>()
            )));
        };
        events::restoring(&self).await?;
        let mut changes = one(column, Bson::Null);
        if let Some((_, updated_at)) = Self::TIMESTAMPS {
            changes.insert(updated_at, now());
        }
        let restored = persist_update(self.clone(), &self, changes, "restore").await?;
        events::restored(&restored).await?;
        Ok(restored)
    }

    /// The document as the server holds it now, trashed or not, or `None`
    /// when it no longer exists.
    ///
    /// # Errors
    ///
    /// As [`Self::find`].
    async fn fresh(&self) -> Result<Option<Self>, FrameworkError> {
        let key = key_to_bson::<Self>(self.key())?;
        Self::with_trashed().where_stored_key(key).first().await
    }

    /// Reload the model from the server, in place.
    ///
    /// # Errors
    ///
    /// As [`Self::find`], and a 404 error when the document no longer
    /// exists.
    async fn refresh(&mut self) -> Result<(), FrameworkError> {
        match self.fresh().await? {
            Some(current) => {
                *self = current;
                Ok(())
            }
            None => Err(gone::<Self>("refresh")),
        }
    }

    /// Append `value` to the array `field` with `$push`, and reload the
    /// model from the result. `field` may be a dotted path into an
    /// embedded document. On a model with timestamps, the same write sets
    /// `updated_at`.
    ///
    /// # Errors
    ///
    /// When `field` is no field of the model or is the key, when `value`
    /// has no BSON form, and when the write fails or the document no
    /// longer exists.
    async fn push<V>(&mut self, field: &str, value: V) -> Result<(), FrameworkError>
    where
        V: Serialize + Send + Sync,
    {
        let update = __rendered_array_update::<Self, V>("$push", field, &value, now_moment())?;
        *self = write_operator(self, update, "push").await?;
        Ok(())
    }

    /// Append `value` to the array `field` unless it holds it already, with
    /// `$addToSet`, as Laravel's `push($field, $value, true)`. Sets
    /// `updated_at` as [`Self::push`] does.
    ///
    /// # Errors
    ///
    /// As [`Self::push`].
    async fn push_unique<V>(&mut self, field: &str, value: V) -> Result<(), FrameworkError>
    where
        V: Serialize + Send + Sync,
    {
        let update = __rendered_array_update::<Self, V>("$addToSet", field, &value, now_moment())?;
        *self = write_operator(self, update, "push_unique").await?;
        Ok(())
    }

    /// Remove every element equal to `value` from the array `field`, with
    /// `$pull`. Sets `updated_at` as [`Self::push`] does.
    ///
    /// # Errors
    ///
    /// As [`Self::push`].
    async fn pull<V>(&mut self, field: &str, value: V) -> Result<(), FrameworkError>
    where
        V: Serialize + Send + Sync,
    {
        let update = __rendered_array_update::<Self, V>("$pull", field, &value, now_moment())?;
        *self = write_operator(self, update, "pull").await?;
        Ok(())
    }

    /// Add `by` to the number `field` with `$inc`, set `updated_at`, and
    /// reload the model. Fires `Updating`, whose attributes carry the
    /// amount under the field's name, and `Updated`, as Laravel's
    /// `increment` does.
    ///
    /// # Errors
    ///
    /// When `by` is no integer or float (the error names `increment`), as
    /// [`Self::push`], and when an `Updating` listener cancels.
    async fn increment<V>(&mut self, field: &str, by: V) -> Result<(), FrameworkError>
    where
        V: Into<Bson> + Send,
    {
        let amount = number(by.into(), "increment")?;
        increment_by(self, field, amount, "increment").await
    }

    /// Subtract `by` from the number `field`; [`Self::increment`] with the
    /// amount negated.
    ///
    /// # Errors
    ///
    /// As [`Self::increment`], naming `decrement`.
    async fn decrement<V>(&mut self, field: &str, by: V) -> Result<(), FrameworkError>
    where
        V: Into<Bson> + Send,
    {
        let amount = negate(number(by.into(), "decrement")?, "decrement")?;
        increment_by(self, field, amount, "decrement").await
    }

    /// Register `observer` for every event of the model, as Laravel's
    /// `User::observe(UserObserver::class)`. See
    /// [`observe`](super::events::observe).
    async fn observe<O: DocumentObserver<Self>>(observer: O) {
        events::observe::<Self, O>(observer).await;
    }

    /// Whether `with(..)` loaded the relation `relation` on this model,
    /// as Laravel's `relationLoaded`. A loaded relation that found no
    /// document is loaded.
    fn relation_loaded(&self, relation: &str) -> bool {
        let _ = relation;
        false
    }

    /// Load the relation path `path` (`"posts"`, or `"posts.comments"`
    /// for a relation of each post) onto every model of `models`, with
    /// one query per relation of the path.
    ///
    /// **Not part of the public API.** `#[suprnova::document]` writes it
    /// for a model that declares relations; [`DocumentQuery::with`]
    /// calls it.
    ///
    /// # Errors
    ///
    /// When the model has no relation of the path's first name, and as
    /// the queries that load the path.
    #[doc(hidden)]
    async fn __eager_load(path: &str, models: &mut [&mut Self]) -> Result<(), FrameworkError> {
        let _ = models;
        Err(super::relations::__unknown_relation::<Self>(path))
    }
}

/// [`DocumentModel::create`] with `forced` set after the guard: the
/// foreign key a relation's `create` writes, which the caller's
/// attributes cannot change and the guard does not drop.
pub(crate) async fn create_guarded<M: DocumentModel>(
    attrs: Document,
    forced: Document,
    call: &str,
) -> Result<M, FrameworkError> {
    let mut attributes = guard::<M>(attrs, call)?;
    for (name, value) in forced {
        if !M::FIELDS.contains(&name.as_str()) {
            return Err(FrameworkError::internal(format!(
                "{call}: `{name}` is no field of {}",
                model_name::<M>()
            )));
        }
        attributes.insert(name, value);
    }
    ensure_key::<M>(&mut attributes, call)?;
    if let Some((created_at, updated_at)) = M::TIMESTAMPS {
        let now = now();
        for field in [created_at, updated_at] {
            if matches!(attributes.get(field), None | Some(Bson::Null)) {
                attributes.insert(field, now.clone());
            }
        }
    }
    let shared: SharedAttributes = Arc::new(Mutex::new(attributes));
    events::saving::<M>(&shared, true).await?;
    events::creating::<M>(&shared).await?;
    let attributes = shared.lock().await.clone();
    let model = from_attributes::<M>(attributes, call)?;
    M::collection()?.insert_one(model.to_document()?).await?;
    events::created(&model).await?;
    events::saved(&model).await?;
    Ok(model)
}

// --- What the provided methods share ---------------------------------------

/// A document of one `key: value` pair: a key the code computes, which
/// `doc!` does not take.
pub(crate) fn one(key: impl Into<String>, value: impl Into<Bson>) -> Document {
    let mut document = Document::new();
    document.insert(key, value);
    document
}

fn now() -> Bson {
    Bson::DateTime(now_moment())
}

fn now_moment() -> ::bson::DateTime {
    ::bson::DateTime::now()
}

/// The last segment of `M`'s path, for messages.
pub(crate) fn model_name<M>() -> &'static str {
    let full = std::any::type_name::<M>();
    full.rsplit("::").next().unwrap_or(full)
}

/// The error for a document that no longer exists when `call` needs it.
fn gone<M: DocumentModel>(call: &str) -> FrameworkError {
    FrameworkError::not_found(format!(
        "{call}: the document of {} no longer exists",
        model_name::<M>()
    ))
}

/// The stored form of `key`.
pub(crate) fn key_to_bson<M: DocumentModel>(key: &M::Key) -> Result<Bson, FrameworkError> {
    ::bson::serialize_to_bson(key).map_err(|error| {
        FrameworkError::internal(format!(
            "the key of {} has no BSON form: {error}",
            model_name::<M>()
        ))
    })
}

fn value_to_bson<V: Serialize>(value: &V, call: &str) -> Result<Bson, FrameworkError> {
    ::bson::serialize_to_bson(value).map_err(|error| {
        FrameworkError::internal(format!("{call}: the value has no BSON form: {error}"))
    })
}

/// The attributes with the key moved to `_id`, first.
pub(crate) fn attributes_to_storage<M: DocumentModel>(mut attributes: Document) -> Document {
    let mut stored = Document::new();
    if let Some(key) = attributes.remove(M::KEY_FIELD) {
        stored.insert("_id", key);
    }
    stored.extend(attributes);
    stored
}

/// The stored document with `_id` moved to the key's field name, first.
pub(crate) fn storage_to_attributes<M: DocumentModel>(mut stored: Document) -> Document {
    let mut attributes = Document::new();
    if let Some(key) = stored.remove("_id") {
        attributes.insert(M::KEY_FIELD, key);
    }
    attributes.extend(stored);
    attributes
}

/// Apply the model's guard to `attrs`, then refuse a name that is no
/// field. `_id` is read as the key's field name.
fn guard<M: DocumentModel>(attrs: Document, call: &str) -> Result<Document, FrameworkError> {
    let attrs = storage_to_attributes::<M>(attrs);
    // The guard works on `Attrs`; it decides by name alone, so the names
    // go through it and the BSON values stay here.
    let mut names = Attrs::new();
    for name in attrs.keys() {
        names.insert(name.clone(), serde_json::Value::Null);
    }
    let kept = M::fillable().apply_checked(names)?;
    let mut guarded = Document::new();
    for (name, value) in attrs {
        if !kept.contains_key(&name) {
            continue;
        }
        if !M::FIELDS.contains(&name.as_str()) {
            return Err(FrameworkError::bad_request(format!(
                "{call}: `{name}` is no field of {}",
                model_name::<M>()
            )));
        }
        guarded.insert(name, value);
    }
    Ok(guarded)
}

/// Give the attributes a key: the one they hold, or a generated one.
fn ensure_key<M: DocumentModel>(
    attributes: &mut Document,
    call: &str,
) -> Result<(), FrameworkError> {
    if !matches!(attributes.get(M::KEY_FIELD), None | Some(Bson::Null)) {
        return Ok(());
    }
    match <M::Key as DocumentKey>::generate() {
        Some(key) => {
            attributes.insert(M::KEY_FIELD, key_to_bson::<M>(&key)?);
            Ok(())
        }
        None => Err(FrameworkError::bad_request(format!(
            "{call}: {} needs its key `{}` in the attributes, and the key must be fillable",
            model_name::<M>(),
            M::KEY_FIELD
        ))),
    }
}

/// The model the attributes describe. A value of the wrong type is the
/// caller's error, so it answers 400.
fn from_attributes<M: DocumentModel>(
    attributes: Document,
    call: &str,
) -> Result<M, FrameworkError> {
    M::from_document(attributes_to_storage::<M>(attributes))
        .map_err(|error| FrameworkError::bad_request(format!("{call}: {}", error.message())))
}

/// Refuse attributes that give `model` another key: MongoDB never changes
/// a stored `_id`.
fn refuse_key_change<M: DocumentModel>(
    model: &M,
    attrs: &Document,
    call: &str,
) -> Result<(), FrameworkError> {
    match attrs.get(M::KEY_FIELD) {
        Some(given) if *given != key_to_bson::<M>(model.key())? => {
            Err(FrameworkError::bad_request(format!(
                "{call}: the key `{}` of a stored {} cannot change",
                M::KEY_FIELD,
                model_name::<M>()
            )))
        }
        _ => Ok(()),
    }
}

/// A field `call` may write: a field of the model, or a dotted path into
/// one, and not the key.
fn writable_field<M: DocumentModel>(field: &str, call: &str) -> Result<String, FrameworkError> {
    let root = field.split('.').next().unwrap_or(field);
    if root == M::KEY_FIELD || root == "_id" {
        return Err(FrameworkError::internal(format!(
            "{call}: the key `{}` cannot be written",
            M::KEY_FIELD
        )));
    }
    if !M::FIELDS.contains(&root) {
        return Err(FrameworkError::internal(format!(
            "{call}: `{field}` is no field of {}",
            model_name::<M>()
        )));
    }
    Ok(field.to_owned())
}

/// `value` when it is an integer or a float, which `$inc` takes.
pub(crate) fn number(value: Bson, call: &str) -> Result<Bson, FrameworkError> {
    match value {
        Bson::Int32(_) | Bson::Int64(_) | Bson::Double(_) => Ok(value),
        other => Err(FrameworkError::internal(format!(
            "{call}: the amount must be an integer or a float, not {other}"
        ))),
    }
}

/// `-value` for a number [`number`] accepted.
pub(crate) fn negate(value: Bson, call: &str) -> Result<Bson, FrameworkError> {
    let negated = match value {
        Bson::Int32(number) => number.checked_neg().map(Bson::Int32),
        Bson::Int64(number) => number.checked_neg().map(Bson::Int64),
        Bson::Double(number) => Some(Bson::Double(-number)),
        _ => None,
    };
    negated.ok_or_else(|| {
        FrameworkError::internal(format!("{call}: the amount has no negative to subtract"))
    })
}

/// The update `push`, `push_unique` and `pull` send for `operator`
/// (`$push`, `$addToSet` or `$pull`): `{operator: {field: value}}`, and on
/// a model with timestamps `{"$set": {updated_at: now}}` in the same
/// update, so the write that changes the array advances `updated_at`.
///
/// **Not part of the public API.** It is `pub` so a test can assert the
/// update without a server; the model's array operators call it, so the
/// test sees what they send.
///
/// # Errors
///
/// When `operator` is no array operator, when `value` has no BSON form,
/// and when `field` is no field of `M` or is the key. Each error names
/// the call the operator belongs to.
#[doc(hidden)]
pub fn __rendered_array_update<M: DocumentModel, V: Serialize>(
    operator: &str,
    field: &str,
    value: &V,
    now: ::bson::DateTime,
) -> Result<Document, FrameworkError> {
    let call = match operator {
        "$push" => "push",
        "$addToSet" => "push_unique",
        "$pull" => "pull",
        other => {
            return Err(FrameworkError::internal(format!(
                "`{other}` is no array operator: the array calls send `$push`, `$addToSet` \
                 or `$pull`"
            )));
        }
    };
    let value = value_to_bson(value, call)?;
    let field = writable_field::<M>(field, call)?;
    let mut update = one(operator, one(field, value));
    set_updated_at::<M>(&mut update, now);
    Ok(update)
}

/// Add `{"$set": {updated_at: now}}` to the one-operator `update` when `M`
/// manages timestamps. Every operator call of the model builds its update
/// through it, so none of them can leave `updated_at` behind.
fn set_updated_at<M: DocumentModel>(update: &mut Document, now: ::bson::DateTime) {
    if let Some((_, updated_at)) = M::TIMESTAMPS {
        update.insert("$set", one(updated_at, Bson::DateTime(now)));
    }
}

/// Apply `update` to `model`'s document and answer the document after it.
async fn write_operator<M: DocumentModel>(
    model: &M,
    update: Document,
    call: &str,
) -> Result<M, FrameworkError> {
    let key = key_to_bson::<M>(model.key())?;
    let stored = M::collection()?
        .find_one_and_update(one("_id", key), update)
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| gone::<M>(call))?;
    M::from_document(stored)
}

async fn increment_by<M: DocumentModel>(
    model: &mut M,
    field: &str,
    amount: Bson,
    call: &str,
) -> Result<(), FrameworkError> {
    let field = writable_field::<M>(field, call)?;
    let previous = model.clone();
    let shared: SharedAttributes = Arc::new(Mutex::new(one(field.clone(), amount)));
    events::updating(&previous, &shared).await?;
    // A listener may change the amount; the write takes what it left.
    let amount = number(
        shared
            .lock()
            .await
            .get(&field)
            .cloned()
            .unwrap_or(Bson::Null),
        call,
    )?;
    let mut update = one("$inc", one(field, amount));
    set_updated_at::<M>(&mut update, now_moment());
    *model = write_operator(model, update, call).await?;
    events::updated(&previous, model).await?;
    Ok(())
}

/// The update path `save`, `update` and `restore` share: the `Saving` and
/// `Updating` events over `changes`, a `$set` of the changed fields, and
/// the `Updated` and `Saved` events with the document as stored after.
async fn persist_update<M: DocumentModel>(
    previous: M,
    base: &M,
    changes: Document,
    call: &str,
) -> Result<M, FrameworkError> {
    let shared: SharedAttributes = Arc::new(Mutex::new(changes));
    events::saving::<M>(&shared, false).await?;
    events::updating(&previous, &shared).await?;
    let changes = shared.lock().await.clone();
    // A listener may have named the key or a field the model lacks.
    refuse_key_change(base, &changes, call)?;
    let mut merged = base.attributes()?;
    for (name, value) in &changes {
        if name != M::KEY_FIELD && !M::FIELDS.contains(&name.as_str()) {
            return Err(FrameworkError::bad_request(format!(
                "{call}: `{name}` is no field of {}",
                model_name::<M>()
            )));
        }
        merged.insert(name.clone(), value.clone());
    }
    // Reading the merged attributes checks every value's type before the
    // write; the stored form of the changed fields is what `$set` writes.
    let candidate = from_attributes::<M>(merged, call)?;
    let stored = candidate.to_document()?;
    let mut set = Document::new();
    for name in changes.keys().filter(|name| *name != M::KEY_FIELD) {
        if let Some(value) = stored.get(name) {
            set.insert(name.clone(), value.clone());
        }
    }
    let current = if set.is_empty() {
        candidate
    } else {
        write_operator(base, one("$set", set), call).await?
    };
    events::updated(&previous, &current).await?;
    events::saved(&current).await?;
    Ok(current)
}

// --- What the macro's code calls --------------------------------------------

/// Read the field `field` of `model` from `storage` in `document`, through
/// serde. A missing value reads as BSON null, or as an empty array when
/// `empty_when_missing`.
///
/// **Not part of the public API.** It is `pub` because the code
/// `#[suprnova::document]` emits calls it.
#[doc(hidden)]
pub fn __read_field<T: DeserializeOwned>(
    document: &mut Document,
    storage: &str,
    model: &str,
    field: &str,
    empty_when_missing: bool,
) -> Result<T, FrameworkError> {
    let (value, missing) = match document.remove(storage) {
        Some(value) => (value, false),
        None if empty_when_missing => (Bson::Array(Vec::new()), false),
        None => (Bson::Null, true),
    };
    ::bson::deserialize_from_bson(value).map_err(|error| {
        if missing {
            FrameworkError::internal(format!(
                "the document of {model} has no `{field}`; give it a value, or make the \
                 field an Option"
            ))
        } else {
            FrameworkError::internal(format!(
                "`{model}::{field}` does not read its stored value: {error}"
            ))
        }
    })
}

/// Read the field `field` of `model` through its cast `C`.
///
/// **Not part of the public API.** It is `pub` because the code
/// `#[suprnova::document]` emits calls it.
#[doc(hidden)]
pub fn __read_cast_field<T, C: DocumentCast<T>>(
    document: &mut Document,
    storage: &str,
    model: &str,
    field: &str,
) -> Result<T, FrameworkError> {
    let value = document.remove(storage).unwrap_or(Bson::Null);
    C::from_bson(value).map_err(|error| {
        FrameworkError::internal(format!(
            "`{model}::{field}` does not read its stored value: {}",
            error.message()
        ))
    })
}

/// Write the field `field` of `model` through serde.
///
/// **Not part of the public API.** It is `pub` because the code
/// `#[suprnova::document]` emits calls it.
#[doc(hidden)]
pub fn __write_field<T: Serialize>(
    value: &T,
    model: &str,
    field: &str,
) -> Result<Bson, FrameworkError> {
    ::bson::serialize_to_bson(value).map_err(|error| {
        FrameworkError::internal(format!("`{model}::{field}` has no BSON form: {error}"))
    })
}

/// Write the field `field` of `model` through its cast `C`.
///
/// **Not part of the public API.** It is `pub` because the code
/// `#[suprnova::document]` emits calls it.
#[doc(hidden)]
pub fn __write_cast_field<T, C: DocumentCast<T>>(
    value: &T,
    model: &str,
    field: &str,
) -> Result<Bson, FrameworkError> {
    C::to_bson(value).map_err(|error| {
        FrameworkError::internal(format!("`{model}::{field}`: {}", error.message()))
    })
}

/// Serialize `model` as its attributes, honouring `hidden` and `visible`:
/// the key under its field name, an `ObjectId` as its hex string, a
/// datetime as RFC 3339 text and a `Decimal128` as its decimal text.
///
/// **Not part of the public API.** It is `pub` because the `Serialize`
/// impl `#[suprnova::document]` emits calls it.
#[doc(hidden)]
pub fn __serialize_document<M: DocumentModel, S: Serializer>(
    model: &M,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let attributes = model.attributes().map_err(serde::ser::Error::custom)?;
    let mut object = serde_json::Map::new();
    for (name, value) in attributes {
        let shown = match M::VISIBLE {
            Some(visible) => visible.contains(&name.as_str()),
            None => !M::HIDDEN.contains(&name.as_str()),
        };
        if shown {
            object.insert(name, bson_to_json(value));
        }
    }
    serde_json::Value::Object(object).serialize(serializer)
}

/// The JSON form of a BSON value, as an application reads it.
fn bson_to_json(value: Bson) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Bson::ObjectId(id) => Value::String(id.to_hex()),
        Bson::DateTime(moment) => {
            match DateTime::<Utc>::from_timestamp_millis(moment.timestamp_millis()) {
                Some(moment) => Value::String(moment.to_rfc3339_opts(SecondsFormat::AutoSi, true)),
                None => Value::from(moment.timestamp_millis()),
            }
        }
        Bson::Decimal128(decimal) => Value::String(decimal.to_string()),
        Bson::Double(number) => serde_json::Number::from_f64(number)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Bson::String(text) => Value::String(text),
        Bson::Boolean(flag) => Value::Bool(flag),
        Bson::Int32(number) => Value::from(number),
        Bson::Int64(number) => Value::from(number),
        Bson::Null | Bson::Undefined => Value::Null,
        Bson::Array(items) => Value::Array(items.into_iter().map(bson_to_json).collect()),
        Bson::Document(document) => Value::Object(
            document
                .into_iter()
                .map(|(name, value)| (name, bson_to_json(value)))
                .collect(),
        ),
        other => serde_json::to_value(&other).unwrap_or(Value::Null),
    }
}

/// Find the document a route parameter names: by the key when `field` is
/// `None` or the key's name, else by the field `field` equal to `value`.
/// A value that is no key binds nothing without a query.
///
/// **Not part of the public API.** It is `pub` because the `RouteBinding`
/// impl `#[suprnova::document]` emits calls it.
#[doc(hidden)]
pub async fn __resolve_document_route_binding<M: DocumentModel>(
    value: &str,
    field: Option<&str>,
    with_trashed: bool,
) -> Result<Option<M>, FrameworkError> {
    let query = if with_trashed {
        M::with_trashed()
    } else {
        M::query()
    };
    match field {
        Some(name) if name != M::KEY_FIELD && name != "_id" => {
            if !M::FIELDS.contains(&name) {
                return Err(FrameworkError::internal(format!(
                    "the route binds {} by `{name}`, which is no field of it",
                    model_name::<M>()
                )));
            }
            query.where_(name, "=", value).first().await
        }
        _ => match <M::Key as DocumentKey>::parse_route_key(value) {
            Some(key) => {
                query
                    .where_stored_key(key_to_bson::<M>(&key)?)
                    .first()
                    .await
            }
            None => Ok(None),
        },
    }
}

// --- Embedded documents -----------------------------------------------------

/// The documents of an `#[embeds_many]` field, as a relation that writes
/// them: the method of the field's name answers it, as laravel-mongodb's
/// `embedsMany` relation. Each call writes to the server and reloads the
/// parent model from the result.
pub struct EmbedsMany<'a, P, T> {
    parent: &'a mut P,
    field: &'static str,
    item: PhantomData<fn() -> T>,
}

impl<'a, P: DocumentModel, T: Serialize + Sync> EmbedsMany<'a, P, T> {
    /// The relation over the field `field` of `parent`. The macro calls
    /// it; an application calls the field's method.
    pub fn new(parent: &'a mut P, field: &'static str) -> Self {
        Self {
            parent,
            field,
            item: PhantomData,
        }
    }

    /// Append `item` to the array, with `$push`.
    ///
    /// # Errors
    ///
    /// When `item` has no BSON form, and when the write fails or the
    /// parent's document no longer exists.
    pub async fn save(self, item: &T) -> Result<(), FrameworkError> {
        let item = value_to_bson(item, "save")?;
        *self.parent =
            write_operator(&*self.parent, one("$push", one(self.field, item)), "save").await?;
        Ok(())
    }

    /// Append every item of `items` to the array, in order, with one
    /// `$push`.
    ///
    /// # Errors
    ///
    /// As [`Self::save`].
    pub async fn save_many(self, items: &[T]) -> Result<(), FrameworkError> {
        let items = value_to_bson(&items, "save_many")?;
        *self.parent = write_operator(
            &*self.parent,
            one("$push", one(self.field, one("$each", items))),
            "save_many",
        )
        .await?;
        Ok(())
    }

    /// Remove every embedded document equal to `item`, with `$pull`.
    ///
    /// # Errors
    ///
    /// As [`Self::save`].
    pub async fn destroy(self, item: &T) -> Result<(), FrameworkError> {
        let item = value_to_bson(item, "destroy")?;
        *self.parent = write_operator(
            &*self.parent,
            one("$pull", one(self.field, item)),
            "destroy",
        )
        .await?;
        Ok(())
    }

    /// Remove every embedded document.
    ///
    /// # Errors
    ///
    /// When the write fails or the parent's document no longer exists.
    pub async fn clear(self) -> Result<(), FrameworkError> {
        *self.parent = write_operator(
            &*self.parent,
            one("$set", one(self.field, Bson::Array(Vec::new()))),
            "clear",
        )
        .await?;
        Ok(())
    }
}

/// The document of an `#[embeds_one]` field, as a relation that writes it:
/// the method of the field's name answers it, as laravel-mongodb's
/// `embedsOne` relation. Each call writes to the server and reloads the
/// parent model from the result.
pub struct EmbedsOne<'a, P, T> {
    parent: &'a mut P,
    field: &'static str,
    item: PhantomData<fn() -> T>,
}

impl<'a, P: DocumentModel, T: Serialize + Sync> EmbedsOne<'a, P, T> {
    /// The relation over the field `field` of `parent`. The macro calls
    /// it; an application calls the field's method.
    pub fn new(parent: &'a mut P, field: &'static str) -> Self {
        Self {
            parent,
            field,
            item: PhantomData,
        }
    }

    /// Store `item` as the embedded document, replacing the one there.
    ///
    /// # Errors
    ///
    /// When `item` has no BSON form, and when the write fails or the
    /// parent's document no longer exists.
    pub async fn save(self, item: &T) -> Result<(), FrameworkError> {
        let item = value_to_bson(item, "save")?;
        *self.parent =
            write_operator(&*self.parent, one("$set", one(self.field, item)), "save").await?;
        Ok(())
    }

    /// Remove the embedded document; the field reads as `None` after.
    ///
    /// # Errors
    ///
    /// When the write fails or the parent's document no longer exists.
    pub async fn delete(self) -> Result<(), FrameworkError> {
        *self.parent = write_operator(
            &*self.parent,
            one("$set", one(self.field, Bson::Null)),
            "delete",
        )
        .await?;
        Ok(())
    }
}
