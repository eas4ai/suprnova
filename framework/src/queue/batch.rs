//! Queued batches: dispatch a group of jobs and track per-job progress.
//!
//! Mirrors Laravel 13's `Illuminate\Bus\Batch` + `BatchRepository`. The
//! batch repository persists batch metadata (total/pending/failed counts,
//! cancellation flag, callback list). Workers update the batch on every
//! settled job, and the batch fires `then`/`catch`/`finally` callbacks
//! once `pending_jobs` hits zero.
//!
//! Differences from Laravel:
//! - `then`/`catch`/`finally` are `Arc<dyn BatchCallback>` trait objects
//!   instead of Closure serialization - Rust closures don't serialize, so
//!   callback registration is per-process. Process restarts lose the
//!   in-flight callbacks; for cross-restart guarantees, define a
//!   `BatchCallback` impl and register it at boot (the registry is
//!   keyed by id so workers can look up after a restart).
//! - Job inclusion is recorded by `batch_id` on the envelope, not by a
//!   `Batchable` trait. Any job can be batched; the worker treats it
//!   uniformly.

use crate::error::FrameworkError;
use crate::queue::Job;
use crate::queue::envelope::Envelope;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use uuid::Uuid;

/// Snapshot of one batch's state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Batch {
    /// Batch identifier (UUID v4 as string).
    pub id: String,
    /// Human-readable batch name set at dispatch.
    pub name: String,
    /// Total jobs ever added to the batch.
    pub total_jobs: u64,
    /// Outstanding jobs awaiting settlement; callbacks fire when this hits 0.
    pub pending_jobs: u64,
    /// Count of jobs that failed terminally.
    pub failed_jobs: u64,
    /// Envelope ids of jobs that failed terminally.
    pub failed_job_ids: Vec<Uuid>,
    /// Per-batch behavior switches (callbacks, fail policy).
    pub options: BatchOptions,
    /// When the batch was first persisted.
    pub created_at: DateTime<Utc>,
    /// When the batch was cancelled, if ever.
    pub cancelled_at: Option<DateTime<Utc>>,
    /// When the batch finalized (`pending_jobs` reached 0), if ever.
    pub finished_at: Option<DateTime<Utc>>,
}

impl Batch {
    /// `true` if every job has settled (pending == 0).
    pub fn finished(&self) -> bool {
        self.pending_jobs == 0
    }

    /// `true` if the batch was cancelled.
    pub fn cancelled(&self) -> bool {
        self.cancelled_at.is_some()
    }

    /// Number of jobs processed (successfully or otherwise). Mirrors
    /// Laravel's `$batch->processedJobs()`.
    pub fn processed_jobs(&self) -> u64 {
        self.total_jobs.saturating_sub(self.pending_jobs)
    }

    /// 0–100 percentage of jobs settled. Mirrors `$batch->progress()`.
    pub fn progress(&self) -> u8 {
        if self.total_jobs == 0 {
            return 100;
        }
        let pct = (self.processed_jobs() as f64 / self.total_jobs as f64) * 100.0;
        pct.round().clamp(0.0, 100.0) as u8
    }
}

/// Per-batch behavior switches. Mirrors Laravel's `$batch->options` array.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BatchOptions {
    /// Names of pre-registered [`BatchCallback`] impls to run when every
    /// job succeeds.
    pub then_callbacks: Vec<String>,
    /// Names of pre-registered impls to run when any job fails.
    pub catch_callbacks: Vec<String>,
    /// Names of pre-registered impls to run after every job settles
    /// (success OR fail).
    pub finally_callbacks: Vec<String>,
    /// If `true`, the first failure cancels the batch.
    pub allow_failures: bool,
}

/// Counts returned by [`BatchRepository::increment_total_jobs`] and the
/// "record success/failure" path. Carries the post-update snapshot the
/// worker uses to decide if callbacks should fire.
#[derive(Debug, Clone, Copy)]
pub struct UpdatedBatchJobCounts {
    /// Outstanding jobs after the update (callbacks fire when this hits 0).
    pub pending_jobs: u64,
    /// Total failed jobs after the update.
    pub failed_jobs: u64,
}

/// Persistence backend for queued-batch metadata. Drivers (memory,
/// database) implement this so workers can update per-job progress
/// atomically and decide when to fire callbacks.
#[async_trait]
pub trait BatchRepository: Send + Sync {
    /// Persist a fresh [`Batch`] row.
    ///
    /// Ids arrive from [`PendingBatch::dispatch`] as fresh UUIDs, so a
    /// repository MAY reject an id it already holds rather than overwrite a
    /// batch that could already have settlements recorded against it -
    /// [`DatabaseBatchRepository`] does.
    async fn store(&self, batch: Batch) -> Result<(), FrameworkError>;
    /// Look up a batch by id; returns `Ok(None)` if no such batch exists.
    async fn find(&self, id: &str) -> Result<Option<Batch>, FrameworkError>;
    /// Atomically add `delta` jobs to the batch's `total_jobs` and
    /// `pending_jobs` counters, returning the post-update snapshot.
    async fn increment_total_jobs(
        &self,
        id: &str,
        delta: u64,
    ) -> Result<UpdatedBatchJobCounts, FrameworkError>;
    /// Atomically decrement `pending_jobs` for a successful settlement,
    /// returning the post-update counts the worker uses for callback gating.
    ///
    /// **Must be idempotent per `job_id`.** Queues are at-least-once, so the
    /// same job can be settled more than once - a redelivery, a duplicated
    /// ack, a worker that died between doing the work and recording it. An
    /// implementation that decrements on every call drives `pending_jobs` to
    /// zero early and fires `then`/`finally` callbacks while jobs are still
    /// running. A durable implementation should enforce this with a unique
    /// constraint on `(batch_id, job_id)` rather than a read-then-write.
    async fn record_successful_job(
        &self,
        id: &str,
        job_id: Uuid,
    ) -> Result<UpdatedBatchJobCounts, FrameworkError>;
    /// Atomically decrement `pending_jobs` and increment `failed_jobs`,
    /// recording `job_id` in `failed_job_ids` and returning the post-update
    /// counts.
    ///
    /// **Must be idempotent per `job_id`**, on the same terms as
    /// [`record_successful_job`](Self::record_successful_job) - and note that
    /// deduplicating `failed_job_ids` alone is not enough, because the
    /// counters are what gate the callbacks.
    async fn record_failed_job(
        &self,
        id: &str,
        job_id: Uuid,
    ) -> Result<UpdatedBatchJobCounts, FrameworkError>;
    /// Mark the batch cancelled. Workers honor the flag via
    /// `SkipIfBatchCancelled` middleware on the next attempt.
    async fn cancel(&self, id: &str) -> Result<(), FrameworkError>;
    /// `Ok(true)` if the batch has been cancelled.
    async fn is_cancelled(&self, id: &str) -> Result<bool, FrameworkError>;
    /// Stamp `finished_at` once `pending_jobs` reaches zero.
    async fn mark_finished(&self, id: &str) -> Result<(), FrameworkError>;
    /// Permanently delete the batch row. Returns `Ok(true)` if a row was
    /// removed.
    async fn delete(&self, id: &str) -> Result<bool, FrameworkError>;
}

