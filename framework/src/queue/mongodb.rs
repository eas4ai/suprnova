//! The MongoDB queue driver, its failed-job store and its batch repository,
//! behind the `database-mongodb` feature, as `laravel-mongodb`'s queue
//! driver, failed-job provider and batch repository (PAR-186).
//!
//! `QUEUE_DRIVER=mongodb` builds all three on the default MongoDB
//! connection: [`MongoQueueDriver`] over `jobs`, [`MongoFailedJobStore`]
//! over `failed_jobs` and [`MongoBatchRepository`] over `job_batches`.
//!
//! # The jobs collection
//!
//! A job is one document with Laravel's fields: `queue`, `payload` (the
//! envelope's JSON with `displayName` and `uuid` added, as the database
//! driver writes it), `attempts`, `reserved_at`, `available_at` and
//! `created_at`, the last three as epoch seconds. Two fields of Suprnova's
//! own hold the reservation: `token`, the token a worker settles the job
//! with, and `reserved_until`, the moment the reservation lapses.
//!
//! A pop is one `findOneAndUpdate`: it picks the earliest available job
//! that no live reservation holds, sets the reservation and adds 1 to
//! `attempts`, all on one document, which the server changes atomically.
//! Two workers racing for one job cannot both receive it. `attempts` counts
//! the deliveries, as in Laravel, so the envelope a pop hands out carries
//! the attempts before this one; a nack keeps the count and a release gives
//! the delivery back.
//!
//! `available_at` holds whole seconds. A job due when it is pushed stores
//! its second rounded down, so it is available at once; a delayed job
//! stores its second rounded up, so it is never handed out before its time.
//!
//! Every pop, listing and `clear` reads only the documents this driver
//! wrote, whose payload starts with `{"schema_version":`, as the database
//! driver does on a table it shares with a Laravel application.

use std::time::Duration;

use ::bson::{Bson, Document, doc};
use ::mongodb::Collection;
use ::mongodb::options::ReturnDocument;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::error::FrameworkError;
use crate::mongodb::MongoConnection;
use crate::mongodb::stores::{
    StoreIndex, bson_time, bson_time_after, chrono_time, create_indexes, store_error,
    validate_collection_name,
};
use crate::queue::batch::{
    Batch, BatchOptions, BatchRepository, TerminalCallbackClaim, UpdatedBatchJobCounts,
};
use crate::queue::database::job_payload;
use crate::queue::driver::{QueueDriver, QueueFilterCapability, Reservation, ReservationToken};
use crate::queue::envelope::{DEFAULT_QUEUE, Envelope};
use crate::queue::failed::{FailedJob, FailedJobStore, SUPRNOVA_CONNECTION_PREFIX, failed_payload};
use crate::queue::inspect::InspectedJob;

/// The collection [`MongoQueueDriver::new`] keeps its jobs in, Laravel's
/// `jobs`.
pub const DEFAULT_MONGO_JOBS_COLLECTION: &str = "jobs";

/// The collection [`MongoFailedJobStore::new`] keeps its records in,
/// Laravel's `failed_jobs`.
pub const DEFAULT_MONGO_FAILED_JOBS_COLLECTION: &str = "failed_jobs";

/// The collection [`MongoBatchRepository::new`] keeps its batches in,
/// Laravel's `job_batches`.
pub const DEFAULT_MONGO_BATCHES_COLLECTION: &str = "job_batches";

/// The regular expression that matches the payloads this driver writes:
/// every serialized [`Envelope`] starts with its `schema_version`.
const OWN_PAYLOAD: &str = "^\\{\"schema_version\":";

/// How the errors of each store name it.
const QUEUE: &str = "the MongoDB queue";
const FAILED: &str = "the MongoDB failed-job store";
const BATCHES: &str = "the MongoDB batch repository";

/// The `queue` a job routed to `queue` is stored under: an unrouted job is
/// on [`DEFAULT_QUEUE`].
fn stored_queue(queue: Option<&str>) -> &str {
    queue.unwrap_or(DEFAULT_QUEUE)
}

/// `at` as the whole second `available_at` stores: rounded down when the
/// job is due by `now`, so it is available at once, and up otherwise, so it
/// is never handed out before its time.
fn stored_seconds(at: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    if at <= now || at.timestamp_subsec_nanos() == 0 {
        at.timestamp()
    } else {
        at.timestamp().saturating_add(1)
    }
}

/// `now` moved forward by `delay`.
///
/// # Errors
///
/// When no date can hold the sum: the delay comes from a caller or a
/// backoff, and a value that large is an error, as for every driver.
fn after(now: DateTime<Utc>, delay: Duration) -> Result<DateTime<Utc>, FrameworkError> {
    chrono::Duration::from_std(delay)
        .ok()
        .and_then(|delay| now.checked_add_signed(delay))
        .ok_or_else(|| {
            FrameworkError::internal(format!(
                "queue delay of {delay:?} runs past the dates the clock can hold"
            ))
        })
}

/// A counter field, whatever numeric type the document holds it in.
fn count_of(document: &Document, field: &str) -> i64 {
    match document.get(field) {
        Some(Bson::Int64(value)) => *value,
        Some(Bson::Int32(value)) => i64::from(*value),
        Some(Bson::Double(value)) if value.is_finite() => *value as i64,
        _ => 0,
    }
}

