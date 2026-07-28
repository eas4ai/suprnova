//! Worker registry + dispatch by job_name.
//!
//! Each `Job` impl registers a deserialize-and-run shim keyed by its
//! `job_name`. Drivers call `dispatch_by_name` to run an inbound payload.
//! Re-registering the same name is allowed (last writer wins) — useful
//! for tests; deterministic in production because each Job has exactly
//! one registration site.
//!
//! # At-least-once delivery and job idempotency
//!
//! Redis-backed queue drivers cannot make `nack` atomic — the
//! re-publish (XADD) and ack (XACK) are two separate commands. A
//! crash between them re-delivers the message. The in-memory driver
//! and database driver are exactly-once-per-attempt, but the worker
//! loop itself doesn't distinguish drivers, so **every job handler
//! in a production deployment must be idempotent**.
//!
//! For typical command-style jobs, wrap the handler body in
//! [`Idempotency::once`](crate::idempotency::Idempotency::once) or
//! [`Idempotency::commit_on_success`](crate::idempotency::Idempotency::commit_on_success)
//! keyed by a stable per-operation key (e.g. the entity id or a
//! caller-supplied request id). Without this, a re-delivered job may
//! execute the same side effect twice. When a retry must return the
//! original outcome rather than merely skip re-execution, use
//! [`Idempotency::remember`](crate::idempotency::Idempotency::remember),
//! which records the success value and replays it to later deliveries.

use crate::error::FrameworkError;
use crate::events::EventFacade;
use crate::lock;
use crate::queue::Job;
use crate::queue::batch::resolve_callback;
use crate::queue::chain::ChainLink;
use crate::queue::driver::QueueDriver;
use crate::queue::envelope::Envelope;
use crate::queue::events as queue_events;
use crate::queue::middleware::{JobMiddleware, Next};
use crate::queue::outcome::JobOutcome;
use crate::queue::retry::next_delay;
use crate::telemetry::Metrics;
use chrono::Utc;
use futures::FutureExt;
use futures::future::BoxFuture;
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Counter name for settlement (ack/nack) failures. Operators can alert on a
/// non-zero rate here: a single failure means at-least-once delivery may
/// re-deliver a successful side effect (ack) or lose attempt accounting (nack).
///
/// Emitted with attributes `operation` (`"ack"` | `"nack"`), `driver`
/// (driver type-name from `QueueDriver::name`), `job` (the `Job::job_name`),
/// and `outcome` (`"success"` for a successful run whose ack failed,
/// `"dead_letter"` for a settled-failed job whose ack failed, `"retry"` for
/// a retried-failure whose nack failed, `"timeout_dead_letter"` for a
/// timeout-exhausted ack failure, `"timeout_retry"` for a timeout-nack
/// failure).
const METRIC_SETTLEMENT_FAILURES: &str = "queue.settlement.failures";

type Dispatcher =
    Arc<dyn Fn(serde_json::Value) -> BoxFuture<'static, Result<(), FrameworkError>> + Send + Sync>;

/// Factory that produces the per-job middleware stack each time a job is
/// dispatched. Middleware can hold per-instance state (lock keys, throttle
/// keys), so we call the factory once per pop rather than caching the
/// stack across runs.
type MiddlewareFactory = Arc<dyn Fn() -> Vec<Arc<dyn JobMiddleware>> + Send + Sync>;

struct Registration {
    dispatcher: Dispatcher,
    middleware: MiddlewareFactory,
}

static REGISTRY: RwLock<Option<HashMap<String, Registration>>> = RwLock::new(None);

/// Register `J` so the worker can dispatch envelopes carrying its
/// `job_name`. Last-write-wins; re-registering the same name replaces
/// the prior dispatcher and emits a `warn` trace event.
pub fn register_job<J: Job>() {
    let dispatcher: Dispatcher = Arc::new(|payload: serde_json::Value| {
        Box::pin(async move {
            let job: J = serde_json::from_value(payload)
                .map_err(|e| FrameworkError::internal(format!("decode job: {e}")))?;
            job.handle().await
        })
    });
    let middleware: MiddlewareFactory = Arc::new(|| J::middleware());
    // Hot-path registry: recover in place on poison so a panic in any
    // other job's registration doesn't kill the inventory-drain at
    // process boot. The critical section is a single HashMap insert.
    let mut g = REGISTRY.write().unwrap_or_else(|e| e.into_inner());
    let name = J::job_name();
    let map = g.get_or_insert_with(HashMap::new);
    if map
        .insert(
            name.to_string(),
            Registration {
                dispatcher,
                middleware,
            },
        )
        .is_some()
    {
        // Keep last-writer-wins (tests rely on re-registration) but make it
        // observable: silently rerouting in-flight messages is a foot-gun in
        // production where the same `job_name` should have exactly one
        // registration site.
        tracing::warn!(
            job = name,
            "register_job replaced an existing dispatcher for this job_name; \
             duplicate registration may indicate inventory + manual registration \
             of the same job (last writer wins)"
        );
    }
}

/// Look up the dispatcher registered under `name` and run it against
/// `payload`. Returns `Err` if no job is registered under that name.
pub async fn dispatch_by_name(
    name: &str,
    payload: serde_json::Value,
) -> Result<(), FrameworkError> {
    let dispatcher = {
        let g = lock::read(&REGISTRY, "queue job registry")?;
        let map = g
            .as_ref()
            .ok_or_else(|| FrameworkError::internal(format!("unknown job: {name}")))?;
        map.get(name)
            .map(|r| r.dispatcher.clone())
            .ok_or_else(|| FrameworkError::internal(format!("unknown job: {name}")))?
    };
    dispatcher(payload).await
}

/// Look up the middleware factory for a job name. Returns an empty list
/// for unregistered jobs (the dispatcher itself will error in that case).
fn middleware_for(name: &str) -> Vec<Arc<dyn JobMiddleware>> {
    let g = match lock::read(&REGISTRY, "queue job registry") {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    g.as_ref()
        .and_then(|m| m.get(name).map(|r| (r.middleware)()))
        .unwrap_or_default()
}

/// Run the middleware pipeline ending in the raw dispatcher. Returns the
/// terminal [`JobOutcome`] OR a handler error (which the worker translates
/// into retry / dead-letter).
///
/// Exposed for test harnesses that want to settle one envelope without
/// running the full worker loop; production code goes through
/// [`run_worker`].
pub async fn run_through_middleware(env: Envelope) -> Result<JobOutcome, FrameworkError> {
    let job_name = env.job_name.clone();
    let mw_stack = middleware_for(&job_name);
    // Build the innermost layer: actually dispatch the job, lift result
    // into JobOutcome::Completed.
    let innermost: Next = Box::new(move |env: Envelope| {
        Box::pin(async move {
            let payload = env.payload.clone();
            dispatch_by_name(&env.job_name, payload).await?;
            Ok(JobOutcome::Completed)
        })
    });

    // Fold middleware in reverse so the first entry runs outermost.
    let chained =
        mw_stack
            .into_iter()
            .rev()
            .fold(innermost, |next: Next, mw: Arc<dyn JobMiddleware>| {
                Box::new(move |env: Envelope| {
                    let mw = mw.clone();
                    Box::pin(async move { mw.handle(env, next).await })
                })
            });

    chained(env).await
}

/// Return all registered job names. Used by admin inspectors and
/// `cargo run --bin app -- jobs:list` (Phase 6B).
pub fn registered_job_names() -> Vec<String> {
    REGISTRY
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|m| {
            let mut v: Vec<_> = m.keys().cloned().collect();
            v.sort();
            v
        })
        .unwrap_or_default()
}