// ---------------------------------------------------------------------------
// Memory repository
// ---------------------------------------------------------------------------

/// A stored batch plus the bookkeeping that keeps its counters idempotent.
///
/// `settled` is deliberately not a field on [`Batch`]: it is repository
/// bookkeeping, not part of the snapshot callers observe or persist.
struct BatchEntry {
    batch: Batch,
    /// Job ids whose settlement has already moved the counters.
    ///
    /// Queues are at-least-once, so the same job can be delivered - and
    /// settled - more than once. Without this, each redelivery decremented
    /// `pending_jobs` again, driving the batch to "finished" while jobs were
    /// still running and firing `then`/`finally` callbacks early.
    settled: HashSet<Uuid>,
}

/// In-process [`BatchRepository`] backed by a `Mutex<HashMap>`. Used as the
/// default when no other repository is installed; lost on process restart.
#[derive(Default)]
pub struct MemoryBatchRepository {
    inner: Mutex<HashMap<String, BatchEntry>>,
}

impl MemoryBatchRepository {
    /// Construct a fresh, empty in-memory batch repository.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl BatchRepository for MemoryBatchRepository {
    async fn store(&self, batch: Batch) -> Result<(), FrameworkError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| FrameworkError::internal("batch repo poisoned"))?;
        g.insert(
            batch.id.clone(),
            BatchEntry {
                batch,
                settled: HashSet::new(),
            },
        );
        Ok(())
    }
    async fn find(&self, id: &str) -> Result<Option<Batch>, FrameworkError> {
        let g = self
            .inner
            .lock()
            .map_err(|_| FrameworkError::internal("batch repo poisoned"))?;
        Ok(g.get(id).map(|e| e.batch.clone()))
    }
    async fn increment_total_jobs(
        &self,
        id: &str,
        delta: u64,
    ) -> Result<UpdatedBatchJobCounts, FrameworkError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| FrameworkError::internal("batch repo poisoned"))?;
        let entry = g
            .get_mut(id)
            .ok_or_else(|| FrameworkError::internal(format!("batch not found: {id}")))?;
        entry.batch.total_jobs += delta;
        entry.batch.pending_jobs += delta;
        Ok(UpdatedBatchJobCounts {
            pending_jobs: entry.batch.pending_jobs,
            failed_jobs: entry.batch.failed_jobs,
        })
    }
    async fn record_successful_job(
        &self,
        id: &str,
        job_id: Uuid,
    ) -> Result<UpdatedBatchJobCounts, FrameworkError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| FrameworkError::internal("batch repo poisoned"))?;
        let entry = g
            .get_mut(id)
            .ok_or_else(|| FrameworkError::internal(format!("batch not found: {id}")))?;
        // `job_id` used to be `_job_id` - ignored entirely - so a redelivered
        // settlement decremented `pending_jobs` a second time.
        if entry.settled.insert(job_id) && entry.batch.pending_jobs > 0 {
            entry.batch.pending_jobs -= 1;
        }
        Ok(UpdatedBatchJobCounts {
            pending_jobs: entry.batch.pending_jobs,
            failed_jobs: entry.batch.failed_jobs,
        })
    }
    async fn record_failed_job(
        &self,
        id: &str,
        job_id: Uuid,
    ) -> Result<UpdatedBatchJobCounts, FrameworkError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| FrameworkError::internal("batch repo poisoned"))?;
        let entry = g
            .get_mut(id)
            .ok_or_else(|| FrameworkError::internal(format!("batch not found: {id}")))?;
        // The `failed_job_ids` dedupe below predates this guard, which made
        // the method look idempotent while both counters still moved on
        // every redelivery - the more misleading of the two states to be in.
        if entry.settled.insert(job_id) {
            if entry.batch.pending_jobs > 0 {
                entry.batch.pending_jobs -= 1;
            }
            entry.batch.failed_jobs += 1;
        }
        if !entry.batch.failed_job_ids.contains(&job_id) {
            entry.batch.failed_job_ids.push(job_id);
        }
        Ok(UpdatedBatchJobCounts {
            pending_jobs: entry.batch.pending_jobs,
            failed_jobs: entry.batch.failed_jobs,
        })
    }
    async fn cancel(&self, id: &str) -> Result<(), FrameworkError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| FrameworkError::internal("batch repo poisoned"))?;
        if let Some(e) = g.get_mut(id) {
            e.batch.cancelled_at = Some(Utc::now());
        }
        Ok(())
    }
    async fn is_cancelled(&self, id: &str) -> Result<bool, FrameworkError> {
        Ok(self.find(id).await?.is_some_and(|b| b.cancelled()))
    }
    async fn mark_finished(&self, id: &str) -> Result<(), FrameworkError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| FrameworkError::internal("batch repo poisoned"))?;
        if let Some(e) = g.get_mut(id) {
            e.batch.finished_at = Some(Utc::now());
        }
        Ok(())
    }
    async fn delete(&self, id: &str) -> Result<bool, FrameworkError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| FrameworkError::internal("batch repo poisoned"))?;
        Ok(g.remove(id).is_some())
    }
}

// ---------------------------------------------------------------------------
// Database repository
// ---------------------------------------------------------------------------

