//! Queue subsystem: facade, drivers, envelope, worker.

pub mod batch;
pub mod chain;
pub mod database;
pub mod driver;
pub mod envelope;
pub mod errors;
pub mod events;
pub mod failed;
pub mod job;
pub mod memory;
pub mod middleware;
pub mod null;
pub mod outcome;
pub mod redis;
pub mod retry;
pub mod routing;
pub mod sync;
pub mod testing;
pub mod worker;

pub use batch::{
    Batch, BatchCallback, BatchOptions, BatchRepository, DEFAULT_BATCH_SETTLEMENTS_TABLE,
    DEFAULT_BATCHES_TABLE, DatabaseBatchRepository, MemoryBatchRepository, PendingBatch,
    UpdatedBatchJobCounts,
};
pub use chain::{ChainLink, PendingChain};
pub use database::DatabaseQueueDriver;
pub use driver::{QueueDriver, Reservation, ReservationToken, Settled};
pub use envelope::{CURRENT_SCHEMA_VERSION, Envelope, EnvelopeError};
pub use errors::{ManuallyFailed, MaxAttemptsExceeded, TimeoutExceeded};
pub use failed::{
    DatabaseFailedJobStore, FailedJob, FailedJobStore, MemoryFailedJobStore, NullFailedJobStore,
};
pub use job::{BackoffSchedule, Job};
pub use memory::MemoryQueueDriver;
pub use middleware::{
    FailOnException, JobMiddleware, Next as JobMiddlewareNext, RateLimited, Skip,
    SkipIfBatchCancelled, ThrottlesExceptions, WithoutOverlapping,
};
pub use null::NullQueueDriver;
pub use outcome::JobOutcome;
pub use redis::RedisQueueDriver;
pub use routing::QueueRoute;
pub use sync::SyncQueueDriver;

use crate::error::FrameworkError;
use crate::lock;
use chrono::Utc;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

static DRIVER: RwLock<Option<Arc<dyn QueueDriver>>> = RwLock::new(None);

/// Process-wide name for the current queue connection. Carried in queue
/// lifecycle events so listeners can distinguish driver instances when an
/// app runs multiple connections at once.
static CONNECTION_NAME: RwLock<Option<String>> = RwLock::new(None);

/// Cache key for the cross-worker restart signal. Worker checks the
/// timestamp every loop iteration; if it's newer than the worker's
/// startup time, the worker exits.
const RESTART_SIGNAL_KEY: &str = "queue:restart-signal";

/// Per-push overrides for one envelope's queue, connection, and retry
/// policy, consumed by [`Queue::push_with`] / [`Queue::later_with`].
/// Every field defaults to `None` ("defer to the normal resolution
/// [`Queue::push`] already runs": [`Queue::route`], then `J`'s own
/// `Job::*` declarations, then the driver default). A `Some` field wins
/// over all of that for this one push.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EnvelopeOverrides {
    /// Queue name. Outranks `Queue::route` and `Job::queue()`.
    pub queue: Option<String>,
    /// Connection name reported on `JobQueueing` / `JobQueued`. Outranks
    /// `Queue::route` and `Job::connection()`.
    pub connection: Option<String>,
    /// Per-attempt timeout. Outranks `Job::timeout()`.
    pub timeout: Option<std::time::Duration>,
    /// Fail-on-timeout. Outranks `Job::fail_on_timeout()`.
    pub fail_on_timeout: Option<bool>,
    /// Max attempts. Outranks `Job::max_tries()`.
    pub max_tries: Option<u32>,
    /// Backoff schedule. Outranks `Job::backoff()`.
    pub backoff: Option<BackoffSchedule>,
}

/// `Queue` facade.
///
/// Configure once at boot via `Queue::set_driver(...)` (or one of the
/// `Queue::use_*` helpers added in later tasks). In tests, install
/// `testing::install_fake()` and assert with `testing::assert_pushed`.
pub struct Queue;

