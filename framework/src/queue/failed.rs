//! Failed-job storage.
//!
//! When the worker dead-letters a job (max_tries exhausted, fatal timeout,
//! manual fail), the [`FailedJobStore`] receives a record carrying the
//! envelope + the failure cause. Records can be listed, retried, forgotten,
//! flushed. Mirrors Laravel 13's `Illuminate\Queue\Failed\*`.
//!
//! Three backends ship:
//! - [`MemoryFailedJobStore`] - in-process Vec, lost on restart. Default
//!   wired by `bootstrap_default`.
//! - [`DatabaseFailedJobStore`] - persists to a `failed_jobs` table via
//!   SeaORM. Production default for the database driver.
//! - [`NullFailedJobStore`] - discards every record. Mirrors Laravel's
//!   `NullFailedJobProvider`.
//!
//! Configure via [`Queue::set_failed_store`](crate::queue::Queue::set_failed_store)
//! at boot.

use crate::database::clauses::quote_identifier;
use crate::database::placeholder::{placeholder, placeholder_list};
use crate::database::stored_datetime::StoredDateTime;
use crate::database::validate_identifier;
use crate::error::FrameworkError;
use crate::queue::envelope::Envelope;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// One persisted failed-job record. The serialized envelope is held verbatim
/// so an operator running `queue:retry <id>` can re-enqueue the exact
/// payload that originally failed.
///
/// The envelope holds the job's payload and the
/// [`Context`](crate::context::Context) it was pushed with, hidden values
/// included, because a retry has to run with what the first attempt had.
/// The `Debug` output therefore gives the envelope's size, never its text:
/// a record that is logged must not carry what the application asked to
/// keep out of logs. Serializing a record writes the envelope in full.
#[derive(Clone, Serialize, Deserialize)]
pub struct FailedJob {
    /// Record id assigned by the store on insert.
    pub id: Uuid,
    /// The connection the job ran on: the name the worker that ran it was
    /// started with. A retry pushes the job back to this connection.
    pub connection: String,
    /// Queue name the job ran on (e.g. `"default"`, `"high"`).
    pub queue: String,
    /// Fully-qualified job type name (matches `Envelope::job_name`).
    pub job_name: String,
    /// Verbatim serialized envelope, suitable for re-enqueue via `queue:retry`.
    pub envelope_json: String,
    /// Formatted display of the final failure cause.
    pub exception: String,
    /// When the record was logged.
    pub failed_at: DateTime<Utc>,
}

impl std::fmt::Debug for FailedJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FailedJob")
            .field("id", &self.id)
            .field("connection", &self.connection)
            .field("queue", &self.queue)
            .field("job_name", &self.job_name)
            .field(
                "envelope_json",
                &format_args!("<{} bytes>", self.envelope_json.len()),
            )
            .field("exception", &self.exception)
            .field("failed_at", &self.failed_at)
            .finish()
    }
}

/// Storage backend for dead-lettered jobs. Drivers (memory, database,
/// null) implement this so the worker can log failures uniformly.
#[async_trait]
pub trait FailedJobStore: Send + Sync {
    /// Persist a new failed-job record. Returns the record's id.
    async fn log(
        &self,
        connection: &str,
        queue: &str,
        env: &Envelope,
        exception: &str,
    ) -> Result<Uuid, FrameworkError>;

    /// All records, newest first.
    async fn all(&self) -> Result<Vec<FailedJob>, FrameworkError>;

    /// IDs only, newest first. Mirrors Laravel's `ids($queue)`.
    async fn ids(&self) -> Result<Vec<Uuid>, FrameworkError>;

    /// Find a single record by id.
    async fn find(&self, id: Uuid) -> Result<Option<FailedJob>, FrameworkError>;

    /// Drop a single record. Returns `true` if a row was removed.
    async fn forget(&self, id: Uuid) -> Result<bool, FrameworkError>;

    /// Drop every record (optionally only those older than `before`).
    /// Returns the number of records dropped.
    async fn flush(&self, before: Option<DateTime<Utc>>) -> Result<u64, FrameworkError>;

    /// Number of records. Mirrors Laravel's `count()`.
    async fn count(&self) -> Result<u64, FrameworkError>;