/// Default table holding one row per batch.
pub const DEFAULT_BATCHES_TABLE: &str = "job_batches";
/// Default table holding one row per settled `(batch, job)` pair.
pub const DEFAULT_BATCH_SETTLEMENTS_TABLE: &str = "job_batch_settlements";

/// SeaORM-backed [`BatchRepository`]. Batch accounting survives a restart, and
/// the settlement counters cannot double-count a redelivered job.
///
/// # Schema (operator-managed)
///
/// ```sql
/// CREATE TABLE job_batches (
///     id            TEXT PRIMARY KEY,
///     name          TEXT NOT NULL,
///     total_jobs    INTEGER NOT NULL,
///     options_json  TEXT NOT NULL,
///     created_at    INTEGER NOT NULL,
///     cancelled_at  INTEGER NULL,
///     finished_at   INTEGER NULL
/// );
///
/// CREATE TABLE job_batch_settlements (
///     batch_id   TEXT NOT NULL,
///     job_id     TEXT NOT NULL,
///     failed     INTEGER NOT NULL,
///     settled_at INTEGER NOT NULL,
///     PRIMARY KEY (batch_id, job_id)
/// );
/// ```
///
/// Same convention as
/// [`DatabaseFailedJobStore`](crate::queue::DatabaseFailedJobStore): the
/// framework does not create these, so they belong in your migrations.
///
/// # Why the counters are derived rather than stored (DATA-02)
///
/// `pending_jobs` and `failed_jobs` are not columns. They are computed from
/// the settlement rows on every read:
///
/// ```text
/// pending_jobs = max(0, total_jobs - COUNT(settlements))
/// failed_jobs  = COUNT(settlements WHERE failed)
/// ```
///
/// Queues are at-least-once, so the same job settles more than once whenever a
/// redelivery happens, an ack is duplicated, or a worker dies between doing the
/// work and recording it. A stored counter decremented per settlement drifts on
/// every one of those, and the drift is not cosmetic: `pending_jobs` is what
/// gates the batch callbacks, so an early zero fires `then` and `finally` while
/// other jobs in the batch are still running.
///
/// [`MemoryBatchRepository`] guards that with a `HashSet` of settled ids, which
/// works in one process and is lost on restart. Here the primary key
/// `(batch_id, job_id)` *is* the guard: a repeat settlement inserts nothing, so
/// there is no counter to get it wrong. Deriving rather than incrementing means
/// the invariant holds even against a repository whose rows were written by
/// another process, an older version, or an operator's `INSERT`.
pub struct DatabaseBatchRepository {
    db: sea_orm::DatabaseConnection,
    batches: String,
    settlements: String,
}

impl std::fmt::Debug for DatabaseBatchRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabaseBatchRepository")
            .field("batches", &self.batches)
            .field("settlements", &self.settlements)
            .finish_non_exhaustive()
    }
}

impl DatabaseBatchRepository {
    /// Open a repository against [`DEFAULT_BATCHES_TABLE`] and
    /// [`DEFAULT_BATCH_SETTLEMENTS_TABLE`].
    pub fn new(db: sea_orm::DatabaseConnection) -> Self {
        Self {
            db,
            batches: DEFAULT_BATCHES_TABLE.to_string(),
            settlements: DEFAULT_BATCH_SETTLEMENTS_TABLE.to_string(),
        }
    }

    /// Open a repository against explicitly-named tables.
    ///
    /// Both names are interpolated into every statement, so both are validated
    /// as SQL identifiers once, here - the same treatment
    /// [`DatabaseQueueDriver::new`](crate::queue::DatabaseQueueDriver::new)
    /// gives its table.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError::param`] when either name fails
    /// [`validate_identifier`](crate::database::validate_identifier).
    pub fn with_tables(
        db: sea_orm::DatabaseConnection,
        batches: String,
        settlements: String,
    ) -> Result<Self, FrameworkError> {
        crate::database::validate_identifier(&batches)?;
        crate::database::validate_identifier(&settlements)?;
        Ok(Self {
            db,
            batches,
            settlements,
        })
    }

    fn backend(&self) -> sea_orm::DatabaseBackend {
        self.db.get_database_backend()
    }

    /// The backend's "insert unless this key already exists" spelling.
    ///
    /// Every supported backend has one; none of them share a syntax. Doing it
    /// this way rather than catching a unique-violation error keeps the
    /// duplicate on the normal path instead of the exceptional one, and avoids
    /// having to classify driver error strings per backend.
    fn insert_settlement_sql(&self) -> Result<String, FrameworkError> {
        use crate::database::placeholder::placeholder_list;
        let cols = format!(
            "({}) VALUES ({})",
            "batch_id, job_id, failed, settled_at",
            placeholder_list(self.backend(), 1, 4)?
        );
        Ok(match self.backend() {
            sea_orm::DatabaseBackend::Sqlite => {
                format!("INSERT OR IGNORE INTO {} {}", self.settlements, cols)
            }
            sea_orm::DatabaseBackend::MySql => {
                format!("INSERT IGNORE INTO {} {}", self.settlements, cols)
            }
            sea_orm::DatabaseBackend::Postgres => {
                format!(
                    "INSERT INTO {} {} ON CONFLICT (batch_id, job_id) DO NOTHING",
                    self.settlements, cols
                )
            }
            _ => {
                return Err(crate::database::unsupported_database_backend(
                    self.backend(),
                ));
            }
        })
    }