impl Queue {
    /// Route every future dispatch of `J` to a connection and/or queue.
    ///
    /// Mirrors Laravel 13's `Queue::route(...)`. Register in
    /// `bootstrap::register()` so which worker pool drains which job is one
    /// visible decision rather than a property scattered across job types:
    ///
    /// ```rust,no_run
    /// # use suprnova::queue::{Job, Queue};
    /// # use suprnova::FrameworkError;
    /// # #[derive(serde::Serialize, serde::Deserialize)]
    /// # struct SendInvoice;
    /// # #[suprnova::async_trait]
    /// # impl Job for SendInvoice {
    /// #     fn job_name() -> &'static str { "SendInvoice" }
    /// #     async fn handle(self) -> Result<(), FrameworkError> { Ok(()) }
    /// # }
    /// Queue::route::<SendInvoice>(Some("redis"), Some("billing"));
    /// ```
    ///
    /// Passing `None` for a field leaves that dimension alone, so the
    /// connection can be routed without disturbing the job's own queue.
    /// A route overrides [`Job::queue`] / [`Job::connection`]; re-registering
    /// the same job replaces the previous rule.
    ///
    /// The two dimensions are not equally deep. The **queue** is honored end
    /// to end: stamped on the envelope, stored by the driver, and filtered by
    /// `queue:work --queue=...`. The **connection** currently resolves only
    /// the connection *name* carried on [`events::JobQueueing`] /
    /// [`events::JobQueued`] — a single process-global driver still receives
    /// every push, so routing the connection does not yet select a different
    /// driver.
    ///
    /// Infallible by design to match Laravel's spelling. The registry is
    /// only unavailable if a previous caller panicked while holding its
    /// lock; that case is logged and the route is dropped. Use
    /// [`Queue::try_route`] when you need to handle it.
    pub fn route<J: Job>(connection: Option<&str>, queue: Option<&str>) {
        if let Err(e) = Self::try_route::<J>(connection, queue) {
            tracing::error!(
                job = J::job_name(),
                error = %e,
                "queue route registration failed; job will use the default queue"
            );
        }
    }

    /// Fallible sibling of [`Queue::route`].
    ///
    /// Returns `Err` only when the route registry's lock is poisoned.
    pub fn try_route<J: Job>(
        connection: Option<&str>,
        queue: Option<&str>,
    ) -> Result<(), FrameworkError> {
        routing::try_set_route::<J>(connection, queue)
    }

    /// The routing rule registered for `J`, if any.
    pub fn route_for<J: Job>() -> Option<routing::QueueRoute> {
        routing::route_for(J::job_name())
    }

    /// Push a typed job. Returns when the envelope is committed to the
    /// driver (NOT when the job runs).
    ///
    /// Honors [`Job::delay`]: when the job declares one, `available_at`
    /// is `now + J::delay()` instead of `now`. Use
    /// [`Queue::push_later`] / [`Queue::later`] for a delay that varies
    /// per dispatch — those take an explicit timestamp and never consult
    /// `Job::delay`.
    pub async fn push<J: Job>(job: J) -> Result<(), FrameworkError> {
        let available_at = resolve_job_delay::<J>(Utc::now())?;
        if testing::is_active() {
            let id = testing::record::<J>(&job, available_at)?;
            Self::dispatch_fake_queued_events::<J>(id).await;
            return Ok(());
        }
        let env = envelope_for::<J>(&job, available_at)?;
        let _ = crate::events::EventFacade::dispatch(events::JobQueueing {
            job_name: J::job_name().into(),
            connection: routing::resolve_connection::<J>(Self::connection_name()),
        })
        .await;
        let drv = current_driver()?;
        let env_id = env.id;
        drv.push(env).await?;
        let _ = crate::events::EventFacade::dispatch(events::JobQueued {
            id: env_id,
            job_name: J::job_name().into(),
            connection: routing::resolve_connection::<J>(Self::connection_name()),
        })
        .await;
        Ok(())
    }

    /// Push a typed job available at `available_at`. Driver is responsible
    /// for honoring the timestamp.
    ///
    /// Does **not** consult [`Job::delay`] — the explicit `available_at`
    /// always wins over the job's own default. [`Queue::push`] is the
    /// entry point that honors `Job::delay`.
    pub async fn push_later<J: Job>(
        job: J,
        available_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), FrameworkError> {
        if testing::is_active() {
            let id = testing::record::<J>(&job, available_at)?;
            Self::dispatch_fake_queued_events::<J>(id).await;
            return Ok(());
        }
        let env = envelope_for::<J>(&job, available_at)?;
        let _ = crate::events::EventFacade::dispatch(events::JobQueueing {
            job_name: J::job_name().into(),
            connection: routing::resolve_connection::<J>(Self::connection_name()),
        })
        .await;
        let drv = current_driver()?;
        let env_id = env.id;
        drv.push(env).await?;
        let _ = crate::events::EventFacade::dispatch(events::JobQueued {
            id: env_id,
            job_name: J::job_name().into(),
            connection: routing::resolve_connection::<J>(Self::connection_name()),
        })
        .await;
        Ok(())
    }

