//! Scheduled task trait and entry types
//!
//! This module defines the `Task` trait for creating struct-based
//! scheduled tasks, as well as internal types for task management.

use super::expression::CronExpression;
use crate::error::FrameworkError;
use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::time::Duration;

/// Default overlap-lock TTL: 30 minutes. Long enough that most scheduled
/// jobs finish well before it expires, short enough that a crashed task
/// holding an in-flight lock unblocks the next tick without operator
/// intervention. Override per task with
/// [`super::TaskBuilder::without_overlapping_for`].
pub const DEFAULT_WITHOUT_OVERLAPPING_TTL: Duration = Duration::from_secs(30 * 60);

/// Default single-server lock TTL: 60 seconds - exactly one minute-aligned
/// tick.
///
/// The window has two edges and both matter. Too short and a replica whose
/// clock or tick lands a few seconds late finds the lock already gone and
/// runs the task a second time, which is the defect this prevents. Too
/// long and the lock outlives its tick, so the *next* due run finds it
/// held and is skipped entirely. One tick is the only value that is right
/// for the `* * * * *` default; coarser schedules should say so with
/// [`super::TaskBuilder::on_one_server_for`].
pub const DEFAULT_ON_ONE_SERVER_TTL: Duration = Duration::from_secs(60);

/// Per-task runtime state shared between schedule entries and any spawned
/// background futures derived from them.
///
/// Holds counters needed to enforce [`TaskBuilder::without_overlapping`] in
/// the absence of a distributed [`Cache`] lock. Wrap in `Arc` so the same
/// instance is observed by the inline call path and any `tokio::spawn`
/// children - they need a shared view of whether a previous run is still
/// in flight.
///
/// [`TaskBuilder::without_overlapping`]: super::TaskBuilder::without_overlapping
/// [`Cache`]: crate::cache::Cache
#[derive(Default)]
pub struct TaskState {
    /// In-process running flag flipped via CAS when a task enters
    /// [`super::TaskEntry::run`] under `without_overlapping = true` without a
    /// usable [`Cache`] lock. Reset on completion regardless of result.
    ///
    /// [`Cache`]: crate::cache::Cache
    pub(crate) in_process_running: AtomicBool,
    /// Number of times this task has been observed and skipped due to an
    /// overlap lock (Cache-side or in-process) **or** because the
    /// same-minute dedup CAS rejected a repeat invocation. Read via
    /// [`TaskState::skip_count`] - the field stays `pub(crate)` so the
    /// atomic implementation can change without breaking external code.
    pub(crate) skip_count: AtomicUsize,
    /// Minutes-since-UNIX-epoch of the most recent invocation attempt.
    /// `fetch_max` against the current minute is the same-minute dedup
    /// gate - if the prior value is `>= now`, we already tried this minute
    /// and the new call must skip. Init to `0`: any post-epoch run wins
    /// the first CAS unconditionally.
    pub(crate) last_run_minute: AtomicI64,
}

impl TaskState {
    /// Build a fresh, idle [`TaskState`] wrapped in `Arc` so the builder
    /// can clone it into both the [`TaskEntry`] and any spawned background
    /// future.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Snapshot the skip counter - convenient for tests that need to assert
    /// "this task was skipped N times" without unwrapping atomics.
    pub fn skip_count(&self) -> usize {
        self.skip_count.load(Ordering::SeqCst)
    }
}

/// Type alias for boxed task handlers
pub type BoxedTask = Arc<dyn TaskHandler + Send + Sync>;

/// Type alias for async task result
pub type TaskResult = Result<(), FrameworkError>;

/// Type alias for boxed future result
pub type BoxedFuture<'a> = Pin<Box<dyn Future<Output = TaskResult> + Send + 'a>>;

/// Internal trait for task execution
///
/// This trait is implemented automatically for `Task` and closure-based tasks.
#[async_trait]
pub trait TaskHandler: Send + Sync {
    /// Execute the task
    async fn handle(&self) -> TaskResult;
}