    /// One query for the whole derived snapshot: the stored total plus the two
    /// settlement counts, correlated on the batch row so a single bound `id`
    /// serves all three (positional `?` backends would otherwise need it bound
    /// once per reference).
    ///
    /// Returns `Ok(None)` when the batch row does not exist.
    async fn counts<C: sea_orm::ConnectionTrait>(
        &self,
        conn: &C,
        id: &str,
    ) -> Result<Option<UpdatedBatchJobCounts>, FrameworkError> {
        use crate::database::placeholder::placeholder;
        let sql = format!(
            "SELECT b.total_jobs, \
             (SELECT COUNT(*) FROM {s} s1 WHERE s1.batch_id = b.id), \
             (SELECT COUNT(*) FROM {s} s2 WHERE s2.batch_id = b.id AND s2.failed = 1) \
             FROM {b} b WHERE b.id = {p}",
            s = self.settlements,
            b = self.batches,
            p = placeholder(self.backend(), 1)?
        );
        let row = conn
            .query_one_raw(sea_orm::Statement::from_sql_and_values(
                self.backend(),
                sql,
                vec![sea_orm::Value::from(id.to_string())],
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("job_batches counts: {e}")))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let total: i64 = row
            .try_get_by_index(0)
            .map_err(|e| FrameworkError::internal(format!("job_batches total col: {e}")))?;
        let settled: i64 = row
            .try_get_by_index(1)
            .map_err(|e| FrameworkError::internal(format!("job_batches settled col: {e}")))?;
        let failed: i64 = row
            .try_get_by_index(2)
            .map_err(|e| FrameworkError::internal(format!("job_batches failed col: {e}")))?;
        Ok(Some(UpdatedBatchJobCounts {
            pending_jobs: total.saturating_sub(settled).max(0) as u64,
            failed_jobs: failed.max(0) as u64,
        }))
    }

    /// Shared body of [`BatchRepository::record_successful_job`] and
    /// [`BatchRepository::record_failed_job`]: they differ only in the `failed`
    /// flag they stamp on the settlement row.
    async fn record(
        &self,
        id: &str,
        job_id: Uuid,
        failed: bool,
    ) -> Result<UpdatedBatchJobCounts, FrameworkError> {
        use sea_orm::TransactionTrait;
        let txn = self
            .db
            .begin()
            .await
            .map_err(|e| FrameworkError::internal(format!("job_batches txn: {e}")))?;

        {
            use sea_orm::ConnectionTrait;
            txn.execute_raw(sea_orm::Statement::from_sql_and_values(
                self.backend(),
                self.insert_settlement_sql()?,
                vec![
                    sea_orm::Value::from(id.to_string()),
                    sea_orm::Value::from(job_id.to_string()),
                    sea_orm::Value::from(i32::from(failed)),
                    sea_orm::Value::from(Utc::now().timestamp()),
                ],
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("job_batch_settlements insert: {e}")))?;
        }

        // Read the derived counts inside the same transaction, so the snapshot
        // the worker gates callbacks on is the one this settlement produced -
        // not one a concurrent settlement moved underneath it.
        let counts = self.counts(&txn, id).await?;

        let Some(counts) = counts else {
            // No batch row: the settlement we just inserted would be an orphan,
            // and rolling back is what removes it. Matches the in-memory
            // repository, which errors on an unknown id rather than inventing
            // one.
            txn.rollback()
                .await
                .map_err(|e| FrameworkError::internal(format!("job_batches rollback: {e}")))?;
            return Err(FrameworkError::internal(format!("batch not found: {id}")));
        };

        txn.commit()
            .await
            .map_err(|e| FrameworkError::internal(format!("job_batches commit: {e}")))?;
        Ok(counts)
    }

    /// Stamp `column` with the current timestamp, if the batch exists.
    async fn stamp(&self, id: &str, column: &'static str) -> Result<(), FrameworkError> {
        use crate::database::placeholder::placeholder;
        use sea_orm::ConnectionTrait;
        self.db
            .execute_raw(sea_orm::Statement::from_sql_and_values(
                self.backend(),
                format!(
                    "UPDATE {} SET {} = {} WHERE id = {}",
                    self.batches,
                    column,
                    placeholder(self.backend(), 1)?,
                    placeholder(self.backend(), 2)?
                ),
                vec![
                    sea_orm::Value::from(Utc::now().timestamp()),
                    sea_orm::Value::from(id.to_string()),
                ],
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("job_batches {column}: {e}")))?;
        Ok(())
    }
}

#[async_trait]
impl BatchRepository for DatabaseBatchRepository {
    /// Ids come from [`PendingBatch::dispatch`] as fresh UUIDs, so this is a
    /// plain `INSERT`: re-storing an id that already exists is a duplicate-key
    /// error rather than a silent overwrite of a batch that may already have
    /// settlements recorded against it.
    async fn store(&self, batch: Batch) -> Result<(), FrameworkError> {
        use crate::database::placeholder::placeholder_list;
        use sea_orm::ConnectionTrait;
        let options_json = serde_json::to_string(&batch.options)
            .map_err(|e| FrameworkError::internal(format!("encode batch options: {e}")))?;
        self.db
            .execute_raw(sea_orm::Statement::from_sql_and_values(
                self.backend(),
                format!(
                    "INSERT INTO {} \
             (id, name, total_jobs, options_json, created_at, cancelled_at, finished_at) \
             VALUES ({})",
                    self.batches,
                    placeholder_list(self.backend(), 1, 7)?
                ),
                vec![
                    sea_orm::Value::from(batch.id.clone()),
                    sea_orm::Value::from(batch.name.clone()),
                    sea_orm::Value::from(batch.total_jobs as i64),
                    sea_orm::Value::from(options_json),
                    sea_orm::Value::from(batch.created_at.timestamp()),
                    sea_orm::Value::from(batch.cancelled_at.map(|t| t.timestamp())),
                    sea_orm::Value::from(batch.finished_at.map(|t| t.timestamp())),
                ],
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("job_batches insert: {e}")))?;
        Ok(())
    }

    async fn find(&self, id: &str) -> Result<Option<Batch>, FrameworkError> {
        use crate::database::placeholder::placeholder;
        use sea_orm::ConnectionTrait;
        let row = self
            .db
            .query_one_raw(sea_orm::Statement::from_sql_and_values(
                self.backend(),
                format!(
                    "SELECT name, total_jobs, options_json, created_at, cancelled_at, finished_at \
             FROM {} WHERE id = {}",
                    self.batches,
                    placeholder(self.backend(), 1)?
                ),
                vec![sea_orm::Value::from(id.to_string())],
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("job_batches select: {e}")))?;
        let Some(row) = row else {
            return Ok(None);
        };

        let col = |i: usize, what: &'static str| {
            move |e: sea_orm::DbErr| {
                FrameworkError::internal(format!("job_batches {what} col ({i}): {e}"))
            }
        };
        let name: String = row.try_get_by_index(0).map_err(col(0, "name"))?;
        let total_jobs: i64 = row.try_get_by_index(1).map_err(col(1, "total_jobs"))?;
        let options_json: String = row.try_get_by_index(2).map_err(col(2, "options"))?;
        let created_at: i64 = row.try_get_by_index(3).map_err(col(3, "created_at"))?;
        let cancelled_at: Option<i64> = row.try_get_by_index(4).map_err(col(4, "cancelled_at"))?;
        let finished_at: Option<i64> = row.try_get_by_index(5).map_err(col(5, "finished_at"))?;