/// A date field, `None` when it is null or absent.
fn date_of(document: &Document, field: &str) -> Option<DateTime<Utc>> {
    match document.get(field) {
        Some(Bson::DateTime(at)) => Some(chrono_time(*at)),
        Some(Bson::Int64(seconds)) => DateTime::<Utc>::from_timestamp(*seconds, 0),
        Some(Bson::Int32(seconds)) => DateTime::<Utc>::from_timestamp(i64::from(*seconds), 0),
        _ => None,
    }
}

/// The text field `field`, or an error naming it and the store.
fn text_of<'a>(
    store: &str,
    document: &'a Document,
    field: &str,
) -> Result<&'a str, FrameworkError> {
    document.get_str(field).map_err(|error| {
        FrameworkError::internal(format!("{store}: the field `{field}` is not text: {error}"))
    })
}

// ---------------------------------------------------------------------------
// The queue driver
// ---------------------------------------------------------------------------

/// [`QueueDriver`] over a MongoDB collection, Laravel's `jobs` by default.
/// See the [module docs](self) for the documents it keeps.
///
/// The indexes it reads by, `{queue, available_at}` and `{token}`, are
/// created on its first operation. It cannot settle a chained job and enqueue
/// its successor as one step without a transaction, which a standalone
/// server lacks, so [`QueueDriver::settle`] answers
/// [`Settled::Unsupported`](crate::queue::Settled::Unsupported) and the
/// worker pushes the successor before it acknowledges, as for Redis.
pub struct MongoQueueDriver {
    jobs: Collection<Document>,
    indexes: OnceCell<()>,
}

impl MongoQueueDriver {
    /// The driver over [`DEFAULT_MONGO_JOBS_COLLECTION`] on `connection`.
    /// Nothing is sent to the server until the first operation.
    pub fn new(connection: &MongoConnection) -> Self {
        Self::over(connection.collection(DEFAULT_MONGO_JOBS_COLLECTION))
    }

    /// The driver over the collection `collection` on `connection`.
    ///
    /// # Errors
    ///
    /// When MongoDB would refuse the name: empty, holding `$` or a NUL
    /// character, or starting with `system.`.
    pub fn with_collection(
        connection: &MongoConnection,
        collection: &str,
    ) -> Result<Self, FrameworkError> {
        validate_collection_name(collection, "the queue's jobs")?;
        Ok(Self::over(connection.collection(collection)))
    }

    fn over(jobs: Collection<Document>) -> Self {
        Self {
            jobs,
            indexes: OnceCell::new(),
        }
    }

    /// The jobs collection, for what the trait does not reach.
    pub fn collection(&self) -> Collection<Document> {
        self.jobs.clone()
    }

    /// The collection, once its indexes exist.
    async fn ready(&self) -> Result<&Collection<Document>, FrameworkError> {
        self.indexes
            .get_or_try_init(|| {
                create_indexes(
                    QUEUE,
                    &self.jobs,
                    vec![
                        StoreIndex::on(doc! { "queue": 1, "available_at": 1 }),
                        StoreIndex::on(doc! { "token": 1 }),
                    ],
                )
            })
            .await?;
        Ok(&self.jobs)
    }

    /// The document a push of `env` at `now` inserts.
    #[doc(hidden)]
    pub fn rendered_push(
        &self,
        env: &Envelope,
        now: DateTime<Utc>,
    ) -> Result<Document, FrameworkError> {
        Ok(doc! {
            "queue": stored_queue(env.queue.as_deref()),
            "payload": job_payload(env)?,
            "attempts": i64::from(env.attempts),
            "reserved_at": Bson::Null,
            "available_at": stored_seconds(env.available_at, now),
            "created_at": env.dispatched_at.timestamp(),
            "token": Bson::Null,
            "reserved_until": Bson::Null,
        })
    }

    /// The `findOneAndUpdate` a pop from `queues` at `now` sends, as
    /// `filter`, `update` and `sort`.
    #[doc(hidden)]
    pub fn rendered_pop(
        &self,
        queues: &[String],
        visibility: Duration,
        now: DateTime<Utc>,
        token: Uuid,
    ) -> Document {
        let (filter, update, sort) = pop_parts(queues, visibility, now, token);
        doc! { "filter": filter, "update": update, "sort": sort }
    }

    /// The update a nack (`consume_attempt`) or a release of `token` with
    /// `delay` at `now` sends, as `filter` and `update`.
    #[doc(hidden)]
    pub fn rendered_requeue(
        &self,
        token: &ReservationToken,
        delay: Duration,
        consume_attempt: bool,
        now: DateTime<Utc>,
    ) -> Result<Document, FrameworkError> {
        let (filter, update) = requeue_parts(token, delay, consume_attempt, now)?;
        Ok(doc! { "filter": filter, "update": update })
    }