// ============================================================================
// Worker loop (Task 8)
// ============================================================================

/// Runtime tuning for [`run_worker`].
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// How long a reservation stays held before another worker may re-claim
    /// the envelope. Drivers that lack lease semantics ignore this.
    pub visibility_timeout: Duration,
    /// Sleep duration when the driver returns no envelope on a poll.
    pub poll_interval: Duration,
    /// Optional hard cap on jobs processed by this worker before it exits
    /// cleanly. `None` runs until cancelled. Used by `queue:work --max-jobs N`
    /// for periodic restart strategies (e.g. release-on-restart deploys).
    pub max_jobs: Option<u64>,
    /// Queues this worker drains. Empty (the default) drains every queue,
    /// which is the behaviour of every worker started before routing existed.
    ///
    /// Set from `queue:work --queue=billing,default` to dedicate a pool to
    /// specific work. A job with no route counts as
    /// [`DEFAULT_QUEUE`](crate::queue::envelope::DEFAULT_QUEUE), so
    /// `--queue=default` still drains unrouted jobs.
    ///
    /// Drivers that cannot filter reject a non-empty value at the first poll
    /// rather than silently draining everything — see
    /// [`QueueDriver::pop_from`](crate::queue::QueueDriver::pop_from).
    pub queues: Vec<String>,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            visibility_timeout: Duration::from_secs(60),
            poll_interval: Duration::from_millis(100),
            max_jobs: None,
            queues: Vec::new(),
        }
    }
}

/// One job's terminal state for the worker's settlement match.
///
/// Carries the dispatch result by type, not by string-matching the error
/// message: a job whose own failure body legitimately contains the words
/// "timed out after" can no longer be misclassified, and a real timeout
/// is observable without parsing.
enum DispatchOutcome {
    /// Middleware pipeline returned a typed outcome.
    Settled(JobOutcome),
    /// Handler returned `Err(...)` and middleware didn't convert it.
    /// Worker decides retry vs dead-letter from `attempts`/`max_tries`.
    Failed(FrameworkError),
    /// Dispatch exceeded the per-job timeout budget.
    TimedOut(Duration),
}