        let counts = self
            .counts(&self.db, id)
            .await?
            .ok_or_else(|| FrameworkError::internal(format!("batch vanished mid-read: {id}")))?;

        Ok(Some(Batch {
            id: id.to_string(),
            name,
            total_jobs: total_jobs.max(0) as u64,
            pending_jobs: counts.pending_jobs,
            failed_jobs: counts.failed_jobs,
            failed_job_ids: self.failed_ids(id).await?,
            options: serde_json::from_str(&options_json)
                .map_err(|e| FrameworkError::internal(format!("decode batch options: {e}")))?,
            created_at: timestamp(created_at, "created_at")?,
            cancelled_at: cancelled_at
                .map(|t| timestamp(t, "cancelled_at"))
                .transpose()?,
            finished_at: finished_at
                .map(|t| timestamp(t, "finished_at"))
                .transpose()?,
        }))
    }

    async fn increment_total_jobs(
        &self,
        id: &str,
        delta: u64,
    ) -> Result<UpdatedBatchJobCounts, FrameworkError> {
        use crate::database::placeholder::placeholder;
        use sea_orm::{ConnectionTrait, TransactionTrait};
        let txn = self
            .db
            .begin()
            .await
            .map_err(|e| FrameworkError::internal(format!("job_batches txn: {e}")))?;
        txn.execute_raw(sea_orm::Statement::from_sql_and_values(
            self.backend(),
            format!(
                "UPDATE {} SET total_jobs = total_jobs + {} WHERE id = {}",
                self.batches,
                placeholder(self.backend(), 1)?,
                placeholder(self.backend(), 2)?
            ),
            vec![
                sea_orm::Value::from(delta as i64),
                sea_orm::Value::from(id.to_string()),
            ],
        ))
        .await
        .map_err(|e| FrameworkError::internal(format!("job_batches increment: {e}")))?;

        // Existence is decided by the follow-up read rather than the update's
        // `rows_affected`: MySQL reports zero rows changed when `delta` is 0
        // even though the row matched, which would turn a no-op growth into a
        // spurious "batch not found".
        let counts = self.counts(&txn, id).await?;
        let Some(counts) = counts else {
            txn.rollback()
                .await
                .map_err(|e| FrameworkError::internal(format!("job_batches rollback: {e}")))?;
            return Err(FrameworkError::internal(format!("batch not found: {id}")));
        };
        txn.commit()
            .await
            .map_err(|e| FrameworkError::internal(format!("job_batches commit: {e}")))?;
        Ok(counts)
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
        self.stamp(id, "cancelled_at").await
    }

    async fn is_cancelled(&self, id: &str) -> Result<bool, FrameworkError> {
        use crate::database::placeholder::placeholder;
        use sea_orm::ConnectionTrait;
        let row = self
            .db
            .query_one_raw(sea_orm::Statement::from_sql_and_values(
                self.backend(),
                format!(
                    "SELECT cancelled_at FROM {} WHERE id = {}",
                    self.batches,
                    placeholder(self.backend(), 1)?
                ),
                vec![sea_orm::Value::from(id.to_string())],
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("job_batches cancelled: {e}")))?;
        let Some(row) = row else { return Ok(false) };
        let at: Option<i64> = row
            .try_get_by_index(0)
            .map_err(|e| FrameworkError::internal(format!("job_batches cancelled col: {e}")))?;
        Ok(at.is_some())
    }

    async fn mark_finished(&self, id: &str) -> Result<(), FrameworkError> {
        self.stamp(id, "finished_at").await
    }

    /// Removes the settlement rows with the batch, in one transaction. Leaving
    /// them behind would let a later batch reusing the id inherit somebody
    /// else's settled jobs and start life already "finished".
    async fn delete(&self, id: &str) -> Result<bool, FrameworkError> {
        use crate::database::placeholder::placeholder;
        use sea_orm::{ConnectionTrait, TransactionTrait};
        let txn = self
            .db
            .begin()
            .await
            .map_err(|e| FrameworkError::internal(format!("job_batches txn: {e}")))?;

        let delete = |table: &str, key: &str| -> Result<sea_orm::Statement, FrameworkError> {
            Ok(sea_orm::Statement::from_sql_and_values(
                self.backend(),
                format!(
                    "DELETE FROM {} WHERE {} = {}",
                    table,
                    key,
                    placeholder(self.backend(), 1)?
                ),
                vec![sea_orm::Value::from(id.to_string())],
            ))
        };

        txn.execute_raw(delete(&self.settlements, "batch_id")?)
            .await
            .map_err(|e| FrameworkError::internal(format!("job_batch_settlements delete: {e}")))?;
        let removed = txn
            .execute_raw(delete(&self.batches, "id")?)
            .await
            .map_err(|e| FrameworkError::internal(format!("job_batches delete: {e}")))?
            .rows_affected()
            > 0;

        txn.commit()
            .await
            .map_err(|e| FrameworkError::internal(format!("job_batches delete commit: {e}")))?;
        Ok(removed)
    }
}

impl DatabaseBatchRepository {
    /// Ids of the jobs that settled as failures, oldest first.
    async fn failed_ids(&self, id: &str) -> Result<Vec<Uuid>, FrameworkError> {
        use crate::database::placeholder::placeholder;
        use sea_orm::ConnectionTrait;
        let rows = self
            .db
            .query_all_raw(sea_orm::Statement::from_sql_and_values(
                self.backend(),
                format!(
                    "SELECT job_id FROM {} WHERE batch_id = {} AND failed = 1 \
             ORDER BY settled_at, job_id",
                    self.settlements,
                    placeholder(self.backend(), 1)?
                ),
                vec![sea_orm::Value::from(id.to_string())],
            ))
            .await
            .map_err(|e| FrameworkError::internal(format!("job_batch_settlements select: {e}")))?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let raw: String = row.try_get_by_index(0).map_err(|e| {
                FrameworkError::internal(format!("job_batch_settlements job_id col: {e}"))
            })?;
            out.push(Uuid::parse_str(&raw).map_err(|e| {
                FrameworkError::internal(format!("job_batch_settlements job_id parse: {e}"))
            })?);
        }
        Ok(out)
    }
}