    /// Emit the `JobQueueing` + `JobQueued` pair from inside
    /// `Queue::fake()`.
    ///
    /// The fake short-circuits before the driver, so without this a test
    /// that installs both `Queue::fake()` and `Event::fake()` records the
    /// push but sees no lifecycle events - the fake and the real path
    /// would disagree about what an enqueue looks like to a listener, and
    /// the fake's envelope id would have nothing to correlate against.
    ///
    /// Only `push` / `push_later` call this, because only `push` /
    /// `push_later` emit the pair on the real path: `bulk` and
    /// `push_unique_at` dispatch neither event, and the fake must not
    /// invent one.
    async fn dispatch_fake_queued_events<J: Job>(id: Uuid) {
        let connection = routing::resolve_connection::<J>(Self::connection_name());
        let _ = crate::events::EventFacade::dispatch(events::JobQueueing {
            job_name: J::job_name().into(),
            connection: connection.clone(),
        })
        .await;
        let _ = crate::events::EventFacade::dispatch(events::JobQueued {
            id,
            job_name: J::job_name().into(),
            connection,
        })
        .await;
    }

    /// Convenience: push with a delay from `now`.
    pub async fn later<J: Job>(delay: std::time::Duration, job: J) -> Result<(), FrameworkError> {
        let available_at = Utc::now()
            + chrono::Duration::from_std(delay)
                .map_err(|e| FrameworkError::internal(format!("delay overflow: {e}")))?;
        Self::push_later(job, available_at).await
    }

    /// Push a typed job with per-push [`EnvelopeOverrides`]. Behaves like
    /// [`Queue::push`], except any field `overrides` sets wins over both
    /// a [`Queue::route`] registered for `J` and `J`'s own `Job::*`
    /// declarations; a field left `None` defers to that same resolution.
    /// `Queue::push(job)` is unchanged sugar for
    /// `Queue::push_with(job, EnvelopeOverrides::default())`.
    pub async fn push_with<J: Job>(
        job: J,
        overrides: EnvelopeOverrides,
    ) -> Result<(), FrameworkError> {
        let available_at = resolve_job_delay::<J>(Utc::now())?;
        Self::push_with_at(job, available_at, overrides).await
    }

    /// `push_with` variant that takes a delay from now, mirroring
    /// [`Queue::later`]'s relationship to [`Queue::push`].
    pub async fn later_with<J: Job>(
        delay: std::time::Duration,
        job: J,
        overrides: EnvelopeOverrides,
    ) -> Result<(), FrameworkError> {
        let available_at = Utc::now()
            + chrono::Duration::from_std(delay)
                .map_err(|e| FrameworkError::internal(format!("delay overflow: {e}")))?;
        Self::push_with_at(job, available_at, overrides).await
    }

    /// Shared body for [`Queue::push_with`] / [`Queue::later_with`].
    /// `overrides.connection`, when set, short-circuits
    /// `routing::resolve_connection` (connection isn't stored on the
    /// envelope, only reported on the events below).
    async fn push_with_at<J: Job>(
        job: J,
        available_at: chrono::DateTime<chrono::Utc>,
        overrides: EnvelopeOverrides,
    ) -> Result<(), FrameworkError> {
        let connection = overrides
            .connection
            .clone()
            .unwrap_or_else(|| routing::resolve_connection::<J>(Self::connection_name()));
        if testing::is_active() {
            // Mirrors `push`/`push_later` under the fake (Design note 4).
            let id = testing::record::<J>(&job, available_at)?;
            let _ = crate::events::EventFacade::dispatch(events::JobQueueing {
                job_name: J::job_name().into(),
                connection: connection.clone(),
            })
            .await;
            let _ = crate::events::EventFacade::dispatch(events::JobQueued {
                id,
                job_name: J::job_name().into(),
                connection,
            })
            .await;
            return Ok(());
        }
        let mut env = envelope_for::<J>(&job, available_at)?;
        apply_overrides(&mut env, &overrides);
        let _ = crate::events::EventFacade::dispatch(events::JobQueueing {
            job_name: J::job_name().into(),
            connection: connection.clone(),
        })
        .await;
        let drv = current_driver()?;
        let env_id = env.id;
        drv.push(env).await?;
        let _ = crate::events::EventFacade::dispatch(events::JobQueued {
            id: env_id,
            job_name: J::job_name().into(),
            connection,
        })
        .await;
        Ok(())
    }

