//! The MongoDB cache store and its locks, behind the `database-mongodb`
//! feature, as `laravel-mongodb`'s cache driver and lock (PAR-187).
//!
//! `CACHE_DRIVER=mongodb` builds [`MongoCache`] on the default MongoDB
//! connection, over the collections `cache` and `cache_locks`.
//!
//! # The documents
//!
//! An entry is one document in `cache`: `_id` (the cache prefix and the
//! key), `value`, `expires_at` (a BSON datetime, or null for an entry that
//! never expires) and `tags`. A value that is a canonical signed 64-bit
//! integer is stored as a BSON integer and every other value as text, as
//! `laravel-mongodb` keeps numbers as numbers, so `$inc` adds to a counter
//! on the server.
//!
//! A lock is one document in `cache_locks`: `_id` (the prefix and the key,
//! unique as every `_id` is), `owner` (the token that holds it) and
//! `expires_at`.
//!
//! # Expiry
//!
//! Both collections have a TTL index on `expires_at`, created on the
//! store's first operation, so the server removes what expired. The server
//! sweeps once a minute, so every read also compares `expires_at` with the
//! time now: an expired entry is never read, whether or not the sweep has
//! removed it yet.

use std::time::Duration;

use ::bson::{Bson, Document, doc};
use ::mongodb::Collection;
use ::mongodb::options::ReturnDocument;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::OnceCell;

use super::config::CacheConfig;
use super::store::{CacheStore, ConditionalIncrement};
use crate::error::FrameworkError;
use crate::mongodb::stores::{
    StoreIndex, TYPE_MISMATCH, bson_time, bson_time_after, create_indexes, is_duplicate_key,
    server_code, store_error, validate_collection_name,
};
use crate::mongodb::{Mongo, MongoConnection};

/// The collection [`MongoCache::new`] keeps its entries in, Laravel's
/// `cache`.
pub const DEFAULT_MONGO_CACHE_COLLECTION: &str = "cache";

/// The collection [`MongoCache::new`] keeps its locks in, Laravel's
/// `cache_locks`.
pub const DEFAULT_MONGO_CACHE_LOCKS_COLLECTION: &str = "cache_locks";

/// How the store's errors name it.
const STORE: &str = "the MongoDB cache";

/// How many times a counter update starts again after it found an expired
/// entry in its way, or an entry another caller changed between its update
/// and its read. Each round removes the expired entry or meets a changed
/// one, so a round that does not finish needs a writer changing the same
/// key that often at the same moment.
const COUNTER_ROUNDS: usize = 16;

/// `raw` as the store keeps it: a BSON integer when it is the canonical
/// decimal of a signed 64-bit integer (no `+`, no leading zero, no `-0`),
/// text otherwise.
fn stored_value(raw: &str) -> Bson {
    match raw.parse::<i64>() {
        Ok(number) if number.to_string() == raw => Bson::Int64(number),
        _ => Bson::String(raw.to_owned()),
    }
}

/// A stored value as the JSON text the store answers.
fn raw_value(value: Option<&Bson>) -> Result<String, FrameworkError> {
    match value {
        Some(Bson::String(text)) => Ok(text.clone()),
        Some(Bson::Int64(number)) => Ok(number.to_string()),
        Some(Bson::Int32(number)) => Ok(number.to_string()),
        Some(Bson::Double(number)) => serde_json::to_string(number)
            .map_err(|error| FrameworkError::internal(format!("{STORE}: {error}"))),
        _ => Err(FrameworkError::internal(format!(
            "{STORE}: an entry holds no value this store wrote"
        ))),
    }
}

/// A counter's value, or the error a value that is no signed 64-bit
/// integer is.
fn counter_value(entry: &Document, operation: &str) -> Result<i64, FrameworkError> {
    match entry.get("value") {
        Some(Bson::Int64(number)) => Ok(*number),
        Some(Bson::Int32(number)) => Ok(i64::from(*number)),
        _ => Err(not_an_integer(operation)),
    }
}

fn not_an_integer(operation: &str) -> FrameworkError {
    FrameworkError::internal(format!(
        "Cache {operation} error: stored value is not a signed 64-bit integer"
    ))
}