/// Turn a stored unix timestamp back into a `DateTime`, naming the column so a
/// corrupt row says which one.
fn timestamp(secs: i64, what: &'static str) -> Result<DateTime<Utc>, FrameworkError> {
    DateTime::<Utc>::from_timestamp(secs, 0)
        .ok_or_else(|| FrameworkError::internal(format!("job_batches: invalid {what}: {secs}")))
}

// ---------------------------------------------------------------------------
// Batch callbacks
// ---------------------------------------------------------------------------

/// Callback fired by the worker when a batch's `then`/`catch`/`finally`
/// condition is met. Implementations are registered once at boot via
/// [`register_callback`] keyed by name; the batch's `options.*_callbacks`
/// hold the names of impls to invoke.
#[async_trait]
pub trait BatchCallback: Send + Sync + 'static {
    /// Callback name - matches the entry in `BatchOptions.then/catch/finally`.
    fn name(&self) -> &'static str;

    /// Run the callback for `batch`. `error` is `Some` for `catch`/`finally`
    /// invocations after a failure; `None` for `then` and for successful
    /// `finally`.
    async fn handle(&self, batch: Batch, error: Option<String>) -> Result<(), FrameworkError>;
}

static CALLBACKS: OnceLock<RwLock<HashMap<String, Arc<dyn BatchCallback>>>> = OnceLock::new();

fn callbacks() -> &'static RwLock<HashMap<String, Arc<dyn BatchCallback>>> {
    CALLBACKS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Register a batch callback so it can be referenced by name from
/// [`BatchOptions::then_callbacks`] / `catch_callbacks` / `finally_callbacks`.
pub fn register_callback(cb: Arc<dyn BatchCallback>) {
    if let Ok(mut g) = callbacks().write() {
        g.insert(cb.name().to_string(), cb);
    }
}

pub(crate) fn resolve_callback(name: &str) -> Option<Arc<dyn BatchCallback>> {
    callbacks().read().ok().and_then(|g| g.get(name).cloned())
}

// ---------------------------------------------------------------------------
// Global repository wiring
// ---------------------------------------------------------------------------

static REPO: RwLock<Option<Arc<dyn BatchRepository>>> = RwLock::new(None);

/// Install the process-wide [`BatchRepository`]. Subsequent calls replace
/// the previous installation; integration tests typically install a fresh
/// [`MemoryBatchRepository`] per case.
pub fn install_repository(repo: Arc<dyn BatchRepository>) {
    if let Ok(mut g) = REPO.write() {
        *g = Some(repo);
    }
}

/// Return the currently installed [`BatchRepository`], or `None` if no
/// repository has been wired (the dispatch path installs the in-memory
/// default before use).
pub fn current_repository() -> Option<Arc<dyn BatchRepository>> {
    REPO.read().ok().and_then(|g| g.clone())
}

pub(crate) fn ensure_default_repository() {
    let installed = REPO.read().ok().and_then(|g| g.clone()).is_some();
    if !installed {
        install_repository(Arc::new(MemoryBatchRepository::new()));
    }
}

// ---------------------------------------------------------------------------
// PendingBatch - builder used by `Bus::batch_queue(...)`
// ---------------------------------------------------------------------------

/// Builder for a queued batch. Mirrors Laravel's `PendingBatch`.
///
/// ```rust,no_run
/// use suprnova::{Job, Queue};
/// use suprnova::FrameworkError;
///
/// # #[derive(serde::Serialize, serde::Deserialize)]
/// # struct MyJob { id: u64 }
/// # #[suprnova::async_trait]
/// # impl Job for MyJob {
/// #     fn job_name() -> &'static str { "MyJob" }
/// #     async fn handle(self) -> Result<(), FrameworkError> { Ok(()) }
/// # }
/// # async fn ex() -> Result<(), Box<dyn std::error::Error>> {
/// let batch_id = Queue::batch()
///     .name("import-users")
///     .add(MyJob { id: 1 })
///     .add(MyJob { id: 2 })
///     .then("notify_complete")
///     .catch("notify_failed")
///     .dispatch()
///     .await?;
/// # Ok(()) }
/// ```
pub struct PendingBatch {
    /// Human-readable batch name (surfaced in events and dashboards).
    pub name: String,
    /// Per-batch behavior switches (callbacks, fail policy).
    pub options: BatchOptions,
    envelopes: Vec<Envelope>,
    /// Jobs added to this batch that declare a debounce window. Collected at
    /// `add` time because `add` returns `Self` and cannot fail; surfaced by
    /// [`PendingBatch::dispatch`] before anything is stored.
    debounce_rejected: Vec<String>,
}

impl Default for PendingBatch {
    fn default() -> Self {
        Self::new()
    }
}

impl PendingBatch {
    /// Construct an empty pending batch with no name and no jobs.
    pub fn new() -> Self {
        Self {
            name: String::new(),
            options: BatchOptions::default(),
            envelopes: Vec::new(),
            debounce_rejected: Vec::new(),
        }
    }

    /// Set the human-readable batch name (surfaced in events and dashboards).
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Add a job to the batch. Builds the envelope NOW so the batch_id
    /// gets stamped before dispatch.
    #[allow(clippy::should_implement_trait)]
    pub fn add<J: Job>(mut self, job: J) -> Self {
        // A superseded batch job is dropped without ever settling, so the
        // batch's pending count never reaches zero and its callbacks never
        // fire. Reject at dispatch rather than let that happen quietly.
        if J::debounce_for().is_some() {
            self.debounce_rejected.push(J::job_name().to_string());
            return self;
        }
        let now = Utc::now();
        let mut env = match crate::queue::build_envelope::<J>(&job, now) {
            Ok(e) => e,
            Err(_) => return self,
        };
        env.batch_id = None; // overwritten on dispatch with the batch id
        self.envelopes.push(env);
        self
    }