    /// Shared body of [`QueueDriver::pop`] and [`QueueDriver::pop_from`].
    async fn pop_filtered(
        &self,
        visibility: Duration,
        queues: &[String],
    ) -> Result<Option<Reservation>, FrameworkError> {
        let token = Uuid::new_v4();
        let (filter, update, sort) = pop_parts(queues, visibility, crate::clock::now(), token);
        let reserved = self
            .ready()
            .await?
            .find_one_and_update(filter, update)
            .sort(sort)
            .return_document(ReturnDocument::After)
            .await
            .map_err(|error| store_error(QUEUE, "pop", error))?;
        let Some(document) = reserved else {
            return Ok(None);
        };
        let mut envelope =
            Envelope::from_json(text_of(QUEUE, &document, "payload")?).map_err(|error| {
                FrameworkError::internal(format!("{QUEUE}: envelope decode: {error}"))
            })?;
        // `attempts` now counts this delivery too; the envelope carries the
        // ones before it, as every driver's does.
        envelope.attempts = attempts_before(count_of(&document, "attempts").saturating_sub(1));
        Ok(Some(Reservation {
            envelope,
            token: ReservationToken(token),
        }))
    }

    /// Return the job `token` holds `delay` from now.
    async fn requeue(
        &self,
        token: &ReservationToken,
        delay: Duration,
        consume_attempt: bool,
        operation: &str,
    ) -> Result<(), FrameworkError> {
        let (filter, update) = requeue_parts(token, delay, consume_attempt, crate::clock::now())?;
        // A token that holds nothing any more updates nothing: idempotent.
        self.ready()
            .await?
            .update_one(filter, update)
            .await
            .map_err(|error| store_error(QUEUE, operation, error))?;
        Ok(())
    }

    /// `COUNT` of the documents `filter` matches.
    async fn count(&self, filter: Document, operation: &str) -> Result<u64, FrameworkError> {
        self.ready()
            .await?
            .count_documents(filter)
            .await
            .map_err(|error| store_error(QUEUE, operation, error))
    }

    /// The documents `filter` matches, oldest due first, as inspected jobs.
    async fn list(
        &self,
        filter: Document,
        operation: &str,
    ) -> Result<Vec<InspectedJob>, FrameworkError> {
        let now = crate::clock::now();
        let documents: Vec<Document> = self
            .ready()
            .await?
            .find(filter)
            .sort(doc! { "available_at": 1, "_id": 1 })
            .await
            .map_err(|error| store_error(QUEUE, operation, error))?
            .try_collect()
            .await
            .map_err(|error| store_error(QUEUE, operation, error))?;
        Ok(documents
            .iter()
            .map(|document| inspected(document, now))
            .collect())
    }
}

/// A delivery count as the envelope's `attempts`.
fn attempts_before(count: i64) -> u32 {
    u32::try_from(count.max(0)).unwrap_or(u32::MAX)
}

/// The jobs this driver wrote, on `queue` when one is named.
fn own_jobs(queue: Option<&str>) -> Document {
    let mut filter = doc! { "payload": { "$regex": OWN_PAYLOAD } };
    if let Some(queue) = queue {
        filter.insert("queue", stored_queue(Some(queue)));
    }
    filter
}

/// Free at `now`: never reserved, or its reservation has lapsed.
fn free_at(now: DateTime<Utc>) -> Bson {
    Bson::Array(vec![
        Bson::Document(doc! { "token": Bson::Null }),
        Bson::Document(doc! { "reserved_until": { "$lte": bson_time(now) } }),
    ])
}

/// Held at `now` by a reservation that has not lapsed.
fn held_at(now: DateTime<Utc>) -> Document {
    doc! { "token": { "$ne": Bson::Null }, "reserved_until": { "$gt": bson_time(now) } }
}

/// The filter, update and sort of a pop.
fn pop_parts(
    queues: &[String],
    visibility: Duration,
    now: DateTime<Utc>,
    token: Uuid,
) -> (Document, Document, Document) {
    let mut filter = own_jobs(None);
    filter.insert("available_at", doc! { "$lte": now.timestamp() });
    filter.insert("$or", free_at(now));
    if !queues.is_empty() {
        let mut names: Vec<&str> = Vec::with_capacity(queues.len());
        for queue in queues {
            let name = stored_queue(Some(queue));
            if !names.contains(&name) {
                names.push(name);
            }
        }
        filter.insert("queue", doc! { "$in": names });
    }
    let update = doc! {
        "$set": {
            "reserved_at": now.timestamp(),
            "token": token.to_string(),
            "reserved_until": bson_time_after(now, visibility),
        },
        "$inc": { "attempts": 1_i64 },
    };
    (filter, update, doc! { "available_at": 1, "_id": 1 })
}

/// The filter and update that return the job `token` holds `delay` after
/// `now`, giving the delivery back unless `consume_attempt`.
fn requeue_parts(
    token: &ReservationToken,
    delay: Duration,
    consume_attempt: bool,
    now: DateTime<Utc>,
) -> Result<(Document, Document), FrameworkError> {
    let available_at = stored_seconds(after(now, delay)?, now);
    let mut update = doc! {
        "$set": {
            "available_at": available_at,
            "reserved_at": Bson::Null,
            "token": Bson::Null,
            "reserved_until": Bson::Null,
        },
    };
    if !consume_attempt {
        update.insert("$inc", doc! { "attempts": -1_i64 });
    }
    Ok((doc! { "token": token.0.to_string() }, update))
}