/// Pull-loop worker: pops one reservation at a time, dispatches by job_name,
/// acks on success, requeues with backoff on failure, drops after max_tries.
///
/// The worker bumps `env.attempts` locally before dispatch. The memory driver's
/// `nack` also bumps `attempts` on its stored copy so the next `pop` returns
/// the correct incremented count (preventing the worker from treating every
/// retry as attempt 1).
///
/// Returns when `shutdown` is cancelled or when `cfg.max_jobs` is reached.
/// A cancel signal interrupts pop polling but never an in-flight handler:
/// a job that's already been popped is allowed to finish (bounded by its
/// own per-job `timeout()` if set) before the worker exits, so in-flight
/// side effects don't get torn mid-stride. Designed to run under
/// `tokio::spawn`.
pub async fn run_worker(
    driver: Arc<dyn QueueDriver>,
    cfg: WorkerConfig,
    shutdown: CancellationToken,
) {
    let connection = crate::queue::Queue::connection_name();
    let worker_started_at = Utc::now().timestamp_millis();
    let _ = EventFacade::dispatch(queue_events::WorkerStarting {
        connection: connection.clone(),
    })
    .await;

    let mut processed: u64 = 0;
    let exit_with = |reason: &'static str, processed: u64, connection: &str| {
        tracing::info!(
            reason,
            processed,
            connection = connection,
            "queue worker exiting"
        );
    };

    let result = loop {
        // Stop accepting new work the moment shutdown fires; the current
        // in-flight job (if any) has already been popped above and will run
        // to completion below before the next iteration sees the cancel.
        if shutdown.is_cancelled() {
            exit_with("cancelled", processed, &connection);
            break ExitReason::Cancelled;
        }
        if let Some(max) = cfg.max_jobs
            && processed >= max
        {
            tracing::info!(
                processed,
                max_jobs = max,
                "queue worker reached max_jobs, exiting cleanly"
            );
            break ExitReason::MaxJobs;
        }
        if let Ok(Some(ts)) = crate::queue::Queue::restart_signal().await
            && ts > worker_started_at
        {
            tracing::info!(
                processed,
                "queue worker received restart signal, exiting cleanly"
            );
            let _ = EventFacade::dispatch(queue_events::WorkerInterrupted {
                connection: connection.clone(),
                processed,
            })
            .await;
            break ExitReason::Restart;
        }

        // Emit per-iteration Looping event before the pop so listeners
        // see the cadence even on empty queues.
        let _ = EventFacade::dispatch(queue_events::Looping {
            connection: connection.clone(),
        })
        .await;

        // Pop OR cancel — whichever happens first. `biased` makes cancel win
        // a tie so a queue under load can still exit promptly.
        let popped = tokio::select! {
            biased;
            _ = shutdown.cancelled() => {
                exit_with("cancelled", processed, &connection);
                break ExitReason::Cancelled;
            }
            res = driver.pop_from(cfg.visibility_timeout, &cfg.queues) => res,
        };

        let popped = match popped {
            Ok(opt) => opt,
            Err(e) => {
                tracing::error!(error = %e, driver = driver.name(), "queue pop failed");
                tokio::select! {
                    _ = shutdown.cancelled() => {
                        exit_with("cancelled", processed, &connection);
                        break ExitReason::Cancelled;
                    }
                    _ = tokio::time::sleep(cfg.poll_interval) => {}
                }
                continue;
            }
        };
        let Some(res) = popped else {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    exit_with("cancelled", processed, &connection);
                    break ExitReason::Cancelled;
                }
                _ = tokio::time::sleep(cfg.poll_interval) => {}
            }
            continue;
        };

        let mut env = res.envelope;
        env.attempts += 1;
        let identity_pre = queue_events::JobIdentity::from_env(&env, &connection);
        let _ = EventFacade::dispatch(queue_events::JobProcessing {
            job: identity_pre.clone(),
        })
        .await;

        let timeout_opt = env.timeout_secs.map(Duration::from_secs);
        let env_for_dispatch = env.clone();
        // Wrap dispatch in a panic boundary so a panicking handler (or panicking
        // middleware) is converted to a `DispatchOutcome::Failed` and flows
        // through the existing retry / dead-letter path. Without the boundary,
        // a panic would unwind out of `run_worker`, kill the worker task, and
        // strand the envelope's reservation until visibility expiry.
        let dispatch_fut =
            AssertUnwindSafe(run_through_middleware(env_for_dispatch)).catch_unwind();

        let outcome = match timeout_opt {
            Some(t) => match tokio::time::timeout(t, dispatch_fut).await {
                Ok(Ok(Ok(o))) => DispatchOutcome::Settled(o),
                Ok(Ok(Err(e))) => DispatchOutcome::Failed(e),
                Ok(Err(panic_payload)) => {
                    DispatchOutcome::Failed(FrameworkError::internal(format!(
                        "job panicked: {}",
                        crate::server::panic_payload_message(&panic_payload)
                    )))
                }
                Err(_elapsed) => DispatchOutcome::TimedOut(t),
            },
            None => match dispatch_fut.await {
                Ok(Ok(o)) => DispatchOutcome::Settled(o),
                Ok(Err(e)) => DispatchOutcome::Failed(e),
                Err(panic_payload) => DispatchOutcome::Failed(FrameworkError::internal(format!(
                    "job panicked: {}",
                    crate::server::panic_payload_message(&panic_payload)
                ))),
            },
        };

        // Resolve the process-global settlement registries once, at settlement
        // time, so every arm below sees the same wiring and tests can drive the
        // settlement helpers with explicit fakes instead of mutating globals.
        let deps = SettlementDeps::current();

        match outcome {
            DispatchOutcome::Settled(JobOutcome::Completed) => {
                handle_completed(&*driver, &res.token, &env, &connection, &deps).await;
            }
            DispatchOutcome::Settled(JobOutcome::Released { delay }) => {
                handle_released(
                    &*driver,
                    &res.token,
                    &mut env,
                    delay,
                    &connection,
                    "middleware",
                )
                .await;
            }
            DispatchOutcome::Settled(JobOutcome::Failed { reason }) => {
                handle_dead_letter(
                    &*driver,
                    &res.token,
                    &env,
                    &connection,
                    &reason,
                    false,
                    &deps,
                )
                .await;
            }
            DispatchOutcome::Settled(JobOutcome::Deleted) => {
                // Middleware decided to drop the job without dead-letter.
                //
                // If this envelope belonged to a batch, the batch's
                // pending_jobs still has to decrement so callbacks can
                // fire. The batch saw the job; the batch must see it
                // settled, even if its handler never ran. Without this,
                // `SkipIfBatchCancelled` would leave a cancelled batch
                // stuck with pending_jobs > 0 forever.
                //
                // DATA-02a: that decrement runs BEFORE the ack, for the same
                // reason documented on [`handle_completed`] — acking first
                // makes a crash in the window drop the reservation with the
                // decrement never applied, and a batch stuck on a non-zero
                // pending count has no recovery path.
                if let Some(batch_id) = env.batch_id.as_deref()
                    && let Some(repo) = deps.batches.as_ref()
                {
                    let counts = repo.record_successful_job(batch_id, env.id).await;
                    if let Ok(c) = counts
                        && c.pending_jobs == 0
                        && let Ok(Some(b)) = repo.find(batch_id).await
                    {
                        let _ = repo.mark_finished(batch_id).await;
                        let phase = terminal_batch_phase(&b);
                        fire_batch_callbacks(&b, phase).await;
                        fire_batch_callbacks(&b, BatchPhase::Finally).await;
                    }
                }

                if let Err(e) = driver.ack(&res.token).await {
                    settlement_failure(&*driver, &env, "ack", "deleted", &e);
                }
                tracing::debug!(job = %env.job_name, id = %env.id, "queue job dropped by middleware");
            }
            DispatchOutcome::Failed(e) => {
                if env.attempts >= env.max_tries {
                    handle_dead_letter(
                        &*driver,
                        &res.token,
                        &env,
                        &connection,
                        &e.to_string(),
                        false,
                        &deps,
                    )
                    .await;
                } else {
                    let _ = EventFacade::dispatch(queue_events::JobExceptionOccurred {
                        job: identity_pre.clone(),
                        exception: e.to_string(),
                    })
                    .await;
                    let delay = next_delay(&env.backoff, env.attempts, None);
                    tracing::warn!(
                        job = %env.job_name,
                        id = %env.id,
                        attempt = env.attempts,
                        retry_in = ?delay,
                        error = %e,
                        "queue job failed, will retry"
                    );
                    if let Err(nack_err) = driver.nack(&res.token, delay).await {
                        settlement_failure(&*driver, &env, "nack", "retry", &nack_err);
                    } else {
                        let _ = EventFacade::dispatch(queue_events::JobReleasedAfterException {
                            job: identity_pre.clone(),
                            exception: e.to_string(),
                            delay_secs: delay.as_secs(),
                        })
                        .await;
                    }
                }
            }
            DispatchOutcome::TimedOut(t) => {
                let _ = EventFacade::dispatch(queue_events::JobTimedOut {
                    job: identity_pre.clone(),
                    timeout: t,
                })
                .await;
                let exhausted = env.fail_on_timeout || env.attempts >= env.max_tries;
                if exhausted {
                    let reason = format!(
                        "job exceeded per-attempt timeout of {} seconds",
                        t.as_secs()
                    );
                    handle_dead_letter(
                        &*driver,
                        &res.token,
                        &env,
                        &connection,
                        &reason,
                        true,
                        &deps,
                    )
                    .await;
                } else {
                    let delay = next_delay(&env.backoff, env.attempts, None);
                    tracing::warn!(
                        job = %env.job_name,
                        id = %env.id,
                        attempt = env.attempts,
                        retry_in = ?delay,
                        timeout_secs = t.as_secs(),
                        "queue job timed out, will retry"
                    );
                    if let Err(nack_err) = driver.nack(&res.token, delay).await {
                        settlement_failure(&*driver, &env, "nack", "timeout_retry", &nack_err);
                    }
                }
            }
        }

        // One settlement = one processed job for the max_jobs cap, regardless
        // of outcome (success/failure/timeout). Settlement-failure logging
        // above is separate from this accounting.
        processed = processed.saturating_add(1);
    };

    let _ = EventFacade::dispatch(queue_events::WorkerStopping {
        connection: connection.clone(),
        processed,
    })
    .await;
    let _ = result;
}

#[derive(Debug)]
enum ExitReason {
    Cancelled,
    MaxJobs,
    Restart,
}

/// The process-global registries the settlement path consults, resolved once
/// per settled job.
///
/// Bundled rather than looked up inline so the settlement helpers can be
/// driven in tests with explicit fakes. Installing a fake into
/// [`crate::queue::failed`]'s or [`crate::queue::batch`]'s global slot would
/// leak into every other test sharing the `--lib` test binary, and the
/// resulting order-dependent failures are exactly what an ordering fix must
/// not introduce.
struct SettlementDeps {
    failed_store: Option<Arc<dyn crate::queue::failed::FailedJobStore>>,
    batches: Option<Arc<dyn crate::queue::batch::BatchRepository>>,
}

impl SettlementDeps {
    /// Snapshot whatever is installed right now.
    fn current() -> Self {
        Self {
            failed_store: crate::queue::failed::current(),
            batches: crate::queue::batch::current_repository(),
        }
    }
}

