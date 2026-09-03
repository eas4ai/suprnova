//! Durable workflow engine
//!
//! Provides a Postgres-backed durable workflow system with step persistence
//! and automatic retries. Inspired by Laravel queues and DBOS.
//!
//! # Delivery semantics
//!
//! Step bodies run with **at-least-once** semantics. The framework
//! persists step outputs durably and replays from cache on retry, but
//! it cannot observe a step's side effects. A crash after a step's
//! external action but before `mark_step_succeeded` commits will cause
//! the step body to run again on the next claim. Treat every step body
//! as idempotent (conditional writes, idempotency keys to external
//! APIs, `INSERT ... ON CONFLICT DO NOTHING`, etc.). See
//! `docs/workflows.md` for patterns.
//!
//! # Example
//!
//! ```rust,ignore
//! use suprnova::{workflow, workflow_step, start_workflow, FrameworkError};
//!
//! #[workflow_step]
//! async fn fetch_user(user_id: i64) -> Result<String, FrameworkError> {
//!     Ok(format!("user:{}", user_id))
//! }
//!
//! #[workflow_step]
//! async fn send_email(user: String) -> Result<(), FrameworkError> {
//!     println!("Sending email to {}", user);
//!     Ok(())
//! }
//!
//! #[workflow]
//! async fn welcome_flow(user_id: i64) -> Result<(), FrameworkError> {
//!     let user = fetch_user(user_id).await?;
//!     send_email(user).await?;
//!     Ok(())
//! }
//!
//! // Enqueue a workflow
//! // let handle = start_workflow!(welcome_flow, 123).await?;
//! // handle.wait_with_timeout(Duration::from_secs(30)).await?;
//!
//! // Run worker (separate process):
//! // suprnova workflow:work
//! ```

pub mod config;
pub mod context;
pub mod entities;
pub mod migrations;
#[doc(hidden)]
pub mod registry;
pub mod store;
pub mod types;

pub use config::WorkflowConfig;
pub use context::WorkflowContext;
pub use types::{StepStatus, WorkflowHandle, WorkflowStatus};

use crate::config::Config;
use crate::error::FrameworkError;
use crate::workflow::types::ClaimedWorkflow;
use chrono::{Duration as ChronoDuration, Utc};
use futures::FutureExt;
use rand::RngExt;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

/// How long a cancelled workflow worker waits for in-flight steps before
/// aborting them.
///
/// Aborting is safe here in a way it is not everywhere: a claimed workflow
/// row carries a lease, so an abandoned step simply lets its lease lapse
/// and another worker reclaims it. That makes a bounded wait strictly
/// better than the unbounded one it replaces, where a step that never
/// returned held the worker open until SIGKILL - and SIGKILL leaves the
/// same lease to lapse anyway, just without the log line saying so.
const WORKFLOW_DRAIN_GRACE: Duration = Duration::from_secs(30);

/// Await every in-flight step until `grace` expires, then abort whatever
/// is left. Returns how many were aborted.
///
/// The post-abort `join_next` loop is not redundant: `abort_all` only
/// *requests* cancellation, and a task keeps running until it is polled
/// again. Returning without draining would let aborted steps continue past
/// the worker's own exit.
async fn drain_in_flight(in_flight: &mut JoinSet<()>, grace: Duration) -> usize {
    if in_flight.is_empty() {
        return 0;
    }
    let deadline = tokio::time::sleep(grace);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            joined = in_flight.join_next() => match joined {
                Some(Err(err)) if err.is_panic() => {
                    tracing::error!(
                        error = %err,
                        "workflow worker task panicked during drain"
                    );
                }
                Some(_) => {}
                None => return 0,
            },
            _ = &mut deadline => {
                let remaining = in_flight.len();
                in_flight.abort_all();
                while in_flight.join_next().await.is_some() {}
                return remaining;
            }
        }
    }
}

/// RAII guard that aborts the wrapped task on drop.
///
/// Wraps the workflow heartbeat task so the lease-renewal loop is guaranteed
/// to stop the moment `process_claimed_workflow` returns or panics - even if
/// a later `?` early-returns from one of the settlement arms. Without this,
/// a leaked heartbeat would keep extending `locked_until` for a workflow no
/// worker is actually running, blocking reclamation forever.
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Spawn the heartbeat task that extends the workflow lease at half the
/// lock-timeout interval while a workflow body executes.
///
/// Returns an `AbortOnDrop` guard. Drop or let-go-of-scope to stop the
/// heartbeat. The interval is `max(lock_timeout / 2, 1s)` so very small
/// timeouts still produce sane tick rates instead of busy-looping.
///
/// The first tick fires immediately at spawn: the claim-to-first-refresh
/// window (claim latency plus scheduling) must never be unguarded, and an
/// immediate refresh of a just-set lease is harmless - it re-extends from
/// now. Skipping it is what left short leases reclaimable before their
/// first heartbeat.
///
/// `worker_id` and `attempts` are the fencing token from the claim that
/// started this run - threaded through to `store::refresh_lock` so a
/// heartbeat that fires after another worker has reclaimed this row (this
/// worker was starved past its lease) cannot extend the new owner's lease
/// under the old owner's name. See `store::refresh_lock` for the fencing
/// mechanism.
fn spawn_lease_heartbeat(
    workflow_id: i64,
    lock_timeout: Duration,
    worker_id: String,
    attempts: i32,
) -> AbortOnDrop {
    let interval = std::cmp::max(lock_timeout / 2, Duration::from_secs(1));
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            match store::refresh_lock_if_owned(workflow_id, lock_timeout, &worker_id, attempts)
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!(
                        workflow_id,
                        worker_id,
                        attempts,
                        "workflow lease heartbeat stopped after ownership was lost"
                    );
                    break;
                }
                Err(err) => tracing::warn!(
                    workflow_id,
                    error = %err,
                    "workflow lease heartbeat failed; retrying on the next tick"
                ),
            }
        }
    });
    AbortOnDrop(handle)
}

/// Start a workflow by name with serialized input JSON.
///
/// Uses [`registry::find_strict`] so a duplicate `#[workflow]`
/// registration aborts the enqueue with a clear error rather than
/// silently picking whichever copy the linker happened to put first.
pub async fn start_named(name: &str, input: &str) -> Result<WorkflowHandle, FrameworkError> {
    if registry::find_strict(name)?.is_none() {
        return Err(FrameworkError::internal(format!(
            "Workflow '{}' is not registered",
            name
        )));
    }

    let config = Config::get::<WorkflowConfig>().unwrap_or_default();
    store::insert_workflow(name, input, config.max_attempts).await
}

/// Workflow worker daemon
pub struct WorkflowWorker {
    config: Arc<WorkflowConfig>,
    worker_id: String,
}

impl Default for WorkflowWorker {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkflowWorker {
    /// Create a worker with config from environment.
    ///
    /// Boot-time invariants are checked here: duplicate `#[workflow]`
    /// registrations are detected, and the config is validated. A
    /// misconfiguration that would deadlock the worker (`concurrency=0`,
    /// negative `retry_backoff_secs`, etc.) is caught with `.expect` at
    /// boot, not at first job pickup, so a failed config crashes the
    /// daemon visibly instead of letting it hang quietly. Callers that
    /// want non-panicking handling can use [`Self::with_config`] after
    /// calling `WorkflowConfig::validate` themselves.
    pub fn new() -> Self {
        let config = Config::get::<WorkflowConfig>().unwrap_or_default();
        // Clamp + warn happens inside `from_env`; this re-check guards
        // programmatic configs that bypassed it.
        if let Err(err) = config.validate() {
            tracing::error!(error = %err, "WorkflowConfig validation failed");
            panic!("WorkflowConfig validation failed: {err}");
        }
        if let Err(err) = registry::assert_no_duplicates() {
            tracing::error!(error = %err, "duplicate workflow registrations detected at worker boot");
            panic!("{err}");
        }
        Self::with_config(config)
    }

    /// Create a worker with a custom config.
    ///
    /// Construction does not validate the config or check the registry.
    /// The worker validates config before its run loop starts; callers that
    /// need construction-time validation can call [`WorkflowConfig::validate`].
    /// Call [`registry::assert_no_duplicates`] separately when needed.
    pub fn with_config(config: WorkflowConfig) -> Self {
        let random: u64 = rand::rng().random();
        let worker_id = format!("{}-{}", std::process::id(), random);
        Self {
            config: Arc::new(config),
            worker_id,
        }
    }

    /// Worker id (process-id + random suffix) used to stamp claimed rows.
    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }

    /// Run the worker loop indefinitely.
    ///
    /// Internally constructs a never-cancelled token and delegates to
    /// [`Self::run_with_cancel`]. Used by the `workflow:work` command
    /// which sets up its own Ctrl-C handling at the binary layer.
    pub async fn work_loop() -> Result<(), FrameworkError> {
        Self::new().run(CancellationToken::new()).await
    }

    /// Run with an external cancellation token.
    ///
    /// When the token fires the worker stops pulling new claims and
    /// awaits every in-flight task in its `JoinSet` before returning.
    /// This is the path the application binary should use so SIGINT /
    /// SIGTERM cleanly drains the worker instead of orphaning in-flight
    /// workflows.
    pub async fn run_with_cancel(self, cancel: CancellationToken) -> Result<(), FrameworkError> {
        self.run(cancel).await
    }

