//! Queue subsystem: facade, drivers, envelope, worker.

pub mod batch;
pub mod chain;
pub mod database;
pub mod debounce;
pub mod driver;
pub mod envelope;
pub mod errors;
pub mod events;
pub mod failed;
pub mod failover;
pub mod inspect;
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
pub use debounce::{DebounceOptions, Debounced};
pub use driver::{QueueDriver, Reservation, ReservationToken, Settled};
pub use envelope::{CURRENT_SCHEMA_VERSION, Envelope, EnvelopeError};
pub use errors::{ManuallyFailed, MaxAttemptsExceeded, TimeoutExceeded};
pub use failed::{
    DatabaseFailedJobStore, FailedJob, FailedJobStore, MemoryFailedJobStore, NullFailedJobStore,
};
pub use failover::FailoverQueueDriver;
pub use inspect::InspectedJob;
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

/// Cache key for the "pause every queue on every connection" switch, set
/// by [`Queue::pause_all`] and cleared by [`Queue::resume_all`]. Checked
/// before the per-queue key by every pause-aware read. Mirrors Laravel's
/// `illuminate:queues:paused`.
const GLOBAL_QUEUE_PAUSE_KEY: &str = "suprnova:queues:paused";

/// Cache key for one queue's own pause switch, set by [`Queue::pause`] and
/// cleared by [`Queue::resume`]. Mirrors Laravel's
/// `illuminate:queue:paused:{connection}:{queue}`.
fn queue_pause_key(connection: &str, queue: &str) -> String {
    format!("suprnova:queue:paused:{connection}:{queue}")
}

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
    /// Whether this one push waits for the surrounding transaction to commit.
    /// Outranks [`Job::after_commit`].
    ///
    /// `Some(true)` defers a job that did not opt in (see
    /// [`Queue::push_after_commit`]); `Some(false)` is Laravel's
    /// `beforeCommit()` - it pushes immediately even inside a transaction, for
    /// the dispatch that must be visible to a worker before the commit lands.
    ///
    /// Unlike every other field here this one never reaches the envelope: it
    /// decides *when* the push happens, not what the pushed envelope contains.
    pub after_commit: Option<bool>,
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
    /// [`events::JobQueued`] - a single process-global driver still receives
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

    /// Redirect every job that resolves to the queue named `from` onto `to`.
    ///
    /// Where [`Queue::route`] is keyed by job type, this is keyed by queue
    /// *name*: it is the operational lever for draining one pool through
    /// another without touching any job's code or any route. Mirrors Laravel's
    /// `Queue::forward($queue, $to)`.
    ///
    /// ```rust,no_run
    /// # use suprnova::Queue;
    /// # fn ex() {
    /// // Every push that resolved to `default` now lands on `high`, and a
    /// // worker started with `--queue=default` drains `high` instead.
    /// Queue::forward("default", "high");
    /// # }
    /// ```
    ///
    /// The redirect applies on **both** sides. On the push side it rewrites the
    /// name after [`Queue::route`] and the job's own [`Job::queue`] have had
    /// their say, and after a per-push [`EnvelopeOverrides::queue`] if one was
    /// given. On the pop side it rewrites the worker's `--queue` list, so the
    /// destination cannot accumulate work no worker claims. A worker started
    /// with no `--queue` at all already drains everything and is unaffected.
    ///
    /// A forward is a single lookup, not a chain: with `a -> b` and `b -> c`
    /// registered, a push that resolved to `a` lands on `b`. A forward that
    /// would close a loop is refused for that reason. Forwarding a queue onto
    /// its own name is the identity - no redirect at all - which is how a
    /// registered forward is neutralized.
    ///
    /// Only future pushes are redirected. Envelopes already sitting on `from`
    /// stay there, and the worker that used to drain them is now claiming `to`,
    /// so drain the source pool before you forward it.
    ///
    /// Pausing is evaluated *before* the redirect, on the names the worker was
    /// started with - so `Queue::pause(conn, "default")` still stops a worker
    /// started on `--queue=default` even while `default` is forwarded. Laravel
    /// orders it the same way.
    ///
    /// Infallible by design to match Laravel's spelling. The failures it
    /// swallows are a refused cycle and a registry left unavailable by a
    /// previous caller panicking while holding its lock; both are logged and
    /// the forward is dropped. Use [`Queue::try_forward`] when you need to
    /// handle them.
    pub fn forward(from: &str, to: &str) {
        Self::log_forward_failure(from, Self::try_forward(from, to, None));
    }

    /// [`Queue::forward`], restricted to one connection name.
    ///
    /// The forward fires only when `connection` equals this process's
    /// connection name - [`Queue::connection_name`], which is
    /// [`Queue::set_connection_name`] if it was set and the driver's own name
    /// otherwise. It is **not** compared against the job's
    /// [`Job::connection`], against a [`Queue::route`]'s connection, or against
    /// a per-push [`EnvelopeOverrides::connection`]; those name what the
    /// lifecycle events report, and a worker has only the process name to gate
    /// its claim list on. Gating the two halves on different values would let a
    /// forward move the push without moving the claim, which strands work. On
    /// any other connection name the forward is inert and the queue name passes
    /// through unchanged.
    ///
    /// # What this cannot do
    ///
    /// Laravel's `forward($queue, $to, $connection)` can also move a forwarded
    /// queue onto a *different* connection, because its `QueueManager` resolves
    /// a driver per connection name. Suprnova has one process-global driver and
    /// the connection name only labels lifecycle events (see this module's
    /// docs), so `connection` here is a **gate**, never a destination: it
    /// decides whether the queue-name redirect applies, and the push still
    /// reaches the same driver either way.
    pub fn forward_on(from: &str, to: &str, connection: &str) {
        Self::log_forward_failure(from, Self::try_forward(from, to, Some(connection)));
    }

    /// Fallible sibling of [`Queue::forward`] / [`Queue::forward_on`].
    ///
    /// `connection` is `None` for "every connection". Returns `Err` when the
    /// forward would close a cycle, or when the forward registry's lock is
    /// poisoned.
    pub fn try_forward(
        from: &str,
        to: &str,
        connection: Option<&str>,
    ) -> Result<(), FrameworkError> {
        routing::try_set_forward(from, to, connection)
    }

    /// The forward registered for the queue named `from`, if any.
    ///
    /// The returned [`QueueRoute`]'s `queue` is the destination and its
    /// `connection` is the gate, `None` meaning "every connection".
    pub fn forward_for(from: &str) -> Option<routing::QueueRoute> {
        routing::forward_for(from)
    }

    /// Shared failure log for the two infallible forward setters, so the
    /// message stays identical whichever spelling registered the forward.
    fn log_forward_failure(from: &str, result: Result<(), FrameworkError>) {
        if let Err(e) = result {
            tracing::error!(
                queue = from,
                error = %e,
                "queue forward registration failed; the queue will not be redirected"
            );
        }
    }

    /// Push a typed job. Returns when the envelope is committed to the
    /// driver (NOT when the job runs).
    ///
    /// Honors [`Job::delay`]: when the job declares one, `available_at`
    /// is `now + J::delay()` instead of `now`. Use
    /// [`Queue::push_later`] / [`Queue::later`] for a delay that varies
    /// per dispatch - those take an explicit timestamp and never consult
    /// `Job::delay`.
    ///
    /// Honors [`Job::after_commit`]: inside a
    /// [`DB::transaction`](crate::DB::transaction) an opted-in job's push
    /// waits for the commit and a rollback discards it.
    pub async fn push<J: Job>(job: J) -> Result<(), FrameworkError> {
        Self::dispatch_push(
            job,
            AvailableAt::FromJobDelay,
            EnvelopeOverrides::default(),
            None,
        )
        .await
    }

    /// Push `job`, deferring it until the surrounding transaction commits.
    ///
    /// Sugar for [`Queue::push_with`] with
    /// [`EnvelopeOverrides::after_commit`] set to `Some(true)` - use it for a
    /// job type that does not opt in via [`Job::after_commit`], typically
    /// because only some of its dispatch sites read rows the surrounding
    /// transaction wrote.
    ///
    /// Outside a transaction this is exactly [`Queue::push`].
    pub async fn push_after_commit<J: Job>(job: J) -> Result<(), FrameworkError> {
        Self::push_with(
            job,
            EnvelopeOverrides {
                after_commit: Some(true),
                ..Default::default()
            },
        )
        .await
    }

    /// Push a typed job available at `available_at`. Driver is responsible
    /// for honoring the timestamp.
    ///
    /// Does **not** consult [`Job::delay`] - the explicit `available_at`
    /// always wins over the job's own default. [`Queue::push`] is the
    /// entry point that honors `Job::delay`.
    ///
    /// [`Job::after_commit`] still applies: the push can wait for the
    /// surrounding transaction, and `available_at` is preserved exactly as
    /// given when it does.
    pub async fn push_later<J: Job>(
        job: J,
        available_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), FrameworkError> {
        Self::dispatch_push(
            job,
            AvailableAt::Fixed(available_at),
            EnvelopeOverrides::default(),
            None,
        )
        .await
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
    /// Only the [`Queue::push`] family calls this, because only that family
    /// emits the pair on the real path: `bulk` and `push_unique_at` dispatch
    /// neither event, and the fake must not invent one.
    ///
    /// `connection` is passed in rather than resolved here so the fake reports
    /// the same connection the real path would, including an
    /// [`EnvelopeOverrides::connection`] that outranks the routing table.
    async fn dispatch_fake_queued_events<J: Job>(id: Uuid, connection: String) {
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
        Self::dispatch_push(job, AvailableAt::FromJobDelay, overrides, None).await
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
        Self::dispatch_push(job, AvailableAt::Fixed(available_at), overrides, None).await
    }

    /// The single funnel for the whole [`Queue::push`] family: fake, then
    /// after-commit deferral, then the real push.
    ///
    /// The fake check comes first and skips deferral entirely, so a test that
    /// pushes inside a transaction can assert on the push without committing
    /// anything - the same choice Laravel's `Bus::fake` makes.
    ///
    /// When the push is deferred, everything below this point moves into the
    /// callback: `available_at` resolution, the envelope, both lifecycle
    /// events and the driver write. Laravel defers the whole of `enqueueUsing`
    /// for the same reason - a listener that observes `JobQueued` for a job
    /// that a rollback then discarded has been told something untrue.
    async fn dispatch_push<J: Job>(
        job: J,
        when: AvailableAt,
        overrides: EnvelopeOverrides,
        debounce: Option<debounce::DebounceOptions>,
    ) -> Result<(), FrameworkError> {
        // Above the fake on purpose, unlike the arming below it. Two
        // declarations that cannot both hold is a bug in the job, not a
        // property of the environment, so `Queue::fake()` must surface it
        // rather than hide it until production. Only the check is hoisted: a
        // fake push writes nothing to the cache, so there is no window to arm.
        if J::debounce_for().is_some() && job.unique_id().is_some() {
            return Err(debounce_conflict(J::job_name()));
        }
        if testing::is_active() {
            let available_at = when.resolve::<J>()?;
            // Records `overrides` too, so a test can assert on the
            // queue/connection/etc a push_with call declared - see
            // `testing::record_with_overrides`.
            let connection = overrides
                .connection
                .clone()
                .unwrap_or_else(|| routing::resolve_connection::<J>(Self::connection_name()));
            let id = testing::record_with_overrides::<J>(&job, available_at, overrides)?;
            Self::dispatch_fake_queued_events::<J>(id, connection).await;
            return Ok(());
        }
        if overrides.after_commit.unwrap_or_else(J::after_commit)
            && crate::database::after_commit::in_transaction()
        {
            return crate::database::after_commit::register_callback(Box::new(move || {
                Box::pin(async move {
                    Self::push_immediately::<J>(job, when, overrides, debounce).await
                })
            }))
            .await;
        }
        Self::push_immediately::<J>(job, when, overrides, debounce).await
    }

    /// Build the envelope, emit `JobQueueing`, write to the driver, emit
    /// `JobQueued`. Shared by the immediate and deferred paths so a deferred
    /// push is byte-for-byte the push that would have happened, only later.
    ///
    /// `overrides.connection`, when set, short-circuits
    /// `routing::resolve_connection` (connection isn't stored on the
    /// envelope, only reported on the events below).
    async fn push_immediately<J: Job>(
        job: J,
        when: AvailableAt,
        overrides: EnvelopeOverrides,
        debounce: Option<debounce::DebounceOptions>,
    ) -> Result<(), FrameworkError> {
        let connection = overrides
            .connection
            .clone()
            .unwrap_or_else(|| routing::resolve_connection::<J>(Self::connection_name()));
        let available_at = when.resolve::<J>()?;
        let mut env = envelope_for::<J>(&job, available_at)?;
        // The forward gate is the process connection name; `connection` above is
        // the resolved name the lifecycle events report, and the two are
        // deliberately different values.
        apply_overrides(&mut env, &overrides, &Self::connection_name());
        // The window is armed here rather than at the entry point so a deferred
        // push arms it at the commit, in the same step that writes the
        // envelope. Arming earlier would let a rolled-back transaction leave an
        // owner token behind for a dispatch that never happened - and the
        // worker would then read an *earlier*, still-queued envelope as
        // superseded and drop it, losing work whose own push succeeded.
        if let Some(armed_at) = arm_debounce::<J>(&job, &mut env, debounce.as_ref()).await?
            && (debounce.is_some()
                || (matches!(when, AvailableAt::FromJobDelay) && J::delay().is_none()))
        {
            // An explicit `available_at` and an explicit `Job::delay` both
            // outrank a declared window, the way Laravel's
            // `is_null($this->job->delay)` guard does. Options handed in at the
            // call site *are* the explicit statement, so they win instead.
            env.available_at = armed_at;
        }
        let _ = crate::events::EventFacade::dispatch(events::JobQueueing {
            job_name: J::job_name().into(),
            connection: connection.clone(),
        })
        .await;
        let env_id = env.id;
        // Cloned before `env` moves into the driver: the cleanup below is
        // owner-checked, so it needs this dispatch's own token and not just the
        // key.
        let armed = env.debounce_owner.clone().map(|owner| {
            (
                debounce_key(&env.job_name, env.debounce_id.as_deref()),
                owner,
            )
        });
        // Resolving the driver is inside the guarded block, not above it: a
        // missing driver after the window was armed is the same hazard as a
        // failed write, and leaving it outside would skip the cleanup.
        let result = async {
            let drv = current_driver()?;
            drv.push(env).await
        }
        .await;
        if let Err(e) = result {
            // The window is armed for an envelope that never reached the queue.
            // Leaving the token in place would make every earlier envelope of
            // this burst look superseded, and the worker would drop work whose
            // own push reported success. Let the window lapse instead - a
            // lapsed window fails open, so whatever is still queued runs. Only
            // while this dispatch still owns it: a newer one that armed and
            // enqueued while this write was failing keeps its window.
            if let Some((key, owner)) = armed
                && let Err(cleanup) = debounce::abandon(&key, &owner).await
            {
                tracing::warn!(
                    job = J::job_name(),
                    error = %cleanup,
                    "a debounced push failed and its window could not be cleared; \
                     envelopes already queued for this window may be dropped as \
                     superseded by a dispatch that never reached the queue"
                );
            }
            return Err(e);
        }
        let _ = crate::events::EventFacade::dispatch(events::JobQueued {
            id: env_id,
            job_name: J::job_name().into(),
            connection,
        })
        .await;
        Ok(())
    }

    /// Push a job with a debounce window supplied at the call site.
    ///
    /// The declarative form is [`Job::debounce_for`] and friends; reach for
    /// this when the window belongs to the *caller* rather than to the job -
    /// which is what [`DebouncedListener`](crate::events::DebouncedListener)
    /// does, and what Laravel's `#[DebounceFor]` attribute on a listener
    /// expresses.
    ///
    /// ```rust,no_run
    /// # use std::time::Duration;
    /// # use suprnova::queue::{DebounceOptions, Job, Queue};
    /// # use suprnova::FrameworkError;
    /// # #[derive(serde::Serialize, serde::Deserialize)]
    /// # struct ReindexOrder { order_id: u32 }
    /// # #[suprnova::async_trait]
    /// # impl Job for ReindexOrder {
    /// #     fn job_name() -> &'static str { "ReindexOrder" }
    /// #     async fn handle(self) -> Result<(), FrameworkError> { Ok(()) }
    /// # }
    /// # async fn ex() -> Result<(), FrameworkError> {
    /// Queue::push_debounced(
    ///     ReindexOrder { order_id: 7 },
    ///     DebounceOptions::new(Duration::from_secs(30))
    ///         .max_wait(Duration::from_secs(300))
    ///         .id("7"),
    /// )
    /// .await?;
    /// # Ok(()) }
    /// ```
    ///
    /// The options win over anything the job declares, including
    /// [`Job::delay`]: naming a window at the call site is the explicit
    /// statement, so the envelope becomes available one window from now.
    ///
    /// Honors [`Job::after_commit`] like the rest of the [`Queue::push`]
    /// family - the window is armed at the commit, in the same step that
    /// writes the envelope, so a rollback arms nothing.
    ///
    /// Returns `Err` when the job also declares [`Job::unique_id`], for the
    /// reason [`Job::debounce_for`] gives.
    pub async fn push_debounced<J: Job>(
        job: J,
        options: debounce::DebounceOptions,
    ) -> Result<(), FrameworkError> {
        Self::dispatch_push(
            job,
            AvailableAt::FromJobDelay,
            EnvelopeOverrides::default(),
            Some(options),
        )
        .await
    }

    /// Push a typed job, but only if no job with the same
    /// `(job_name, J::unique_id(&job))` was successfully enqueued in the
    /// last [`Job::unique_for`].
    ///
    /// Honors [`Job::delay`], the same as [`Queue::push`]: when the job
    /// declares one, `available_at` is `now + J::delay()` instead of `now`.
    /// Use [`Queue::push_unique_later`] / [`Queue::later_unique`] for a
    /// delay that varies per dispatch - those take an explicit timestamp
    /// and never consult `Job::delay`.
    ///
    /// Three outcomes, two of which are `Ok(true)`:
    ///
    /// - `Ok(true)` - the envelope was pushed under an unbroken dedupe
    ///   lease. Uniqueness held.
    /// - `Ok(true)`, **plus a logged warning** - the envelope was pushed,
    ///   but the dedupe lease was lost mid-push
    ///   ([`Idempotent::FreshUnfenced`](crate::idempotency::Idempotent::FreshUnfenced)),
    ///   so a concurrent caller may have pushed a duplicate for the same
    ///   unique id. The job is on the queue either way; only the uniqueness
    ///   claim is unproven. Handlers are already required to tolerate
    ///   redelivery, which is what makes `true` the honest answer here -
    ///   the alternative, `false`, would tell the caller a job that is
    ///   about to run was never queued.
    /// - `Ok(false)` - a live dedupe key already existed for this
    ///   `(job_name, unique_id)`, so nothing was pushed.
    ///
    /// Backed by [`Idempotency::commit_on_success`](crate::idempotency::Idempotency::commit_on_success):
    /// a push failure releases the dedupe key so the caller can retry; a
    /// successful push holds the key for `unique_for` to gate re-submissions.
    ///
    /// Requires the cache layer to be bootstrapped (the dedupe lock lives
    /// in [`Cache`](crate::cache::Cache)). Returns an internal error if
    /// `J::unique_id(&job)` returns `None`.
    /// Honors [`Job::after_commit`] too, with one asymmetry that matters: the
    /// dedupe lock is taken **now**, so a second `push_unique` inside the same
    /// transaction is still suppressed, and only the envelope waits for the
    /// commit. A rollback releases that lock owner-scoped.
    pub async fn push_unique<J: Job>(job: J) -> Result<bool, FrameworkError> {
        Self::push_unique_at::<J>(job, AvailableAt::FromJobDelay).await
    }

    /// `push_unique` variant that schedules the envelope for delivery at
    /// `available_at` (combines with the configured driver's delayed-job
    /// strategy: ZSET on Redis, `available_at` column on the database
    /// driver, virtual-clock DelayQueue on the memory driver).
    pub async fn push_unique_later<J: Job>(
        job: J,
        available_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, FrameworkError> {
        Self::push_unique_at::<J>(job, AvailableAt::Fixed(available_at)).await
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
        Self::push_unique_at::<J>(job, AvailableAt::Fixed(available_at)).await
    }

    /// Common path for the three `*_unique*` entrypoints - builds the
    /// dedupe key, runs the enqueue under `Idempotency::commit_on_success`,
    /// and reports `true` for `Fresh` and `FreshUnfenced` (the envelope
    /// reached the driver either way), `false` only for `Duplicate`.
    async fn push_unique_at<J: Job>(job: J, when: AvailableAt) -> Result<bool, FrameworkError> {
        // The conflict is in the declarations, not in which entry point was
        // called: a job reaching the queue through `push_unique` must not have
        // its declared window quietly demoted to nothing. Above the fake for
        // the reason `dispatch_push` gives.
        if J::debounce_for().is_some() && job.unique_id().is_some() {
            return Err(debounce_conflict(J::job_name()));
        }
        if testing::is_active() {
            // In fake mode, dedupe is irrelevant - record and report fresh.
            testing::record::<J>(&job, when.resolve::<J>()?)?;
            return Ok(true);
        }
        let id = job.unique_id().ok_or_else(|| {
            FrameworkError::internal(
                "Queue::push_unique requires Job::unique_id(&self) to return Some(...)",
            )
        })?;
        let ttl = J::unique_for();
        let key = unique_key(J::job_name(), &id);
        // The closure below takes `id` by value to stamp the envelope's
        // idempotency key, so keep a copy for the event payload.
        let unique_id = id.clone();
        // Read before the lock is taken so the decision is made on the task
        // that owns the ambient transaction; `commit_on_success_owned` runs the
        // body on this same task, but reading it once keeps that an
        // implementation detail rather than a dependency.
        let defer = J::after_commit() && crate::database::after_commit::in_transaction();
        let deferred_key = key.clone();

        // `commit_on_success_owned` rather than `commit_on_success`: the owner
        // token of the lock we are holding right now has to reach the envelope,
        // because for a `unique_until_processing` job the worker - a different
        // task, possibly a different process - is what releases it.
        let (outcome, _owner) =
            crate::idempotency::Idempotency::commit_on_success_owned(&key, ttl, move |owner| {
                // Converted outside the async block so the future borrows
                // nothing from `owner`, which the higher-ranked closure bound
                // forbids.
                let owner_token = owner.map(str::to_owned);
                async move {
                    if defer {
                        // The lock stays taken through the transaction: dedupe
                        // has to work for a second dispatch inside the same
                        // transaction, so only the envelope waits.
                        return Self::defer_unique_push::<J>(
                            job,
                            when,
                            id,
                            owner_token,
                            deferred_key,
                        )
                        .await;
                    }
                    let mut env = envelope_for::<J>(&job, when.resolve::<J>()?)?;
                    env.idempotency_key = Some(id);
                    env.unique_lock_owner = owner_token;
                    let drv = current_driver()?;
                    drv.push(env).await
                }
            })
            .await?;

        // Exhaustive on purpose. `matches!(outcome, Fresh(()))` collapsed
        // `FreshUnfenced` - the body ran, the envelope IS on the queue, only
        // the dedupe lease was lost - into `false`, which this function
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

    /// Hold the dedupe lock this call just took, and move the envelope itself
    /// into the surrounding transaction's commit.
    ///
    /// Two callbacks go on the transaction, and the pair is the whole point:
    /// the commit one publishes the envelope, and the rollback one hands the
    /// lock back. Without the second, a dispatch that never happened would
    /// keep blocking re-dispatch for the rest of `unique_for`.
    ///
    /// The release is owner-scoped ([`Idempotency::release_owned`](crate::idempotency::Idempotency::release_owned)):
    /// there is no release-by-key and no force-release anywhere in the
    /// framework, because a forced release can delete a lock a newer dispatch
    /// now holds.
    async fn defer_unique_push<J: Job>(
        job: J,
        when: AvailableAt,
        unique_id: String,
        owner: Option<String>,
        lock_key: String,
    ) -> Result<(), FrameworkError> {
        if let Some(owner) = owner.clone() {
            let key = lock_key.clone();
            crate::database::after_commit::register_rollback_callback(Box::new(move || {
                Box::pin(async move {
                    crate::idempotency::Idempotency::release_owned(&key, &owner).await?;
                    Ok(())
                })
            }))
            .await?;
        }
        crate::database::after_commit::register_callback(Box::new(move || {
            Box::pin(async move {
                let mut env = envelope_for::<J>(&job, when.resolve::<J>()?)?;
                env.idempotency_key = Some(unique_id);
                env.unique_lock_owner = owner.clone();
                let result = async {
                    let drv = current_driver()?;
                    drv.push(env).await
                }
                .await;
                if let Err(e) = result {
                    // The dedupe key gates re-submission of a dispatch that
                    // happened; this one did not. Same rule
                    // `commit_on_success` applies when its body fails, just one
                    // commit later - and the error still surfaces, so the
                    // release is a cleanup, not a recovery.
                    if let Some(owner) = owner
                        && let Err(release_err) =
                            crate::idempotency::Idempotency::release_owned(&lock_key, &owner).await
                    {
                        tracing::warn!(
                            error = %release_err,
                            "after-commit unique push failed and its dedupe lock could not \
                             be released; re-dispatch is blocked until the lock expires"
                        );
                    }
                    return Err(e);
                }
                Ok(())
            })
        }))
        .await
    }

    /// Push every job in `jobs` onto the queue. Mirrors Laravel's
    /// `Queue::bulk($jobs, $data, $queue)`. Each job is encoded and
    /// committed via the driver's [`QueueDriver::bulk_push`] hook (with a
    /// serial-push default).
    ///
    /// Honors [`Job::delay`], resolved once for the whole call: every
    /// element of `jobs` shares the same concrete `J`, so they share the
    /// same declared delay.
    ///
    /// Honors [`Job::after_commit`] the same way, and for the same reason the
    /// partition is all-or-nothing: `jobs` is monomorphic, so one `J` decides
    /// for the whole batch. Laravel partitions a heterogeneous array here;
    /// Suprnova has nothing to partition.
    pub async fn bulk<J: Job + Clone>(jobs: Vec<J>) -> Result<(), FrameworkError> {
        if testing::is_active() {
            let available_at = resolve_job_delay::<J>(Utc::now())?;
            for j in jobs {
                testing::record::<J>(&j, available_at)?;
            }
            return Ok(());
        }
        if J::after_commit() && crate::database::after_commit::in_transaction() {
            return crate::database::after_commit::register_callback(Box::new(move || {
                Box::pin(async move { Self::bulk_immediately::<J>(jobs).await })
            }))
            .await;
        }
        Self::bulk_immediately::<J>(jobs).await
    }

    /// Encode every job and hand the batch to the driver. Split out of
    /// [`Queue::bulk`] so the deferred path resolves `Job::delay` against the
    /// commit rather than against the push, exactly as a single deferred push
    /// does.
    async fn bulk_immediately<J: Job + Clone>(jobs: Vec<J>) -> Result<(), FrameworkError> {
        let available_at = resolve_job_delay::<J>(Utc::now())?;
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

    /// Every envelope whose `available_at <= now` and which is not
    /// currently reserved, optionally filtered to one `queue`. Mirrors
    /// Laravel's `Queue::pendingJobs($queue)`; `queue: None` collapses that
    /// with the separate `allPendingJobs()` into one call. See
    /// [`QueueDriver::pending_jobs`] for the trait's error-default
    /// contract.
    pub async fn pending_jobs(queue: Option<&str>) -> Result<Vec<InspectedJob>, FrameworkError> {
        current_driver()?.pending_jobs(queue).await
    }

    /// Every envelope whose `available_at > now`, optionally filtered to
    /// one `queue`. Mirrors Laravel's `Queue::delayedJobs($queue)` /
    /// `allDelayedJobs()`.
    pub async fn delayed_jobs(queue: Option<&str>) -> Result<Vec<InspectedJob>, FrameworkError> {
        current_driver()?.delayed_jobs(queue).await
    }

    /// Every envelope currently held by an unfinished reservation,
    /// optionally filtered to one `queue`. Mirrors Laravel's
    /// `Queue::reservedJobs($queue)` / `allReservedJobs()`.
    pub async fn reserved_jobs(queue: Option<&str>) -> Result<Vec<InspectedJob>, FrameworkError> {
        current_driver()?.reserved_jobs(queue).await
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

    /// Pause job processing for one queue on one connection. Mirrors
    /// Laravel's `Queue::pause($connection, $queue)`.
    ///
    /// Backed by [`Cache::forever`](crate::cache::Cache::forever) - the
    /// same cache-backed worker-control-signal shape as [`Queue::restart`].
    /// Dispatches [`events::QueuePaused`].
    ///
    /// A worker only honors this when it was started with an explicit
    /// `--queue=...` list that names `queue` - see
    /// [`WorkerConfig`](crate::queue::worker::WorkerConfig) and the
    /// "Pausing queues" section of the queue manual chapter for why an
    /// unfiltered worker cannot apply a per-queue pause.
    pub async fn pause(connection: &str, queue: &str) -> Result<(), FrameworkError> {
        crate::cache::Cache::forever(&queue_pause_key(connection, queue), &true).await?;
        let _ = crate::events::EventFacade::dispatch(events::QueuePaused {
            connection: connection.to_string(),
            queue: queue.to_string(),
        })
        .await;
        Ok(())
    }

    /// Resume one queue previously paused with [`Queue::pause`]. Mirrors
    /// Laravel's `Queue::resume($connection, $queue)`. Idempotent -
    /// resuming a queue that isn't paused is not an error. Dispatches
    /// [`events::QueueResumed`].
    pub async fn resume(connection: &str, queue: &str) -> Result<(), FrameworkError> {
        crate::cache::Cache::forget(&queue_pause_key(connection, queue)).await?;
        let _ = crate::events::EventFacade::dispatch(events::QueueResumed {
            connection: connection.to_string(),
            queue: queue.to_string(),
        })
        .await;
        Ok(())
    }

    /// Pause job processing for every queue on every connection. Mirrors
    /// Laravel's `Queue::pauseAll()`. The worker gate checks this before
    /// any per-queue key and short-circuits every `--queue=...` filter,
    /// exactly like Laravel's `pausedQueues`. Dispatches
    /// [`events::QueuesPaused`].
    pub async fn pause_all() -> Result<(), FrameworkError> {
        crate::cache::Cache::forever(GLOBAL_QUEUE_PAUSE_KEY, &true).await?;
        let _ = crate::events::EventFacade::dispatch(events::QueuesPaused).await;
        Ok(())
    }

    /// Clear the global pause set by [`Queue::pause_all`]. Mirrors
    /// Laravel's `Queue::resumeAll()`.
    ///
    /// **Does not clear a per-queue pause set by [`Queue::pause`].** A
    /// queue paused individually stays paused after a global resume - this
    /// is Laravel's own semantics (`QueueManager::resumeAll` only forgets
    /// the global key), kept here so the two pause dimensions stay
    /// independently controllable: an operator who paused `billing` on
    /// purpose should not have it silently reopened by an unrelated
    /// "resume everything" call. Dispatches [`events::QueuesResumed`].
    pub async fn resume_all() -> Result<(), FrameworkError> {
        crate::cache::Cache::forget(GLOBAL_QUEUE_PAUSE_KEY).await?;
        let _ = crate::events::EventFacade::dispatch(events::QueuesResumed).await;
        Ok(())
    }

    /// True if `queue` on `connection` is paused - either individually via
    /// [`Queue::pause`], or because [`Queue::pause_all`] paused everything.
    /// Mirrors Laravel's `Queue::isPaused($connection, $queue)`.
    pub async fn is_paused(connection: &str, queue: &str) -> Result<bool, FrameworkError> {
        if is_globally_paused().await? {
            return Ok(true);
        }
        Ok(
            crate::cache::Cache::get::<bool>(&queue_pause_key(connection, queue))
                .await?
                .unwrap_or(false),
        )
    }

    /// Which of `queues` are currently paused on `connection`, in the same
    /// order they were given. Mirrors Laravel's
    /// `Queue::getPausedQueues($connection, $queues)`. When the global
    /// switch is set, every entry in `queues` comes back paused.
    pub async fn paused_queues(
        connection: &str,
        queues: &[String],
    ) -> Result<Vec<String>, FrameworkError> {
        if is_globally_paused().await? {
            return Ok(queues.to_vec());
        }
        let mut paused = Vec::with_capacity(queues.len());
        for queue in queues {
            if crate::cache::Cache::get::<bool>(&queue_pause_key(connection, queue))
                .await?
                .unwrap_or(false)
            {
                paused.push(queue.clone());
            }
        }
        Ok(paused)
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
    /// `attempts`, `available_at`, `idempotency_key`, and
    /// `unique_lock_owner`, pushes it through the configured driver, then
    /// deletes the failed-job record. Mirrors `php artisan queue:retry <id>`.
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
        env.unique_lock_owner = None;
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
            env.unique_lock_owner = None;
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
        // the boot path for every future caller - matches the framework's
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

/// The global pause switch's current state. A free function (not a `Queue`
/// method) because both [`Queue::is_paused`] / [`Queue::paused_queues`]
/// and the worker's pause gate need it without a `queue` argument - there
/// is nothing left to filter once the answer is already "everything is
/// paused." Propagates a cache error faithfully; callers on the fail-open
/// path (the worker gate) fold it with `.unwrap_or(false)` themselves.
pub(crate) async fn is_globally_paused() -> Result<bool, FrameworkError> {
    Ok(crate::cache::Cache::get::<bool>(GLOBAL_QUEUE_PAUSE_KEY)
        .await?
        .unwrap_or(false))
}

/// Whether queue workers, and the `queue:pause` command, honor pause
/// signals at all. Mirrors Laravel's `Worker::$pausable`. Reads
/// `QUEUE_PAUSABLE` fresh - unset, or anything other than `"false"` /
/// `"0"`, means enabled. `queue:resume` never checks this: disabling the
/// ability to *create* a pause must not also disable the ability to
/// *clear* one.
pub(crate) fn pausable_from_env() -> bool {
    !matches!(
        std::env::var("QUEUE_PAUSABLE").as_deref(),
        Ok("false") | Ok("0")
    )
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
/// `QUEUE_DRIVER=failover` additionally reads `QUEUE_FAILOVER_CONNECTIONS` (a
/// comma-separated, priority-ordered list such as `redis,database`) and wires a
/// [`FailoverQueueDriver`] over one inner driver per entry - see the "Failover
/// connections" section of the queue manual chapter.
///
/// Unlike [`bootstrap_default`], this call **always replaces** the registered
/// driver - long-running processes (workers, tests) that re-invoke
/// `bootstrap_from_env` after `QUEUE_DRIVER` changes (or after an earlier
/// Redis/database boot) will pick up the new driver instead of being pinned to
/// the first one installed.
pub async fn bootstrap_from_env() -> Result<(), FrameworkError> {
    let requested = std::env::var("QUEUE_DRIVER").unwrap_or_else(|_| "memory".into());
    let driver = match requested.as_str() {
        "failover" => build_failover_from_env().await?,
        // `None` is an unrecognized *name*, a typo this call absorbs exactly as
        // it always has. An `Err` is a name it does recognize whose backend will
        // not come up, which is a real boot failure and propagates. Keeping the
        // two apart in the type is what stops the recognized-name list from
        // existing in two places.
        other => match build_driver_from_env(other).await? {
            Some(driver) => driver,
            None => {
                tracing::warn!(driver = %other, "unknown QUEUE_DRIVER, falling back to memory");
                Arc::new(memory::MemoryQueueDriver::new()) as Arc<dyn QueueDriver>
            }
        },
    };
    Queue::set_driver(driver);
    Ok(())
}

/// Build one queue driver by connection name, reading that driver's own env.
///
/// Split out of [`bootstrap_from_env`] because a failover connection needs to
/// build several of these from one boot, and every inner connection must be
/// configured exactly the way it would be if it were `QUEUE_DRIVER` on its own.
///
/// Returns `Ok(None)` for a name this build does not recognize, leaving each
/// caller to decide what an unrecognized name means: `bootstrap_from_env` warns
/// and falls back to memory, exactly as it always has, while
/// `build_failover_from_env` rejects it. That distinction has to exist -
/// inside `QUEUE_FAILOVER_CONNECTIONS` a typo that quietly became an in-memory
/// connection would splice an ephemeral backend into a durable chain - and
/// carrying it in the return type keeps the list of recognized names in exactly
/// one place, here.
///
/// An `Err`, by contrast, always means a recognized backend that would not come
/// up. Every caller propagates that.
async fn build_driver_from_env(name: &str) -> Result<Option<Arc<dyn QueueDriver>>, FrameworkError> {
    match name {
        "memory" => Ok(Some(Arc::new(memory::MemoryQueueDriver::new()))),
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
            Ok(Some(Arc::new(d)))
        }
        "database" => {
            let table = std::env::var("QUEUE_DB_TABLE").unwrap_or_else(|_| "jobs".into());
            // Requires DB::init() (or DB::init_with(...)) to have been called first.
            let db = crate::database::DB::connection().map_err(|e| {
                FrameworkError::internal(format!(
                    "the `database` queue connection requires DB::init() to run first: {e}"
                ))
            })?;
            // DatabaseConnection is Arc-backed (SeaORM pool), so clone is cheap.
            // `new` validates QUEUE_DB_TABLE as a SQL identifier - a malformed
            // env value fails here instead of reaching SQL composition.
            let driver = database::DatabaseQueueDriver::new(db.inner().clone(), table)?;

            // The `failed_jobs` table is part of this driver's contract -
            // `queue:retry` reads it, and `Queue::retry_failed` fails
            // without it. Binding the driver and leaving the failed-jobs
            // store unset meant a database-backed queue dead-lettered into
            // nothing unless the app wired one by hand, which nothing in
            // the scaffold or the docs prompted anyone to do.
            //
            // Only for this driver. `memory` is ephemeral by construction,
            // and `redis` has no table to write to, so inventing a
            // database dependency for either would be worse than the gap.
            // A failover chain that lists `database` anywhere therefore still
            // gets a durable dead-letter store, which is the right outcome:
            // the store is bound to the database, not to the queue's rank in
            // the chain.
            let failed_table =
                std::env::var("QUEUE_FAILED_DB_TABLE").unwrap_or_else(|_| "failed_jobs".into());
            match failed::DatabaseFailedJobStore::new(db.inner().clone(), failed_table) {
                Ok(store) => Queue::set_failed_store(Arc::new(store)),
                Err(e) => {
                    // A malformed table name is a misconfiguration, not a
                    // reason to take the queue down - the worker now logs
                    // the whole envelope when no store is bound, so
                    // failures stay recoverable either way.
                    tracing::error!(
                        error = %e,
                        "QUEUE_FAILED_DB_TABLE is not a valid identifier; dead-lettered \
                         jobs will be logged rather than persisted"
                    );
                }
            }
            Ok(Some(Arc::new(driver)))
        }
        _ => Ok(None),
    }
}

/// Wire a [`FailoverQueueDriver`] from `QUEUE_FAILOVER_CONNECTIONS`.
///
/// Every entry is built by [`build_driver_from_env`], so an inner connection
/// is configured exactly as it would be were it `QUEUE_DRIVER` alone - the
/// `database` entry still needs `DB::init()` first, and still brings its
/// failed-jobs store with it.
async fn build_failover_from_env() -> Result<Arc<dyn QueueDriver>, FrameworkError> {
    // A blank value is treated as missing: `QUEUE_FAILOVER_CONNECTIONS=` in a
    // `.env` is a half-finished edit, not a request for a queue with nowhere
    // to push.
    let list = std::env::var("QUEUE_FAILOVER_CONNECTIONS")
        .ok()
        .filter(|raw| !raw.trim().is_empty())
        .ok_or_else(|| {
            FrameworkError::internal(
                "QUEUE_DRIVER=failover requires QUEUE_FAILOVER_CONNECTIONS \
                 (comma-separated, e.g. `redis,database`)",
            )
        })?;

    let mut drivers: Vec<(String, Arc<dyn QueueDriver>)> = Vec::new();
    for name in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if name == "failover" {
            // Nesting would let a chain reference itself and recurse at boot,
            // and it buys nothing a flat list cannot express.
            return Err(FrameworkError::internal(
                "QUEUE_FAILOVER_CONNECTIONS must not contain `failover` (no nesting)",
            ));
        }
        let driver = build_driver_from_env(name).await?.ok_or_else(|| {
            FrameworkError::internal(format!(
                "QUEUE_FAILOVER_CONNECTIONS names unknown queue connection `{name}`; \
                 expected one of memory, redis, database"
            ))
        })?;
        drivers.push((name.to_string(), driver));
    }
    Ok(Arc::new(FailoverQueueDriver::new(drivers)?))
}

/// How a push decides its `available_at`, carried far enough down the call
/// chain that a deferred push can decide it again at commit time.
///
/// This exists because the two entry-point families mean different things by a
/// delay, and after-commit dispatch makes the difference observable:
///
/// - [`Queue::push`] / [`Queue::push_with`] apply [`Job::delay`], which reads
///   "wait this long after dispatch". Deferred, dispatch is the commit, so the
///   delay is re-resolved then - a job with a five-minute delay is available
///   five minutes after the commit, not five minutes after a `push` that a
///   long transaction then sat on.
/// - [`Queue::push_later`] / [`Queue::later`] / [`Queue::later_with`] carry an
///   absolute timestamp the caller computed. That is the caller's intent about
///   a moment in time, so the deferral preserves it exactly.
#[derive(Clone, Copy)]
enum AvailableAt {
    /// Resolve [`Job::delay`] against the moment the push actually happens.
    FromJobDelay,
    /// Use the caller's timestamp verbatim.
    Fixed(chrono::DateTime<chrono::Utc>),
}

impl AvailableAt {
    fn resolve<J: Job>(&self) -> Result<chrono::DateTime<chrono::Utc>, FrameworkError> {
        match self {
            Self::FromJobDelay => resolve_job_delay::<J>(Utc::now()),
            Self::Fixed(at) => Ok(*at),
        }
    }
}

/// Resolve `available_at` for the entry points that consult
/// [`Job::delay`]: [`Queue::push`], [`Queue::push_with`], [`Queue::bulk`],
/// and [`Queue::push_unique`]. Returns `base` unchanged when the job
/// declares no delay.
///
/// `push_later` / `later` / `later_with` / [`Queue::push_unique_later`] /
/// [`Queue::later_unique`] never reach it - they take an explicit
/// `available_at` (or delay) from the caller, and that always wins over the
/// job's own declared default.
///
/// Which of the two applies is carried as an [`AvailableAt`] rather than a
/// resolved timestamp, so a push deferred to a transaction commit can resolve
/// the delay against the commit while an explicit timestamp survives the
/// deferral unchanged.
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

/// The idempotency key a `push_unique` dispatch takes its dedupe lock under.
///
/// Shared by the push side ([`Queue::push_unique`]) and the worker side (the
/// `unique_until_processing` release), because those two are the only places
/// that address this key and a drift between them is invisible: the release
/// would report "nothing to release" and the lock would linger for its full
/// TTL.
pub(crate) fn unique_key(job_name: &str, id: &str) -> String {
    format!("queue-unique:{job_name}:{id}")
}

/// The cache key a debounced dispatch takes its window under.
///
/// Shared by the push side and the worker side, because those two are the only
/// places that address this key and a drift between them is invisible: the
/// worker would find no owner, fail open, and run every envelope in the burst.
pub(crate) fn debounce_key(job_name: &str, id: Option<&str>) -> String {
    format!("queue-debounce:{job_name}:{}", id.unwrap_or(""))
}

/// The refusal both push families raise for a job declaring debouncing *and*
/// uniqueness.
///
/// Laravel throws a `LogicException` here; house rule 2 says public-surface
/// code returns `Result`. Shared so the two entry points cannot drift into
/// naming different things as the problem.
fn debounce_conflict(job_name: &str) -> FrameworkError {
    FrameworkError::internal(format!(
        "job `{job_name}` declares both debounce_for() and unique_id(): debouncing keeps \
         the last dispatch of a burst while uniqueness keeps the first, so the two \
         cannot both apply. Drop one."
    ))
}

/// Arm the debounce window for a push, stamp the envelope, and report the
/// moment the window asks the envelope to become available at.
///
/// `Ok(None)` means the job is not debounced and the caller's `available_at`
/// stands. `Ok(Some(ts))` is the timestamp the envelope should become available
/// at, which callers that took an explicit `available_at` from the user ignore -
/// Laravel guards its own delay assignment with `is_null($this->job->delay)`,
/// and an explicitly requested delay is a stronger statement than a declared
/// window.
async fn arm_debounce<J: Job>(
    job: &J,
    env: &mut Envelope,
    options: Option<&debounce::DebounceOptions>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, FrameworkError> {
    let (window, max_wait, id) = match options {
        Some(o) => (o.window, o.max_wait, o.id.clone()),
        None => match J::debounce_for() {
            Some(window) => (window, J::max_debounce_wait(), job.debounce_id()),
            None => return Ok(None),
        },
    };
    // Uniqueness keeps the first dispatch and suppresses the rest; debouncing
    // keeps the last and drops the rest. A job declaring both has no coherent
    // reading, and Laravel raises here too (`PendingDispatch::acquireDebounceLock`).
    if job.unique_id().is_some() {
        return Err(debounce_conflict(J::job_name()));
    }
    let key = debounce_key(J::job_name(), id.as_deref());
    // Converted before the window is armed, not after. It depends only on
    // `window`, and failing this conversion below `acquire` would leave an
    // owner token in the cache with no envelope behind it - which is the one
    // way a debounce can silently discard work.
    let window_delay = chrono::Duration::from_std(window)
        .map_err(|e| FrameworkError::internal(format!("debounce window overflow: {e}")))?;
    let armed = debounce::acquire(&key, window, max_wait).await?;
    env.debounce_id = id;
    env.debounce_owner = Some(armed.owner);
    let delay = if armed.max_wait_exceeded {
        // The burst has been deferring this long enough; queue it immediately.
        chrono::Duration::zero()
    } else {
        window_delay
    };
    Ok(Some(Utc::now() + delay))
}

fn envelope_for<J: Job>(
    job: &J,
    available_at: chrono::DateTime<chrono::Utc>,
) -> Result<Envelope, FrameworkError> {
    build_envelope::<J>(job, available_at)
}

/// Overlay `overrides` onto an already-resolved envelope, after
/// `envelope_for` - see [`EnvelopeOverrides`]. No schema change: every
/// touched field already exists on the frozen envelope.
///
/// `connection` is the **process** connection name, not the one this push
/// resolved to: an explicit queue override replaces the name `build_envelope`
/// already forwarded, so the override has to be forwarded in its place, and it
/// has to be gated on the same value every other half of the redirect uses.
/// Forwarding the already-forwarded envelope instead would make forwards
/// transitive, which they are not.
fn apply_overrides(env: &mut Envelope, overrides: &EnvelopeOverrides, connection: &str) {
    if let Some(queue) = &overrides.queue {
        env.queue = routing::forwarded_queue(Some(queue.as_str()), connection);
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
    // Routing decides the name; the forwards map then redirects it, exactly
    // where Laravel's driver-level `getQueue()` calls `resolveQueue()`.
    //
    // The gate is the *process* connection name, never the one routing or the
    // job resolved. The worker has only `Queue::connection_name()` to gate its
    // claim list on, so any other value here would let `forward_on` move one
    // half of the pair and strand work on the other. That is sound because the
    // connection dimension is a label rather than a driver selector (see the
    // `routing` module docs). Gated on `has_forwards` so a deployment that
    // never forwards does not pay the lookup on every push.
    let mut queue = routing::resolve_queue::<J>();
    if routing::has_forwards() {
        queue = routing::forwarded_queue(queue.as_deref(), &Queue::connection_name());
    }
    Ok(Envelope {
        schema_version: CURRENT_SCHEMA_VERSION,
        id: Uuid::new_v4(),
        job_name: J::job_name().to_string(),
        queue,
        payload,
        dispatched_at: Utc::now(),
        available_at,
        attempts: 0,
        max_tries: J::max_tries(),
        backoff: J::backoff(),
        timeout_secs,
        fail_on_timeout: J::fail_on_timeout(),
        idempotency_key: None,
        unique_lock_owner: None,
        debounce_id: None,
        debounce_owner: None,
        batch_id: None,
        chain_remaining: Vec::new(),
    })
}