    /// Push a typed job, but only if no job with the same
    /// `(job_name, J::unique_id(&job))` was successfully enqueued in the
    /// last [`Job::unique_for`].
    ///
    /// Three outcomes, two of which are `Ok(true)`:
    ///
    /// - `Ok(true)` — the envelope was pushed under an unbroken dedupe
    ///   lease. Uniqueness held.
    /// - `Ok(true)`, **plus a logged warning** — the envelope was pushed,
    ///   but the dedupe lease was lost mid-push
    ///   ([`Idempotent::FreshUnfenced`](crate::idempotency::Idempotent::FreshUnfenced)),
    ///   so a concurrent caller may have pushed a duplicate for the same
    ///   unique id. The job is on the queue either way; only the uniqueness
    ///   claim is unproven. Handlers are already required to tolerate
    ///   redelivery, which is what makes `true` the honest answer here —
    ///   the alternative, `false`, would tell the caller a job that is
    ///   about to run was never queued.
    /// - `Ok(false)` — a live dedupe key already existed for this
    ///   `(job_name, unique_id)`, so nothing was pushed.
    ///
    /// Backed by [`Idempotency::commit_on_success`](crate::idempotency::Idempotency::commit_on_success):
    /// a push failure releases the dedupe key so the caller can retry; a
    /// successful push holds the key for `unique_for` to gate re-submissions.
    ///
    /// Requires the cache layer to be bootstrapped (the dedupe lock lives
    /// in [`Cache`](crate::cache::Cache)). Returns an internal error if
    /// `J::unique_id(&job)` returns `None`.
    pub async fn push_unique<J: Job>(job: J) -> Result<bool, FrameworkError> {
        Self::push_unique_at::<J>(job, Utc::now()).await
    }