    async fn run(self, cancel: CancellationToken) -> Result<(), FrameworkError> {
        self.config.validate()?;

        let poll = Duration::from_millis(self.config.poll_interval_ms);
        let semaphore = Arc::new(Semaphore::new(self.config.concurrency));
        let mut in_flight: JoinSet<()> = JoinSet::new();

        tracing::info!(
            worker_id = %self.worker_id,
            concurrency = self.config.concurrency,
            poll_interval_ms = self.config.poll_interval_ms,
            lock_timeout_secs = self.config.lock_timeout_secs,
            max_attempts = self.config.max_attempts,
            retry_backoff_secs = self.config.retry_backoff_secs,
            "workflow worker started"
        );

        loop {
            // Drain finished tasks every iteration so the JoinSet never
            // grows without bound between cancellation rounds. This also
            // surfaces any task panic that escaped `process_claimed_workflow`
            // (it shouldn't - there's a catch_unwind inside - but the
            // tracing event makes the leak observable).
            while let Some(joined) = in_flight.try_join_next() {
                if let Err(err) = joined
                    && err.is_panic()
                {
                    tracing::error!(
                        error = %err,
                        "workflow worker task panicked outside the catch_unwind boundary"
                    );
                }
            }

            if cancel.is_cancelled() {
                tracing::info!(
                    worker_id = %self.worker_id,
                    in_flight = in_flight.len(),
                    "workflow worker draining in-flight tasks before exit"
                );
                // Bounded: the drain used to await every in-flight task with
                // no deadline, so a single workflow step that never returns
                // held the worker open until SIGKILL. Cancellation already
                // closed admission - this arm is only reached once
                // `cancel.is_cancelled()`, and the loop returns rather than
                // claiming again - so the only question left is how long to
                // wait for work already running.
                let abandoned = drain_in_flight(&mut in_flight, WORKFLOW_DRAIN_GRACE).await;
                if abandoned > 0 {
                    tracing::warn!(
                        worker_id = %self.worker_id,
                        abandoned,
                        grace_secs = WORKFLOW_DRAIN_GRACE.as_secs(),
                        "workflow drain deadline exceeded; aborted in-flight steps. \
                         Their leases lapse and another worker reclaims them."
                    );
                }
                tracing::info!(worker_id = %self.worker_id, "workflow worker stopped");
                return Ok(());
            }

            // Acquire-or-cancel: if the token fires while every slot is
            // taken we must not block on the semaphore - the next iter
            // would never see `is_cancelled`. Race the permit against
            // the cancel signal.
            let permit = tokio::select! {
                biased;
                _ = cancel.cancelled() => continue,
                permit = semaphore.clone().acquire_owned() => permit.unwrap(),
            };

            // Race the claim against cancellation too - if the DB is
            // slow and Ctrl-C fires, we shouldn't wait a full claim
            // round-trip to exit.
            let claim = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    drop(permit);
                    continue;
                }
                res = store::claim_next_workflow(&self.worker_id, &self.config) => res,
            };

            match claim {
                Ok(Some(claimed)) => {
                    let config = self.config.clone();
                    let workflow_id = claimed.id;
                    let workflow_name = claimed.name.clone();
                    in_flight.spawn(async move {
                        if let Err(err) = process_claimed_workflow(claimed, config).await {
                            tracing::error!(
                                workflow_id,
                                workflow_name = %workflow_name,
                                error = %err,
                                "workflow execution returned error after settlement; row state is likely consistent but inspect manually"
                            );
                        }
                        drop(permit);
                    });
                }
                Ok(None) => {
                    drop(permit);
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => continue,
                        _ = tokio::time::sleep(poll) => {}
                    }
                }
                Err(err) => {
                    tracing::error!(
                        worker_id = %self.worker_id,
                        error = %err,
                        "workflow claim failed; backing off"
                    );
                    drop(permit);
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => continue,
                        _ = tokio::time::sleep(poll) => {}
                    }
                }
            }
        }
    }
}

async fn process_claimed_workflow(
    claimed: ClaimedWorkflow,
    config: Arc<WorkflowConfig>,
) -> Result<(), FrameworkError> {
    // `claimed.worker_id` / `claimed.attempts` are the fencing token
    // returned by the claim (see `ClaimedWorkflow`'s docs). Every mutation
    // below presents this token back to the store so a worker whose lease
    // was reclaimed mid-flight (starved past `locked_until`) cannot
    // overwrite the row another worker now owns.
    let entry = match registry::find(&claimed.name) {
        Some(entry) => entry,
        None => {
            store::mark_failed(
                claimed.id,
                "Workflow not registered",
                &claimed.worker_id,
                claimed.attempts,
            )
            .await?;
            return Ok(());
        }
    };

    let lock_timeout = Duration::from_secs(config.lock_timeout_secs);
    let ctx = WorkflowContext::new(
        claimed.id,
        lock_timeout,
        claimed.worker_id.clone(),
        claimed.attempts,
    );

    // Extend the workflow lease while the body runs so long-running steps
    // do not get reclaimed mid-flight by another worker. The pre/post-step
    // refreshes in `WorkflowContext::run_step_with_input` cover the step
    // boundaries, but they do nothing while a step future is awaiting
    // (network I/O, sleeps, retries). Without this, a step that takes
    // longer than `lock_timeout_secs` (default 30s) lets
    // `claim_next_workflow` reclaim the workflow under our feet.
    //
    // The guard aborts the heartbeat task on drop. That's load-bearing -
    // each settle arm uses `?`, so an early return must not leak the
    // heartbeat task and have it keep extending `locked_until` for a
    // workflow nobody is running.
    let _heartbeat = spawn_lease_heartbeat(
        claimed.id,
        lock_timeout,
        claimed.worker_id.clone(),
        claimed.attempts,
    );

    // Run the workflow body inside a panic boundary so a panicking handler
    // does not strand the row. The spawn site only logs Err returns; a panic
    // would otherwise unwind the spawned task and skip the requeue/mark_failed
    // path entirely, leaving status='running' until the lease expires -
    // and the lease itself only matters now that `claim_next_workflow`
    // reclaims expired-running rows. The boundary mirrors the request-path
    // pattern in `server::execute_chain_safely`: catch the unwind, downcast
    // the payload, fold into the existing Err arm so the row goes through
    // the same retry/fail accounting as a returned `FrameworkError`.
    let body = AssertUnwindSafe(ctx.enter(async { (entry.run)(&claimed.input).await }));
    let result = match body.catch_unwind().await {
        Ok(inner) => inner,
        Err(panic) => {
            let msg = crate::server::panic_payload_message(&panic);
            tracing::error!(
                workflow_id = claimed.id,
                workflow_name = %claimed.name,
                attempts = claimed.attempts,
                max_attempts = claimed.max_attempts,
                panic = %msg,
                "workflow handler panicked - routing through retry/fail path"
            );
            Err(FrameworkError::internal(format!(
                "workflow handler panicked: {msg}"
            )))
        }
    };

    match result {
        Ok(output) => {
            store::mark_succeeded(claimed.id, &output, &claimed.worker_id, claimed.attempts)
                .await?;
        }
        Err(err) => {
            if claimed.attempts < claimed.max_attempts {
                let backoff = config.retry_backoff_secs * claimed.attempts as i64;
                let next_run_at = Utc::now().naive_utc() + ChronoDuration::seconds(backoff);
                store::requeue(
                    claimed.id,
                    &err.to_string(),
                    next_run_at,
                    &claimed.worker_id,
                    claimed.attempts,
                )
                .await?;
            } else {
                store::mark_failed(
                    claimed.id,
                    &err.to_string(),
                    &claimed.worker_id,
                    claimed.attempts,
                )
                .await?;
            }
        }
    }

    Ok(())
}