/// Trait for defining scheduled tasks
///
/// Implement this trait on a struct to create a reusable scheduled task.
/// Schedule configuration is done via the fluent builder API when registering.
///
/// # Example
///
/// ```rust,no_run
/// use suprnova::{Task, TaskResult};
/// use async_trait::async_trait;
///
/// pub struct CleanupLogsTask;
///
/// impl CleanupLogsTask {
///     pub fn new() -> Self {
///         Self
///     }
/// }
///
/// #[async_trait]
/// impl Task for CleanupLogsTask {
///     async fn handle(&self) -> TaskResult {
///         // Cleanup logic here
///         println!("Cleaning up old log files...");
///         Ok(())
///     }
/// }
///
/// // Register in schedule.rs with fluent API:
/// // schedule.add(
/// //     schedule.task(CleanupLogsTask::new())
/// //         .daily()
/// //         .at("03:00")
/// //         .name("cleanup:logs")
/// // );
/// ```
#[async_trait]
pub trait Task: Send + Sync {
    /// Execute the task
    async fn handle(&self) -> TaskResult;
}

// Implement TaskHandler for any type implementing Task
#[async_trait]
impl<T: Task> TaskHandler for T {
    async fn handle(&self) -> TaskResult {
        Task::handle(self).await
    }
}

/// A registered task entry in the schedule
///
/// This struct holds all the information about a scheduled task,
/// including its schedule expression, configuration, and the task itself.
pub struct TaskEntry {
    /// Unique name for the task
    pub name: String,
    /// Cron expression defining when the task runs
    pub expression: CronExpression,
    /// The task handler
    pub task: BoxedTask,
    /// Optional description
    pub description: Option<String>,
    /// Prevent overlapping runs
    pub without_overlapping: bool,
    /// Run in background (non-blocking)
    pub run_in_background: bool,
    /// TTL applied to the overlap lock when `without_overlapping` is set.
    /// Acts as a safety net for crashed tasks that fail to release the
    /// lock - the next tick after this duration sees a fresh lock and can
    /// proceed.
    pub overlap_ttl: Duration,
    /// Run on exactly one server per due tick - see
    /// [`super::TaskBuilder::on_one_server`].
    pub on_one_server: bool,
    /// TTL applied to the single-server election lock. Unlike
    /// `overlap_ttl` this is not a crash safety net: expiry is the *only*
    /// thing that releases the lock, because holding it past the handler
    /// is what makes a late replica lose the election.
    pub one_server_ttl: Duration,
    /// Shared runtime state - in-process overlap flag and skip counter.
    pub state: Arc<TaskState>,
}

impl TaskEntry {
    /// Check if this task is due to run now
    pub fn is_due(&self) -> bool {
        self.expression.is_due()
    }

    /// Run the task, honouring `without_overlapping` if it is set.
    ///
    /// When the flag is enabled the executor first tries a distributed
    /// [`Cache::lock`] (so multi-process deployments coordinate); when
    /// `Cache` is not bootstrapped at all (`FrameworkError::ServiceNotFound`)
    /// the executor degrades to a per-process `AtomicBool` CAS and emits a
    /// single warn-once telling the operator they're getting the weaker
    /// guarantee. A contended lock is treated as a successful skip - the
    /// task returns `Ok(())` and increments the [`TaskState`] skip counter
    /// so observability surfaces can see it without poisoning the
    /// `schedule:run` exit code.
    ///
    /// A *bootstrapped* `Cache` that fails to acquire the lock for another
    /// reason (a Redis connection blip, for example) is a different case
    /// and is **not** treated as "absent": falling back to the in-process
    /// flag there would let every replica run the task at once. That path
    /// fails closed - the task is skipped for this tick and the error is
    /// returned rather than swallowed.
    ///
    /// [`Cache::lock`]: crate::cache::Cache::lock
    pub async fn run(&self) -> TaskResult {
        run_handler_with_optional_overlap_guard(
            &self.name,
            Arc::clone(&self.task),
            self.without_overlapping,
            self.overlap_ttl,
            self.on_one_server,
            self.one_server_ttl,
            Arc::clone(&self.state),
        )
        .await
    }