    /// `push_unique` variant that schedules the envelope for delivery at
    /// `available_at` (combines with the configured driver's delayed-job
    /// strategy: ZSET on Redis, `available_at` column on the database
    /// driver, virtual-clock DelayQueue on the memory driver).
    pub async fn push_unique_later<J: Job>(
        job: J,
        available_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, FrameworkError> {
        Self::push_unique_at::<J>(job, available_at).await
    }

    /// `push_unique` variant that takes a delay from now (the unique
    /// analogue of [`Queue::later`]).
    pub async fn later_unique<J: Job>(
        delay: std::time::Duration,
        job: J,
    ) -> Result<bool, FrameworkError> {
        let available_at = Utc::now()
            + chrono::Duration::from_std(delay)
                .map_err(|e| FrameworkError::internal(format!("delay overflow: {e}")))?;
        Self::push_unique_at::<J>(job, available_at).await
    }

    /// Common path for the three `*_unique*` entrypoints — builds the
    /// dedupe key, runs the enqueue under `Idempotency::commit_on_success`,
    /// and reports `true` for `Fresh` and `FreshUnfenced` (the envelope
    /// reached the driver either way), `false` only for `Duplicate`.
    async fn push_unique_at<J: Job>(
        job: J,
        available_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, FrameworkError> {
        if testing::is_active() {
            // In fake mode, dedupe is irrelevant — record and report fresh.
            testing::record::<J>(&job, available_at)?;
            return Ok(true);
        }
        let id = job.unique_id().ok_or_else(|| {
            FrameworkError::internal(
                "Queue::push_unique requires Job::unique_id(&self) to return Some(...)",
            )
        })?;
        let ttl = J::unique_for();
        let key = format!("queue-unique:{}:{}", J::job_name(), id);
        // The closure below takes `id` by value to stamp the envelope's
        // idempotency key, so keep a copy for the event payload.
        let unique_id = id.clone();

        let outcome =
            crate::idempotency::Idempotency::commit_on_success(&key, ttl, move || async move {
                let mut env = envelope_for::<J>(&job, available_at)?;
                env.idempotency_key = Some(id);
                let drv = current_driver()?;
                drv.push(env).await
            })
            .await?;

        // Exhaustive on purpose. `matches!(outcome, Fresh(()))` collapsed
        // `FreshUnfenced` — the body ran, the envelope IS on the queue, only
        // the dedupe lease was lost — into `false`, which this function
        // documents as "suppressed as a duplicate". A `match` also means a
        // future `Idempotent` variant fails to compile here instead of
        // silently joining whichever arm `matches!` happened to exclude.
        match outcome {
            crate::idempotency::Idempotent::Fresh(()) => Ok(true),
            crate::idempotency::Idempotent::FreshUnfenced(()) => {
                tracing::warn!(
                    job = J::job_name(),
                    unique_key = %key,
                    "Queue::push_unique enqueued the job but lost the dedupe lease \
                     while pushing; the envelope is on the queue, but exclusivity \
                     could not be proven, so a duplicate may exist for this unique id"
                );
                Ok(true)
            }
            crate::idempotency::Idempotent::Duplicate => {
                // Match `Duplicate` explicitly rather than inverting `Fresh`:
                // `FreshUnfenced` means the body ran (an envelope WAS
                // published) under a lost lease, and reporting that as
                // "skipped" would be a lie to every listener.
                let _ = crate::events::EventFacade::dispatch(events::UniqueJobSkipped {
                    job_name: J::job_name().into(),
                    unique_id,
                    connection: routing::resolve_connection::<J>(Self::connection_name()),
                })
                .await;
                Ok(false)
            }
        }
    }

    /// Push every job in `jobs` onto the queue. Mirrors Laravel's
    /// `Queue::bulk($jobs, $data, $queue)`. Each job is encoded and
    /// committed via the driver's [`QueueDriver::bulk_push`] hook (with a
    /// serial-push default).
    ///
    /// Honors [`Job::delay`], resolved once for the whole call: every
    /// element of `jobs` shares the same concrete `J`, so they share the
    /// same declared delay.
    pub async fn bulk<J: Job + Clone>(jobs: Vec<J>) -> Result<(), FrameworkError> {
        let available_at = resolve_job_delay::<J>(Utc::now())?;
        if testing::is_active() {
            for j in jobs {
                testing::record::<J>(&j, available_at)?;
            }
            return Ok(());
        }
        let mut envs = Vec::with_capacity(jobs.len());
        for j in jobs {
            envs.push(envelope_for::<J>(&j, available_at)?);
        }
        let drv = current_driver()?;
        drv.bulk_push(envs).await
    }

    /// Begin a queued batch builder. Mirrors `Bus::batch([...])`.
    ///
    /// Add jobs with `.add(job)`, register `then`/`catch`/`finally`
    /// callbacks by name, then `.dispatch()` to push every job through
    /// the configured driver under one batch id.
    pub fn batch() -> PendingBatch {
        PendingBatch::new()
    }

    /// Begin a queued chain builder. Mirrors `Bus::chain([...])`.
    pub fn chain() -> PendingChain {
        PendingChain::new()
    }

    /// Total envelopes currently held by the driver
    /// (pending + delayed + reserved).
    pub async fn size() -> Result<u64, FrameworkError> {
        current_driver()?.size().await
    }

    /// Envelopes whose `available_at <= now` and which are not reserved.
    pub async fn pending_size() -> Result<u64, FrameworkError> {
        current_driver()?.pending_size().await
    }

    /// Envelopes whose `available_at > now`.
    pub async fn delayed_size() -> Result<u64, FrameworkError> {
        current_driver()?.delayed_size().await
    }

    /// Envelopes currently held by an unfinished reservation.
    pub async fn reserved_size() -> Result<u64, FrameworkError> {
        current_driver()?.reserved_size().await
    }

    /// Drop every envelope on the configured driver. Returns the number
    /// of envelopes removed. Mirrors `Queue::clear($queue)`.
    pub async fn clear() -> Result<u64, FrameworkError> {
        current_driver()?.clear().await
    }

    /// Broadcast a restart signal to every worker on this connection.
    /// Workers poll the cache key once per loop and exit cleanly when
    /// the signal's timestamp is newer than their startup time. Mirrors
    /// Laravel's `php artisan queue:restart`.
    ///
    /// Requires the cache subsystem to be bootstrapped (the signal lives
    /// in [`Cache`](crate::cache::Cache)). The timestamp is stored in
    /// milliseconds so tightly-clustered `restart()` calls in tests are
    /// distinguishable.
    pub async fn restart() -> Result<(), FrameworkError> {
        let now = Utc::now().timestamp_millis();
        crate::cache::Cache::put(RESTART_SIGNAL_KEY, &now, None).await?;
        Ok(())
    }

    /// Read the latest restart-signal millisecond timestamp set by
    /// [`Queue::restart`]. Returns `None` when no signal has been issued.
    pub async fn restart_signal() -> Result<Option<i64>, FrameworkError> {
        crate::cache::Cache::get::<i64>(RESTART_SIGNAL_KEY).await
    }

    /// Replace the failed-jobs store (where the worker writes dead-lettered
    /// envelopes). Defaults to [`MemoryFailedJobStore`] when not set.
    pub fn set_failed_store(store: Arc<dyn FailedJobStore>) {
        failed::install(store);
    }

    /// Read the configured failed-jobs store. Returns `None` when none has
    /// been wired (in which case the worker still dead-letters via tracing
    /// but doesn't persist a record).
    pub fn failed_store() -> Option<Arc<dyn FailedJobStore>> {
        failed::current()
    }

    /// Re-enqueue a previously dead-lettered job by id. Loads the
    /// envelope from the configured [`FailedJobStore`], resets its
    /// `attempts`, `available_at`, and `idempotency_key`, pushes it
    /// through the configured driver, then deletes the failed-job
    /// record. Mirrors `php artisan queue:retry <id>`.
    ///
    /// Returns `Ok(true)` when the record was retried, `Ok(false)` when
    /// the id had no record in the store.
    pub async fn retry_failed(id: Uuid) -> Result<bool, FrameworkError> {
        let store = failed::current().ok_or_else(|| {
            FrameworkError::internal(
                "Queue::retry_failed requires a failed-jobs store; call \
                 Queue::set_failed_store(...) first",
            )
        })?;
        let Some(record) = store.find(id).await? else {
            return Ok(false);
        };
        let mut env = Envelope::from_json(&record.envelope_json)
            .map_err(|e| FrameworkError::internal(format!("retry_failed: decode envelope: {e}")))?;
        env.attempts = 0;
        env.available_at = Utc::now();
        env.idempotency_key = None;
        let drv = current_driver()?;
        drv.push(env).await?;
        store.forget(id).await?;
        Ok(true)
    }

    /// Re-enqueue every failed-job record (optionally only those older
    /// than `before`). Returns the number of records retried. Mirrors
    /// `php artisan queue:retry all` plus `queue:flush` semantics: each
    /// retried envelope is pushed AND removed from the store.
    pub async fn retry_all_failed(
        before: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<u64, FrameworkError> {
        let store = failed::current().ok_or_else(|| {
            FrameworkError::internal(
                "Queue::retry_all_failed requires a failed-jobs store; call \
                 Queue::set_failed_store(...) first",
            )
        })?;
        let records = store.all().await?;
        let drv = current_driver()?;
        let mut count: u64 = 0;
        for record in records {
            if let Some(cutoff) = before
                && record.failed_at >= cutoff
            {
                continue;
            }
            let Ok(mut env) = Envelope::from_json(&record.envelope_json) else {
                continue;
            };
            env.attempts = 0;
            env.available_at = Utc::now();
            env.idempotency_key = None;
            drv.push(env).await?;
            store.forget(record.id).await?;
            count += 1;
        }
        Ok(count)
    }

    /// Replace the batch repository. Defaults to [`MemoryBatchRepository`]
    /// on first use.
    pub fn set_batch_repository(repo: Arc<dyn BatchRepository>) {
        batch::install_repository(repo);
    }

    /// Read the configured batch repository.
    pub fn batch_repository() -> Option<Arc<dyn BatchRepository>> {
        batch::current_repository()
    }

    /// Set the connection name carried in queue lifecycle events. Defaults
    /// to the driver's `name()` if not overridden.
    pub fn set_connection_name(name: impl Into<String>) {
        if let Ok(mut g) = CONNECTION_NAME.write() {
            *g = Some(name.into());
        }
    }

    /// Resolve the connection name for events: explicit override → driver
    /// name → "default".
    pub fn connection_name() -> String {
        if let Ok(g) = CONNECTION_NAME.read()
            && let Some(n) = g.as_ref()
        {
            return n.clone();
        }
        current_driver()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| "default".into())
    }

    /// Replace the registered driver. Primarily for boot-time wiring;
    /// in tests prefer `testing::install_fake()`.
    pub fn set_driver(driver: Arc<dyn QueueDriver>) {
        // The driver registry is a single-slot `Option<Arc<dyn QueueDriver>>`;
        // the critical section is a single assignment. Recover in place
        // on poison so a panic in some other registry user doesn't kill
        // the boot path for every future caller — matches the framework's
        // hot-registry convention (data::registry, payments registry).
        *DRIVER.write().unwrap_or_else(|e| e.into_inner()) = Some(driver);
    }

    /// Return the registered driver's `name()` for observability (admin,
    /// `queue:work` startup log, debug). Returns the same `FrameworkError`
    /// that [`Queue::push`] would surface when no driver is registered.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError::internal`] when the driver registry is
    /// poisoned, or when no driver has been wired (call
    /// [`bootstrap_default`] / [`bootstrap_from_env`] / [`Queue::set_driver`]
    /// at boot).
    pub fn driver_name() -> Result<&'static str, FrameworkError> {
        Ok(current_driver()?.name())
    }

    /// Return the registered driver as an `Arc<dyn QueueDriver>` so callers
    /// (workers, admin inspectors) can use it directly. Most app code should
    /// prefer the [`Queue::push`] facade.
    ///
    /// # Errors
    ///
    /// Same conditions as [`Queue::driver_name`].
    pub fn driver() -> Result<Arc<dyn QueueDriver>, FrameworkError> {
        current_driver()
    }
}

pub(crate) fn current_driver() -> Result<Arc<dyn QueueDriver>, FrameworkError> {
    lock::read(&DRIVER, "queue driver registry")?
        .clone()
        .ok_or_else(|| {
            FrameworkError::internal(
                "queue driver not initialized; call Queue::set_driver(...) or install a test fake",
            )
        })
}

/// Wire the in-memory queue driver as the default. Idempotent.
pub async fn bootstrap_default() {
    if lock::read(&DRIVER, "queue driver registry")
        .map(|g| g.is_some())
        .unwrap_or(false)
    {
        return;
    }
    Queue::set_driver(Arc::new(memory::MemoryQueueDriver::new()));
}

/// Read `QUEUE_DRIVER` env and configure the matching driver. Falls back to the
/// in-memory default on any unrecognized value or when `QUEUE_DRIVER` is unset.
///
/// Unlike [`bootstrap_default`], this call **always replaces** the registered
/// driver — long-running processes (workers, tests) that re-invoke
/// `bootstrap_from_env` after `QUEUE_DRIVER` changes (or after an earlier
/// Redis/database boot) will pick up the new driver instead of being pinned to
/// the first one installed.
pub async fn bootstrap_from_env() -> Result<(), FrameworkError> {
    let driver = std::env::var("QUEUE_DRIVER").unwrap_or_else(|_| "memory".into());
    match driver.as_str() {
        "memory" => Queue::set_driver(Arc::new(memory::MemoryQueueDriver::new())),
        "redis" => {
            let url = std::env::var("QUEUE_REDIS_URL")
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
            let stream =
                std::env::var("QUEUE_REDIS_STREAM").unwrap_or_else(|_| "suprnova-queue".into());
            let group = std::env::var("QUEUE_REDIS_GROUP").unwrap_or_else(|_| "default".into());
            let consumer =
                std::env::var("QUEUE_REDIS_CONSUMER").unwrap_or_else(|_| "consumer-1".into());
            let visibility = std::time::Duration::from_secs(
                std::env::var("QUEUE_VISIBILITY_TIMEOUT_SECS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(60),
            );
            let d = redis::RedisQueueDriver::connect(&url, &stream, &group, &consumer, visibility)
                .await?;
            Queue::set_driver(Arc::new(d));
        }
        "database" => {
            let table = std::env::var("QUEUE_DB_TABLE").unwrap_or_else(|_| "jobs".into());
            // Requires DB::init() (or DB::init_with(...)) to have been called first.
            let db = crate::database::DB::connection().map_err(|e| {
                FrameworkError::internal(format!(
                    "QUEUE_DRIVER=database requires DB::init() to run first: {e}"
                ))
            })?;
            // DatabaseConnection is Arc-backed (SeaORM pool), so clone is cheap.
            // `new` validates QUEUE_DB_TABLE as a SQL identifier — a malformed
            // env value fails here instead of reaching SQL composition.
            let driver = database::DatabaseQueueDriver::new(db.inner().clone(), table)?;
            Queue::set_driver(Arc::new(driver));

            // The `failed_jobs` table is part of this driver's contract —
            // `queue:retry` reads it, and `Queue::retry_failed` fails
            // without it. Binding the driver and leaving the failed-jobs
            // store unset meant a database-backed queue dead-lettered into
            // nothing unless the app wired one by hand, which nothing in
            // the scaffold or the docs prompted anyone to do.
            //
            // Only for this driver. `memory` is ephemeral by construction,
            // and `redis` has no table to write to, so inventing a
            // database dependency for either would be worse than the gap.
            let failed_table =
                std::env::var("QUEUE_FAILED_DB_TABLE").unwrap_or_else(|_| "failed_jobs".into());
            match failed::DatabaseFailedJobStore::new(db.inner().clone(), failed_table) {
                Ok(store) => Queue::set_failed_store(Arc::new(store)),
                Err(e) => {
                    // A malformed table name is a misconfiguration, not a
                    // reason to take the queue down — the worker now logs
                    // the whole envelope when no store is bound, so
                    // failures stay recoverable either way.
                    tracing::error!(
                        error = %e,
                        "QUEUE_FAILED_DB_TABLE is not a valid identifier; dead-lettered \
                         jobs will be logged rather than persisted"
                    );
                }
            }
        }
        other => {
            tracing::warn!(driver = %other, "unknown QUEUE_DRIVER, falling back to memory");
            Queue::set_driver(Arc::new(memory::MemoryQueueDriver::new()));
        }
    }
    Ok(())
}

/// Resolve `available_at` for the two entry points that consult
/// [`Job::delay`]: [`Queue::push`] and [`Queue::bulk`]. Returns `base`
/// unchanged when the job declares no delay.
///
/// `push_later` / `later` / the `*_unique*` family never call this — they
/// take an explicit `available_at` (or delay) from the caller, and that
/// always wins over the job's own declared default.
fn resolve_job_delay<J: Job>(
    base: chrono::DateTime<chrono::Utc>,
) -> Result<chrono::DateTime<chrono::Utc>, FrameworkError> {
    match J::delay() {
        Some(delay) => {
            let delta = chrono::Duration::from_std(delay)
                .map_err(|e| FrameworkError::internal(format!("Job::delay() overflow: {e}")))?;
            Ok(base + delta)
        }
        None => Ok(base),
    }
}

fn envelope_for<J: Job>(
    job: &J,
    available_at: chrono::DateTime<chrono::Utc>,
) -> Result<Envelope, FrameworkError> {
    build_envelope::<J>(job, available_at)
}

/// Overlay `overrides` onto an already-resolved envelope, after
/// `envelope_for` — see [`EnvelopeOverrides`]. No schema change: every
/// touched field already exists on the frozen envelope.
fn apply_overrides(env: &mut Envelope, overrides: &EnvelopeOverrides) {
    if let Some(queue) = &overrides.queue {
        env.queue = Some(queue.clone());
    }
    if let Some(max_tries) = overrides.max_tries {
        env.max_tries = max_tries;
    }
    if let Some(backoff) = &overrides.backoff {
        env.backoff = backoff.clone();
    }
    if let Some(timeout) = overrides.timeout {
        env.timeout_secs = Some(timeout.as_secs());
    }
    if let Some(fail_on_timeout) = overrides.fail_on_timeout {
        env.fail_on_timeout = fail_on_timeout;
    }
}

/// Build an envelope for the typed job. Used by [`Queue::push`] and by
/// [`PendingBatch::add`] / [`PendingChain::add`]. `pub(crate)` because
/// external code goes through the facade.
pub(crate) fn build_envelope<J: Job>(
    job: &J,
    available_at: chrono::DateTime<chrono::Utc>,
) -> Result<Envelope, FrameworkError> {
    let payload = serde_json::to_value(job)
        .map_err(|e| FrameworkError::internal(format!("encode job: {e}")))?;
    let timeout_secs = J::timeout().map(|d| d.as_secs());
    Ok(Envelope {
        schema_version: CURRENT_SCHEMA_VERSION,
        id: Uuid::new_v4(),
        job_name: J::job_name().to_string(),
        queue: routing::resolve_queue::<J>(),
        payload,
        dispatched_at: Utc::now(),
        available_at,
        attempts: 0,
        max_tries: J::max_tries(),
        backoff: J::backoff(),
        timeout_secs,
        fail_on_timeout: J::fail_on_timeout(),
        idempotency_key: None,
        batch_id: None,
        chain_remaining: Vec::new(),
    })
}