/// The `expires_at` of an entry written at `now` for `ttl`; null for none.
fn expiry(ttl: Option<Duration>, now: DateTime<Utc>) -> Bson {
    match ttl {
        Some(ttl) => Bson::DateTime(bson_time_after(now, ttl)),
        None => Bson::Null,
    }
}

/// Unexpired at `now`: no expiry, or one still to come.
fn live(now: DateTime<Utc>) -> Document {
    doc! {
        "$or": [
            { "expires_at": Bson::Null },
            { "expires_at": { "$gt": bson_time(now) } },
        ]
    }
}

/// Expired by `now`. A null expiry is never expired: `$lte` on a date
/// matches only dates.
fn expired(now: DateTime<Utc>) -> Document {
    doc! { "expires_at": { "$lte": bson_time(now) } }
}

/// `text` as a regular expression that matches it character for
/// character.
fn regex_literal(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if "\\^$.|?*+()[]{}".contains(character) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// [`CacheStore`] over MongoDB collections, Laravel's `cache` and
/// `cache_locks` by default. See the [module docs](self) for the documents.
///
/// Every write is one server operation on one document, which the server
/// applies atomically: [`CacheStore::add_raw`] is an upsert that collides
/// with a live entry, each counter update is one `findOneAndUpdate` with
/// `$inc`, and [`CacheStore::acquire_lock`] is an upsert that collides with
/// a live lock. The locks live in MongoDB, which every process shares.
pub struct MongoCache {
    entries: Collection<Document>,
    locks: Collection<Document>,
    prefix: String,
    default_ttl: Option<Duration>,
    indexes: OnceCell<()>,
}

impl MongoCache {
    /// The store over [`DEFAULT_MONGO_CACHE_COLLECTION`] and
    /// [`DEFAULT_MONGO_CACHE_LOCKS_COLLECTION`] on `connection`, with the
    /// prefix and the default TTL of `config`. Nothing is sent to the
    /// server until the first operation.
    pub fn new(connection: &MongoConnection, config: &CacheConfig) -> Self {
        Self::over(
            connection.collection(DEFAULT_MONGO_CACHE_COLLECTION),
            connection.collection(DEFAULT_MONGO_CACHE_LOCKS_COLLECTION),
            config,
        )
    }

    /// The store over the collections `entries` and `locks` on
    /// `connection`.
    ///
    /// # Errors
    ///
    /// When MongoDB would refuse either name: empty, holding `$` or a NUL
    /// character, or starting with `system.`.
    pub fn with_collections(
        connection: &MongoConnection,
        config: &CacheConfig,
        entries: &str,
        locks: &str,
    ) -> Result<Self, FrameworkError> {
        validate_collection_name(entries, "the cache entries")?;
        validate_collection_name(locks, "the cache locks")?;
        Ok(Self::over(
            connection.collection(entries),
            connection.collection(locks),
            config,
        ))
    }

    /// The store `CACHE_DRIVER=mongodb` builds: [`Self::new`] on the
    /// default MongoDB connection, which the boot registers before the
    /// cache.
    ///
    /// # Errors
    ///
    /// When no MongoDB connection is registered, with the reason
    /// [`Mongo::connection`] gives, which names `MONGODB_URI`.
    pub fn from_config(config: &CacheConfig) -> Result<Self, FrameworkError> {
        let connection = Mongo::connection().map_err(|error| {
            FrameworkError::internal(format!(
                "CACHE_DRIVER=mongodb runs on the default MongoDB connection: {error}"
            ))
        })?;
        Ok(Self::new(&connection, config))
    }

    fn over(
        entries: Collection<Document>,
        locks: Collection<Document>,
        config: &CacheConfig,
    ) -> Self {
        Self {
            entries,
            locks,
            prefix: config.prefix.clone(),
            default_ttl: (config.default_ttl > 0).then(|| Duration::from_secs(config.default_ttl)),
            indexes: OnceCell::new(),
        }
    }

    /// The `_id` of the entry or the lock `key`.
    fn key(&self, key: &str) -> String {
        format!("{}{key}", self.prefix)
    }

    /// The indexes of the entries and of the locks, by role.
    fn indexes() -> (Vec<StoreIndex>, Vec<StoreIndex>) {
        (
            vec![
                StoreIndex::expiring("expires_at"),
                StoreIndex::on(doc! { "tags": 1 }),
            ],
            vec![StoreIndex::expiring("expires_at")],
        )
    }

    /// Create the indexes once per store.
    async fn ready(&self) -> Result<(), FrameworkError> {
        self.indexes
            .get_or_try_init(|| async {
                let (entries, locks) = Self::indexes();
                create_indexes(STORE, &self.entries, entries).await?;
                create_indexes(STORE, &self.locks, locks).await
            })
            .await?;
        Ok(())
    }

    /// The indexes the store creates on its first operation, as the server
    /// lists them, under `cache` and `cache_locks`.
    #[doc(hidden)]
    pub fn rendered_indexes(&self) -> Document {
        let (entries, locks) = Self::indexes();
        let rendered = |indexes: Vec<StoreIndex>| -> Vec<Bson> {
            indexes
                .iter()
                .map(|index| Bson::Document(index.rendered()))
                .collect()
        };
        doc! { "cache": rendered(entries), "cache_locks": rendered(locks) }
    }

    /// The `_id` condition that scopes a query to this store's prefix, or
    /// an empty document when the prefix is empty. One helper for `flush`
    /// and tag flushes, so both keep the same boundary.
    fn within_prefix(&self) -> Document {
        if self.prefix.is_empty() {
            doc! {}
        } else {
            doc! { "_id": { "$regex": format!("^{}", regex_literal(&self.prefix)) } }
        }
    }

    /// The whole entry a write of `value` under `key` with `tags` stores.
    fn entry(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
        tags: &[&str],
        now: DateTime<Utc>,
    ) -> Document {
        doc! {
            "_id": self.key(key),
            "value": stored_value(value),
            "expires_at": expiry(ttl, now),
            "tags": tags.to_vec(),
        }
    }

    /// The replacement a put sends, as `filter`, `replacement` and
    /// `upsert`.
    #[doc(hidden)]
    pub fn rendered_put(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
        now: DateTime<Utc>,
    ) -> Document {
        self.rendered_tagged_put(&[], key, value, ttl, now)
    }

    /// The replacement a tagged put sends, as [`Self::rendered_put`].
    #[doc(hidden)]
    pub fn rendered_tagged_put(
        &self,
        tags: &[&str],
        key: &str,
        value: &str,
        ttl: Option<Duration>,
        now: DateTime<Utc>,
    ) -> Document {
        doc! {
            "filter": { "_id": self.key(key) },
            "replacement": self.entry(key, value, ttl, tags, now),
            "upsert": true,
        }
    }

    /// The upsert an add sends: it replaces an expired entry, and collides
    /// with a live one on `_id`.
    #[doc(hidden)]
    pub fn rendered_add(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
        now: DateTime<Utc>,
    ) -> Document {
        let mut filter = doc! { "_id": self.key(key) };
        filter.extend(expired(now));
        doc! {
            "filter": filter,
            "replacement": self.entry(key, value, ttl, &[], now),
            "upsert": true,
        }
    }

    /// The `findOneAndUpdate` an increment by `amount` sends.
    #[doc(hidden)]
    pub fn rendered_increment(&self, key: &str, amount: i64, now: DateTime<Utc>) -> Document {
        let (filter, update) = self.increment_parts(key, amount, None, now);
        doc! { "filter": filter, "update": update, "upsert": true }
    }

    /// The `findOneAndUpdate` an increment below `ceiling` sends.
    #[doc(hidden)]
    pub fn rendered_increment_if_below(
        &self,
        key: &str,
        amount: i64,
        ceiling: i64,
        now: DateTime<Utc>,
    ) -> Document {
        let (filter, update) = self.increment_parts(key, amount, Some(ceiling), now);
        doc! { "filter": filter, "update": update, "upsert": ceiling > 0 }
    }

    /// The delete a flush of `tags` sends. It is scoped to the store's
    /// prefix, so a store never removes another store's entry that shares
    /// the tag.
    #[doc(hidden)]
    pub fn rendered_flush_tags(&self, tags: &[&str]) -> Document {
        let mut filter = doc! { "tags": { "$in": tags.to_vec() } };
        filter.extend(self.within_prefix());
        doc! { "filter": filter }
    }

    /// The upsert an acquire of the lock `key` for `owner` sends: it takes
    /// an expired lock over, and collides with a live one on `_id`.
    #[doc(hidden)]
    pub fn rendered_acquire_lock(
        &self,
        key: &str,
        owner: &str,
        ttl: Duration,
        now: DateTime<Utc>,
    ) -> Document {
        let mut filter = doc! { "_id": self.key(key) };
        filter.extend(expired(now));
        doc! {
            "filter": filter,
            "update": { "$set": { "owner": owner, "expires_at": bson_time_after(now, ttl) } },
            "upsert": true,
        }
    }

    /// The filter and update of a counter step: a live entry, below
    /// `ceiling` when one is given, gets `amount` added; a missing one is
    /// created with `amount` and no expiry.
    fn increment_parts(
        &self,
        key: &str,
        amount: i64,
        ceiling: Option<i64>,
        now: DateTime<Utc>,
    ) -> (Document, Document) {
        let mut filter = doc! { "_id": self.key(key) };
        if let Some(ceiling) = ceiling {
            filter.insert("value", doc! { "$lt": ceiling });
        }
        filter.extend(live(now));
        let update = doc! {
            "$inc": { "value": amount },
            "$setOnInsert": { "expires_at": Bson::Null, "tags": Vec::<Bson>::new() },
        };
        (filter, update)
    }

    /// The live entry `key`, if there is one.
    async fn live_entry(
        &self,
        key: &str,
        now: DateTime<Utc>,
        operation: &str,
    ) -> Result<Option<Document>, FrameworkError> {
        let mut filter = doc! { "_id": self.key(key) };
        filter.extend(live(now));
        self.entries
            .find_one(filter)
            .await
            .map_err(|error| store_error(STORE, operation, error))
    }

    /// Remove the entry `key` if it expired, so an upsert can take its
    /// `_id`.
    async fn remove_expired(
        &self,
        key: &str,
        now: DateTime<Utc>,
        operation: &str,
    ) -> Result<(), FrameworkError> {
        let mut filter = doc! { "_id": self.key(key) };
        filter.extend(expired(now));
        self.entries
            .delete_one(filter)
            .await
            .map_err(|error| store_error(STORE, operation, error))?;
        Ok(())
    }

    /// Write the whole entry, replacing any entry under the key.
    async fn replace(&self, rendered: Document, operation: &str) -> Result<(), FrameworkError> {
        let (filter, replacement) = parts(rendered, "replacement")?;
        self.ready().await?;
        for round in 0..2 {
            match self
                .entries
                .replace_one(filter.clone(), &replacement)
                .upsert(true)
                .await
            {
                Ok(_) => return Ok(()),
                // Two upserts of a new key at once: the loser's insert
                // collides, and the next round replaces the winner's.
                Err(error) if round == 0 && is_duplicate_key(&error) => {}
                Err(error) => return Err(store_error(STORE, operation, error)),
            }
        }
        Ok(())
    }

    /// Add `amount` to the counter `key`, as one `findOneAndUpdate`.
    async fn add_to_counter(
        &self,
        key: &str,
        amount: i64,
        operation: &str,
    ) -> Result<i64, FrameworkError> {
        self.ready().await?;
        for _ in 0..COUNTER_ROUNDS {
            let now = crate::clock::now();
            let (filter, update) = self.increment_parts(key, amount, None, now);
            match self
                .entries
                .find_one_and_update(filter, update)
                .upsert(true)
                .return_document(ReturnDocument::After)
                .await
            {
                Ok(Some(entry)) => return counter_value(&entry, operation),
                Ok(None) => {}
                // An expired entry holds the `_id` the upsert needs.
                Err(error) if is_duplicate_key(&error) => {
                    self.remove_expired(key, now, operation).await?;
                }
                Err(error) if server_code(&error) == Some(TYPE_MISMATCH) => {
                    return Err(not_an_integer(operation));
                }
                Err(error) => return Err(store_error(STORE, operation, error)),
            }
        }
        Err(contended(operation, key))
    }
}

/// The error of a counter that other writers kept changing.
fn contended(operation: &str, key: &str) -> FrameworkError {
    FrameworkError::internal(format!(
        "Cache {operation} error: the entry {key:?} changed under each of {COUNTER_ROUNDS} attempts"
    ))
}

/// The `filter` and the part `second` of a rendered operation.
fn parts(mut rendered: Document, second: &str) -> Result<(Document, Document), FrameworkError> {
    match (rendered.remove("filter"), rendered.remove(second)) {
        (Some(Bson::Document(filter)), Some(Bson::Document(other))) => Ok((filter, other)),
        _ => Err(FrameworkError::internal(format!(
            "{STORE}: an operation without its filter and {second}"
        ))),
    }
}

#[async_trait]
impl CacheStore for MongoCache {
    async fn get_raw(&self, key: &str) -> Result<Option<String>, FrameworkError> {
        self.ready().await?;
        self.live_entry(key, crate::clock::now(), "get")
            .await?
            .map(|entry| raw_value(entry.get("value")))
            .transpose()
    }

    async fn put_raw(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), FrameworkError> {
        self.replace(
            self.rendered_put(key, value, ttl, crate::clock::now()),
            "put",
        )
        .await
    }

    /// One upsert whose filter matches only an expired entry: a missing key
    /// is inserted, an expired entry replaced, and a live entry makes the
    /// insert collide on `_id`, which answers `false`.
    async fn add_raw(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<bool, FrameworkError> {
        let (filter, replacement) = parts(
            self.rendered_add(key, value, ttl, crate::clock::now()),
            "replacement",
        )?;
        self.ready().await?;
        match self
            .entries
            .replace_one(filter, &replacement)
            .upsert(true)
            .await
        {
            Ok(result) => Ok(result.upserted_id.is_some() || result.matched_count > 0),
            Err(error) if is_duplicate_key(&error) => Ok(false),
            Err(error) => Err(store_error(STORE, "add", error)),
        }
    }

    fn default_ttl(&self) -> Option<Duration> {
        self.default_ttl
    }

    async fn has(&self, key: &str) -> Result<bool, FrameworkError> {
        self.ready().await?;
        Ok(self
            .live_entry(key, crate::clock::now(), "has")
            .await?
            .is_some())
    }

    /// Removes the entry, expired or not, and answers whether a live one
    /// went.
    async fn forget(&self, key: &str) -> Result<bool, FrameworkError> {
        self.ready().await?;
        let now = crate::clock::now();
        let removed = self
            .entries
            .find_one_and_delete(doc! { "_id": self.key(key) })
            .await
            .map_err(|error| store_error(STORE, "forget", error))?;
        Ok(removed.is_some_and(|entry| match entry.get("expires_at") {
            Some(Bson::DateTime(at)) => *at > bson_time(now),
            _ => true,
        }))
    }

    /// Removes the entries under this store's prefix, and only those, so
    /// two applications that share the collection keep apart. The locks
    /// stay.
    async fn flush(&self) -> Result<(), FrameworkError> {
        self.ready().await?;
        self.entries
            .delete_many(self.within_prefix())
            .await
            .map_err(|error| store_error(STORE, "flush", error))?;
        Ok(())
    }

    async fn increment(&self, key: &str, amount: i64) -> Result<i64, FrameworkError> {
        self.add_to_counter(key, amount, "increment").await
    }

    async fn decrement(&self, key: &str, amount: i64) -> Result<i64, FrameworkError> {
        let amount = amount.checked_neg().ok_or_else(|| {
            FrameworkError::internal("Cache decrement error: signed 64-bit integer overflow")
        })?;
        self.add_to_counter(key, amount, "decrement").await
    }

    /// One `findOneAndUpdate` whose filter holds the comparison with the
    /// ceiling, so no caller gets past it. When it matches nothing, the
    /// live entry answers how full the counter is.
    async fn increment_if_below(
        &self,
        key: &str,
        amount: i64,
        ceiling: i64,
    ) -> Result<ConditionalIncrement, FrameworkError> {
        const OPERATION: &str = "increment";
        self.ready().await?;
        for _ in 0..COUNTER_ROUNDS {
            let now = crate::clock::now();
            let (filter, update) = self.increment_parts(key, amount, Some(ceiling), now);
            match self
                .entries
                .find_one_and_update(filter, update)
                .upsert(ceiling > 0)
                .return_document(ReturnDocument::After)
                .await
            {
                Ok(Some(entry)) => {
                    return counter_value(&entry, OPERATION).map(ConditionalIncrement::Incremented);
                }
                Ok(None) => {}
                Err(error) if is_duplicate_key(&error) => {}
                Err(error) => return Err(store_error(STORE, OPERATION, error)),
            }
            match self.live_entry(key, now, OPERATION).await? {
                Some(entry) => {
                    let value = counter_value(&entry, OPERATION)?;
                    if value >= ceiling {
                        return Ok(ConditionalIncrement::Unchanged(value));
                    }
                    // Below the ceiling again: another caller changed it
                    // between the update and the read.
                }
                None if ceiling <= 0 => return Ok(ConditionalIncrement::Unchanged(0)),
                // An expired entry holds the `_id` the upsert needs.
                None => self.remove_expired(key, now, OPERATION).await?,
            }
        }
        Err(contended(OPERATION, key))
    }

    async fn tagged_put_raw(
        &self,
        tags: &[&str],
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), FrameworkError> {
        self.replace(
            self.rendered_tagged_put(tags, key, value, ttl, crate::clock::now()),
            "tagged put",
        )
        .await
    }

    /// One delete of every entry whose current tags hold any of `tags`. The
    /// flush is scoped to the store's prefix, so another store's entry with
    /// the same tag stays. An untagged write cleared the entry's tags, so
    /// it is not removed.
    async fn flush_tags(&self, tags: &[&str]) -> Result<(), FrameworkError> {
        if tags.is_empty() {
            return Ok(());
        }
        self.ready().await?;
        let mut rendered = self.rendered_flush_tags(tags);
        let Some(Bson::Document(filter)) = rendered.remove("filter") else {
            return Err(FrameworkError::internal(format!(
                "{STORE}: a flush without its filter"
            )));
        };
        self.entries
            .delete_many(filter)
            .await
            .map_err(|error| store_error(STORE, "flush tags", error))?;
        Ok(())
    }

    async fn acquire_lock(
        &self,
        key: &str,
        ttl: Duration,
    ) -> Result<Option<String>, FrameworkError> {
        let owner = uuid::Uuid::new_v4().to_string();
        let (filter, update) = parts(
            self.rendered_acquire_lock(key, &owner, ttl, crate::clock::now()),
            "update",
        )?;
        self.ready().await?;
        match self.locks.update_one(filter, update).upsert(true).await {
            Ok(_) => Ok(Some(owner)),
            // A live lock holds the key.
            Err(error) if is_duplicate_key(&error) => Ok(None),
            Err(error) => Err(store_error(STORE, "lock acquire", error)),
        }
    }

    async fn release_lock(&self, key: &str, token: &str) -> Result<bool, FrameworkError> {
        self.ready().await?;
        let mut filter = doc! { "_id": self.key(key), "owner": token };
        filter.insert("expires_at", doc! { "$gt": bson_time(crate::clock::now()) });
        Ok(self
            .locks
            .delete_one(filter)
            .await
            .map_err(|error| store_error(STORE, "lock release", error))?
            .deleted_count
            > 0)
    }

    async fn refresh_lock(
        &self,
        key: &str,
        token: &str,
        ttl: Duration,
    ) -> Result<bool, FrameworkError> {
        self.ready().await?;
        let now = crate::clock::now();
        let filter = doc! {
            "_id": self.key(key),
            "owner": token,
            "expires_at": { "$gt": bson_time(now) },
        };
        Ok(self
            .locks
            .update_one(
                filter,
                doc! { "$set": { "expires_at": bson_time_after(now, ttl) } },
            )
            .await
            .map_err(|error| store_error(STORE, "lock refresh", error))?
            .matched_count
            > 0)
    }

    async fn touch(&self, key: &str, ttl: Duration) -> Result<bool, FrameworkError> {
        self.ready().await?;
        let now = crate::clock::now();
        let mut filter = doc! { "_id": self.key(key) };
        filter.extend(live(now));
        Ok(self
            .entries
            .update_one(
                filter,
                doc! { "$set": { "expires_at": bson_time_after(now, ttl) } },
            )
            .await
            .map_err(|error| store_error(STORE, "touch", error))?
            .matched_count
            > 0)
    }

    fn locks_are_shared(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "mongodb"
    }
}