    /// Register a `BatchCallback` (by name) to run when every job
    /// succeeds.
    pub fn then(mut self, callback_name: impl Into<String>) -> Self {
        self.options.then_callbacks.push(callback_name.into());
        self
    }

    /// Register a `BatchCallback` (by name) to run on first failure.
    pub fn catch(mut self, callback_name: impl Into<String>) -> Self {
        self.options.catch_callbacks.push(callback_name.into());
        self
    }

    /// Register a `BatchCallback` (by name) to run after the batch
    /// finishes (success OR fail).
    pub fn finally(mut self, callback_name: impl Into<String>) -> Self {
        self.options.finally_callbacks.push(callback_name.into());
        self
    }

    /// Allow the batch to continue after a job fails. Default: false
    /// (first failure cancels remaining jobs via `SkipIfBatchCancelled`).
    pub fn allow_failures(mut self) -> Self {
        self.options.allow_failures = true;
        self
    }

    /// Number of jobs accumulated so far.
    pub fn len(&self) -> usize {
        self.envelopes.len()
    }

    /// `true` when no jobs have been added.
    pub fn is_empty(&self) -> bool {
        self.envelopes.is_empty()
    }

    /// Persist the batch and dispatch every queued job via the configured
    /// driver. Returns the batch id.
    ///
    /// # A push that fails mid-loop (DATA-02)
    ///
    /// A half-pushed batch left as-is sits unfinished forever: workers only
    /// see the envelopes that made it into the queue, so `pending_jobs` can
    /// never reach 0 and `then`/`catch`/`finally` never fire.
    ///
    /// This used to be handled by deleting the batch row, which traded that
    /// for something worse. The envelopes that *had* landed were still in the
    /// queue and still stamped with the batch id, so every one of them settled
    /// against a batch that no longer existed - `Err(batch not found)`, on
    /// every delivery, forever, with no operator action that reconciles it.
    /// The worker even had to be written not to let that error hold the
    /// reservation, or the orphans would have spun on visibility expiry with
    /// no exit.
    ///
    /// Instead the batch is *settled*: every envelope that was not pushed is
    /// recorded as a failed job, and the batch is cancelled. That keeps the
    /// accounting true - `total_jobs` still counts what was asked for,
    /// `failed_job_ids` names exactly the jobs that never made it - and lets
    /// the ones already queued settle normally against a batch that is still
    /// there. Cancellation makes [`SkipIfBatchCancelled`] drop the rest, so
    /// pending still reaches zero and the terminal callbacks still fire.
    ///
    /// If nothing was pushed at all there is no worker left to drive that last
    /// settlement, so the callbacks fire here.
    ///
    /// The caller gets the original push error either way.
    ///
    /// [`SkipIfBatchCancelled`]: crate::queue::SkipIfBatchCancelled
    pub async fn dispatch(self) -> Result<String, FrameworkError> {
        if !self.debounce_rejected.is_empty() {
            return Err(FrameworkError::internal(format!(
                "these jobs declare debounce_for() and cannot be batched: {}. A \
                 superseded job is dropped without settling, which would leave the \
                 batch's pending count above zero and its callbacks unfired",
                self.debounce_rejected.join(", ")
            )));
        }
        ensure_default_repository();
        let repo = current_repository()
            .ok_or_else(|| FrameworkError::internal("batch repository not initialized"))?;

        let id = Uuid::new_v4().to_string();
        let total = self.envelopes.len() as u64;
        let batch = Batch {
            id: id.clone(),
            name: self.name.clone(),
            total_jobs: total,
            pending_jobs: total,
            failed_jobs: 0,
            failed_job_ids: Vec::new(),
            options: self.options.clone(),
            created_at: Utc::now(),
            cancelled_at: None,
            finished_at: None,
        };
        repo.store(batch).await?;

        let driver = crate::queue::current_driver()?;
        let mut remaining = self.envelopes.into_iter();
        let mut pushed = 0usize;
        while let Some(mut env) = remaining.next() {
            env.batch_id = Some(id.clone());
            let undispatched = env.id;
            if let Err(e) = driver.push(env).await {
                // Everything from here on never reached the queue, starting
                // with the one that just failed.
                let orphans: Vec<Uuid> = std::iter::once(undispatched)
                    .chain(remaining.map(|e| e.id))
                    .collect();
                settle_undispatched(repo.as_ref(), &id, &orphans, pushed == 0).await;
                return Err(e);
            }
            pushed += 1;
        }
        Ok(id)
    }
}