    /// Get a human-readable description of the schedule
    pub fn schedule_description(&self) -> &str {
        self.expression.expression()
    }
}

/// Single warn-once latch for the "Cache not installed, falling back to
/// in-process overlap protection" message. Mirrors the precedent in
/// `features::middleware::warn_once_if_no_evaluator` so production logs
/// don't get flooded on every minute-aligned tick.
static CACHE_FALLBACK_WARNED: AtomicBool = AtomicBool::new(false);

fn warn_cache_fallback_once() {
    if !CACHE_FALLBACK_WARNED.swap(true, Ordering::SeqCst) {
        tracing::warn!(
            target: "suprnova::schedule",
            "without_overlapping() falling back to in-process AtomicBool protection - \
             Cache is not bootstrapped. Multi-process deployments (multiple `schedule:work` \
             or external-cron `schedule:run` callers) will NOT see each other's locks. \
             Configure Cache (CACHE_DRIVER=memory|redis) before relying on cross-process \
             overlap protection."
        );
    }
}

/// RAII guard that clears [`TaskState::in_process_running`] on drop -
/// including when the guarded handler panics.
///
/// The in-process fallback used to clear the flag with a plain
/// `.store(false, ...)` placed *after* `handler.handle().await`. A
/// panicking handler unwinds straight past that line: the `catch_unwind`
/// boundaries that convert task panics into `Err(...)` live outside this
/// function (`schedule/mod.rs`'s background-spawn and inline paths), so by
/// the time a panic is caught, this function's stack has already unwound
/// and the flag update never ran. `in_process_running` has no TTL (unlike
/// the Redis lock it stands in for), so a leaked `true` value jams the task
/// for the rest of the process's life - every later tick sees the flag set
/// and skips forever. Binding a guard whose `Drop` clears the flag makes
/// the release run during unwinding too, the same way `AbortOnDrop` in
/// `workflow::mod` guarantees heartbeat cleanup on early return.
struct InProcessOverlapGuard<'a> {
    flag: &'a AtomicBool,
}

impl Drop for InProcessOverlapGuard<'_> {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

/// Single warn-once latch for the "single-server election is running on a
/// per-process cache" message, outside production where that is allowed.
static ONE_SERVER_MEMORY_WARNED: AtomicBool = AtomicBool::new(false);

fn warn_one_server_memory_once(task: &str) {
    if !ONE_SERVER_MEMORY_WARNED.swap(true, Ordering::SeqCst) {
        tracing::warn!(
            target: "suprnova::schedule",
            task = %task,
            "on_one_server() is holding a per-process lock - Cache is not \
             bootstrapped, so replicas cannot see each other's elections and \
             every replica will run this task. Bootstrap Cache with \
             CACHE_DRIVER=redis before relying on single-server execution. \
             (In production this is a boot failure, not a warning.)"
        );
    }
}

/// Win, or lose, the election to run `name` for tick `minute`.
///
/// Returns `true` when this process owns the tick and should run the
/// handler.
///
/// # Why the lock is never released
///
/// [`Cache::lock`] has no `Drop` auto-release, and this deliberately does
/// not call `release()`. The lock's job is not to bracket the handler - it
/// is to make a replica that arrives *later in the same tick* find the
/// tick already claimed. Releasing on completion would hand the tick to
/// the next replica to look, which is the whole defect. It expires on its
/// TTL, and that expiry is what frees the following tick.
///
/// This is the one place `without_overlapping` and `on_one_server` differ
/// in kind rather than degree, and it is why one cannot be built from the
/// other.
///
/// [`Cache::lock`]: crate::cache::Cache::lock
async fn claim_tick_for_this_server(
    name: &str,
    minute: i64,
    ttl: Duration,
    state: &Arc<TaskState>,
) -> bool {
    // The tick is in the key, not just the task name. Two replicas racing
    // the same minute contend on one key; the same task next minute is a
    // different key and contends with nobody.
    let key = format!("schedule:one-server:{name}:{minute}");
    match crate::cache::Cache::lock(&key, ttl).await {
        Ok(Some(_guard)) => {
            // Dropped without releasing, on purpose. See the doc above.
            true
        }
        Ok(None) => {
            tracing::info!(
                target: "suprnova::schedule",
                task = %name,
                tick = minute,
                "skipped: another server claimed this tick",
            );
            state.skip_count.fetch_add(1, Ordering::SeqCst);
            false
        }
        Err(FrameworkError::ServiceNotFound { .. }) => {
            // Cache is not bootstrapped at all. In production this never
            // reaches here - `Schedule::validate_single_server_locking`
            // fails the boot. Outside production, running is the useful
            // behaviour for a single-process dev loop; warn so nobody
            // mistakes it for the real guarantee.
            warn_one_server_memory_once(name);
            true
        }
        Err(err) => {
            // Cache is bootstrapped but the lock attempt failed - a Redis
            // blip, say. Fail CLOSED, matching `without_overlapping`'s
            // stance: running anyway would let every replica through at
            // exactly the moment coordination is unavailable, which is the
            // worst possible time to multiply a task's side effects. A
            // skipped tick is recoverable; duplicate billing is not.
            tracing::error!(
                target: "suprnova::schedule",
                task = %name,
                tick = minute,
                error = %err,
                "skipped: could not reach the cache to claim this tick; \
                 failing closed rather than risk every replica running it",
            );
            state.skip_count.fetch_add(1, Ordering::SeqCst);
            false
        }
    }
}