/// Enqueue a workflow by function name with serialized args
///
/// Example:
/// ```rust,no_run
/// # use suprnova::{start_workflow, FrameworkError};
/// # fn my_workflow() {}
/// # async fn ex() -> Result<(), FrameworkError> {
/// let handle = start_workflow!(my_workflow, 42, "hello").await?;
/// # let _ = handle;
/// # Ok(()) }
/// ```
#[macro_export]
macro_rules! start_workflow {
    ($workflow:path $(, $arg:expr)* $(,)?) => {{
        async {
            let __name = stringify!($workflow);
            let __name = if __name.contains("::") {
                __name.to_string()
            } else {
                format!("{}::{}", module_path!(), __name)
            };
            let __name = __name.replace(' ', "");
            let __input = ::suprnova::serde_json::to_string(&( $($arg,)* ))
                .map_err(|e| ::suprnova::FrameworkError::internal(format!("Workflow input serialize error: {}", e)))?;
            ::suprnova::workflow::start_named(&__name, &__input).await
        }
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestDatabase;
    use sea_orm_migration::prelude::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use suprnova_macros::{workflow, workflow_step};

    static ALWAYS_CALLS: AtomicUsize = AtomicUsize::new(0);
    static FLAKY_CALLS: AtomicUsize = AtomicUsize::new(0);
    static CACHE_CALLS: AtomicUsize = AtomicUsize::new(0);
    static INPUT_MISMATCH_CALLS: AtomicUsize = AtomicUsize::new(0);

    #[workflow_step]
    async fn always_step() -> Result<i32, FrameworkError> {
        ALWAYS_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(1)
    }

    #[workflow_step]
    async fn flaky_step() -> Result<i32, FrameworkError> {
        let attempt = FLAKY_CALLS.fetch_add(1, Ordering::SeqCst);
        if attempt == 0 {
            Err(FrameworkError::internal("flaky"))
        } else {
            Ok(2)
        }
    }

    #[workflow]
    async fn test_workflow() -> Result<i32, FrameworkError> {
        let a = always_step().await?;
        let b = flaky_step().await?;
        Ok(a + b)
    }

    #[workflow]
    async fn name_norm_workflow(value: i32) -> Result<i32, FrameworkError> {
        Ok(value)
    }

    #[workflow]
    async fn panicking_workflow() -> Result<i32, FrameworkError> {
        panic!("boom");
    }

    // Sleep duration for the heartbeat regression test below.
    // Long enough to outlive the 2s lease the test sets, short enough to
    // keep the test snappy.
    const SLOW_STEP_SLEEP_MS: u64 = 2_500;

    #[workflow_step]
    async fn slow_step() -> Result<i32, FrameworkError> {
        tokio::time::sleep(Duration::from_millis(SLOW_STEP_SLEEP_MS)).await;
        Ok(7)
    }

    #[workflow]
    async fn slow_workflow() -> Result<i32, FrameworkError> {
        let v = slow_step().await?;
        Ok(v)
    }

    #[tokio::test]
    async fn test_step_caching() {
        let _db = setup_db().await;
        CACHE_CALLS.store(0, Ordering::SeqCst);

        let handle = store::insert_workflow("cache", "{}", 3)
            .await
            .expect("workflow insert");

        // `WorkflowContext::new` now takes the claim's fencing token
        // (worker_id + attempts) - `refresh_lock` inside `run_step_with_input`
        // presents it back to the store, so the row must actually be claimed
        // first or every refresh would be a fenced-out no-op.
        let claimed = store::mark_running(handle.id(), "test-worker", Duration::from_secs(30))
            .await
            .expect("mark running");
        let ctx = WorkflowContext::new(
            handle.id(),
            Duration::from_secs(30),
            claimed.worker_id.clone(),
            claimed.attempts,
        );
        let ctx_inner = ctx.clone();
        let _ = ctx
            .enter(async move {
                ctx_inner
                    .run_step_with_input(
                        "cache-step",
                        serde_json::to_string(&()).unwrap(),
                        || async {
                            CACHE_CALLS.fetch_add(1, Ordering::SeqCst);
                            Ok::<_, FrameworkError>(42)
                        },
                    )
                    .await
                    .unwrap()
            })
            .await;

        let claimed2 = store::mark_running(handle.id(), "test-worker", Duration::from_secs(30))
            .await
            .expect("mark running again");
        let ctx2 = WorkflowContext::new(
            handle.id(),
            Duration::from_secs(30),
            claimed2.worker_id.clone(),
            claimed2.attempts,
        );
        let ctx2_inner = ctx2.clone();
        let value = ctx2
            .enter(async move {
                ctx2_inner
                    .run_step_with_input(
                        "cache-step",
                        serde_json::to_string(&()).unwrap(),
                        || async {
                            CACHE_CALLS.fetch_add(1, Ordering::SeqCst);
                            Ok::<_, FrameworkError>(99)
                        },
                    )
                    .await
                    .unwrap()
            })
            .await;

        assert_eq!(value, 42);
        assert_eq!(CACHE_CALLS.load(Ordering::SeqCst), 1);
    }

    // Replaying the same step name+index with a *different* serialized input
    // must fail loud rather than silently returning the cached output from
    // the prior input. Without the determinism guard, the second call would
    // return the cached `42` even though the caller passed input `7` -
    // corrupting any downstream step that branches on this step's output.
    #[tokio::test]
    async fn test_step_replay_with_mismatched_input_errors() {
        let _db = setup_db().await;
        INPUT_MISMATCH_CALLS.store(0, Ordering::SeqCst);

        let handle = store::insert_workflow("input-mismatch", "{}", 3)
            .await
            .expect("workflow insert");

        // First pass: record a succeeded step with input `5`. Claim the row
        // first so the fencing token `WorkflowContext::new` now requires
        // (worker_id + attempts) matches what's persisted.
        let claimed = store::mark_running(handle.id(), "test-worker", Duration::from_secs(30))
            .await
            .expect("mark running");
        let ctx = WorkflowContext::new(
            handle.id(),
            Duration::from_secs(30),
            claimed.worker_id.clone(),
            claimed.attempts,
        );
        let ctx_inner = ctx.clone();
        let first = ctx
            .enter(async move {
                ctx_inner
                    .run_step_with_input(
                        "mismatch-step",
                        serde_json::to_string(&5_i32).unwrap(),
                        || async {
                            INPUT_MISMATCH_CALLS.fetch_add(1, Ordering::SeqCst);
                            Ok::<_, FrameworkError>(42_i32)
                        },
                    )
                    .await
            })
            .await
            .expect("first run records the step");
        assert_eq!(first, 42);
        assert_eq!(INPUT_MISMATCH_CALLS.load(Ordering::SeqCst), 1);

        // Replay with a different input at the same step name+index.
        // Must return an error rather than the stale `42`.
        let claimed2 = store::mark_running(handle.id(), "test-worker", Duration::from_secs(30))
            .await
            .expect("mark running again");
        let ctx2 = WorkflowContext::new(
            handle.id(),
            Duration::from_secs(30),
            claimed2.worker_id.clone(),
            claimed2.attempts,
        );
        let ctx2_inner = ctx2.clone();
        let replayed = ctx2
            .enter(async move {
                ctx2_inner
                    .run_step_with_input(
                        "mismatch-step",
                        serde_json::to_string(&7_i32).unwrap(),
                        || async {
                            INPUT_MISMATCH_CALLS.fetch_add(1, Ordering::SeqCst);
                            Ok::<_, FrameworkError>(999_i32)
                        },
                    )
                    .await
            })
            .await;

        let err = replayed.expect_err(
            "replay with mismatched input must error, not silently return the cached output",
        );
        let msg = err.to_string();
        assert!(
            msg.contains("input mismatch"),
            "error must explain the determinism violation, got: {msg}"
        );
        assert!(
            msg.contains("deterministic"),
            "error must reference the determinism contract, got: {msg}"
        );
        // The step closure must NOT have run on the failed replay - the
        // guard short-circuits before the user function is invoked.
        assert_eq!(
            INPUT_MISMATCH_CALLS.load(Ordering::SeqCst),
            1,
            "step closure must not run when input mismatch is detected"
        );
    }

    #[tokio::test]
    async fn test_reclaimed_workflow_rejects_stale_step_completion() {
        let _db = setup_db().await;
        let handle = store::insert_workflow("stale-step-completion", "{}", 3)
            .await
            .expect("workflow insert");

        let claimed_a = store::mark_running(handle.id(), "worker-a", Duration::from_secs(30))
            .await
            .expect("worker A claims workflow");
        let ctx_a = WorkflowContext::new(
            handle.id(),
            Duration::from_secs(30),
            claimed_a.worker_id,
            claimed_a.attempts,
        );
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let stale = tokio::spawn(async move {
            ctx_a
                .run_step_with_input("race-step", "{}".to_string(), move || async move {
                    started_tx.send(()).expect("signal stale step start");
                    release_rx.await.expect("release stale step");
                    Ok::<_, FrameworkError>("stale".to_string())
                })
                .await
        });
        started_rx.await.expect("stale step entered its body");

        let claimed_b = store::mark_running(handle.id(), "worker-b", Duration::from_secs(30))
            .await
            .expect("worker B reclaims workflow");
        let ctx_b = WorkflowContext::new(
            handle.id(),
            Duration::from_secs(30),
            claimed_b.worker_id,
            claimed_b.attempts,
        );
        let winner = ctx_b
            .run_step_with_input("race-step", "{}".to_string(), || async {
                Ok::<_, FrameworkError>("winner".to_string())
            })
            .await
            .expect("current owner completes the step");
        assert_eq!(winner, "winner");

        release_tx.send(()).expect("release worker A");
        let stale_error = stale
            .await
            .expect("worker A task joins")
            .expect_err("the reclaimed worker must not complete the step");
        assert!(
            stale_error.to_string().contains("lease lost"),
            "stale completion must report lease loss, got: {stale_error}"
        );

        let step = store::load_step(handle.id(), 0, "race-step")
            .await
            .expect("load race step")
            .expect("race step exists");
        assert_eq!(step.status, StepStatus::Succeeded.as_str());
        assert_eq!(step.output.as_deref(), Some("\"winner\""));
        assert!(step.error.is_none());
        assert_eq!(step.attempts, 2);
    }

    #[tokio::test]
    async fn test_reclaimed_workflow_rejects_stale_step_admission() {
        let _db = setup_db().await;
        let handle = store::insert_workflow("stale-step-admission", "{}", 3)
            .await
            .expect("workflow insert");

        let claimed_a = store::mark_running(handle.id(), "worker-a", Duration::from_secs(30))
            .await
            .expect("worker A claims workflow");
        let ctx_a = WorkflowContext::new(
            handle.id(),
            Duration::from_secs(30),
            claimed_a.worker_id,
            claimed_a.attempts,
        );

        store::mark_running(handle.id(), "worker-b", Duration::from_secs(30))
            .await
            .expect("worker B reclaims workflow");

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_in_step = calls.clone();
        let stale_error = ctx_a
            .run_step_with_input("never-run", "{}".to_string(), move || async move {
                calls_in_step.fetch_add(1, Ordering::SeqCst);
                Ok::<_, FrameworkError>("stale".to_string())
            })
            .await
            .expect_err("a stale context must be rejected before its step body runs");

        assert!(
            stale_error.to_string().contains("lease lost"),
            "stale admission must report lease loss, got: {stale_error}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(
            store::load_step_by_index(handle.id(), 0)
                .await
                .expect("load stale step index")
                .is_none(),
            "a stale context must not create a step record"
        );
    }

    #[tokio::test]
    async fn test_retry_flow() {
        let _db = setup_db().await;
        ALWAYS_CALLS.store(0, Ordering::SeqCst);
        FLAKY_CALLS.store(0, Ordering::SeqCst);

        let input = serde_json::to_string(&()).unwrap();
        let handle = start_named(&format!("{}::{}", module_path!(), "test_workflow"), &input)
            .await
            .expect("start workflow");

        let claimed = store::mark_running(handle.id(), "test-worker", Duration::from_secs(30))
            .await
            .expect("mark running");

        let config = WorkflowConfig::from_env();
        process_claimed_workflow(claimed, Arc::new(config))
            .await
            .expect("process workflow");

        let status = store::get_workflow_status(handle.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Pending);

        let claimed = store::mark_running(handle.id(), "test-worker", Duration::from_secs(30))
            .await
            .expect("mark running again");

        let config = WorkflowConfig::from_env();
        process_claimed_workflow(claimed, Arc::new(config))
            .await
            .expect("process workflow again");

        let status = store::get_workflow_status(handle.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Succeeded);
        assert_eq!(ALWAYS_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(FLAKY_CALLS.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_name_normalization() {
        let _db = setup_db().await;

        let handle = start_workflow!(name_norm_workflow, 5)
            .await
            .expect("start workflow macro");

        let record = store::get_workflow_record(handle.id()).await.unwrap();
        let expected = format!("{}::{}", module_path!(), "name_norm_workflow");
        assert_eq!(record.name, expected);
    }

    // A panicking workflow handler must NOT strand the row in 'running'.
    // With attempts < max_attempts, the panic is routed through the same
    // requeue arm as a returned Err, so the row goes back to Pending with
    // the panic message stamped in the error column. When the attempt
    // budget is exhausted, the row lands in Failed instead. Verifies
    // `process_claimed_workflow` returns Ok(()) in both cases (the panic
    // was caught and folded into the result accounting).
    #[tokio::test]
    async fn test_panic_requeues_under_budget() {
        let _db = setup_db().await;

        let workflow_name = format!("{}::{}", module_path!(), "panicking_workflow");
        let input = serde_json::to_string(&()).unwrap();

        // max_attempts = 3, attempts will increment to 1 after mark_running,
        // so 1 < 3 - the requeue arm fires.
        let handle = store::insert_workflow(&workflow_name, &input, 3)
            .await
            .expect("insert workflow");

        let claimed = store::mark_running(handle.id(), "test-worker", Duration::from_secs(30))
            .await
            .expect("mark running");
        assert_eq!(claimed.attempts, 1);
        assert_eq!(claimed.max_attempts, 3);

        let config = WorkflowConfig::from_env();
        process_claimed_workflow(claimed, Arc::new(config))
            .await
            .expect(
                "process_claimed_workflow returned Err - the panic boundary should have caught it",
            );

        let status = store::get_workflow_status(handle.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Pending, "row must be requeued");

        let record = store::get_workflow_record(handle.id()).await.unwrap();
        let err = record
            .error
            .expect("error column should carry panic message");
        assert!(
            err.contains("boom"),
            "panic payload 'boom' must reach the error column, got: {err}"
        );
        assert!(
            err.contains("panicked"),
            "error must record that it came from a panic, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_panic_marks_failed_when_budget_exhausted() {
        let _db = setup_db().await;

        let workflow_name = format!("{}::{}", module_path!(), "panicking_workflow");
        let input = serde_json::to_string(&()).unwrap();

        // max_attempts = 1: after mark_running, attempts = 1, so 1 < 1 is
        // false and the mark_failed arm fires.
        let handle = store::insert_workflow(&workflow_name, &input, 1)
            .await
            .expect("insert workflow");

        let claimed = store::mark_running(handle.id(), "test-worker", Duration::from_secs(30))
            .await
            .expect("mark running");
        assert_eq!(claimed.attempts, 1);
        assert_eq!(claimed.max_attempts, 1);

        let config = WorkflowConfig::from_env();
        process_claimed_workflow(claimed, Arc::new(config))
            .await
            .expect(
                "process_claimed_workflow returned Err - the panic boundary should have caught it",
            );

        let status = store::get_workflow_status(handle.id()).await.unwrap();
        assert_eq!(status, WorkflowStatus::Failed, "row must be marked failed");

        let record = store::get_workflow_record(handle.id()).await.unwrap();
        let err = record
            .error
            .expect("error column should carry panic message");
        assert!(
            err.contains("boom"),
            "panic payload 'boom' must reach the error column, got: {err}"
        );
    }

    // A workflow body that outlives the lock-timeout window must not
    // strand its row to reclamation. The fix: a heartbeat task spawned
    // inside `process_claimed_workflow` extends `locked_until` at half
    // the lock-timeout interval until the body resolves. Without the
    // heartbeat, the only mid-body lease refreshes are the per-step
    // pre/post refreshes in `WorkflowContext::run_step_with_input` -
    // a single step that runs longer than `lock_timeout_secs` would
    // therefore go the entire `f().await` window with the lease frozen
    // at the value set by the pre-step refresh, and another worker can
    // reclaim it under our feet.
    //
    // The regression check counts DISTINCT `locked_until` values seen
    // during the workflow body, excluding the pre-step refresh (which
    // happens before the step starts and is unrelated to the heartbeat).
    // Snapshot strategy:
    //
    //   * baseline = locked_until once the pre-step refresh has landed
    //     (status='running' on a step row and step started_at populated).
    //     This factors out the per-step refresh path so its single bump
    //     can't false-pass the test.
    //   * Then poll the row while the step is sleeping and record every
    //     distinct locked_until > baseline that appears before the body
    //     completes.
    //
    // With heartbeat: at least one tick fires during the 2.5s sleep
    // (interval = lock_timeout/2 = 1s), so at least one post-baseline
    // value lands → advances ≥ 1.
    //
    // Without heartbeat: nothing refreshes the lease between the
    // pre-step refresh and the step's completion, so no post-baseline
    // value appears → advances = 0 and the assertion fails.
    //
    // Backend-agnostic: this test never calls `claim_next_workflow`
    // (Postgres-only), only `process_claimed_workflow` + `refresh_lock`,
    // both SQLite-compatible.
    #[tokio::test]
    async fn test_long_running_step_extends_lease() {
        let _db = setup_db().await;

        let workflow_name = format!("{}::{}", module_path!(), "slow_workflow");
        let input = serde_json::to_string(&()).unwrap();

        let handle = store::insert_workflow(&workflow_name, &input, 3)
            .await
            .expect("insert workflow");

        // Mark the row running with a short 2s lease.
        let claimed = store::mark_running(handle.id(), "test-worker", Duration::from_secs(2))
            .await
            .expect("mark running");

        // Drive the body in the background so we can poll the row from
        // this task while the step is still sleeping.
        let mut config = WorkflowConfig::from_env();
        config.lock_timeout_secs = 2;
        let workflow_id = handle.id();
        let body =
            tokio::spawn(async move { process_claimed_workflow(claimed, Arc::new(config)).await });

        // Wait for the pre-step refresh to land (the step row appears
        // with status='running' and started_at set). That value of
        // locked_until becomes our baseline - anything strictly greater
        // than this in the polling loop below can only have been
        // written by the heartbeat.
        let baseline_lock = {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            loop {
                if std::time::Instant::now() >= deadline {
                    panic!("step row never appeared with status='running'");
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
                let step = store::load_step(workflow_id, 0, "slow_step")
                    .await
                    .expect("load step");
                if let Some(s) = step
                    && s.status == StepStatus::Running.as_str()
                    && s.started_at.is_some()
                {
                    // Step has started - capture the workflow lease as
                    // it stands after the pre-step refresh.
                    let record = store::get_workflow_record(workflow_id)
                        .await
                        .expect("load workflow record");
                    break record
                        .locked_until
                        .expect("pre-step refresh should set locked_until");
                }
            }
        };

        // Count distinct post-baseline locked_until values that appear
        // while the body is still running. Heartbeat firings show up
        // here; pre-step / post-step refreshes do not (pre-step is
        // baseline, post-step lands after status changes away from
        // 'running').
        let mut post_baseline_advances: std::collections::BTreeSet<chrono::NaiveDateTime> =
            std::collections::BTreeSet::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if std::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
            let record = store::get_workflow_record(workflow_id)
                .await
                .expect("poll workflow record");
            if record.status != WorkflowStatus::Running.as_str() {
                // Body has settled - post-step refresh and mark_succeeded
                // have either fired or are about to. Stop counting; we
                // only care about mid-body advances.
                break;
            }
            if let Some(current) = record.locked_until
                && current > baseline_lock
            {
                post_baseline_advances.insert(current);
            }
        }

        assert!(
            !post_baseline_advances.is_empty(),
            "expected heartbeat to extend locked_until at least once while the long-running step \
             was executing; baseline (post-pre-step-refresh) = {baseline_lock}, no advance observed"
        );

        // The body must still settle cleanly - the heartbeat guard
        // must abort the renewal task on drop, leaving the final
        // `mark_succeeded` write authoritative and the row in
        // Succeeded.
        body.await
            .expect("workflow body task panicked")
            .expect("process_claimed_workflow returned Err");

        let status = store::get_workflow_status(workflow_id).await.unwrap();
        assert_eq!(
            status,
            WorkflowStatus::Succeeded,
            "workflow must reach Succeeded after the heartbeat-guarded body completes"
        );
    }

    // Crash recovery: a worker that died mid-flight leaves a row in
    // status='running' whose `locked_until` lease eventually expires.
    // `claim_next_workflow` must reclaim that row so another worker can
    // pick the workflow up. SQLite is filtered out at the top of
    // `claim_next_workflow` (the SQL uses FOR UPDATE SKIP LOCKED +
    // returning, Postgres-only), so this test is env-gated on a real
    // Postgres reachable via `DATABASE_URL`. Ignored by default; ran in
    // CI environments that provision a Postgres for the workflow suite.
    #[tokio::test]
    #[ignore = "requires Postgres at DATABASE_URL"]
    async fn test_claim_reclaims_expired_running_row() {
        use crate::container::testing::TestContainer;
        use crate::database::DbConnection;
        use crate::database::config::DatabaseConfig;
        use sea_orm::ConnectionTrait;

        let Some(pg_url) = postgres_url_or_skip("claim_reclaims_expired_running_row") else {
            return;
        };

        let _guard = TestContainer::fake();
        let config = DatabaseConfig::builder()
            .url(&pg_url)
            .max_connections(2)
            .min_connections(1)
            .logging(false)
            .build();
        let conn = DbConnection::connect(&config).await.expect("pg connect");

        recreate_postgres_workflow_tables(&conn).await;

        TestContainer::singleton(conn.clone());

        // Insert a workflow row, then manually mark it 'running' with an
        // already-expired lease - simulating a worker that crashed and
        // never released its lock.
        let handle = store::insert_workflow("recoverable", "{}", 3)
            .await
            .expect("insert workflow");

        conn.inner()
            .execute_unprepared(&format!(
                "UPDATE workflows
                 SET status='running',
                     attempts=2,
                     worker_id='dead-worker',
                     locked_until=NOW() - INTERVAL '1 hour',
                     started_at=NOW() - INTERVAL '1 hour'
                 WHERE id={}",
                handle.id()
            ))
            .await
            .expect("simulate crashed worker");

        let cfg = WorkflowConfig::from_env();
        let claimed = store::claim_next_workflow("recovery-worker", &cfg)
            .await
            .expect("claim_next_workflow")
            .expect("expected to reclaim the expired-running row");

        assert_eq!(claimed.id, handle.id());
        assert_eq!(
            claimed.attempts, 3,
            "the final legal reclaim must increment attempts to max_attempts"
        );

        let record = store::get_workflow_record(handle.id()).await.unwrap();
        assert_eq!(record.status, WorkflowStatus::Running.as_str());
        assert_eq!(record.worker_id.as_deref(), Some("recovery-worker"));
    }

    #[tokio::test]
    #[ignore = "requires Postgres at DATABASE_URL"]
    async fn test_expired_running_workflow_at_attempt_budget_is_failed_not_reclaimed() {
        use crate::container::testing::TestContainer;
        use crate::database::DbConnection;
        use crate::database::config::DatabaseConfig;
        use sea_orm::ConnectionTrait;

        let Some(pg_url) = postgres_url_or_skip("expired_running_at_attempt_budget") else {
            return;
        };

        let _guard = TestContainer::fake();
        let config = DatabaseConfig::builder()
            .url(&pg_url)
            .max_connections(2)
            .min_connections(1)
            .logging(false)
            .build();
        let conn = DbConnection::connect(&config).await.expect("pg connect");

        recreate_postgres_workflow_tables(&conn).await;
        TestContainer::singleton(conn.clone());

        let exhausted = store::insert_workflow("exhausted", "{}", 3)
            .await
            .expect("insert exhausted workflow");
        conn.inner()
            .execute_unprepared(&format!(
                "UPDATE workflows
                 SET status='running',
                     attempts=3,
                     worker_id='dead-worker',
                     locked_until=NOW() - INTERVAL '1 hour',
                     started_at=NOW() - INTERVAL '1 hour'
                 WHERE id={}",
                exhausted.id()
            ))
            .await
            .expect("simulate exhausted crashed worker");

        let ready = store::insert_workflow("ready", "{}", 3)
            .await
            .expect("insert ready workflow");
        let cfg = WorkflowConfig::from_env();
        let claimed = store::claim_next_workflow("recovery-worker", &cfg)
            .await
            .expect("claim_next_workflow")
            .expect("ready workflow should still be claimed");

        assert_eq!(claimed.id, ready.id());
        assert_eq!(claimed.attempts, 1);

        let terminal = store::get_workflow_record(exhausted.id())
            .await
            .expect("load terminal workflow");
        assert_eq!(terminal.status, WorkflowStatus::Failed.as_str());
        assert_eq!(
            terminal.attempts, 3,
            "cleanup must not spend another attempt"
        );
        assert!(terminal.worker_id.is_none());
        assert!(terminal.locked_until.is_none());
        assert!(terminal.completed_at.is_some());
        assert!(
            terminal
                .error
                .as_deref()
                .is_some_and(|error| error.contains("attempt budget exhausted"))
        );

        let completed_at = terminal.completed_at;
        let error = terminal.error;
        assert!(
            store::claim_next_workflow("another-worker", &cfg)
                .await
                .expect("second claim")
                .is_none(),
            "terminal cleanup must be idempotent"
        );
        let unchanged = store::get_workflow_record(exhausted.id())
            .await
            .expect("reload terminal workflow");
        assert_eq!(unchanged.attempts, 3);
        assert_eq!(unchanged.completed_at, completed_at);
        assert_eq!(unchanged.error, error);
    }

    #[tokio::test]
    #[ignore = "requires Postgres at DATABASE_URL"]
    async fn test_postgres_reclaim_rejects_stale_step_completion() {
        use crate::container::testing::TestContainer;
        use crate::database::DbConnection;
        use crate::database::config::DatabaseConfig;
        use sea_orm::ConnectionTrait;

        let Some(pg_url) = postgres_url_or_skip("postgres_reclaim_rejects_stale_step") else {
            return;
        };

        let _guard = TestContainer::fake();
        let config = DatabaseConfig::builder()
            .url(&pg_url)
            .max_connections(4)
            .min_connections(1)
            .logging(false)
            .build();
        let conn = DbConnection::connect(&config).await.expect("pg connect");
        recreate_postgres_workflow_tables(&conn).await;
        TestContainer::singleton(conn.clone());

        let handle = store::insert_workflow("postgres-step-fence", "{}", 3)
            .await
            .expect("insert workflow");
        let claim_config = WorkflowConfig::from_env();
        let claimed_a = store::claim_next_workflow("worker-a", &claim_config)
            .await
            .expect("worker A claim")
            .expect("pending workflow is claimable");
        let ctx_a = WorkflowContext::new(
            handle.id(),
            Duration::from_secs(30),
            claimed_a.worker_id,
            claimed_a.attempts,
        );

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let stale = tokio::spawn(async move {
            ctx_a
                .run_step_with_input("race-step", "{}".to_string(), move || async move {
                    started_tx.send(()).expect("signal stale step start");
                    release_rx.await.expect("release stale step");
                    Ok::<_, FrameworkError>("stale".to_string())
                })
                .await
        });
        started_rx.await.expect("stale step entered its body");

        conn.inner()
            .execute_unprepared(&format!(
                "UPDATE workflows SET locked_until=NOW() - INTERVAL '1 hour' WHERE id={}",
                handle.id()
            ))
            .await
            .expect("expire worker A lease");
        let claimed_b = store::claim_next_workflow("worker-b", &claim_config)
            .await
            .expect("worker B claim")
            .expect("expired workflow is reclaimable");
        assert_eq!(claimed_b.id, handle.id());
        assert_eq!(claimed_b.attempts, 2);

        let ctx_b = WorkflowContext::new(
            handle.id(),
            Duration::from_secs(30),
            claimed_b.worker_id,
            claimed_b.attempts,
        );
        let winner = ctx_b
            .run_step_with_input("race-step", "{}".to_string(), || async {
                Ok::<_, FrameworkError>("winner".to_string())
            })
            .await
            .expect("current owner completes step");
        assert_eq!(winner, "winner");

        release_tx.send(()).expect("release worker A");
        let stale_error = stale
            .await
            .expect("worker A task joins")
            .expect_err("reclaimed worker must not complete step");
        assert!(stale_error.to_string().contains("lease lost"));

        let step = store::load_step(handle.id(), 0, "race-step")
            .await
            .expect("load race step")
            .expect("race step exists");
        assert_eq!(step.status, StepStatus::Succeeded.as_str());
        assert_eq!(step.output.as_deref(), Some("\"winner\""));
        assert_eq!(step.attempts, 2);
    }

    #[tokio::test]
    #[ignore = "requires MySQL or MariaDB at MYSQL_TEST_URL"]
    async fn test_mysql_same_second_refresh_confirms_current_owner() {
        use crate::container::testing::TestContainer;
        use crate::database::DbConnection;
        use crate::database::config::DatabaseConfig;
        use chrono::Timelike;

        let Some(mysql_url) = mysql_url_or_skip("mysql_same_second_refresh") else {
            return;
        };

        let _guard = TestContainer::fake();
        let config = DatabaseConfig::builder()
            .url(&mysql_url)
            .max_connections(2)
            .min_connections(1)
            .logging(false)
            .build();
        let conn = DbConnection::connect(&config).await.expect("mysql connect");
        recreate_mysql_workflow_tables(&conn).await;
        TestContainer::singleton(conn.clone());

        let handle = store::insert_workflow("mysql-refresh-fallback", "{}", 3)
            .await
            .expect("insert workflow");
        let claimed = store::mark_running(handle.id(), "worker-a", Duration::from_secs(30))
            .await
            .expect("claim workflow");
        let now = chrono::Utc::now()
            .naive_utc()
            .with_nanosecond(0)
            .expect("zero nanoseconds is valid");

        assert!(
            store::refresh_lock_if_owned_at(
                handle.id(),
                Duration::ZERO,
                "worker-a",
                claimed.attempts,
                now,
            )
            .await
            .expect("initial fixed-time refresh")
        );

        assert!(
            store::refresh_lock_if_owned_at(
                handle.id(),
                Duration::ZERO,
                "worker-a",
                claimed.attempts,
                now,
            )
            .await
            .expect("same-second refresh must read ownership back")
        );
        assert!(
            !store::refresh_lock_if_owned_at(
                handle.id(),
                Duration::ZERO,
                "stale-worker",
                claimed.attempts,
                now,
            )
            .await
            .expect("stale refresh query")
        );

        let context = WorkflowContext::new(
            handle.id(),
            Duration::from_secs(30),
            claimed.worker_id,
            claimed.attempts,
        );
        let value = context
            .run_step_with_input("mysql-step", "{}".to_string(), || async {
                Ok::<_, FrameworkError>(42_i32)
            })
            .await
            .expect("execute step against fresh MySQL schema");
        assert_eq!(value, 42);
        let step = store::load_step(handle.id(), 0, "mysql-step")
            .await
            .expect("load MySQL step")
            .expect("MySQL step exists");
        assert_eq!(step.status, StepStatus::Succeeded.as_str());
        assert_eq!(step.output.as_deref(), Some("42"));
    }

    #[tokio::test]
    #[ignore = "requires MySQL or MariaDB at MYSQL_TEST_URL"]
    async fn test_mysql_legacy_timestamp_migration_preserves_values() {
        use crate::container::testing::TestContainer;
        use crate::database::DbConnection;
        use crate::database::config::DatabaseConfig;
        use sea_orm::{ConnectionTrait, DbBackend, Statement};

        let Some(mysql_url) = mysql_url_or_skip("mysql_legacy_timestamp_migration") else {
            return;
        };

        let _guard = TestContainer::fake();
        let config = DatabaseConfig::builder()
            .url(&mysql_url)
            .max_connections(1)
            .min_connections(1)
            .logging(false)
            .build();
        let conn = DbConnection::connect(&config).await.expect("mysql connect");
        conn.inner()
            .execute_unprepared("SET time_zone = '-04:00'")
            .await
            .expect("set non-UTC test session");
        recreate_legacy_mysql_workflow_tables(&conn).await;

        conn.inner()
            .execute_unprepared(
                "INSERT INTO workflows (
                       id, name, status, input, attempts, max_attempts,
                       next_run_at, locked_until, worker_id, created_at,
                       updated_at, started_at, completed_at
                   ) VALUES (
                       1, 'legacy-workflow', 'running', '{}', 1, 3,
                       '2026-09-01 12:34:56', '2026-09-01 12:34:56',
                       'legacy-worker', '2026-09-01 12:34:56',
                       '2026-09-01 12:34:56', '2026-09-01 12:34:56',
                       '2026-09-01 12:34:56'
                   )",
            )
            .await
            .expect("seed legacy workflow");
        conn.inner()
            .execute_unprepared(
                "INSERT INTO workflow_steps (
                       id, workflow_id, step_index, step_name, status, input,
                       attempts, created_at, updated_at, started_at, completed_at
                   ) VALUES (
                       1, 1, 0, 'legacy-step', 'succeeded', '{}', 1,
                       '2026-09-01 12:34:56', '2026-09-01 12:34:56',
                       '2026-09-01 12:34:56', '2026-09-01 12:34:56'
                   )",
            )
            .await
            .expect("seed legacy workflow step");

        let manager = SchemaManager::new(conn.inner());
        migrations::NormalizeWorkflowDateTimesForMysql
            .up(&manager)
            .await
            .expect("normalize legacy MySQL workflow date columns");

        let type_row = conn
            .inner()
            .query_one_raw(Statement::from_string(
                DbBackend::MySql,
                "SELECT COUNT(*) AS compatible_count
                   FROM information_schema.columns
                   WHERE table_schema = DATABASE()
                     AND data_type = 'datetime'
                     AND (
                       (
                         column_name IN ('created_at', 'updated_at')
                         AND is_nullable = 'NO'
                         AND LOWER(column_default) IN (
                           'current_timestamp', 'current_timestamp()'
                         )
                       )
                       OR
                       (
                         column_name IN (
                           'next_run_at', 'locked_until', 'started_at', 'completed_at'
                         )
                         AND is_nullable = 'YES'
                         AND (
                           column_default IS NULL OR UPPER(column_default) = 'NULL'
                         )
                       )
                     )
                     AND (
                       (table_name = 'workflows' AND column_name IN (
                         'next_run_at', 'locked_until', 'created_at',
                         'updated_at', 'started_at', 'completed_at'
                       ))
                       OR
                       (table_name = 'workflow_steps' AND column_name IN (
                         'created_at', 'updated_at', 'started_at', 'completed_at'
                       ))
                     )"
                .to_string(),
            ))
            .await
            .expect("query normalized MySQL column metadata")
            .expect("compatible column count row");
        let compatible_count: i64 = type_row
            .try_get("", "compatible_count")
            .expect("decode compatible column count");
        assert_eq!(compatible_count, 10);

        let values_row = conn
            .inner()
            .query_one_raw(Statement::from_string(
                DbBackend::MySql,
                "SELECT CONCAT_WS('|',
                       DATE_FORMAT(w.next_run_at, '%Y-%m-%d %H:%i:%s'),
                       DATE_FORMAT(w.locked_until, '%Y-%m-%d %H:%i:%s'),
                       DATE_FORMAT(w.created_at, '%Y-%m-%d %H:%i:%s'),
                       DATE_FORMAT(w.updated_at, '%Y-%m-%d %H:%i:%s'),
                       DATE_FORMAT(w.started_at, '%Y-%m-%d %H:%i:%s'),
                       DATE_FORMAT(w.completed_at, '%Y-%m-%d %H:%i:%s'),
                       DATE_FORMAT(s.created_at, '%Y-%m-%d %H:%i:%s'),
                       DATE_FORMAT(s.updated_at, '%Y-%m-%d %H:%i:%s'),
                       DATE_FORMAT(s.started_at, '%Y-%m-%d %H:%i:%s'),
                       DATE_FORMAT(s.completed_at, '%Y-%m-%d %H:%i:%s')
                   ) AS values_after
                   FROM workflows AS w
                   JOIN workflow_steps AS s ON s.workflow_id = w.id
                   WHERE w.id = 1 AND s.id = 1"
                    .to_string(),
            ))
            .await
            .expect("query migrated workflow and step values")
            .expect("migrated workflow and step row");
        let values_after: String = values_row
            .try_get("", "values_after")
            .expect("decode migrated workflow and step values");
        assert_eq!(values_after, vec!["2026-09-01 12:34:56"; 10].join("|"));

        TestContainer::singleton(conn.clone());
        store::get_workflow_record(1)
            .await
            .expect("decode migrated workflow entity");
        store::load_step(1, 0, "legacy-step")
            .await
            .expect("decode migrated workflow step entity")
            .expect("migrated workflow step exists");
    }

    // A cancelled worker must drain in-flight workflows before returning.
    // Spawns a worker that has no rows to claim (so it idles in the
    // poll/sleep path), cancels the token, and asserts run_with_cancel
    // resolves cleanly to Ok(()) - i.e. the cancellation path exits the
    // loop rather than blocking on the semaphore or the next claim.
    #[tokio::test]
    async fn test_worker_run_with_cancel_returns_cleanly() {
        let _db = setup_db().await;

        let mut config = WorkflowConfig::from_env();
        // Tighten poll so the loop reaches a cancellation check fast.
        config.poll_interval_ms = 20;
        let worker = WorkflowWorker::with_config(config);
        let cancel = CancellationToken::new();
        let cancel_for_worker = cancel.clone();

        let handle = tokio::spawn(async move { worker.run_with_cancel(cancel_for_worker).await });

        // Let the worker reach its idle/sleep path.
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();

        // The worker must return within a small window after cancel.
        // 1s budget covers the longest path (poll round-trip + drain).
        let result = tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("worker did not exit within 1s of cancellation")
            .expect("worker task panicked");

        result.expect("run_with_cancel must return Ok on graceful drain");
    }

    #[tokio::test]
    async fn worker_rejects_zero_lock_timeout_before_starting() {
        let config = WorkflowConfig {
            poll_interval_ms: 20,
            concurrency: 1,
            lock_timeout_secs: 0,
            max_attempts: 3,
            retry_backoff_secs: 5,
        };
        let worker = WorkflowWorker::with_config(config);
        let cancel = CancellationToken::new();
        cancel.cancel();

        let err = worker
            .run_with_cancel(cancel)
            .await
            .expect_err("zero lock timeout must fail before worker startup");
        assert!(
            err.to_string().contains("lock_timeout_secs"),
            "error must name lock_timeout_secs, got: {err}",
        );
    }

    // wait_with_timeout must return a timeout error when the workflow
    // never reaches Succeeded/Failed within the deadline. We point the
    // handle at a workflow id that doesn't exist; status() returns
    // FrameworkError("Workflow not found"), so the inner future returns
    // Err immediately. To test the timeout path itself, we create a
    // valid pending workflow and never let any worker pick it up.
    #[tokio::test]
    async fn test_wait_with_timeout_fires_on_stuck_workflow() {
        let _db = setup_db().await;

        let handle = store::insert_workflow("stuck", "{}", 3)
            .await
            .expect("insert workflow");

        // No worker is running, the workflow will sit at Pending. A
        // 250 ms timeout must fire and the call must return an error
        // mentioning the timeout.
        let start = std::time::Instant::now();
        let err = handle
            .wait_with_timeout(Duration::from_millis(250))
            .await
            .expect_err("wait_with_timeout must error on a stuck workflow");
        let elapsed = start.elapsed();

        let msg = err.to_string();
        assert!(
            msg.to_lowercase().contains("timed out") || msg.to_lowercase().contains("timeout"),
            "error must reference the timeout, got: {msg}"
        );
        // The timeout must actually have fired around the deadline,
        // not after polling forever. 3 s ceiling tolerates CI jitter
        // while still failing if the timeout was ignored entirely.
        assert!(
            elapsed < Duration::from_secs(3),
            "wait_with_timeout must respect the deadline; elapsed = {:?}",
            elapsed
        );
    }

    // Once the workflow reaches Succeeded, wait_with_timeout returns
    // it without hitting the deadline. Regression for the case where
    // the timeout wrapper swallows the Ok branch.
    #[tokio::test]
    async fn test_wait_with_timeout_returns_finished_status() {
        let _db = setup_db().await;

        let handle = store::insert_workflow("quick", "{}", 3)
            .await
            .expect("insert workflow");

        // Claim (so the row has a fencing token to settle against), then
        // mark it Succeeded directly - no full worker run involved.
        let claimed = store::mark_running(handle.id(), "test-worker", Duration::from_secs(30))
            .await
            .expect("mark running");
        store::mark_succeeded(
            handle.id(),
            "\"done\"",
            &claimed.worker_id,
            claimed.attempts,
        )
        .await
        .expect("mark succeeded");

        let status = handle
            .wait_with_timeout(Duration::from_secs(1))
            .await
            .expect("wait must succeed on an already-finished workflow");
        assert_eq!(status, WorkflowStatus::Succeeded);
    }

    // -------------------------------------------------------------------------
    // DATA-03 regression: fencing on settlement writes (worker_id + attempts)
    // -------------------------------------------------------------------------

    // Happy path: a single worker claims and settles a workflow using its own
    // fencing token. Must still succeed normally post-fix - the fencing
    // predicate must not reject the legitimate, still-current owner.
    #[tokio::test]
    async fn test_fenced_settlement_happy_path_succeeds() {
        let _db = setup_db().await;

        let handle = store::insert_workflow("fencing-happy-path", "{}", 3)
            .await
            .expect("insert workflow");

        let claimed = store::mark_running(handle.id(), "solo-worker", Duration::from_secs(30))
            .await
            .expect("claim workflow");
        assert_eq!(claimed.worker_id, "solo-worker");
        assert_eq!(claimed.attempts, 1);

        store::mark_succeeded(
            claimed.id,
            "\"happy-result\"",
            &claimed.worker_id,
            claimed.attempts,
        )
        .await
        .expect("settlement with a matching fencing token must succeed");

        let record = store::get_workflow_record(handle.id()).await.unwrap();
        assert_eq!(record.status, WorkflowStatus::Succeeded.as_str());
        assert_eq!(record.output.as_deref(), Some("\"happy-result\""));
        assert!(
            record.worker_id.is_none(),
            "settlement must release the lease (worker_id cleared)"
        );
    }

    // Two workers, one lease expiry: worker A claims, its lease lapses
    // (simulated the same way `test_retry_flow` and the crash-recovery test
    // simulate reclamation - a second `mark_running` call, mirroring what
    // `claim_next_workflow`'s UPDATE does on reclaim: bump attempts, overwrite
    // worker_id), worker B reclaims and settles first. When A's stale run
    // finally finishes and tries to settle with its OLD fencing token, the
    // write must be a fenced no-op: no error, no retry, and - the actual
    // defect under test - B's already-committed result must NOT be
    // overwritten.
    #[tokio::test]
    async fn test_stale_worker_settlement_does_not_overwrite_winner() {
        let _db = setup_db().await;

        let handle = store::insert_workflow("fencing-race", "{}", 3)
            .await
            .expect("insert workflow");

        // Worker A claims first.
        let claimed_a = store::mark_running(handle.id(), "worker-a", Duration::from_secs(30))
            .await
            .expect("worker A claims");
        assert_eq!(claimed_a.attempts, 1);
        assert_eq!(claimed_a.worker_id, "worker-a");

        // Worker A's lease lapses and worker B reclaims the row - attempts
        // increments, worker_id is overwritten, exactly like
        // `claim_next_workflow`'s reclaim arm.
        let claimed_b = store::mark_running(handle.id(), "worker-b", Duration::from_secs(30))
            .await
            .expect("worker B reclaims");
        assert_eq!(claimed_b.attempts, 2);
        assert_eq!(claimed_b.worker_id, "worker-b");

        // B is the legitimate current owner and settles successfully.
        store::mark_succeeded(
            claimed_b.id,
            "\"b-result\"",
            &claimed_b.worker_id,
            claimed_b.attempts,
        )
        .await
        .expect("worker B settles with a matching fencing token");

        let record = store::get_workflow_record(handle.id()).await.unwrap();
        assert_eq!(record.status, WorkflowStatus::Succeeded.as_str());
        assert_eq!(record.output.as_deref(), Some("\"b-result\""));

        // Worker A - unaware its lease was reclaimed - finally finishes its
        // stale run and tries to settle with its now-stale token
        // (worker_id="worker-a", attempts=1). Must return Ok (lease lost is
        // not an error condition) and must NOT touch the row B already
        // settled.
        store::mark_succeeded(
            claimed_a.id,
            "\"a-result-stale\"",
            &claimed_a.worker_id,
            claimed_a.attempts,
        )
        .await
        .expect("a fenced-out stale settlement must return Ok, not Err");

        let record_after = store::get_workflow_record(handle.id()).await.unwrap();
        assert_eq!(
            record_after.status,
            WorkflowStatus::Succeeded.as_str(),
            "status must remain exactly what B committed"
        );
        assert_eq!(
            record_after.output.as_deref(),
            Some("\"b-result\""),
            "the winner's output must survive; the stale worker's write must be dropped by fencing"
        );
    }

    // Framework-owned migrations are exposed so consumer apps can
    // register the schema without copying the table definitions.
    // Regression: the modules existed but weren't re-exported under
    // the `migrations` submodule.
    #[test]
    fn test_framework_migrations_are_exposed() {
        use sea_orm_migration::MigrationName;
        let wf = migrations::CreateWorkflowsTable;
        let st = migrations::CreateWorkflowStepsTable;
        let normalize = migrations::NormalizeWorkflowDateTimesForMysql;
        assert!(
            wf.name().contains("workflows"),
            "workflows migration name must reference the table: {}",
            wf.name()
        );
        assert!(
            st.name().contains("workflow_steps"),
            "workflow_steps migration name must reference the table: {}",
            st.name()
        );
        assert!(
            normalize
                .name()
                .contains("normalize_workflow_datetime_columns"),
            "normalizer migration name must describe its compatibility change: {}",
            normalize.name()
        );
        // Names must be distinct so the migrator doesn't dedupe them.
        assert_ne!(wf.name(), st.name());
        assert_ne!(wf.name(), normalize.name());
        assert_ne!(st.name(), normalize.name());
    }

    fn postgres_url_or_skip(test_name: &str) -> Option<String> {
        match std::env::var("DATABASE_URL") {
            Ok(url) if url.starts_with("postgres://") || url.starts_with("postgresql://") => {
                Some(url)
            }
            Ok(_) => {
                eprintln!("[{test_name}] skipping: DATABASE_URL is not a Postgres URL");
                None
            }
            Err(_) => {
                eprintln!("[{test_name}] skipping: DATABASE_URL not set");
                None
            }
        }
    }

    fn mysql_url_or_skip(test_name: &str) -> Option<String> {
        match std::env::var("MYSQL_TEST_URL") {
            Ok(url) if url.starts_with("mysql://") => Some(url),
            Ok(_) => {
                eprintln!("[{test_name}] skipping: MYSQL_TEST_URL is not a MySQL URL");
                None
            }
            Err(_) => {
                eprintln!("[{test_name}] skipping: MYSQL_TEST_URL not set");
                None
            }
        }
    }

    async fn recreate_postgres_workflow_tables(conn: &crate::database::DbConnection) {
        use sea_orm::ConnectionTrait;

        conn.inner()
            .execute_unprepared("DROP TABLE IF EXISTS workflow_steps")
            .await
            .expect("drop workflow_steps test table");
        conn.inner()
            .execute_unprepared("DROP TABLE IF EXISTS workflows")
            .await
            .expect("drop workflows test table");

        let manager = SchemaManager::new(conn.inner());
        CreateWorkflowsTable
            .up(&manager)
            .await
            .expect("create workflows test table");
        CreateWorkflowStepsTable
            .up(&manager)
            .await
            .expect("create workflow_steps test table");
    }

    async fn recreate_mysql_workflow_tables(conn: &crate::database::DbConnection) {
        use sea_orm::ConnectionTrait;

        conn.inner()
            .execute_unprepared("DROP TABLE IF EXISTS workflow_steps")
            .await
            .expect("drop workflow_steps test table");
        conn.inner()
            .execute_unprepared("DROP TABLE IF EXISTS workflows")
            .await
            .expect("drop workflows test table");

        let manager = SchemaManager::new(conn.inner());
        migrations::CreateWorkflowsTable
            .up(&manager)
            .await
            .expect("create production workflows table");
        migrations::CreateWorkflowStepsTable
            .up(&manager)
            .await
            .expect("create production workflow_steps table");
    }

    async fn recreate_legacy_mysql_workflow_tables(conn: &crate::database::DbConnection) {
        use sea_orm::ConnectionTrait;

        recreate_mysql_workflow_tables(conn).await;
        conn.inner()
            .execute_unprepared(
                "ALTER TABLE workflows
                       MODIFY COLUMN next_run_at TIMESTAMP NULL DEFAULT NULL,
                       MODIFY COLUMN locked_until TIMESTAMP NULL DEFAULT NULL,
                       MODIFY COLUMN created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                       MODIFY COLUMN updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                       MODIFY COLUMN started_at TIMESTAMP NULL DEFAULT NULL,
                       MODIFY COLUMN completed_at TIMESTAMP NULL DEFAULT NULL",
            )
            .await
            .expect("convert workflows table to legacy TIMESTAMP columns");
        conn.inner()
            .execute_unprepared(
                "ALTER TABLE workflow_steps
                       MODIFY COLUMN created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                       MODIFY COLUMN updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                       MODIFY COLUMN started_at TIMESTAMP NULL DEFAULT NULL,
                       MODIFY COLUMN completed_at TIMESTAMP NULL DEFAULT NULL",
            )
            .await
            .expect("convert workflow_steps table to legacy TIMESTAMP columns");
    }

    async fn setup_db() -> TestDatabase {
        TestDatabase::fresh::<TestMigrator>()
            .await
            .expect("test db")
    }

    pub struct TestMigrator;

    #[async_trait::async_trait]
    impl MigratorTrait for TestMigrator {
        fn migrations() -> Vec<Box<dyn MigrationTrait>> {
            vec![
                Box::new(CreateWorkflowsTable),
                Box::new(CreateWorkflowStepsTable),
            ]
        }
    }

    pub struct CreateWorkflowsTable;

    impl MigrationName for CreateWorkflowsTable {
        // Explicit, file-stable version. `DeriveMigrationName` derives from
        // the parent module path, which collides with `CreateWorkflowStepsTable`
        // because both live in the same `tests` module.
        fn name(&self) -> &str {
            "m20240101_000001_create_workflows"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for CreateWorkflowsTable {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .create_table(
                    Table::create()
                        .table(Workflows::Table)
                        .if_not_exists()
                        .col(
                            ColumnDef::new(Workflows::Id)
                                .big_integer()
                                .not_null()
                                .auto_increment()
                                .primary_key(),
                        )
                        .col(ColumnDef::new(Workflows::Name).string().not_null())
                        .col(ColumnDef::new(Workflows::Status).string().not_null())
                        .col(ColumnDef::new(Workflows::Input).text().not_null())
                        .col(ColumnDef::new(Workflows::Output).text().null())
                        .col(ColumnDef::new(Workflows::Error).text().null())
                        .col(ColumnDef::new(Workflows::Attempts).integer().not_null())
                        .col(ColumnDef::new(Workflows::MaxAttempts).integer().not_null())
                        .col(ColumnDef::new(Workflows::NextRunAt).date_time().null())
                        .col(ColumnDef::new(Workflows::LockedUntil).date_time().null())
                        .col(ColumnDef::new(Workflows::WorkerId).string().null())
                        .col(
                            ColumnDef::new(Workflows::CreatedAt)
                                .date_time()
                                .not_null()
                                .default(Expr::current_timestamp()),
                        )
                        .col(
                            ColumnDef::new(Workflows::UpdatedAt)
                                .date_time()
                                .not_null()
                                .default(Expr::current_timestamp()),
                        )
                        .col(ColumnDef::new(Workflows::StartedAt).date_time().null())
                        .col(ColumnDef::new(Workflows::CompletedAt).date_time().null())
                        .to_owned(),
                )
                .await?;

            manager
                .create_index(
                    Index::create()
                        .name("idx_workflows_status")
                        .table(Workflows::Table)
                        .col(Workflows::Status)
                        .to_owned(),
                )
                .await?;

            manager
                .create_index(
                    Index::create()
                        .name("idx_workflows_next_run_at")
                        .table(Workflows::Table)
                        .col(Workflows::NextRunAt)
                        .to_owned(),
                )
                .await?;

            manager
                .create_index(
                    Index::create()
                        .name("idx_workflows_locked_until")
                        .table(Workflows::Table)
                        .col(Workflows::LockedUntil)
                        .to_owned(),
                )
                .await
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .drop_table(Table::drop().table(Workflows::Table).to_owned())
                .await
        }
    }

    pub struct CreateWorkflowStepsTable;

    impl MigrationName for CreateWorkflowStepsTable {
        fn name(&self) -> &str {
            "m20240101_000002_create_workflow_steps"
        }
    }

    #[async_trait::async_trait]
    impl MigrationTrait for CreateWorkflowStepsTable {
        async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .create_table(
                    Table::create()
                        .table(WorkflowSteps::Table)
                        .if_not_exists()
                        .col(
                            ColumnDef::new(WorkflowSteps::Id)
                                .big_integer()
                                .not_null()
                                .auto_increment()
                                .primary_key(),
                        )
                        .col(
                            ColumnDef::new(WorkflowSteps::WorkflowId)
                                .big_integer()
                                .not_null(),
                        )
                        .col(
                            ColumnDef::new(WorkflowSteps::StepIndex)
                                .integer()
                                .not_null(),
                        )
                        .col(ColumnDef::new(WorkflowSteps::StepName).string().not_null())
                        .col(ColumnDef::new(WorkflowSteps::Status).string().not_null())
                        .col(ColumnDef::new(WorkflowSteps::Input).text().not_null())
                        .col(ColumnDef::new(WorkflowSteps::Output).text().null())
                        .col(ColumnDef::new(WorkflowSteps::Error).text().null())
                        .col(ColumnDef::new(WorkflowSteps::Attempts).integer().not_null())
                        .col(
                            ColumnDef::new(WorkflowSteps::CreatedAt)
                                .date_time()
                                .not_null()
                                .default(Expr::current_timestamp()),
                        )
                        .col(
                            ColumnDef::new(WorkflowSteps::UpdatedAt)
                                .date_time()
                                .not_null()
                                .default(Expr::current_timestamp()),
                        )
                        .col(ColumnDef::new(WorkflowSteps::StartedAt).date_time().null())
                        .col(
                            ColumnDef::new(WorkflowSteps::CompletedAt)
                                .date_time()
                                .null(),
                        )
                        .to_owned(),
                )
                .await?;

            manager
                .create_index(
                    Index::create()
                        .name("idx_workflow_steps_workflow_id")
                        .table(WorkflowSteps::Table)
                        .col(WorkflowSteps::WorkflowId)
                        .to_owned(),
                )
                .await?;

            manager
                .create_index(
                    Index::create()
                        .name("idx_workflow_steps_unique")
                        .table(WorkflowSteps::Table)
                        .col(WorkflowSteps::WorkflowId)
                        .col(WorkflowSteps::StepIndex)
                        .unique()
                        .to_owned(),
                )
                .await
        }

        async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
            manager
                .drop_table(Table::drop().table(WorkflowSteps::Table).to_owned())
                .await
        }
    }

    #[derive(DeriveIden)]
    enum Workflows {
        Table,
        Id,
        Name,
        Status,
        Input,
        Output,
        Error,
        Attempts,
        MaxAttempts,
        NextRunAt,
        LockedUntil,
        WorkerId,
        CreatedAt,
        UpdatedAt,
        StartedAt,
        CompletedAt,
    }

    #[derive(DeriveIden)]
    enum WorkflowSteps {
        Table,
        Id,
        WorkflowId,
        StepIndex,
        StepName,
        Status,
        Input,
        Output,
        Error,
        Attempts,
        CreatedAt,
        UpdatedAt,
        StartedAt,
        CompletedAt,
    }

    /// The workflow drain used to await every in-flight step forever, so a
    /// step that never returns held the worker open until SIGKILL. An
    /// abandoned step is safe here - its lease lapses and another worker
    /// reclaims it - but only if the drain actually gives up.
    #[tokio::test]
    async fn the_drain_abandons_steps_that_outlive_the_grace() {
        let mut in_flight: JoinSet<()> = JoinSet::new();
        in_flight.spawn(async {});
        in_flight.spawn(async {
            std::future::pending::<()>().await;
        });

        let started = std::time::Instant::now();
        let abandoned = drain_in_flight(&mut in_flight, Duration::from_millis(150)).await;

        assert_eq!(abandoned, 1, "the hung step must be reported as abandoned");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the drain must return at its deadline, not wait for the hung step"
        );
    }

    #[tokio::test]
    async fn the_drain_returns_as_soon_as_every_step_finishes() {
        let mut in_flight: JoinSet<()> = JoinSet::new();
        for _ in 0..4 {
            in_flight.spawn(async {});
        }

        let started = std::time::Instant::now();
        let abandoned = drain_in_flight(&mut in_flight, Duration::from_secs(30)).await;

        assert_eq!(abandoned, 0);
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