/// Settle a successful run: chain link first, batch accounting next, ack last.
///
/// # Why the ack goes last (DATA-02a)
///
/// Acking first drops the reservation while the follow-up is still unwritten.
/// A crash in that window — and a rolling restart samples it once per in-flight
/// job, so it is not theoretical — leaves the job gone from the queue with its
/// successor never enqueued. The chain then stalls permanently: nothing is left
/// in the queue to retry from, and no operator action recovers it.
///
/// Ordering the push before the ack converts that silent permanent loss into a
/// detectable duplicate. The reservation stays live, visibility expiry
/// redelivers the envelope, and the handler runs a second time. That trade is
/// deliberate and it is safe because duplicate execution is already the
/// framework's delivery contract — see the module header: every production
/// handler must be idempotent, because Redis-backed drivers cannot make `nack`
/// atomic either. When the duplication is caused by a failing `ack` it is also
/// counted by [`METRIC_SETTLEMENT_FAILURES`], so operators can alert on the
/// rate; when it is caused by a failing push it is logged at ERROR with the
/// driver, job and envelope id. Silent loss has neither.
///
/// This is not the fully atomic version. A settlement that is transactional
/// with the follow-up (outbox pattern) removes the duplicate window entirely
/// and is planned for v0.8.0; until then this ordering is the safe half of the
/// trade.
///
/// # Why the chain push blocks the ack but batch accounting does not
///
/// A failed chain push is transient — a driver or network fault worth
/// redelivering for — so it returns early WITHOUT acking, exactly like
/// [`handle_released`], and the original is redelivered on visibility expiry.
///
/// A batch repository error is frequently *permanent*:
/// [`PendingBatch::dispatch`](crate::queue::batch::PendingBatch::dispatch)
/// deletes the batch row when a mid-loop push fails, and the envelopes that
/// already landed then get `Err(batch not found)` forever. Refusing to ack on
/// that would spin those orphans on visibility expiry with no exit. So the
/// batch step runs before the ack (a crash replays it rather than losing it)
/// but its error does not hold the reservation.
async fn handle_completed(
    driver: &dyn QueueDriver,
    token: &crate::queue::driver::ReservationToken,
    env: &Envelope,
    connection: &str,
    deps: &SettlementDeps,
) {
    // 1. Dispatch next link in chain (if any) onto the SAME driver that
    // settled this job. The worker is bound to a specific
    // `Arc<dyn QueueDriver>` at `run_worker(driver, ...)`; resolving
    // through `current_driver()` would re-pick whichever driver is
    // registered globally, which differs from the bound one under
    // multi-connection setups (e.g. one worker per connection) and
    // would silently land the next link on the wrong queue.
    if !env.chain_remaining.is_empty() {
        let mut tail = env.chain_remaining.clone();
        let next: ChainLink = tail.remove(0);
        let mut next_env = next.to_envelope();
        next_env.chain_remaining = tail;
        next_env.batch_id = env.batch_id.clone();
        if let Err(e) = driver.push(next_env).await {
            tracing::error!(
                job = %env.job_name,
                id = %env.id,
                driver = driver.name(),
                error = %e,
                "queue chain: next link push failed; reservation left intact for \
                 visibility-expiry redelivery"
            );
            return;
        }
    }

    // 2. Notify batch repository (best-effort — see the doc comment).
    if let Some(batch_id) = env.batch_id.as_deref()
        && let Some(repo) = deps.batches.as_ref()
    {
        let counts = repo.record_successful_job(batch_id, env.id).await;
        if let Ok(c) = counts
            && c.pending_jobs == 0
        {
            let _ = repo.mark_finished(batch_id).await;
            if let Ok(Some(b)) = repo.find(batch_id).await {
                let phase = terminal_batch_phase(&b);
                fire_batch_callbacks(&b, phase).await;
                fire_batch_callbacks(&b, BatchPhase::Finally).await;
            }
        }
    }

    // 3. Only now drop the reservation.
    if let Err(e) = driver.ack(token).await {
        settlement_failure(driver, env, "ack", "success", &e);
    } else {
        tracing::debug!(job = %env.job_name, id = %env.id, "queue job ok");
    }

    // 4. Observation only — these carry no recovery value, so they run after
    // the reservation is settled and never gate it.
    let _ = EventFacade::dispatch(queue_events::JobProcessed {
        job: queue_events::JobIdentity::from_env(env, connection),
    })
    .await;
    let _ = EventFacade::dispatch(queue_events::JobAttempted {
        job: queue_events::JobIdentity::from_env(env, connection),
    })
    .await;
}

async fn handle_released(
    driver: &dyn QueueDriver,
    token: &crate::queue::driver::ReservationToken,
    env: &mut Envelope,
    delay: Duration,
    connection: &str,
    reason: &str,
) {
    // Released means "try again WITHOUT burning an attempt". A naive
    // `driver.nack(token, delay)` would re-publish the driver's stored
    // copy with `attempts += 1` (per the trait contract), defeating the
    // purpose. So instead:
    //   1. Decrement the local copy back to its pre-dispatch attempt count.
    //   2. PUSH the local copy with `available_at` shifted by `delay`.
    //   3. ACK the original reservation (drop the driver's copy) — only
    //      after the push succeeds.
    // Push-before-ack keeps the released job safe across every failure mode.
    // A push `Err` returns early WITHOUT acking, so the reservation stays
    // live and the original is redelivered on visibility expiry — the job is
    // never lost. A crash between push and ack leaves both copies, yielding a
    // benign at-least-once duplicate (deduped downstream via `env.id`), which
    // for a release (lock busy, throttle exceeded) just produces another
    // release attempt and is strictly better than dropping the job.
    env.attempts = env.attempts.saturating_sub(1);
    let new_available = Utc::now()
        + match chrono::Duration::from_std(delay) {
            Ok(d) => d,
            Err(_) => chrono::Duration::seconds(0),
        };
    env.available_at = new_available;
    if let Err(e) = driver.push(env.clone()).await {
        tracing::error!(
            job = %env.job_name,
            id = %env.id,
            driver = driver.name(),
            error = %e,
            "queue released-push failed; reservation left intact for visibility-expiry redelivery"
        );
        return;
    }
    if let Err(e) = driver.ack(token).await {
        settlement_failure(driver, env, "ack", "released", &e);
        return;
    }
    let _ = EventFacade::dispatch(queue_events::JobReleased {
        job: queue_events::JobIdentity::from_env(env, connection),
        delay_secs: delay.as_secs(),
        reason: reason.into(),
    })
    .await;
    tracing::debug!(
        job = %env.job_name,
        id = %env.id,
        retry_in = ?delay,
        "queue job released without burning attempt"
    );
}