/// Shared implementation used by both [`TaskEntry::run`] (inline) and the
/// `tokio::spawn`'d background path in `schedule::run_tasks_into`. Pulled out
/// as a free function so the spawned `async move` future can capture the
/// `'static` arguments it needs without borrowing from `&TaskEntry`.
pub(crate) async fn run_handler_with_optional_overlap_guard(
    name: &str,
    handler: BoxedTask,
    without_overlapping: bool,
    overlap_ttl: Duration,
    on_one_server: bool,
    one_server_ttl: Duration,
    state: Arc<TaskState>,
) -> TaskResult {
    // Same-minute dedup (always on, regardless of `without_overlapping`).
    // `fetch_max` returns the previous value and atomically bumps the
    // stored value to the max of (prev, now). If the previous value was
    // already at-or-past `now_minute`, this minute has already been
    // claimed - skip silently with a tick to `skip_count`. The audit's
    // HIGH #3 case (a daemon loop or repeated `schedule:run` invocation
    // executing the same minute-level task multiple times) is closed at
    // this gate; cross-process protection is layered on by Cache::lock
    // inside the `without_overlapping` branch below.
    let now_minute = chrono::Local::now().timestamp() / 60;
    let prev_minute = state
        .last_run_minute
        .fetch_max(now_minute, Ordering::SeqCst);
    if prev_minute >= now_minute {
        tracing::info!(
            target: "suprnova::schedule",
            task = %name,
            "skipped: already attempted for minute {now_minute}",
        );
        state.skip_count.fetch_add(1, Ordering::SeqCst);
        return Ok(());
    }

    // Cross-replica election. The dedup above is an `AtomicI64` in *this*
    // process, so N replicas each claim the same minute for themselves and
    // all N run - measured as exactly that: three replicas, three
    // executions per minute, every minute.
    if on_one_server && !claim_tick_for_this_server(name, now_minute, one_server_ttl, &state).await
    {
        return Ok(());
    }

    if !without_overlapping {
        return handler.handle().await;
    }
    let lock_key = format!("schedule:lock:{name}");
    match crate::cache::Cache::lock(&lock_key, overlap_ttl).await {
        Ok(Some(guard)) => {
            let result = handler.handle().await;
            if let Err(e) = guard.release().await {
                tracing::warn!(
                    target: "suprnova::schedule",
                    error = %e,
                    "schedule: failed to release task lock; it will expire via TTL",
                );
            }
            result
        }
        Ok(None) => {
            tracing::info!(
                target: "suprnova::schedule",
                task = %name,
                "skipped: previous run still holds the overlap lock",
            );
            state.skip_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        Err(FrameworkError::ServiceNotFound { .. }) => {
            // Cache genuinely isn't bootstrapped (no `CacheStore` binding in
            // the container) - degrade to in-process CAS. Warn operator once
            // that they're getting the weaker, single-process guarantee.
            warn_cache_fallback_once();
            if state
                .in_process_running
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                // Guard released on drop - including on unwind if `handler`
                // panics - so the flag can never stick at `true` forever.
                let _guard = InProcessOverlapGuard {
                    flag: &state.in_process_running,
                };
                handler.handle().await
            } else {
                tracing::info!(
                    target: "suprnova::schedule",
                    task = %name,
                    "skipped: in-process overlap flag already set",
                );
                state.skip_count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }
        Err(err) => {
            // Cache IS bootstrapped but the lock acquisition itself failed
            // (e.g. a Redis connection blip returning
            // `FrameworkError::Internal` from `RedisCache::acquire_lock`).
            // This is NOT "cache absent" - silently degrading to the
            // in-process AtomicBool here would convert a cross-process lock
            // into N independent per-process flags, and every replica would
            // run the task concurrently, which is exactly what
            // `without_overlapping()` exists to prevent. Fail CLOSED
            // instead: skip this tick and surface the error so the failure
            // is visible, rather than risk duplicate side effects. A task
            // that doesn't run this tick is recoverable next tick;
            // duplicate side effects generally are not.
            tracing::error!(
                target: "suprnova::schedule",
                task = %name,
                error = %err,
                "without_overlapping: cache lock acquisition failed (not just absent) - \
                 skipping this run rather than risk every replica running it simultaneously",
            );
            Err(err)
        }
    }
}

/// Wrapper for closure-based tasks
pub(crate) struct ClosureTask<F>
where
    F: Fn() -> BoxedFuture<'static> + Send + Sync,
{
    pub(crate) handler: F,
}

#[async_trait]
impl<F> TaskHandler for ClosureTask<F>
where
    F: Fn() -> BoxedFuture<'static> + Send + Sync,
{
    async fn handle(&self) -> TaskResult {
        (self.handler)().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestTask;

    #[async_trait]
    impl Task for TestTask {
        async fn handle(&self) -> TaskResult {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_task_trait() {
        let task = TestTask;

        let result: TaskResult = Task::handle(&task).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_task_entry() {
        let task = TestTask;
        let entry = TaskEntry {
            name: "test-task".to_string(),
            expression: CronExpression::every_minute(),
            task: Arc::new(task),
            description: Some("A test task".to_string()),
            without_overlapping: false,
            run_in_background: false,
            overlap_ttl: DEFAULT_WITHOUT_OVERLAPPING_TTL,
            on_one_server: false,
            one_server_ttl: DEFAULT_ON_ONE_SERVER_TTL,
            state: TaskState::new(),
        };

        assert_eq!(entry.name, "test-task");
        assert_eq!(entry.schedule_description(), "* * * * *");

        let result = entry.run().await;
        assert!(result.is_ok());
    }

    // -------------------------------------------------------------------------
    // OPS-01#2 regression: fail closed on a genuine cache error, and the
    // in-process fallback flag must survive a panicking handler.
    // -------------------------------------------------------------------------

    /// A `CacheStore` whose `acquire_lock` always fails with a "real" backend
    /// error, while every other operation delegates to a working in-memory
    /// backend. Simulates a Redis connection blip on a *bootstrapped* cache -
    /// distinct from "no `CacheStore` binding at all", which is the only case
    /// that should degrade to the in-process fallback. Mirrors the
    /// `FailingReleaseCache` pattern in `framework/tests/idempotency.rs`.
    struct LockErroringCache(crate::cache::InMemoryCache, LockFailure);

    /// Which failure `LockErroringCache::acquire_lock` reports.
    ///
    /// `Absent` exists because the in-process fallback branch is only
    /// reachable via `ServiceNotFound`, and simply *not binding* a
    /// `CacheStore` does not produce it reliably: `TestContainer::fake()`
    /// layers an empty container above the global one and lookup falls
    /// through, so any test that binds a `CacheStore` globally makes
    /// "cache absent" unreachable for every test that runs after it in the
    /// same binary. Reporting the same error a missing binding would raise
    /// keeps the branch under test deterministic regardless of suite order.
    #[derive(Clone, Copy)]
    enum LockFailure {
        /// A bootstrapped cache whose backend blipped - must fail closed.
        Blip,
        /// No `CacheStore` bound - must degrade to the in-process guard.
        Absent,
    }

    #[async_trait]
    impl crate::cache::CacheStore for LockErroringCache {
        async fn get_raw(&self, key: &str) -> Result<Option<String>, FrameworkError> {
            self.0.get_raw(key).await
        }
        async fn put_raw(
            &self,
            key: &str,
            value: &str,
            ttl: Option<Duration>,
        ) -> Result<(), FrameworkError> {
            self.0.put_raw(key, value, ttl).await
        }
        async fn has(&self, key: &str) -> Result<bool, FrameworkError> {
            self.0.has(key).await
        }
        async fn forget(&self, key: &str) -> Result<bool, FrameworkError> {
            self.0.forget(key).await
        }
        async fn flush(&self) -> Result<(), FrameworkError> {
            self.0.flush().await
        }
        async fn increment(&self, key: &str, amount: i64) -> Result<i64, FrameworkError> {
            self.0.increment(key, amount).await
        }
        async fn decrement(&self, key: &str, amount: i64) -> Result<i64, FrameworkError> {
            self.0.decrement(key, amount).await
        }
        async fn tagged_put_raw(
            &self,
            tags: &[&str],
            key: &str,
            value: &str,
            ttl: Option<Duration>,
        ) -> Result<(), FrameworkError> {
            self.0.tagged_put_raw(tags, key, value, ttl).await
        }
        async fn flush_tags(&self, tags: &[&str]) -> Result<(), FrameworkError> {
            self.0.flush_tags(tags).await
        }
        async fn acquire_lock(
            &self,
            _key: &str,
            _ttl: Duration,
        ) -> Result<Option<String>, FrameworkError> {
            match self.1 {
                LockFailure::Blip => {
                    Err(FrameworkError::internal("synthetic Redis connection blip"))
                }
                LockFailure::Absent => Err(FrameworkError::service_not_found::<
                    dyn crate::cache::CacheStore,
                >()),
            }
        }
        async fn release_lock(&self, key: &str, token: &str) -> Result<bool, FrameworkError> {
            self.0.release_lock(key, token).await
        }
        async fn refresh_lock(
            &self,
            key: &str,
            token: &str,
            ttl: Duration,
        ) -> Result<bool, FrameworkError> {
            self.0.refresh_lock(key, token, ttl).await
        }
        async fn touch(&self, key: &str, ttl: Duration) -> Result<bool, FrameworkError> {
            self.0.touch(key, ttl).await
        }
    }

    struct CountingTask(Arc<AtomicUsize>);

    #[async_trait]
    impl Task for CountingTask {
        async fn handle(&self) -> TaskResult {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// A bootstrapped `Cache` whose lock acquisition errors (not "absent")
    /// must fail CLOSED: the handler must not run. Simulates 3 independent
    /// replicas - each with its own fresh `TaskState`, exactly as separate
    /// processes would have - sharing only the (broken) `Cache`. Before the
    /// fix, `Err(_)` unconditionally degraded to the in-process `AtomicBool`,
    /// which is per-process: every "replica" here would have happily run the
    /// handler concurrently, exactly what `without_overlapping()` exists to
    /// prevent.
    #[tokio::test]
    async fn without_overlapping_fails_closed_on_cache_error_not_absence() {
        use crate::cache::{CacheStore, InMemoryCache};
        use crate::testing::TestContainer;

        let _scope = TestContainer::fake();
        let store: Arc<dyn CacheStore> = Arc::new(LockErroringCache(
            InMemoryCache::with_prefix("ops01:"),
            LockFailure::Blip,
        ));
        TestContainer::bind::<dyn CacheStore>(store);

        let ran = Arc::new(AtomicUsize::new(0));

        for _ in 0..3 {
            let handler: BoxedTask = Arc::new(CountingTask(ran.clone()));
            // Fresh state per iteration - simulates independent replica
            // processes, which never share an in-process AtomicBool in
            // reality.
            let result = run_handler_with_optional_overlap_guard(
                "replica-task",
                handler,
                true,
                Duration::from_secs(30),
                false,
                DEFAULT_ON_ONE_SERVER_TTL,
                TaskState::new(),
            )
            .await;
            assert!(
                result.is_err(),
                "a cache lock error must fail closed and surface as Err, not silently degrade"
            );
        }

        assert_eq!(
            ran.load(Ordering::SeqCst),
            0,
            "handler must never run while the lock backend is erroring - running it would mean \
             every replica executed the task simultaneously"
        );
    }

    /// The in-process fallback flag must be released by RAII, not by a
    /// plain post-await `.store(false, ...)` - a panicking handler unwinds
    /// straight past that line. Before the fix this test would hang forever
    /// on the second call (the flag stuck at `true`, no TTL to self-heal),
    /// or the second `assert_eq!` would see `ran == 0`.
    #[tokio::test]
    async fn without_overlapping_in_process_flag_releases_after_handler_panics() {
        use crate::cache::{CacheStore, InMemoryCache};
        use crate::testing::TestContainer;

        // Bind a store that reports the cache as absent, rather than binding
        // nothing and hoping the global container is also empty. Both produce
        // the identical `Err(ServiceNotFound)` that routes through the
        // in-process AtomicBool fallback - the path this regression targets -
        // but only this one is deterministic: `TestContainer::fake()` layers
        // an empty container above the global one and lookup falls through,
        // so a `CacheStore` bound globally by any earlier test in this binary
        // would silently divert us onto the cache path instead. That is
        // exactly how this test passed alone and failed in the full suite.
        let _scope = TestContainer::fake();
        let store: Arc<dyn CacheStore> = Arc::new(LockErroringCache(
            InMemoryCache::with_prefix("ops01-absent:"),
            LockFailure::Absent,
        ));
        TestContainer::bind::<dyn CacheStore>(store);

        struct PanickingTask;
        #[async_trait]
        impl Task for PanickingTask {
            async fn handle(&self) -> TaskResult {
                panic!("intentional panic for RAII-guard regression test");
            }
        }

        let state = TaskState::new();
        let handler: BoxedTask = Arc::new(PanickingTask);

        // No catch_unwind at this layer by design (that boundary lives one
        // level up, in `schedule/mod.rs`) - spawn on a tokio task so the
        // panic is contained to that task instead of aborting the test
        // process, and assert it really was a panic.
        let join = tokio::spawn(run_handler_with_optional_overlap_guard(
            "panicky-overlap-task",
            handler,
            true,
            Duration::from_secs(30),
            false,
            DEFAULT_ON_ONE_SERVER_TTL,
            state.clone(),
        ));
        let outcome = join.await;
        assert!(
            outcome.is_err() && outcome.unwrap_err().is_panic(),
            "handler panic must propagate out of this layer uncaught"
        );

        assert!(
            !state.in_process_running.load(Ordering::SeqCst),
            "RAII guard must have cleared in_process_running while unwinding, even though the \
             plain store(false, ...) after the awaited call never ran"
        );

        // Next tick, same TaskState: must actually run, not be skipped
        // forever because the flag never reset. Reset the same-minute CAS
        // to simulate the minute rolling over, same as the other
        // without_overlapping regression tests in `schedule/mod.rs`.
        state.last_run_minute.store(0, Ordering::SeqCst);

        let ran = Arc::new(AtomicUsize::new(0));
        let handler2: BoxedTask = Arc::new(CountingTask(ran.clone()));
        let result = run_handler_with_optional_overlap_guard(
            "panicky-overlap-task",
            handler2,
            true,
            Duration::from_secs(30),
            false,
            DEFAULT_ON_ONE_SERVER_TTL,
            state,
        )
        .await;
        assert!(
            result.is_ok(),
            "the tick after a panic must run normally, not be skipped forever"
        );
        assert_eq!(
            ran.load(Ordering::SeqCst),
            1,
            "handler must actually execute - the overlap flag was not stuck at true"
        );
    }
}