    /// Confirm the store can write a record, before a worker pops its first
    /// job. A worker over a store that cannot write fails its first dead
    /// letter, keeps the reservation and fails it again on every visibility
    /// timeout, so `queue:work`, [`run_worker`](crate::queue::worker::run_worker)
    /// and [`run_worker_on`](crate::queue::worker::run_worker_on) refuse to
    /// start instead.
    ///
    /// The default accepts: a store with nothing outside the process to
    /// check, such as the memory and null stores, can always write.
    /// [`DatabaseFailedJobStore`] checks its table's columns.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] naming what the store cannot write to.
    async fn check(&self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Memory backend
// ---------------------------------------------------------------------------

/// In-process [`FailedJobStore`] backed by a `Mutex<Vec>`. Lost on
/// process restart; use [`DatabaseFailedJobStore`] for persistence.
#[derive(Default)]
pub struct MemoryFailedJobStore {
    rows: Mutex<Vec<FailedJob>>,
}

impl MemoryFailedJobStore {
    /// Construct a fresh, empty in-memory store.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl FailedJobStore for MemoryFailedJobStore {
    async fn log(
        &self,
        connection: &str,
        queue: &str,
        env: &Envelope,
        exception: &str,
    ) -> Result<Uuid, FrameworkError> {
        let id = Uuid::new_v4();
        let envelope_json = env.to_json().map_err(|e| {
            FrameworkError::internal(format!("encode envelope for failed_jobs: {e}"))
        })?;
        let row = FailedJob {
            id,
            connection: connection.into(),
            queue: queue.into(),
            job_name: env.job_name.clone(),
            envelope_json,
            exception: exception.into(),
            failed_at: crate::clock::now(),
        };
        let mut g = self
            .rows
            .lock()
            .map_err(|_| FrameworkError::internal("failed_jobs store poisoned"))?;
        g.push(row);
        Ok(id)
    }

    async fn all(&self) -> Result<Vec<FailedJob>, FrameworkError> {
        let g = self
            .rows
            .lock()
            .map_err(|_| FrameworkError::internal("failed_jobs store poisoned"))?;
        let mut v = g.clone();
        v.sort_by_key(|r| std::cmp::Reverse(r.failed_at));
        Ok(v)
    }

    async fn ids(&self) -> Result<Vec<Uuid>, FrameworkError> {
        Ok(self.all().await?.into_iter().map(|r| r.id).collect())
    }

    async fn find(&self, id: Uuid) -> Result<Option<FailedJob>, FrameworkError> {
        let g = self
            .rows
            .lock()
            .map_err(|_| FrameworkError::internal("failed_jobs store poisoned"))?;
        Ok(g.iter().find(|r| r.id == id).cloned())
    }

    async fn forget(&self, id: Uuid) -> Result<bool, FrameworkError> {
        let mut g = self
            .rows
            .lock()
            .map_err(|_| FrameworkError::internal("failed_jobs store poisoned"))?;
        let before = g.len();
        g.retain(|r| r.id != id);
        Ok(g.len() < before)
    }

    async fn flush(&self, before: Option<DateTime<Utc>>) -> Result<u64, FrameworkError> {
        let mut g = self
            .rows
            .lock()
            .map_err(|_| FrameworkError::internal("failed_jobs store poisoned"))?;
        let before_len = g.len();
        match before {
            Some(cutoff) => g.retain(|r| r.failed_at >= cutoff),
            None => g.clear(),
        }
        Ok((before_len - g.len()) as u64)
    }

    async fn count(&self) -> Result<u64, FrameworkError> {
        let g = self
            .rows
            .lock()
            .map_err(|_| FrameworkError::internal("failed_jobs store poisoned"))?;
        Ok(g.len() as u64)
    }
}

// ---------------------------------------------------------------------------
// Null backend
// ---------------------------------------------------------------------------

/// [`FailedJobStore`] that discards every record. Mirrors Laravel's
/// `NullFailedJobProvider` - use it to disable failed-job retention
/// without removing the worker's logging path.
#[derive(Default)]
pub struct NullFailedJobStore;

impl NullFailedJobStore {
    /// Construct a fresh null store.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl FailedJobStore for NullFailedJobStore {
    async fn log(
        &self,
        _connection: &str,
        _queue: &str,
        _env: &Envelope,
        _exception: &str,
    ) -> Result<Uuid, FrameworkError> {
        Ok(Uuid::new_v4())
    }
    async fn all(&self) -> Result<Vec<FailedJob>, FrameworkError> {
        Ok(vec![])
    }
    async fn ids(&self) -> Result<Vec<Uuid>, FrameworkError> {
        Ok(vec![])
    }
    async fn find(&self, _id: Uuid) -> Result<Option<FailedJob>, FrameworkError> {
        Ok(None)
    }
    async fn forget(&self, _id: Uuid) -> Result<bool, FrameworkError> {
        Ok(false)
    }
    async fn flush(&self, _before: Option<DateTime<Utc>>) -> Result<u64, FrameworkError> {
        Ok(0)
    }
    async fn count(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

// ---------------------------------------------------------------------------
// Database backend
// ---------------------------------------------------------------------------

/// What the `connection` column holds before the Suprnova connection name.
///
/// Laravel's `queue:retry` pushes a row's payload to the connection the row
/// names and deletes the row once the push succeeds. A Suprnova row naming
/// `database` would reach Laravel's own `database` connection, whose worker
/// cannot run the job. No connection in Laravel 13's default
/// `config/queue.php` takes a name with this prefix, so Laravel's retry
/// throws "connection has not been configured" before it forgets the row.
pub const SUPRNOVA_CONNECTION_PREFIX: &str = "suprnova:";

/// The columns the database store writes, in the order it writes them.
const WRITTEN_COLUMNS: [&str; 6] = [
    "uuid",
    "connection",
    "queue",
    "payload",
    "exception",
    "failed_at",
];

/// SeaORM-backed failed-job store over Laravel 13's `failed_jobs` layout.
///
/// [`CreateFailedJobsTable`](crate::queue::migrations::CreateFailedJobsTable)
/// creates the table; a table Laravel's own migration created works the
/// same:
///
/// ```sql
/// CREATE TABLE failed_jobs (
///     id         BIGINT PRIMARY KEY AUTO_INCREMENT,
///     uuid       VARCHAR(255) NOT NULL UNIQUE,
///     connection VARCHAR(255) NOT NULL,
///     queue      VARCHAR(255) NOT NULL,
///     payload    LONGTEXT NOT NULL,
///     exception  LONGTEXT NOT NULL,
///     failed_at  DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
/// );
/// CREATE INDEX failed_jobs_connection_queue_failed_at_index
///     ON failed_jobs (connection, queue, failed_at);
/// ```
///
/// Each record gets a fresh `uuid`, never the envelope's id: a dead letter
/// whose acknowledgement failed is redelivered and dead-lettered again, and
/// a second row with the envelope's id would break `uuid UNIQUE`, keep the
/// reservation and loop. `payload` is the envelope with the job's name
/// added as `displayName` and the row's uuid as `uuid`, the two keys
/// Laravel's readers take from a payload. `connection` is the Suprnova
/// connection behind [`SUPRNOVA_CONNECTION_PREFIX`].
///
/// The `table` argument is validated as a SQL identifier once at construction
/// (same shape as [`crate::queue::database::DatabaseQueueDriver::new`]).
pub struct DatabaseFailedJobStore {
    db: DatabaseConnection,
    /// The table as configured, for the catalog check and for messages.
    table: String,
    /// The table quoted for the connection's backend, segment by segment,
    /// as every statement names it: the migration that created it quoted
    /// it, so Postgres keeps `FailedJobs` as written and no engine reads
    /// `select` as a keyword.
    quoted: String,
}

impl DatabaseFailedJobStore {
    /// Open a database-backed store against `table`. The table name is
    /// validated as a SQL identifier - invalid input returns
    /// [`FrameworkError`] rather than reaching the database.
    pub fn new(db: DatabaseConnection, table: String) -> Result<Self, FrameworkError> {
        validate_identifier(&table)?;
        let quoted = quote_identifier(db.get_database_backend(), &table);
        Ok(Self { db, table, quoted })
    }

    fn backend(&self) -> DatabaseBackend {
        self.db.get_database_backend()
    }

    fn select_columns(&self) -> String {
        format!(
            "SELECT uuid, connection, queue, payload, exception, failed_at FROM {}",
            self.quoted
        )
    }
}

/// The `payload` a failed-job row stores for `env`: the envelope's JSON
/// object with `displayName` set to the job's name and `uuid` to the row's
/// uuid. [`Envelope`] ignores fields it does not know, so the payload
/// decodes back into the envelope for `queue:retry`.
pub(crate) fn failed_payload(env: &Envelope, uuid: Uuid) -> Result<String, FrameworkError> {
    let mut value = serde_json::to_value(env)
        .map_err(|e| FrameworkError::internal(format!("encode envelope for failed_jobs: {e}")))?;
    let object = value.as_object_mut().ok_or_else(|| {
        FrameworkError::internal("encode envelope for failed_jobs: not a JSON object")
    })?;
    object.insert(
        "displayName".to_owned(),
        serde_json::Value::String(env.job_name.clone()),
    );
    object.insert(
        "uuid".to_owned(),
        serde_json::Value::String(uuid.to_string()),
    );
    serde_json::to_string(&value)
        .map_err(|e| FrameworkError::internal(format!("encode envelope for failed_jobs: {e}")))
}

/// The current time at whole seconds, the precision of `failed_at` on
/// every engine Laravel's layout uses.
fn now_whole_seconds() -> chrono::NaiveDateTime {
    let now = crate::clock::now();
    DateTime::<Utc>::from_timestamp(now.timestamp(), 0)
        .unwrap_or(now)
        .naive_utc()
}

#[async_trait]
impl FailedJobStore for DatabaseFailedJobStore {
    async fn log(
        &self,
        connection: &str,
        queue: &str,
        env: &Envelope,
        exception: &str,
    ) -> Result<Uuid, FrameworkError> {
        let id = Uuid::new_v4();
        let payload = failed_payload(env, id)?;
        let stmt = Statement::from_sql_and_values(
            self.backend(),
            format!(
                "INSERT INTO {} (uuid, connection, queue, payload, exception, failed_at) \
                 VALUES ({})",
                self.quoted,
                placeholder_list(self.backend(), 1, 6)?
            ),
            vec![
                sea_orm::Value::from(id.to_string()),
                sea_orm::Value::from(format!("{SUPRNOVA_CONNECTION_PREFIX}{connection}")),
                sea_orm::Value::from(queue.to_string()),
                sea_orm::Value::from(payload),
                sea_orm::Value::from(exception.to_string()),
                sea_orm::Value::from(now_whole_seconds()),
            ],
        );
        self.db
            .execute_raw(stmt)
            .await
            .map_err(|e| FrameworkError::internal(format!("failed_jobs insert: {e}")))?;
        Ok(id)
    }

    async fn all(&self) -> Result<Vec<FailedJob>, FrameworkError> {
        let rows = self
            .db
            .query_all_raw(Statement::from_string(
                self.backend(),
                format!("{} ORDER BY failed_at DESC, id DESC", self.select_columns()),
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("failed_jobs select: {e}")))?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(decode_row(&row)?);
        }
        Ok(out)
    }

    async fn ids(&self) -> Result<Vec<Uuid>, FrameworkError> {
        Ok(self.all().await?.into_iter().map(|r| r.id).collect())
    }

    async fn find(&self, id: Uuid) -> Result<Option<FailedJob>, FrameworkError> {
        let stmt = Statement::from_sql_and_values(
            self.backend(),
            format!(
                "{} WHERE uuid = {}",
                self.select_columns(),
                placeholder(self.backend(), 1)?
            ),
            vec![sea_orm::Value::from(id.to_string())],
        );
        let row = self
            .db
            .query_one_raw(stmt)
            .await
            .map_err(|e| FrameworkError::internal(format!("failed_jobs find: {e}")))?;
        match row {
            Some(r) => Ok(Some(decode_row(&r)?)),
            None => Ok(None),
        }
    }

    async fn forget(&self, id: Uuid) -> Result<bool, FrameworkError> {
        let stmt = Statement::from_sql_and_values(
            self.backend(),
            format!(
                "DELETE FROM {} WHERE uuid = {}",
                self.quoted,
                placeholder(self.backend(), 1)?
            ),
            vec![sea_orm::Value::from(id.to_string())],
        );
        let r = self
            .db
            .execute_raw(stmt)
            .await
            .map_err(|e| FrameworkError::internal(format!("failed_jobs forget: {e}")))?;
        Ok(r.rows_affected() > 0)
    }

    async fn flush(&self, before: Option<DateTime<Utc>>) -> Result<u64, FrameworkError> {
        let stmt = match before {
            Some(cutoff) => Statement::from_sql_and_values(
                self.backend(),
                format!(
                    "DELETE FROM {} WHERE failed_at < {}",
                    self.quoted,
                    placeholder(self.backend(), 1)?
                ),
                vec![sea_orm::Value::from(cutoff.naive_utc())],
            ),
            None => Statement::from_string(self.backend(), format!("DELETE FROM {}", self.quoted)),
        };
        let r = self
            .db
            .execute_raw(stmt)
            .await
            .map_err(|e| FrameworkError::internal(format!("failed_jobs flush: {e}")))?;
        Ok(r.rows_affected())
    }

    async fn count(&self) -> Result<u64, FrameworkError> {
        let row = self
            .db
            .query_one_raw(Statement::from_string(
                self.backend(),
                format!("SELECT COUNT(*) FROM {}", self.quoted),
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("failed_jobs count: {e}")))?;
        let n: i64 = match row {
            Some(r) => r
                .try_get_by_index(0)
                .map_err(|e| FrameworkError::internal(format!("failed_jobs count col: {e}")))?,
            None => 0,
        };
        Ok(n.max(0) as u64)
    }

    /// Refuse a table this store cannot write. A worker that started on one
    /// would fail its first dead letter, leave the reservation, and fail it
    /// again on every visibility timeout without end.
    async fn check(&self) -> Result<(), FrameworkError> {
        let columns = crate::database::catalog::table_columns(&self.db, &self.table)
            .await
            .map_err(|e| {
                FrameworkError::internal(format!(
                    "the failed-jobs table `{}` could not be inspected: {e}",
                    self.table
                ))
            })?;
        check_failed_jobs_columns(&self.table, &columns, self.backend())
    }
}

/// The verdict of [`FailedJobStore::check`] on a table's columns.
fn check_failed_jobs_columns(
    table: &str,
    columns: &[crate::database::catalog::CatalogColumn],
    backend: DatabaseBackend,
) -> Result<(), FrameworkError> {
    use crate::database::catalog::column;
    const REMEDY: &str = "list suprnova::queue::migrations::CreateFailedJobsTable in the \
                          application's Migrator and run `migrate`";
    if columns.is_empty() {
        return Err(FrameworkError::internal(format!(
            "the failed-jobs table `{table}` does not exist; {REMEDY}"
        )));
    }
    if let Some(earlier) = ["envelope_json", "job_name"]
        .into_iter()
        .find(|name| column(columns, name).is_some())
    {
        let missing = WRITTEN_COLUMNS
            .into_iter()
            .find(|name| column(columns, name).is_none())
            .unwrap_or("uuid");
        return Err(FrameworkError::internal(format!(
            "the failed-jobs table `{table}` is in the earlier Suprnova layout: it has the \
             column `{earlier}` and lacks `{missing}`; {REMEDY}, which reshapes it with its rows"
        )));
    }
    for name in WRITTEN_COLUMNS {
        if column(columns, name).is_none() {
            return Err(FrameworkError::internal(format!(
                "the failed-jobs table `{table}` lacks the column `{name}` the failed-jobs \
                 store writes; {REMEDY}"
            )));
        }
    }
    if backend == DatabaseBackend::MySql {
        for name in ["payload", "exception"] {
            if let Some(found) = column(columns, name)
                && found.data_type != "longtext"
            {
                return Err(FrameworkError::internal(format!(
                    "the column `{name}` of the failed-jobs table `{table}` is {}, narrower \
                     than LONGTEXT: a large job or trace would be refused. Alter it to \
                     LONGTEXT, as Laravel's migration creates it",
                    found.data_type.to_ascii_uppercase()
                )));
            }
        }
    }
    Ok(())
}

fn decode_row(row: &sea_orm::QueryResult) -> Result<FailedJob, FrameworkError> {
    let id_s: String = row
        .try_get_by_index(0)
        .map_err(|e| FrameworkError::internal(format!("failed_jobs uuid col: {e}")))?;
    let id = Uuid::parse_str(&id_s).map_err(|e| {
        FrameworkError::internal(format!("failed_jobs uuid `{id_s}` is not a UUID: {e}"))
    })?;
    let stored_connection: String = row
        .try_get_by_index(1)
        .map_err(|e| FrameworkError::internal(format!("failed_jobs connection col: {e}")))?;
    // A row Laravel wrote keeps its connection as Laravel named it; a
    // Suprnova row names its own connection behind the prefix.
    let connection = stored_connection
        .strip_prefix(SUPRNOVA_CONNECTION_PREFIX)
        .map(str::to_owned)
        .unwrap_or(stored_connection);
    let queue: String = row
        .try_get_by_index(2)
        .map_err(|e| FrameworkError::internal(format!("failed_jobs queue col: {e}")))?;
    let envelope_json: String = row
        .try_get_by_index(3)
        .map_err(|e| FrameworkError::internal(format!("failed_jobs payload col: {e}")))?;
    let exception: String = row
        .try_get_by_index(4)
        .map_err(|e| FrameworkError::internal(format!("failed_jobs exception col: {e}")))?;
    let failed_at = row
        .try_get_by_index::<StoredDateTime>(5)
        .map_err(|e| FrameworkError::internal(format!("failed_jobs failed_at col: {e}")))?
        .and_utc();
    // Laravel's payload and this store's both carry `displayName`.
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
        queue,
        job_name,
        envelope_json,
        exception,
        failed_at,
    })
}

// ---------------------------------------------------------------------------
// Global registration
// ---------------------------------------------------------------------------

use std::sync::RwLock;

static STORE: RwLock<Option<Arc<dyn FailedJobStore>>> = RwLock::new(None);

pub(crate) fn install(store: Arc<dyn FailedJobStore>) {
    if let Ok(mut g) = STORE.write() {
        *g = Some(store);
    }
}

pub(crate) fn current() -> Option<Arc<dyn FailedJobStore>> {
    STORE.read().ok().and_then(|g| g.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::{BackoffSchedule, CURRENT_SCHEMA_VERSION};

    #[tokio::test]
    async fn the_debug_output_of_a_record_never_shows_the_envelope() {
        let mut envelope = env("DebuggedJob");
        envelope.context = Some(crate::context::ContextSnapshot {
            data: Default::default(),
            hidden: [("api_key".to_owned(), serde_json::json!("s3cret"))].into(),
        });
        let store = MemoryFailedJobStore::new();
        let id = store
            .log("memory", "default", &envelope, "boom")
            .await
            .unwrap();
        let record = store.find(id).await.unwrap().unwrap();

        let shown = format!("{record:?}");

        assert!(
            record.envelope_json.contains("s3cret"),
            "the record keeps the envelope whole, a retry needs it"
        );
        assert!(
            !shown.contains("s3cret"),
            "a hidden value was shown: {shown}"
        );
        assert!(shown.contains("DebuggedJob") && shown.contains("boom"));
        assert!(shown.contains("bytes>"));
    }

    fn env(name: &str) -> Envelope {
        Envelope {
            schema_version: CURRENT_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            job_name: name.into(),
            queue: None,
            payload: serde_json::json!({}),
            dispatched_at: crate::clock::now(),
            available_at: crate::clock::now(),
            attempts: 0,
            max_tries: 3,
            backoff: BackoffSchedule::default(),
            timeout_secs: None,
            fail_on_timeout: false,
            idempotency_key: None,
            unique_lock_owner: None,
            debounce_id: None,
            debounce_owner: None,
            batch_id: None,
            chain_remaining: Vec::new(),
            context: None,
        }
    }

    #[tokio::test]
    async fn memory_store_round_trips_records() {
        let store = MemoryFailedJobStore::new();
        let id = store
            .log("default", "default", &env("A"), "boom")
            .await
            .unwrap();
        let id2 = store
            .log("default", "default", &env("B"), "kaboom")
            .await
            .unwrap();
        assert_eq!(store.count().await.unwrap(), 2);
        let all = store.all().await.unwrap();
        assert_eq!(all.len(), 2);
        assert!(store.find(id).await.unwrap().is_some());
        assert!(store.forget(id).await.unwrap());
        assert_eq!(store.count().await.unwrap(), 1);
        assert_eq!(store.flush(None).await.unwrap(), 1);
        assert_eq!(store.count().await.unwrap(), 0);
        // forget on missing id is false
        assert!(!store.forget(id2).await.unwrap());
    }

    #[tokio::test]
    async fn null_store_is_inert() {
        let store = NullFailedJobStore::new();
        let _ = store
            .log("default", "default", &env("A"), "boom")
            .await
            .unwrap();
        assert_eq!(store.count().await.unwrap(), 0);
        assert!(store.all().await.unwrap().is_empty());
    }
}