/// Settle a terminally-failed run: failed-jobs record first, batch accounting
/// next, ack last.
///
/// # Why the ack goes last (DATA-02a)
///
/// Same discipline as [`handle_completed`], and here the stakes are higher: the
/// failed-jobs record *is* the recovery path. `queue:retry` re-pushes the
/// envelope stored by [`FailedJobStore::log`](crate::queue::FailedJobStore::log),
/// so acking first and crashing before the write leaves the envelope in neither
/// the queue nor the failed store — permanently and silently gone, with no
/// operator action that brings it back.
///
/// Writing the record before dropping the reservation trades that away for a
/// duplicate: a failing write returns early WITHOUT acking, visibility expiry
/// redelivers the envelope, the handler runs (and presumably fails) again, and
/// the write is retried. Duplicate execution is already the framework's
/// documented delivery contract (see the module header), and the failure is
/// visible — an ERROR log per cycle carrying driver, job and envelope id, plus
/// [`METRIC_SETTLEMENT_FAILURES`] whenever the duplication comes from a failing
/// `ack` rather than a failing write.
///
/// **Operator note:** a store that fails *permanently* — a
/// [`DatabaseFailedJobStore`](crate::queue::DatabaseFailedJobStore) pointed at
/// a missing or unmigrated `failed_jobs` table — now recycles dead-lettered
/// jobs on visibility expiry instead of discarding them. That is intentional:
/// a misconfigured failure store should be loud rather than quietly eat every
/// dead letter. Install
/// [`NullFailedJobStore`](crate::queue::NullFailedJobStore) to opt out of
/// retention deliberately; it accepts every record, so it never blocks an ack.
///
/// Batch accounting stays best-effort for the reason given on
/// [`handle_completed`]: a deleted batch row returns a permanent error, and
/// gating the ack on it would strand the orphaned envelopes forever.
/// Which callback a batch fires once its last job settles.
///
/// `Then` means "the whole batch succeeded", so it is only correct when
/// nothing failed *and* nobody cancelled the batch. Every settlement path
/// that can drive `pending_jobs` to zero has to agree on this, which is
/// exactly what went wrong before it was a shared function: the
/// `JobOutcome::Deleted` arm branched on `failed_jobs > 0` alone and fired
/// `Then` for a cancelled batch — despite a comment on the other copy
/// saying the paths must agree. That arm is the likeliest way to reach the
/// case, too, since `SkipIfBatchCancelled` settles every remaining job of a
/// cancelled batch as `Deleted`, leaving `failed_jobs` at zero while the
/// last one drives pending to zero.
///
/// Keep this the single source of truth; do not re-inline the condition.
fn terminal_batch_phase(batch: &crate::queue::batch::Batch) -> BatchPhase {
    if batch.failed_jobs > 0 || batch.cancelled() {
        BatchPhase::Catch
    } else {
        BatchPhase::Then
    }
}

async fn handle_dead_letter(
    driver: &dyn QueueDriver,
    token: &crate::queue::driver::ReservationToken,
    env: &Envelope,
    connection: &str,
    reason: &str,
    is_timeout: bool,
    deps: &SettlementDeps,
) {
    tracing::error!(
        job = %env.job_name,
        id = %env.id,
        attempts = env.attempts,
        reason = %reason,
        "queue job dead-lettered"
    );

    // 1. Persist to failed-jobs store. The queue recorded is the one the
    // envelope actually died on — `queue:retry` re-pushes the stored
    // envelope, and an operator triaging a dedicated pool filters failed
    // jobs by this column, so writing "default" for a routed job would
    // hide its failures from the very pool that owns them.
    if let Some(store) = deps.failed_store.as_ref()
        && let Err(e) = store
            .log(
                connection,
                env.queue
                    .as_deref()
                    .unwrap_or(crate::queue::envelope::DEFAULT_QUEUE),
                env,
                reason,
            )
            .await
    {
        tracing::error!(
            job = %env.job_name,
            id = %env.id,
            driver = driver.name(),
            error = %e,
            "queue failed-jobs store rejected the record; reservation left intact \
             for visibility-expiry redelivery"
        );
        return;
    }

    // 2. Notify batch repository of failure (and cancel if !allow_failures).
    // Best-effort — see the doc comment.
    if let Some(batch_id) = env.batch_id.as_deref()
        && let Some(repo) = deps.batches.as_ref()
    {
        let counts = repo.record_failed_job(batch_id, env.id).await;
        if let Ok(c) = counts {
            // Cancel-on-first-failure unless allow_failures is set.
            if let Ok(Some(b)) = repo.find(batch_id).await {
                if !b.options.allow_failures {
                    let _ = repo.cancel(batch_id).await;
                }
                if c.pending_jobs == 0 {
                    let _ = repo.mark_finished(batch_id).await;
                    fire_batch_callbacks(&b, BatchPhase::Catch).await;
                    fire_batch_callbacks(&b, BatchPhase::Finally).await;
                }
            }
        }
    }

    // 3. Only now drop the reservation.
    if let Err(ack_err) = driver.ack(token).await {
        let outcome = if is_timeout {
            "timeout_dead_letter"
        } else {
            "dead_letter"
        };
        settlement_failure(driver, env, "ack", outcome, &ack_err);
    }

    // 4. Observation only — never gates the settlement.
    let _ = EventFacade::dispatch(queue_events::JobFailed {
        job: queue_events::JobIdentity::from_env(env, connection),
        exception: reason.to_string(),
    })
    .await;
}

fn settlement_failure(
    driver: &dyn QueueDriver,
    env: &Envelope,
    operation: &'static str,
    outcome: &'static str,
    err: &FrameworkError,
) {
    let msg = match (operation, outcome) {
        ("ack", "success") => {
            "queue ack failed after successful run; \
             job may be re-delivered (at-least-once)"
        }
        ("ack", "dead_letter") => {
            "queue ack failed for dead-lettered job; \
             reservation may stay until visibility expiry"
        }
        ("ack", "timeout_dead_letter") => {
            "queue ack failed for timed-out dead-lettered job; \
             reservation may stay until visibility expiry"
        }
        ("ack", "deleted") => {
            "queue ack failed for middleware-dropped job; \
             reservation may stay until visibility expiry"
        }
        ("nack", "retry") => {
            "queue nack failed; reservation may be redelivered \
             after visibility expiry without bumped attempts"
        }
        ("nack", "timeout_retry") => {
            "queue nack failed after timeout; reservation may be \
             redelivered after visibility expiry without bumped attempts"
        }
        ("nack", "released") => {
            "queue nack failed for released job; \
             reservation may be redelivered after visibility expiry"
        }
        _ => "queue settlement failed",
    };
    tracing::error!(
        job = %env.job_name,
        id = %env.id,
        driver = driver.name(),
        error = %err,
        operation,
        outcome,
        "{msg}"
    );
    Metrics::counter(METRIC_SETTLEMENT_FAILURES).inc_with(&[
        ("operation", operation),
        ("driver", driver.name()),
        ("job", env.job_name.as_str()),
        ("outcome", outcome),
    ]);
}

// `PartialEq`/`Debug` so `terminal_batch_phase` can be asserted on directly
// — the phase choice is a correctness rule (a cancelled batch must never
// report success), and asserting it needs the value, not a side effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchPhase {
    Then,
    Catch,
    Finally,
}