/// One stored job as the inspection calls report it. A payload that does
/// not decode is reported as unparseable rather than dropped, as the
/// database driver does: one corrupt job must not hide the others.
fn inspected(document: &Document, now: DateTime<Utc>) -> InspectedJob {
    let payload = document.get_str("payload").unwrap_or_default();
    match Envelope::from_json(payload) {
        Ok(mut envelope) => {
            let held = document.get_str("token").is_ok()
                && document
                    .get_datetime("reserved_until")
                    .is_ok_and(|until| chrono_time(*until) > now);
            let count = count_of(document, "attempts");
            envelope.attempts = attempts_before(if held { count - 1 } else { count });
            InspectedJob::from_envelope(&envelope)
        }
        Err(error) => {
            let name = serde_json::from_str::<serde_json::Value>(payload)
                .ok()
                .and_then(|value| {
                    value
                        .get("displayName")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_default();
            tracing::warn!(
                job_name = %name,
                error = %error,
                "queue listing: payload failed to parse; reporting the job as unparseable \
                 instead of dropping it"
            );
            InspectedJob {
                id: None,
                queue: None,
                name,
                attempts: 0,
                payload: serde_json::json!({ "unparseable": true }),
                created_at: None,
            }
        }
    }
}

#[async_trait]
impl QueueDriver for MongoQueueDriver {
    async fn push(&self, env: Envelope) -> Result<(), FrameworkError> {
        let document = self.rendered_push(&env, crate::clock::now())?;
        self.ready()
            .await?
            .insert_one(document)
            .await
            .map_err(|error| store_error(QUEUE, "push", error))?;
        Ok(())
    }

    async fn pop(
        &self,
        visibility_timeout: Duration,
    ) -> Result<Option<Reservation>, FrameworkError> {
        self.pop_filtered(visibility_timeout, &[]).await
    }

    fn queue_filter_capability(&self) -> QueueFilterCapability {
        QueueFilterCapability::Supported
    }

    async fn pop_from(
        &self,
        visibility_timeout: Duration,
        queues: &[String],
    ) -> Result<Option<Reservation>, FrameworkError> {
        self.pop_filtered(visibility_timeout, queues).await
    }

    async fn ack(&self, token: &ReservationToken) -> Result<(), FrameworkError> {
        self.ready()
            .await?
            .delete_one(doc! { "token": token.0.to_string() })
            .await
            .map_err(|error| store_error(QUEUE, "ack", error))?;
        Ok(())
    }

    async fn nack(
        &self,
        token: &ReservationToken,
        requeue_delay: Duration,
    ) -> Result<(), FrameworkError> {
        self.requeue(token, requeue_delay, true, "nack").await
    }

    /// In place, giving the delivery back: the next delivery carries the
    /// attempts this one did.
    async fn release(
        &self,
        token: &ReservationToken,
        _env: &Envelope,
        delay: Duration,
    ) -> Result<(), FrameworkError> {
        self.requeue(token, delay, false, "release").await
    }

    async fn size(&self, queue: Option<&str>) -> Result<u64, FrameworkError> {
        self.count(own_jobs(queue), "size").await
    }

    async fn pending_size(&self, queue: Option<&str>) -> Result<u64, FrameworkError> {
        self.count(pending_filter(queue, crate::clock::now()), "pending_size")
            .await
    }

    async fn delayed_size(&self, queue: Option<&str>) -> Result<u64, FrameworkError> {
        self.count(delayed_filter(queue, crate::clock::now()), "delayed_size")
            .await
    }

    async fn reserved_size(&self, queue: Option<&str>) -> Result<u64, FrameworkError> {
        self.count(reserved_filter(queue, crate::clock::now()), "reserved_size")
            .await
    }

    async fn pending_jobs(&self, queue: Option<&str>) -> Result<Vec<InspectedJob>, FrameworkError> {
        self.list(pending_filter(queue, crate::clock::now()), "pending_jobs")
            .await
    }

    async fn delayed_jobs(&self, queue: Option<&str>) -> Result<Vec<InspectedJob>, FrameworkError> {
        self.list(delayed_filter(queue, crate::clock::now()), "delayed_jobs")
            .await
    }

    async fn reserved_jobs(
        &self,
        queue: Option<&str>,
    ) -> Result<Vec<InspectedJob>, FrameworkError> {
        self.list(reserved_filter(queue, crate::clock::now()), "reserved_jobs")
            .await
    }

    async fn clear(&self, queue: Option<&str>) -> Result<u64, FrameworkError> {
        Ok(self
            .ready()
            .await?
            .delete_many(own_jobs(queue))
            .await
            .map_err(|error| store_error(QUEUE, "clear", error))?
            .deleted_count)
    }

    /// One `insertMany` for the whole list.
    async fn bulk_push(&self, envs: Vec<Envelope>) -> Result<(), FrameworkError> {
        if envs.is_empty() {
            return Ok(());
        }
        let now = crate::clock::now();
        let documents = envs
            .iter()
            .map(|env| self.rendered_push(env, now))
            .collect::<Result<Vec<_>, _>>()?;
        self.ready()
            .await?
            .insert_many(documents)
            .await
            .map_err(|error| store_error(QUEUE, "bulk push", error))?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "mongodb"
    }
}

/// Available by `now` and free: what a pop would take.
fn pending_filter(queue: Option<&str>, now: DateTime<Utc>) -> Document {
    let mut filter = own_jobs(queue);
    filter.insert("available_at", doc! { "$lte": now.timestamp() });
    filter.insert("$or", free_at(now));
    filter
}

/// Not yet available at `now`.
fn delayed_filter(queue: Option<&str>, now: DateTime<Utc>) -> Document {
    let mut filter = own_jobs(queue);
    filter.insert("available_at", doc! { "$gt": now.timestamp() });
    filter
}

/// Held by a live reservation at `now`.
fn reserved_filter(queue: Option<&str>, now: DateTime<Utc>) -> Document {
    let mut filter = own_jobs(queue);
    filter.extend(held_at(now));
    filter
}

// ---------------------------------------------------------------------------
// The failed-job store
// ---------------------------------------------------------------------------

/// [`FailedJobStore`] over a MongoDB collection, Laravel's `failed_jobs` by
/// default, as `laravel-mongodb`'s failed-job provider.
///
/// A record is one document: `uuid`, `connection` (the Suprnova connection
/// behind [`SUPRNOVA_CONNECTION_PREFIX`], as the database store writes it),
/// `queue`, `payload` (the envelope with `displayName` and `uuid` added),
/// `exception`, and `failed_at` as a BSON datetime. The indexes on `uuid`
/// and `failed_at` are created on the first operation. A collection needs
/// no migration, so [`FailedJobStore::check`] has nothing to refuse.
pub struct MongoFailedJobStore {
    failed: Collection<Document>,
    indexes: OnceCell<()>,
}

impl MongoFailedJobStore {
    /// The store over [`DEFAULT_MONGO_FAILED_JOBS_COLLECTION`] on
    /// `connection`.
    pub fn new(connection: &MongoConnection) -> Self {
        Self::over(connection.collection(DEFAULT_MONGO_FAILED_JOBS_COLLECTION))
    }

    /// The store over the collection `collection` on `connection`.
    ///
    /// # Errors
    ///
    /// When MongoDB would refuse the name, as for
    /// [`MongoQueueDriver::with_collection`].
    pub fn with_collection(
        connection: &MongoConnection,
        collection: &str,
    ) -> Result<Self, FrameworkError> {
        validate_collection_name(collection, "the failed jobs")?;
        Ok(Self::over(connection.collection(collection)))
    }

    fn over(failed: Collection<Document>) -> Self {
        Self {
            failed,
            indexes: OnceCell::new(),
        }
    }

    /// The failed-jobs collection, for what the trait does not reach.
    pub fn collection(&self) -> Collection<Document> {
        self.failed.clone()
    }

    async fn ready(&self) -> Result<&Collection<Document>, FrameworkError> {
        self.indexes
            .get_or_try_init(|| {
                create_indexes(
                    FAILED,
                    &self.failed,
                    vec![
                        StoreIndex::on(doc! { "uuid": 1 }),
                        StoreIndex::on(doc! { "failed_at": -1 }),
                    ],
                )
            })
            .await?;
        Ok(&self.failed)
    }
}

/// One stored record as a [`FailedJob`].
fn failed_job(document: &Document) -> Result<FailedJob, FrameworkError> {
    let uuid = text_of(FAILED, document, "uuid")?;
    let id = Uuid::parse_str(uuid).map_err(|error| {
        FrameworkError::internal(format!(
            "{FAILED}: the uuid `{uuid}` is not a UUID: {error}"
        ))
    })?;
    let stored_connection = text_of(FAILED, document, "connection")?;
    // A record Laravel wrote keeps its connection as Laravel named it.
    let connection = stored_connection
        .strip_prefix(SUPRNOVA_CONNECTION_PREFIX)
        .unwrap_or(stored_connection)
        .to_owned();
    let envelope_json = text_of(FAILED, document, "payload")?.to_owned();
    let job_name = serde_json::from_str::<serde_json::Value>(&envelope_json)
        .ok()
        .and_then(|payload| {
            payload
                .get("displayName")
                .or_else(|| payload.get("job_name"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default();
    Ok(FailedJob {
        id,
        connection,
        queue: text_of(FAILED, document, "queue")?.to_owned(),
        job_name,
        envelope_json,
        exception: document.get_str("exception").unwrap_or_default().to_owned(),
        failed_at: date_of(document, "failed_at").ok_or_else(|| {
            FrameworkError::internal(format!("{FAILED}: the record {id} has no failed_at"))
        })?,
    })
}

#[async_trait]
impl FailedJobStore for MongoFailedJobStore {
    async fn log(
        &self,
        connection: &str,
        queue: &str,
        env: &Envelope,
        exception: &str,
    ) -> Result<Uuid, FrameworkError> {
        let id = Uuid::new_v4();
        let document = doc! {
            "uuid": id.to_string(),
            "connection": format!("{SUPRNOVA_CONNECTION_PREFIX}{connection}"),
            "queue": queue,
            "payload": failed_payload(env, id)?,
            "exception": exception,
            "failed_at": bson_time(crate::clock::now()),
        };
        self.ready()
            .await?
            .insert_one(document)
            .await
            .map_err(|error| store_error(FAILED, "log", error))?;
        Ok(id)
    }

    async fn all(&self) -> Result<Vec<FailedJob>, FrameworkError> {
        let documents: Vec<Document> = self
            .ready()
            .await?
            .find(doc! {})
            .sort(doc! { "failed_at": -1, "_id": -1 })
            .await
            .map_err(|error| store_error(FAILED, "all", error))?
            .try_collect()
            .await
            .map_err(|error| store_error(FAILED, "all", error))?;
        documents.iter().map(failed_job).collect()
    }

    async fn ids(&self) -> Result<Vec<Uuid>, FrameworkError> {
        Ok(self
            .all()
            .await?
            .into_iter()
            .map(|record| record.id)
            .collect())
    }

    async fn find(&self, id: Uuid) -> Result<Option<FailedJob>, FrameworkError> {
        self.ready()
            .await?
            .find_one(doc! { "uuid": id.to_string() })
            .await
            .map_err(|error| store_error(FAILED, "find", error))?
            .as_ref()
            .map(failed_job)
            .transpose()
    }

    async fn forget(&self, id: Uuid) -> Result<bool, FrameworkError> {
        Ok(self
            .ready()
            .await?
            .delete_one(doc! { "uuid": id.to_string() })
            .await
            .map_err(|error| store_error(FAILED, "forget", error))?
            .deleted_count
            > 0)
    }

    async fn flush(&self, before: Option<DateTime<Utc>>) -> Result<u64, FrameworkError> {
        let filter = match before {
            Some(cutoff) => doc! { "failed_at": { "$lt": bson_time(cutoff) } },
            None => doc! {},
        };
        Ok(self
            .ready()
            .await?
            .delete_many(filter)
            .await
            .map_err(|error| store_error(FAILED, "flush", error))?
            .deleted_count)
    }

    async fn count(&self) -> Result<u64, FrameworkError> {
        self.ready()
            .await?
            .count_documents(doc! {})
            .await
            .map_err(|error| store_error(FAILED, "count", error))
    }
}

// ---------------------------------------------------------------------------
// The batch repository
// ---------------------------------------------------------------------------

/// [`BatchRepository`] over a MongoDB collection, Laravel's `job_batches`
/// by default, as `laravel-mongodb`'s batch repository.
///
/// A batch is one document keyed by its id, with Laravel's fields
/// (`name`, `total_jobs`, `pending_jobs`, `failed_jobs`, `failed_job_ids`,
/// `options`, `created_at`, `cancelled_at`, `finished_at`, the dates as
/// BSON datetimes) and `settled_job_ids`, the jobs whose settlement already
/// moved the counters.
///
/// Each settlement is one update of that document: it moves the counters
/// with `$inc` and adds the job to `settled_job_ids`, and its filter
/// requires the job to be absent from that list. The server applies it to
/// the one document atomically, so jobs that settle at once never lose a
/// step, and a job delivered twice settles once. The terminal callbacks are
/// claimed by an update that sets `finished_at` only while it is null, so
/// one caller wins.
///
/// MongoDB holds a document of at most 16 MiB, and each settled job adds
/// about 50 bytes to the batch's, so a batch holds about 300,000 jobs. A
/// settlement past that is the server's error.
pub struct MongoBatchRepository {
    batches: Collection<Document>,
}

impl MongoBatchRepository {
    /// The repository over [`DEFAULT_MONGO_BATCHES_COLLECTION`] on
    /// `connection`.
    pub fn new(connection: &MongoConnection) -> Self {
        Self {
            batches: connection.collection(DEFAULT_MONGO_BATCHES_COLLECTION),
        }
    }

    /// The repository over the collection `collection` on `connection`.
    ///
    /// # Errors
    ///
    /// When MongoDB would refuse the name, as for
    /// [`MongoQueueDriver::with_collection`].
    pub fn with_collection(
        connection: &MongoConnection,
        collection: &str,
    ) -> Result<Self, FrameworkError> {
        validate_collection_name(collection, "the job batches")?;
        Ok(Self {
            batches: connection.collection(collection),
        })
    }

    /// The batches collection, for what the trait does not reach.
    pub fn collection(&self) -> Collection<Document> {
        self.batches.clone()
    }

    /// The update that settles `job_id` in the batch `id`, as `filter` and
    /// `update`.
    #[doc(hidden)]
    pub fn rendered_settlement(id: &str, job_id: Uuid, failed: bool) -> Document {
        let (filter, update) = settlement_parts(id, job_id, failed, true);
        doc! { "filter": filter, "update": update }
    }

    /// The update that claims the terminal callbacks of the batch `id` at
    /// `now`, as `filter` and `update`.
    #[doc(hidden)]
    pub fn rendered_claim(id: &str, now: DateTime<Utc>) -> Document {
        let (filter, update) = claim_parts(id, now);
        doc! { "filter": filter, "update": update }
    }

    /// The batch `id`, or the error a missing batch is.
    async fn existing(&self, id: &str, operation: &str) -> Result<Document, FrameworkError> {
        self.batches
            .find_one(doc! { "_id": id })
            .await
            .map_err(|error| store_error(BATCHES, operation, error))?
            .ok_or_else(|| FrameworkError::internal(format!("batch not found: {id}")))
    }

    /// Shared body of the two settlements.
    async fn record(
        &self,
        id: &str,
        job_id: Uuid,
        failed: bool,
    ) -> Result<UpdatedBatchJobCounts, FrameworkError> {
        // A job not yet settled, with jobs pending: one update moves the
        // counters and records the job.
        for pending_left in [true, false] {
            let (filter, update) = settlement_parts(id, job_id, failed, pending_left);
            let settled = self
                .batches
                .find_one_and_update(filter, update)
                .return_document(ReturnDocument::After)
                .await
                .map_err(|error| store_error(BATCHES, "settlement", error))?;
            if let Some(document) = settled {
                return Ok(counts(&document));
            }
            // No job is pending: the settlement is recorded, and a failure
            // counted, without taking `pending_jobs` below zero.
        }
        // Already settled, or no such batch. A failure is listed either
        // way, as the memory repository lists it.
        if failed {
            self.batches
                .update_one(
                    doc! { "_id": id },
                    doc! { "$addToSet": { "failed_job_ids": job_id.to_string() } },
                )
                .await
                .map_err(|error| store_error(BATCHES, "settlement", error))?;
        }
        Ok(counts(&self.existing(id, "settlement").await?))
    }
}

/// The filter and update that settle `job_id`, while jobs are pending
/// (`pending_left`) or once none is.
fn settlement_parts(
    id: &str,
    job_id: Uuid,
    failed: bool,
    pending_left: bool,
) -> (Document, Document) {
    let job = job_id.to_string();
    let mut filter = doc! { "_id": id, "settled_job_ids": { "$ne": job.as_str() } };
    let mut increments = Document::new();
    if pending_left {
        filter.insert("pending_jobs", doc! { "$gt": 0_i64 });
        increments.insert("pending_jobs", -1_i64);
    }
    if failed {
        increments.insert("failed_jobs", 1_i64);
    }
    let mut update = Document::new();
    if !increments.is_empty() {
        update.insert("$inc", increments);
    }
    update.insert("$push", doc! { "settled_job_ids": job.as_str() });
    if failed {
        update.insert("$addToSet", doc! { "failed_job_ids": job.as_str() });
    }
    (filter, update)
}

/// The filter and update that claim the terminal callbacks of the batch
/// `id` at `now`: `finished_at` is set only while it is null and no job is
/// pending.
fn claim_parts(id: &str, now: DateTime<Utc>) -> (Document, Document) {
    (
        doc! { "_id": id, "pending_jobs": 0_i64, "finished_at": Bson::Null },
        doc! { "$set": { "finished_at": bson_time(now) } },
    )
}

/// The counters of a stored batch.
fn counts(document: &Document) -> UpdatedBatchJobCounts {
    UpdatedBatchJobCounts {
        pending_jobs: u64::try_from(count_of(document, "pending_jobs")).unwrap_or(0),
        failed_jobs: u64::try_from(count_of(document, "failed_jobs")).unwrap_or(0),
    }
}

/// A stored batch as a [`Batch`].
fn batch_of(document: &Document) -> Result<Batch, FrameworkError> {
    let id = text_of(BATCHES, document, "_id")?.to_owned();
    let failed_job_ids = match document.get_array("failed_job_ids") {
        Ok(ids) => ids
            .iter()
            .filter_map(Bson::as_str)
            .map(|job| {
                Uuid::parse_str(job).map_err(|error| {
                    FrameworkError::internal(format!(
                        "{BATCHES}: the failed job id `{job}` is not a UUID: {error}"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        Err(_) => Vec::new(),
    };
    let options = match document.get_document("options") {
        Ok(options) => {
            ::bson::deserialize_from_document::<BatchOptions>(options.clone()).map_err(|error| {
                FrameworkError::internal(format!("{BATCHES}: the options of {id}: {error}"))
            })?
        }
        Err(_) => BatchOptions::default(),
    };
    let counted = counts(document);
    Ok(Batch {
        name: document.get_str("name").unwrap_or_default().to_owned(),
        total_jobs: u64::try_from(count_of(document, "total_jobs")).unwrap_or(0),
        pending_jobs: counted.pending_jobs,
        failed_jobs: counted.failed_jobs,
        failed_job_ids,
        options,
        created_at: date_of(document, "created_at").ok_or_else(|| {
            FrameworkError::internal(format!("{BATCHES}: the batch {id} has no created_at"))
        })?,
        cancelled_at: date_of(document, "cancelled_at"),
        finished_at: date_of(document, "finished_at"),
        id,
    })
}

/// A counter as the signed 64-bit integer MongoDB stores.
fn stored_count(value: u64, what: &str) -> Result<i64, FrameworkError> {
    i64::try_from(value)
        .map_err(|_| FrameworkError::internal(format!("{BATCHES}: {what} {value} is too large")))
}

#[async_trait]
impl BatchRepository for MongoBatchRepository {
    /// A plain insert keyed by the id: storing an id that exists is the
    /// server's duplicate-key error, never an overwrite of a batch that may
    /// have settlements.
    async fn store(&self, batch: Batch) -> Result<(), FrameworkError> {
        let options = ::bson::serialize_to_document(&batch.options).map_err(|error| {
            FrameworkError::internal(format!("{BATCHES}: the options of {}: {error}", batch.id))
        })?;
        let failed_job_ids: Vec<String> =
            batch.failed_job_ids.iter().map(Uuid::to_string).collect();
        let document = doc! {
            "_id": batch.id.as_str(),
            "name": batch.name.as_str(),
            "total_jobs": stored_count(batch.total_jobs, "total_jobs")?,
            "pending_jobs": stored_count(batch.pending_jobs, "pending_jobs")?,
            "failed_jobs": stored_count(batch.failed_jobs, "failed_jobs")?,
            "failed_job_ids": failed_job_ids,
            "options": options,
            "created_at": bson_time(batch.created_at),
            "cancelled_at": batch.cancelled_at.map(bson_time),
            "finished_at": batch.finished_at.map(bson_time),
            "settled_job_ids": [],
        };
        self.batches
            .insert_one(document)
            .await
            .map_err(|error| store_error(BATCHES, "store", error))?;
        Ok(())
    }

    async fn find(&self, id: &str) -> Result<Option<Batch>, FrameworkError> {
        self.batches
            .find_one(doc! { "_id": id })
            .projection(doc! { "settled_job_ids": 0 })
            .await
            .map_err(|error| store_error(BATCHES, "find", error))?
            .as_ref()
            .map(batch_of)
            .transpose()
    }

    /// One `$inc` of both counters, refused once the batch has finished.
    async fn increment_total_jobs(
        &self,
        id: &str,
        delta: u64,
    ) -> Result<UpdatedBatchJobCounts, FrameworkError> {
        if delta == 0 {
            return Ok(counts(&self.existing(id, "increment").await?));
        }
        let delta = stored_count(delta, "a job count")?;
        let grown = self
            .batches
            .find_one_and_update(
                doc! {
                    "_id": id,
                    "$or": [ { "total_jobs": 0_i64 }, { "pending_jobs": { "$gt": 0_i64 } } ],
                },
                doc! { "$inc": { "total_jobs": delta, "pending_jobs": delta } },
            )
            .return_document(ReturnDocument::After)
            .await
            .map_err(|error| store_error(BATCHES, "increment", error))?;
        match grown {
            Some(document) => Ok(counts(&document)),
            None => {
                self.existing(id, "increment").await?;
                Err(FrameworkError::internal(format!(
                    "cannot add jobs to completed batch: {id}"
                )))
            }
        }
    }

    async fn record_successful_job(
        &self,
        id: &str,
        job_id: Uuid,
    ) -> Result<UpdatedBatchJobCounts, FrameworkError> {
        self.record(id, job_id, false).await
    }

    async fn record_failed_job(
        &self,
        id: &str,
        job_id: Uuid,
    ) -> Result<UpdatedBatchJobCounts, FrameworkError> {
        self.record(id, job_id, true).await
    }

    async fn cancel(&self, id: &str) -> Result<(), FrameworkError> {
        self.batches
            .update_one(
                doc! { "_id": id },
                doc! { "$set": { "cancelled_at": bson_time(crate::clock::now()) } },
            )
            .await
            .map_err(|error| store_error(BATCHES, "cancel", error))?;
        Ok(())
    }

    async fn is_cancelled(&self, id: &str) -> Result<bool, FrameworkError> {
        Ok(self
            .batches
            .find_one(doc! { "_id": id })
            .projection(doc! { "cancelled_at": 1 })
            .await
            .map_err(|error| store_error(BATCHES, "is_cancelled", error))?
            .is_some_and(|document| date_of(&document, "cancelled_at").is_some()))
    }

    async fn mark_finished(&self, id: &str) -> Result<(), FrameworkError> {
        self.claim_terminal_callbacks(id).await.map(|_| ())
    }

    /// One update sets `finished_at` while it is null and no job is
    /// pending; the document as it was answers the cancellation, from the
    /// same atomic step.
    async fn claim_terminal_callbacks(
        &self,
        id: &str,
    ) -> Result<Option<TerminalCallbackClaim>, FrameworkError> {
        let now = crate::clock::now();
        // The claim time at the precision the document stores it.
        let claimed_at = chrono_time(bson_time(now));
        let (filter, update) = claim_parts(id, now);
        let before = self
            .batches
            .find_one_and_update(filter, update)
            .return_document(ReturnDocument::Before)
            .await
            .map_err(|error| store_error(BATCHES, "callback claim", error))?;
        if let Some(document) = before {
            return Ok(Some(TerminalCallbackClaim {
                finished_at: claimed_at,
                cancelled_at: date_of(&document, "cancelled_at"),
            }));
        }
        let document = self.existing(id, "callback claim").await?;
        let pending = count_of(&document, "pending_jobs");
        if pending != 0 {
            return Err(FrameworkError::internal(format!(
                "batch is not terminal: {id} has {pending} pending jobs"
            )));
        }
        Ok(None)
    }

    /// The settled jobs live on the batch's document, so they go with it.
    async fn delete(&self, id: &str) -> Result<bool, FrameworkError> {
        Ok(self
            .batches
            .delete_one(doc! { "_id": id })
            .await
            .map_err(|error| store_error(BATCHES, "delete", error))?
            .deleted_count
            > 0)
    }
}
