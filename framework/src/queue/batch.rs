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
//!   instead of Closure serialization — Rust closures don't serialize, so
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
    /// same job can be settled more than once — a redelivery, a duplicated
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
    /// [`record_successful_job`](Self::record_successful_job) — and note that
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
    /// Queues are at-least-once, so the same job can be delivered — and
    /// settled — more than once. Without this, each redelivery decremented
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
        // `job_id` used to be `_job_id` — ignored entirely — so a redelivered
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
        // every redelivery — the more misleading of the two states to be in.
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
// Batch callbacks
// ---------------------------------------------------------------------------

/// Callback fired by the worker when a batch's `then`/`catch`/`finally`
/// condition is met. Implementations are registered once at boot via
/// [`register_callback`] keyed by name; the batch's `options.*_callbacks`
/// hold the names of impls to invoke.
#[async_trait]
pub trait BatchCallback: Send + Sync + 'static {
    /// Callback name — matches the entry in `BatchOptions.then/catch/finally`.
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
// PendingBatch — builder used by `Bus::batch_queue(...)`
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
    /// If any `driver.push` fails mid-loop, the batch row is deleted before
    /// returning the error. A half-pushed batch with `pending_jobs == total`
    /// would otherwise sit unfinished forever — workers only see the
    /// envelopes that made it into the queue, so pending can never reach 0
    /// and `then`/`catch`/`finally` would never fire. Deleting the batch
    /// makes the in-flight pushes orphan (their `record_successful_job`
    /// returns `Err` and the worker logs but does not finalize), and the
    /// caller gets a hard error to surface or retry.
    pub async fn dispatch(self) -> Result<String, FrameworkError> {
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
        for mut env in self.envelopes {
            env.batch_id = Some(id.clone());
            if let Err(e) = driver.push(env).await {
                // Roll the batch back so it can't sit stuck on the
                // permanently-non-zero `pending_jobs`. Repository delete
                // failures are logged but do not mask the original push
                // error — the caller needs the original cause.
                if let Err(del_err) = repo.delete(&id).await {
                    tracing::warn!(
                        batch_id = %id,
                        error = %del_err,
                        "queue batch dispatch: failed to delete partially-pushed batch row"
                    );
                }
                return Err(e);
            }
        }
        Ok(id)
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
            "the same job settled twice must not decrement twice — two more \
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
    /// counted twice either — the batch already consumed its pending slot.
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
            "three distinct jobs settle the batch — the idempotency guard \
             must key on the job id, not suppress every repeat call"
        );
    }
}