async fn fire_batch_callbacks(batch: &crate::queue::batch::Batch, phase: BatchPhase) {
    let names = match phase {
        BatchPhase::Then => &batch.options.then_callbacks,
        BatchPhase::Catch => &batch.options.catch_callbacks,
        BatchPhase::Finally => &batch.options.finally_callbacks,
    };
    let error = if matches!(phase, BatchPhase::Catch) {
        Some("one or more jobs in the batch failed".to_string())
    } else {
        None
    };
    for name in names {
        if let Some(cb) = resolve_callback(name) {
            if let Err(e) = cb.handle(batch.clone(), error.clone()).await {
                tracing::error!(
                    batch = %batch.id,
                    callback = name,
                    error = %e,
                    "batch callback returned an error"
                );
            }
        } else {
            tracing::warn!(
                batch = %batch.id,
                callback = name,
                "batch callback name has no registered handler"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::BackoffSchedule;
    use crate::queue::CURRENT_SCHEMA_VERSION;
    use crate::queue::batch::{
        Batch, BatchOptions, BatchRepository, MemoryBatchRepository, UpdatedBatchJobCounts,
    };
    use crate::queue::driver::{Reservation, ReservationToken};
    use crate::queue::failed::{FailedJob, FailedJobStore};
    use async_trait::async_trait;
    use chrono::DateTime;
    use std::sync::Mutex;
    use uuid::Uuid;

    /// Ordered record of every settlement-visible operation, shared by all
    /// the fakes in one test.
    ///
    /// Ordering is the whole point of DATA-02a, and a per-fake call counter
    /// cannot express "the follow-up landed BEFORE the ack" when the
    /// follow-up and the ack live on different objects. One shared log can.
    #[derive(Default)]
    struct OpLog(Mutex<Vec<&'static str>>);

    impl OpLog {
        fn record(&self, op: &'static str) {
            self.0.lock().unwrap_or_else(|e| e.into_inner()).push(op);
        }

        fn ops(&self) -> Vec<&'static str> {
            self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
        }

        fn count(&self, op: &str) -> usize {
            self.ops().iter().filter(|o| **o == op).count()
        }
    }

    /// Records the ack/push calls the settlement helpers make, with a knob to
    /// fail `push` so we can prove the reservation is left intact (the job
    /// survives) when the re-enqueue cannot land.
    struct RecordingDriver {
        push_fails: bool,
        ops: Arc<OpLog>,
        pushed: Mutex<Vec<Envelope>>,
    }

    impl RecordingDriver {
        fn new(push_fails: bool) -> Self {
            Self::with_log(push_fails, Arc::new(OpLog::default()))
        }

        fn with_log(push_fails: bool, ops: Arc<OpLog>) -> Self {
            Self {
                push_fails,
                ops,
                pushed: Mutex::new(Vec::new()),
            }
        }

        fn ack_count(&self) -> usize {
            self.ops.count("ack")
        }

        fn push_count(&self) -> usize {
            self.ops.count("push")
        }
    }

    #[async_trait]
    impl QueueDriver for RecordingDriver {
        async fn push(&self, env: Envelope) -> Result<(), FrameworkError> {
            self.ops.record("push");
            if self.push_fails {
                return Err(FrameworkError::internal("push exploded"));
            }
            self.pushed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(env);
            Ok(())
        }

        async fn pop(
            &self,
            _visibility_timeout: Duration,
        ) -> Result<Option<Reservation>, FrameworkError> {
            Ok(None)
        }

        async fn ack(&self, _token: &ReservationToken) -> Result<(), FrameworkError> {
            self.ops.record("ack");
            Ok(())
        }

        async fn nack(
            &self,
            _token: &ReservationToken,
            _requeue_delay: Duration,
        ) -> Result<(), FrameworkError> {
            self.ops.record("nack");
            Ok(())
        }
    }

    /// Failed-jobs store that logs into the shared [`OpLog`] and can be made
    /// to reject every record, standing in for an unmigrated `failed_jobs`
    /// table.
    struct RecordingFailedStore {
        ops: Arc<OpLog>,
        fails: bool,
        /// `(queue, job_name, exception)` per accepted record.
        records: Mutex<Vec<(String, String, String)>>,
    }

    impl RecordingFailedStore {
        fn new(ops: Arc<OpLog>, fails: bool) -> Self {
            Self {
                ops,
                fails,
                records: Mutex::new(Vec::new()),
            }
        }

        fn records(&self) -> Vec<(String, String, String)> {
            self.records
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        }
    }

    #[async_trait]
    impl FailedJobStore for RecordingFailedStore {
        async fn log(
            &self,
            _connection: &str,
            queue: &str,
            env: &Envelope,
            exception: &str,
        ) -> Result<Uuid, FrameworkError> {
            self.ops.record("failed_store.log");
            if self.fails {
                return Err(FrameworkError::internal("no such table: failed_jobs"));
            }
            self.records
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((
                    queue.to_string(),
                    env.job_name.clone(),
                    exception.to_string(),
                ));
            Ok(Uuid::new_v4())
        }

        async fn all(&self) -> Result<Vec<FailedJob>, FrameworkError> {
            Ok(Vec::new())
        }
        async fn ids(&self) -> Result<Vec<Uuid>, FrameworkError> {
            Ok(Vec::new())
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

    /// Real [`MemoryBatchRepository`] behaviour plus shared-log recording and
    /// a knob that makes the two settlement writes fail — the shape of a
    /// batch row deleted by `PendingBatch::dispatch`'s rollback, which
    /// returns `Err(batch not found)` for every envelope that already landed.
    struct RecordingBatchRepo {
        inner: MemoryBatchRepository,
        ops: Arc<OpLog>,
        record_fails: bool,
    }

    impl RecordingBatchRepo {
        fn new(ops: Arc<OpLog>, record_fails: bool) -> Self {
            Self {
                inner: MemoryBatchRepository::new(),
                ops,
                record_fails,
            }
        }
    }

    #[async_trait]
    impl BatchRepository for RecordingBatchRepo {
        async fn store(&self, batch: Batch) -> Result<(), FrameworkError> {
            self.inner.store(batch).await
        }
        async fn find(&self, id: &str) -> Result<Option<Batch>, FrameworkError> {
            self.inner.find(id).await
        }
        async fn increment_total_jobs(
            &self,
            id: &str,
            delta: u64,
        ) -> Result<UpdatedBatchJobCounts, FrameworkError> {
            self.inner.increment_total_jobs(id, delta).await
        }
        async fn record_successful_job(
            &self,
            id: &str,
            job_id: Uuid,
        ) -> Result<UpdatedBatchJobCounts, FrameworkError> {
            self.ops.record("batch.record_success");
            if self.record_fails {
                return Err(FrameworkError::internal(format!("batch not found: {id}")));
            }
            self.inner.record_successful_job(id, job_id).await
        }
        async fn record_failed_job(
            &self,
            id: &str,
            job_id: Uuid,
        ) -> Result<UpdatedBatchJobCounts, FrameworkError> {
            self.ops.record("batch.record_failed");
            if self.record_fails {
                return Err(FrameworkError::internal(format!("batch not found: {id}")));
            }
            self.inner.record_failed_job(id, job_id).await
        }
        async fn cancel(&self, id: &str) -> Result<(), FrameworkError> {
            self.inner.cancel(id).await
        }
        async fn is_cancelled(&self, id: &str) -> Result<bool, FrameworkError> {
            self.inner.is_cancelled(id).await
        }
        async fn mark_finished(&self, id: &str) -> Result<(), FrameworkError> {
            self.inner.mark_finished(id).await
        }
        async fn delete(&self, id: &str) -> Result<bool, FrameworkError> {
            self.inner.delete(id).await
        }
    }

    /// No failed-jobs store and no batch repository installed — the wiring a
    /// bare `run_worker` sees before `bootstrap_default`.
    fn no_deps() -> SettlementDeps {
        SettlementDeps {
            failed_store: None,
            batches: None,
        }
    }

    fn chain_link(name: &str) -> ChainLink {
        ChainLink {
            job_name: name.into(),
            payload: serde_json::json!({}),
            max_tries: 3,
            timeout_secs: None,
            fail_on_timeout: false,
            backoff: BackoffSchedule::default(),
            queue: None,
        }
    }

    /// Persist a batch with `total` outstanding jobs and hand back its id.
    async fn seed_batch(repo: &RecordingBatchRepo, total: u64) -> String {
        let id = Uuid::new_v4().to_string();
        repo.store(Batch {
            id: id.clone(),
            name: "settlement".into(),
            total_jobs: total,
            pending_jobs: total,
            failed_jobs: 0,
            failed_job_ids: Vec::new(),
            options: BatchOptions::default(),
            created_at: Utc::now(),
            cancelled_at: None,
            finished_at: None,
        })
        .await
        .expect("seed batch");
        id
    }

    fn batch_with(failed_jobs: u64, cancelled_at: Option<DateTime<Utc>>) -> Batch {
        Batch {
            id: "b-phase".into(),
            name: "phase".into(),
            total_jobs: 1,
            pending_jobs: 0,
            failed_jobs,
            failed_job_ids: Vec::new(),
            options: BatchOptions::default(),
            created_at: Utc::now(),
            cancelled_at,
            finished_at: None,
        }
    }

    /// A cancelled batch must never report success, no matter which
    /// settlement path drove its last job to zero. The `JobOutcome::Deleted`
    /// arm used to branch on `failed_jobs` alone and fired `Then` here —
    /// and `SkipIfBatchCancelled` makes that the *normal* way a cancelled
    /// batch finishes, since it settles every remaining job as `Deleted`
    /// and so leaves `failed_jobs` at zero.
    #[test]
    fn cancelled_batch_finalizes_via_catch_not_then() {
        let cancelled = batch_with(0, Some(Utc::now()));
        assert!(
            cancelled.cancelled(),
            "fixture must actually be cancelled, else this proves nothing"
        );
        assert_eq!(
            terminal_batch_phase(&cancelled),
            BatchPhase::Catch,
            "a cancelled batch fires Catch — Then would tell the caller a \
             batch they cancelled had succeeded"
        );
    }

    #[test]
    fn failed_batch_finalizes_via_catch_and_clean_batch_via_then() {
        assert_eq!(
            terminal_batch_phase(&batch_with(1, None)),
            BatchPhase::Catch,
            "any failed job means the batch did not wholly succeed"
        );
        assert_eq!(
            terminal_batch_phase(&batch_with(0, None)),
            BatchPhase::Then,
            "a clean, uncancelled batch is the only case that earns Then"
        );
    }

    fn fresh_env(name: &str, attempts: u32) -> Envelope {
        Envelope {
            schema_version: CURRENT_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            job_name: name.into(),
            queue: None,
            payload: serde_json::json!({}),
            dispatched_at: Utc::now(),
            available_at: Utc::now(),
            attempts,
            max_tries: 3,
            backoff: BackoffSchedule::default(),
            timeout_secs: None,
            fail_on_timeout: false,
            idempotency_key: None,
            batch_id: None,
            chain_remaining: Vec::new(),
        }
    }

    #[tokio::test]
    async fn released_pushes_before_acking_so_a_failed_push_keeps_the_job() {
        // Push fails: the reservation must NOT be acked, so the original
        // survives for visibility-expiry redelivery rather than being lost.
        let driver = RecordingDriver::new(true);
        let token = ReservationToken(Uuid::new_v4());
        let mut env = fresh_env("J", 1);

        handle_released(
            &driver,
            &token,
            &mut env,
            Duration::from_secs(5),
            "test",
            "middleware",
        )
        .await;

        assert_eq!(
            driver.push_count(),
            1,
            "the released copy is pushed first, before any ack"
        );
        assert_eq!(
            driver.ack_count(),
            0,
            "a failed push must leave the reservation un-acked so the job survives"
        );
    }

    #[tokio::test]
    async fn released_acks_after_a_successful_push() {
        // Push succeeds: both the re-enqueue and the ack run, and the pushed
        // copy carries the decremented attempt count and shifted availability.
        let driver = RecordingDriver::new(false);
        let token = ReservationToken(Uuid::new_v4());
        let mut env = fresh_env("J", 2);
        let before = env.available_at;

        handle_released(
            &driver,
            &token,
            &mut env,
            Duration::from_secs(30),
            "test",
            "middleware",
        )
        .await;

        assert_eq!(driver.push_count(), 1);
        assert_eq!(
            driver.ack_count(),
            1,
            "the original reservation is acked only after the push lands"
        );

        let pushed = driver.pushed.lock().unwrap_or_else(|e| e.into_inner());
        let copy = pushed.first().expect("a released copy was pushed");
        assert_eq!(
            copy.attempts, 1,
            "release does not burn an attempt — the pre-dispatch count is restored"
        );
        assert!(
            copy.available_at > before,
            "the released copy is delayed by the requested duration"
        );
    }

    // ------------------------------------------------------------------
    // DATA-02a: settle before acking
    //
    // Each of these asserts the ORDER of operations, not just that they
    // happened. Reverting `handle_completed` / `handle_dead_letter` to
    // ack-first flips the recorded sequence and fails them.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn completed_pushes_the_chain_link_before_acking() {
        let ops = Arc::new(OpLog::default());
        let driver = RecordingDriver::with_log(false, ops.clone());
        let token = ReservationToken(Uuid::new_v4());
        let mut env = fresh_env("Head", 1);
        env.chain_remaining = vec![chain_link("Tail")];

        handle_completed(&driver, &token, &env, "test", &no_deps()).await;

        assert_eq!(
            ops.ops(),
            vec!["push", "ack"],
            "the successor must be enqueued before the reservation is dropped — \
             acking first means a crash in the window loses the chain forever"
        );
        let pushed = driver.pushed.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            pushed.first().expect("next link pushed").job_name,
            "Tail",
            "the pushed envelope is the next chain link"
        );
    }

    #[tokio::test]
    async fn completed_chain_push_failure_leaves_the_job_redeliverable() {
        // Failure mode: the follow-up cannot land. The reservation must stay
        // live so visibility expiry redelivers the envelope — a duplicate run
        // is recoverable, a dropped chain is not.
        let ops = Arc::new(OpLog::default());
        let driver = RecordingDriver::with_log(true, ops.clone());
        let token = ReservationToken(Uuid::new_v4());
        let mut env = fresh_env("Head", 1);
        env.chain_remaining = vec![chain_link("Tail")];

        handle_completed(&driver, &token, &env, "test", &no_deps()).await;

        assert_eq!(ops.ops(), vec!["push"], "the settlement stops at the push");
        assert_eq!(
            driver.ack_count(),
            0,
            "a failed chain push must leave the reservation un-acked so the job \
             is redelivered rather than silently lost"
        );
    }

    #[tokio::test]
    async fn completed_without_a_chain_still_acks() {
        // The common case must not regress: no follow-up work, one ack.
        let ops = Arc::new(OpLog::default());
        let driver = RecordingDriver::with_log(false, ops.clone());
        let token = ReservationToken(Uuid::new_v4());
        let env = fresh_env("Solo", 1);

        handle_completed(&driver, &token, &env, "test", &no_deps()).await;

        assert_eq!(ops.ops(), vec!["ack"]);
        assert_eq!(driver.push_count(), 0, "no chain, nothing to push");
    }

    #[tokio::test]
    async fn completed_decrements_the_batch_before_acking() {
        let ops = Arc::new(OpLog::default());
        let driver = RecordingDriver::with_log(false, ops.clone());
        let repo = Arc::new(RecordingBatchRepo::new(ops.clone(), false));
        let batch_id = seed_batch(&repo, 2).await;
        let token = ReservationToken(Uuid::new_v4());
        let mut env = fresh_env("Member", 1);
        env.batch_id = Some(batch_id.clone());

        handle_completed(
            &driver,
            &token,
            &env,
            "test",
            &SettlementDeps {
                failed_store: None,
                batches: Some(repo.clone()),
            },
        )
        .await;

        assert_eq!(
            ops.ops(),
            vec!["batch.record_success", "ack"],
            "the batch must see the job settled before the reservation is \
             dropped — otherwise a crash strands the batch on pending > 0"
        );
        let snap = repo.find(&batch_id).await.unwrap().expect("batch exists");
        assert_eq!(snap.pending_jobs, 1, "the decrement actually applied");
    }

    #[tokio::test]
    async fn completed_acks_even_when_the_batch_repository_errors() {
        // Failure mode, and the reason batch accounting is best-effort:
        // `PendingBatch::dispatch` deletes the batch row when a mid-loop push
        // fails, so envelopes that already landed get `batch not found`
        // forever. Gating the ack on that would spin them on visibility
        // expiry with no exit.
        let ops = Arc::new(OpLog::default());
        let driver = RecordingDriver::with_log(false, ops.clone());
        let repo = Arc::new(RecordingBatchRepo::new(ops.clone(), true));
        let token = ReservationToken(Uuid::new_v4());
        let mut env = fresh_env("Orphan", 1);
        env.batch_id = Some(Uuid::new_v4().to_string());

        handle_completed(
            &driver,
            &token,
            &env,
            "test",
            &SettlementDeps {
                failed_store: None,
                batches: Some(repo),
            },
        )
        .await;

        assert_eq!(
            ops.ops(),
            vec!["batch.record_success", "ack"],
            "a permanently-failing batch write must not hold the reservation"
        );
    }

    #[tokio::test]
    async fn dead_letter_records_the_failure_before_acking() {
        let ops = Arc::new(OpLog::default());
        let driver = RecordingDriver::with_log(false, ops.clone());
        let store = Arc::new(RecordingFailedStore::new(ops.clone(), false));
        let token = ReservationToken(Uuid::new_v4());
        let env = fresh_env("Doomed", 3);

        handle_dead_letter(
            &driver,
            &token,
            &env,
            "test-conn",
            "boom",
            false,
            &SettlementDeps {
                failed_store: Some(store.clone()),
                batches: None,
            },
        )
        .await;

        assert_eq!(
            ops.ops(),
            vec!["failed_store.log", "ack"],
            "the failed-jobs record IS the recovery path — it must be durable \
             before the queue copy is dropped"
        );
        assert_eq!(
            store.records(),
            vec![(
                crate::queue::envelope::DEFAULT_QUEUE.to_string(),
                "Doomed".to_string(),
                "boom".to_string()
            )],
            "the record carries the queue the envelope died on and the cause"
        );
    }

    #[tokio::test]
    async fn dead_letter_store_failure_leaves_the_job_redeliverable() {
        // Failure mode: an unmigrated `failed_jobs` table. Pre-fix this
        // silently swallowed the job — acked first, record never written, no
        // `queue:retry` possible. Now the reservation survives.
        let ops = Arc::new(OpLog::default());
        let driver = RecordingDriver::with_log(false, ops.clone());
        let store = Arc::new(RecordingFailedStore::new(ops.clone(), true));
        let token = ReservationToken(Uuid::new_v4());
        let env = fresh_env("Doomed", 3);

        handle_dead_letter(
            &driver,
            &token,
            &env,
            "test-conn",
            "boom",
            false,
            &SettlementDeps {
                failed_store: Some(store),
                batches: None,
            },
        )
        .await;

        assert_eq!(
            ops.ops(),
            vec!["failed_store.log"],
            "the settlement stops at the rejected write"
        );
        assert_eq!(
            driver.ack_count(),
            0,
            "a rejected failure record must leave the reservation un-acked so \
             the dead letter is redelivered rather than lost in both places"
        );
    }

    #[tokio::test]
    async fn dead_letter_without_a_store_still_acks() {
        // Retention is optional; no store installed must not wedge the queue.
        let ops = Arc::new(OpLog::default());
        let driver = RecordingDriver::with_log(false, ops.clone());
        let token = ReservationToken(Uuid::new_v4());
        let env = fresh_env("Doomed", 3);

        handle_dead_letter(&driver, &token, &env, "test-conn", "boom", true, &no_deps()).await;

        assert_eq!(ops.ops(), vec!["ack"]);
    }

    #[tokio::test]
    async fn dead_letter_records_failure_and_batch_before_acking() {
        // Both follow-ups, in order, ahead of the ack — and a failing batch
        // write still does not hold the reservation.
        let ops = Arc::new(OpLog::default());
        let driver = RecordingDriver::with_log(false, ops.clone());
        let store = Arc::new(RecordingFailedStore::new(ops.clone(), false));
        let repo = Arc::new(RecordingBatchRepo::new(ops.clone(), true));
        let token = ReservationToken(Uuid::new_v4());
        let mut env = fresh_env("Doomed", 3);
        env.batch_id = Some(Uuid::new_v4().to_string());

        handle_dead_letter(
            &driver,
            &token,
            &env,
            "test-conn",
            "boom",
            false,
            &SettlementDeps {
                failed_store: Some(store),
                batches: Some(repo),
            },
        )
        .await;

        assert_eq!(
            ops.ops(),
            vec!["failed_store.log", "batch.record_failed", "ack"],
            "both follow-ups precede the ack; only the failed-jobs write gates it"
        );
    }
}