/// Close out the jobs a failed [`PendingBatch::dispatch`] never enqueued.
///
/// Repository errors here are logged, never returned: the caller needs the
/// original push error, and a bookkeeping failure on top of it is a second
/// fact, not a replacement for the first.
async fn settle_undispatched(
    repo: &dyn BatchRepository,
    id: &str,
    orphans: &[Uuid],
    nothing_was_pushed: bool,
) {
    for job_id in orphans {
        if let Err(e) = repo.record_failed_job(id, *job_id).await {
            tracing::warn!(
                batch_id = %id,
                job_id = %job_id,
                error = %e,
                "queue batch dispatch: could not record an undispatched job as failed"
            );
        }
    }
    if let Err(e) = repo.cancel(id).await {
        tracing::warn!(
            batch_id = %id,
            error = %e,
            "queue batch dispatch: could not cancel the partially-dispatched batch"
        );
    }

    // With at least one job in the queue, a worker settles the last one and
    // fires the callbacks on the normal path. With none, this is the last
    // chance anything runs them.
    if nothing_was_pushed {
        if let Err(e) = repo.mark_finished(id).await {
            tracing::warn!(batch_id = %id, error = %e, "queue batch dispatch: mark_finished failed");
        }
        match repo.find(id).await {
            Ok(Some(batch)) => {
                crate::queue::worker::fire_batch_callbacks(
                    &batch,
                    crate::queue::worker::BatchPhase::Catch,
                )
                .await;
                crate::queue::worker::fire_batch_callbacks(
                    &batch,
                    crate::queue::worker::BatchPhase::Finally,
                )
                .await;
            }
            Ok(None) => tracing::warn!(
                batch_id = %id,
                "queue batch dispatch: batch vanished before its callbacks could fire"
            ),
            Err(e) => tracing::warn!(
                batch_id = %id,
                error = %e,
                "queue batch dispatch: could not load the batch to fire its callbacks"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(name: &str, total: u64) -> Batch {
        Batch {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            total_jobs: total,
            pending_jobs: total,
            failed_jobs: 0,
            failed_job_ids: Vec::new(),
            options: BatchOptions::default(),
            created_at: Utc::now(),
            cancelled_at: None,
            finished_at: None,
        }
    }

    #[tokio::test]
    async fn memory_repo_record_success_decrements_pending() {
        let repo = MemoryBatchRepository::new();
        let b = fresh("X", 3);
        let id = b.id.clone();
        repo.store(b).await.unwrap();
        let u = repo
            .record_successful_job(&id, Uuid::new_v4())
            .await
            .unwrap();
        assert_eq!(u.pending_jobs, 2);
        assert_eq!(u.failed_jobs, 0);
    }

    #[tokio::test]
    async fn memory_repo_record_failure_increments_failed_and_decrements_pending() {
        let repo = MemoryBatchRepository::new();
        let b = fresh("X", 3);
        let id = b.id.clone();
        repo.store(b).await.unwrap();
        let job_id = Uuid::new_v4();
        let u = repo.record_failed_job(&id, job_id).await.unwrap();
        assert_eq!(u.pending_jobs, 2);
        assert_eq!(u.failed_jobs, 1);
        let snap = repo.find(&id).await.unwrap().unwrap();
        assert_eq!(snap.failed_job_ids, vec![job_id]);
    }

    #[tokio::test]
    async fn memory_repo_cancel_sets_flag() {
        let repo = MemoryBatchRepository::new();
        let b = fresh("X", 3);
        let id = b.id.clone();
        repo.store(b).await.unwrap();
        assert!(!repo.is_cancelled(&id).await.unwrap());
        repo.cancel(&id).await.unwrap();
        assert!(repo.is_cancelled(&id).await.unwrap());
    }

    #[test]
    fn batch_progress_is_percentage() {
        let mut b = fresh("X", 4);
        b.pending_jobs = 1;
        assert_eq!(b.progress(), 75);
        b.pending_jobs = 0;
        assert!(b.finished());
        assert_eq!(b.progress(), 100);
    }

    // ---- DATA-02b: settlement counters must be idempotent per job -------
    //
    // Queues are at-least-once. The same job gets settled twice whenever a
    // redelivery happens, an ack is duplicated, or a worker dies between
    // doing the work and recording it. `record_successful_job` took a
    // `_job_id` it never looked at, so each of those decremented
    // `pending_jobs` again.
    //
    // The consequence is not a wrong number on a dashboard: `pending_jobs`
    // is what gates the batch callbacks, so an early zero fires `then` and
    // `finally` while other jobs in the batch are still running.

    #[tokio::test]
    async fn a_redelivered_success_settles_the_job_only_once() {
        let repo = MemoryBatchRepository::new();
        let b = fresh("redelivery", 3);
        let id = b.id.clone();
        repo.store(b).await.unwrap();

        let job = Uuid::new_v4();
        let first = repo.record_successful_job(&id, job).await.unwrap();
        let second = repo.record_successful_job(&id, job).await.unwrap();

        assert_eq!(first.pending_jobs, 2, "the first settlement counts");
        assert_eq!(
            second.pending_jobs, 2,
            "the same job settled twice must not decrement twice - two more \
             jobs are still pending and the batch is not finished"
        );
    }

    /// The failure path was *half* guarded: it deduplicated
    /// `failed_job_ids` while still moving both counters, which reads as if
    /// redelivery had been considered.
    #[tokio::test]
    async fn a_redelivered_failure_counts_once_in_both_counters() {
        let repo = MemoryBatchRepository::new();
        let b = fresh("redelivery-fail", 3);
        let id = b.id.clone();
        repo.store(b).await.unwrap();

        let job = Uuid::new_v4();
        repo.record_failed_job(&id, job).await.unwrap();
        let second = repo.record_failed_job(&id, job).await.unwrap();

        assert_eq!(
            second.failed_jobs, 1,
            "one failing job is one failure, however many times it is redelivered"
        );
        assert_eq!(
            second.pending_jobs, 2,
            "and it may only consume one pending slot"
        );

        let snap = repo.find(&id).await.unwrap().expect("batch exists");
        assert_eq!(
            snap.failed_job_ids,
            vec![job],
            "the id list stays deduplicated too"
        );
    }

    /// A job that succeeds and is then redelivered and *fails* must not be
    /// counted twice either - the batch already consumed its pending slot.
    #[tokio::test]
    async fn a_job_that_settles_both_ways_consumes_one_slot() {
        let repo = MemoryBatchRepository::new();
        let b = fresh("mixed", 2);
        let id = b.id.clone();
        repo.store(b).await.unwrap();

        let job = Uuid::new_v4();
        repo.record_successful_job(&id, job).await.unwrap();
        let after = repo.record_failed_job(&id, job).await.unwrap();

        assert_eq!(
            after.pending_jobs, 1,
            "one job, one slot, regardless of how many settlements arrive"
        );
        assert_eq!(
            after.failed_jobs, 0,
            "the job had already settled successfully; a late failure for the \
             same id must not retroactively fail the batch"
        );
    }

    /// The control: distinct jobs must still each count, or the guard would
    /// have turned a double-decrement into a batch that never finishes.
    #[tokio::test]
    async fn distinct_jobs_each_settle_normally() {
        let repo = MemoryBatchRepository::new();
        let b = fresh("distinct", 3);
        let id = b.id.clone();
        repo.store(b).await.unwrap();

        repo.record_successful_job(&id, Uuid::new_v4())
            .await
            .unwrap();
        repo.record_successful_job(&id, Uuid::new_v4())
            .await
            .unwrap();
        let third = repo
            .record_successful_job(&id, Uuid::new_v4())
            .await
            .unwrap();

        assert_eq!(
            third.pending_jobs, 0,
            "three distinct jobs settle the batch - the idempotency guard \
             must key on the job id, not suppress every repeat call"
        );
    }
}
