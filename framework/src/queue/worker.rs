//! Worker registry + dispatch by job_name.
//!
//! Each `Job` impl registers a deserialize-and-run shim keyed by its
//! `job_name`. Drivers call `dispatch_by_name` to run an inbound payload.
//! Re-registering the same name is allowed (last writer wins) - useful
//! for tests; deterministic in production because each Job has exactly
//! one registration site.
//!
//! # At-least-once delivery and job idempotency
//!
//! Redis-backed queue drivers cannot make `nack` atomic - the
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
use crate::queue::driver::{QueueDriver, Settled};
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
    /// Snapshot of [`Job::unique_until_processing`] taken at registration.
    ///
    /// Registry metadata rather than payload sniffing: the worker has to know
    /// this before it decides to release the lock, and deserializing the job
    /// just to ask would put a decode on the path of every popped envelope -
    /// including the ones whose decode is the thing that fails.
    unique_until_processing: bool,
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
    let unique_until_processing = J::unique_until_processing();
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
                unique_until_processing,
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

/// Whether the job registered under `name` opted into
/// [`Job::unique_until_processing`].
///
/// `false` for an unregistered name: the dispatcher is about to fail that
/// envelope anyway, and releasing a uniqueness lock for a job nobody can run
/// would let duplicates pile in behind a job that never executes.
pub(crate) fn job_is_unique_until_processing(name: &str) -> bool {
    let Ok(g) = lock::read(&REGISTRY, "queue job registry") else {
        return false;
    };
    g.as_ref()
        .and_then(|m| m.get(name).map(|r| r.unique_until_processing))
        .unwrap_or(false)
}

/// Release a unique-until-processing lock, owner-scoped, best-effort.
///
/// `Ok(false)` - owner mismatch, or the lock already expired - is not an
/// error: either a newer dispatch holds the lock and must keep it, or the TTL
/// beat us. A store failure is logged and swallowed, because a lock that
/// outlives its job by at most `unique_for` is the documented degradation,
/// while failing the job over it would turn a cache hiccup into a retry storm.
///
/// ### Why there is no ownerless fallback
///
/// Laravel releases an ownerless unique lock with `forceRelease()` when the
/// job is on its first attempt (`UniqueLock::release`). Suprnova does not: a
/// forced release deletes whichever lock is there, including one a newer
/// dispatch acquired seconds ago, and the only envelopes that reach here
/// without an owner token are ones serialized before the token existed. Those
/// keep exactly the TTL-expiry behaviour they shipped with.
async fn release_unique_lock_if_held(env: &Envelope) {
    let Some(id) = env.idempotency_key.as_deref() else {
        return;
    };
    let Some(owner) = env
        .unique_lock_owner
        .as_deref()
        .filter(|owner| !owner.is_empty())
    else {
        return;
    };
    let key = crate::queue::unique_key(&env.job_name, id);
    if let Err(e) = crate::idempotency::Idempotency::release_owned(&key, owner).await {
        tracing::warn!(
            job = %env.job_name,
            id = %env.id,
            error = %e,
            "unique-until-processing lock release failed; the lock now expires by TTL"
        );
    }
}

/// Whether this envelope was superseded by a newer dispatch of the same
/// debounce window.
///
/// Fails **open** at every uncertainty: an envelope with no token was not
/// debounced, a window whose key is gone (evicted, expired) is not evidence
/// that somebody else owns it, and a cache error is not evidence of anything.
/// Only a token that is present and different means "a newer dispatch owns this
/// window", which is the one case where dropping the job is correct. Getting
/// this backwards would silently discard work.
async fn envelope_was_superseded(env: &Envelope) -> bool {
    let Some(owner) = env
        .debounce_owner
        .as_deref()
        .filter(|owner| !owner.is_empty())
    else {
        return false;
    };
    let key = crate::queue::debounce_key(&env.job_name, env.debounce_id.as_deref());
    match crate::queue::debounce::current_owner(&key).await {
        Ok(Some(current)) => current != owner,
        Ok(None) => false,
        Err(e) => {
            tracing::warn!(
                job = %env.job_name,
                id = %env.id,
                error = %e,
                "debounce owner lookup failed; running the job rather than dropping it"
            );
            false
        }
    }
}

/// Start a fresh max-wait window for an envelope that is about to actually run.
///
/// A no-op for a non-debounced envelope. See
/// [`release_max_wait`](crate::queue::debounce::release_max_wait) for why the
/// reset belongs at the start of every run and not only when max wait fired.
async fn reset_debounce_max_wait(env: &Envelope) {
    if env
        .debounce_owner
        .as_deref()
        .filter(|owner| !owner.is_empty())
        .is_none()
    {
        return;
    }
    let key = crate::queue::debounce_key(&env.job_name, env.debounce_id.as_deref());
    if let Err(e) = crate::queue::debounce::release_max_wait(&key).await {
        tracing::warn!(
            job = %env.job_name,
            id = %env.id,
            error = %e,
            "could not reset the debounce max-wait window; the next burst may measure \
             its maximum wait from this one's first dispatch"
        );
    }
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
    let unique_until_processing = job_is_unique_until_processing(&job_name);
    // Build the innermost layer: actually dispatch the job, lift result
    // into JobOutcome::Completed.
    let innermost: Next = Box::new(move |env: Envelope| {
        Box::pin(async move {
            // Processing begins here, and this is the last point at which
            // every middleware has passed the job through - Laravel releases
            // the uniqueness lock in exactly this position (the pipeline's
            // `->then(...)`), so a middleware that sends the job back to the
            // queue never gets its lock released out from under it.
            //
            // Under the sync driver this runs inline inside the very
            // `push_unique` call that took the lock, so the job releases a lock
            // its own caller still holds a guard for. That is the correct
            // outcome - processing HAS begun - but the caller's lease renewer
            // then reports a lost lease, and `push_unique` reports
            // `FreshUnfenced`. Both warnings are expected on that path; the
            // queues chapter says so.
            if unique_until_processing {
                release_unique_lock_if_held(&env).await;
            }
            // Laravel #61281: start a fresh max-wait window at the start of
            // every actual run, not only when max wait fired. Without this a
            // job that reached the worker by the ordinary debounce path leaves
            // the previous burst's `first_dispatched_at` behind, and the NEXT
            // burst measures its maximum wait from a first dispatch that was
            // never its own - so its very first dispatch can look overdue and
            // fire with no delay at all.
            reset_debounce_max_wait(&env).await;
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
    /// rather than silently draining everything - see
    /// [`QueueDriver::pop_from`].
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

/// Which queues this loop iteration may poll, plus which it observed paused.
///
/// The first element is `None` for "nothing to poll this iteration" - the
/// caller treats that exactly like an empty `pop_from` result: sleep, loop
/// again, never touch the driver. The second is the paused set, which the
/// caller diffs against the previous iteration to emit
/// [`WorkerQueuePaused`](crate::queue::events::WorkerQueuePaused) /
/// [`WorkerQueueResumed`](crate::queue::events::WorkerQueueResumed) exactly
/// once per transition.
///
/// Reporting and deciding are separate on purpose: the diff is the part with
/// the off-by-one risk, and keeping it out of here lets it be unit-tested
/// without a running worker. Laravel splits the same way, between
/// `getPausedQueues` and `raisePausedQueueEvents`.
///
/// `pausable == false` skips the check entirely and returns `cfg_queues`
/// unfiltered with an empty paused set, mirroring Laravel's
/// `Worker::getPausedQueues` returning `[]` when `Worker::$pausable` is false.
///
/// A cache error fails OPEN - folded into "nothing is paused" via
/// `.unwrap_or(...)`, the same contract `run_worker`'s restart-signal check
/// applies a few lines above via `if let Ok(Some(ts)) = ...`. An unreachable
/// cache must not silently freeze every worker in the fleet over what is, from
/// the worker's point of view, an optional control signal.
///
/// `cfg_queues.is_empty()` - a worker started without `--queue`, which drains
/// every queue the driver holds - can only honor the *global* pause. There is
/// nothing to intersect a per-queue pause against:
/// [`QueueDriver::pop_from`](crate::queue::QueueDriver::pop_from) never reports
/// which queue names exist. Such a worker also has no names to put in the
/// paused set, which is why the caller tracks that case as a separate boolean
/// and reports it with a `None` queue. Name queues with `--queue=a,b` to make
/// them individually pausable and individually reported.
async fn pause_gate(
    connection: &str,
    cfg_queues: &[String],
    pausable: bool,
) -> (Option<Vec<String>>, Vec<String>) {
    if !pausable {
        return (Some(cfg_queues.to_vec()), Vec::new());
    }
    if crate::queue::is_globally_paused().await.unwrap_or(false) {
        // Under the global switch every named queue is paused, which is what
        // `Queue::paused_queues` reports too (and what Laravel's
        // `getPausedQueues` returns). Reporting the names here rather than
        // short-circuiting past them is what lets a `--queue=a,b` worker emit
        // per-queue events under `pause_all` exactly as it does under `pause`.
        return (None, cfg_queues.to_vec());
    }
    if cfg_queues.is_empty() {
        return (Some(Vec::new()), Vec::new());
    }
    let paused = crate::queue::Queue::paused_queues(connection, cfg_queues)
        .await
        .unwrap_or_default();
    let active: Vec<String> = cfg_queues
        .iter()
        .filter(|q| !paused.contains(q))
        .cloned()
        .collect();
    if active.is_empty() {
        (None, paused)
    } else {
        (Some(active), paused)
    }
}

/// Which queues changed pause state since the last iteration.
///
/// Returns `(newly_paused, newly_resumed)`, each in the order the names appear
/// in the slice they came from, so the emitted event order is deterministic.
/// Slice `contains` rather than a set: a worker's queue list is single digits
/// long, and preserving order is worth more here than the asymptotics.
/// Mirrors the two `array_diff` calls in Laravel's `raisePausedQueueEvents`.
fn diff_paused_queues(previous: &[String], current: &[String]) -> (Vec<String>, Vec<String>) {
    let newly_paused: Vec<String> = current
        .iter()
        .filter(|q| !previous.contains(q))
        .cloned()
        .collect();
    let newly_resumed: Vec<String> = previous
        .iter()
        .filter(|q| !current.contains(q))
        .cloned()
        .collect();
    (newly_paused, newly_resumed)
}

/// Emit one event per queue that changed state, then adopt `current` as the
/// new baseline.
///
/// Best-effort like every other queue event: a listener that errors must not
/// cost the worker a loop iteration, so the dispatch result is dropped.
async fn raise_paused_queue_events(
    connection: &str,
    previous: &mut Vec<String>,
    current: Vec<String>,
) {
    let (newly_paused, newly_resumed) = diff_paused_queues(previous, &current);
    for queue in newly_paused {
        let _ = EventFacade::dispatch(queue_events::WorkerQueuePaused {
            connection: connection.to_string(),
            queue: Some(queue),
        })
        .await;
    }
    for queue in newly_resumed {
        let _ = EventFacade::dispatch(queue_events::WorkerQueueResumed {
            connection: connection.to_string(),
            queue: Some(queue),
        })
        .await;
    }
    *previous = current;
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
    // Read once per worker lifetime, mirroring Laravel's `Worker::$pausable`
    // static: an operator's escape hatch, not something that should change
    // mid-run.
    let pausable = crate::queue::pausable_from_env();
    let _ = EventFacade::dispatch(queue_events::WorkerStarting {
        connection: connection.clone(),
    })
    .await;

    let mut processed: u64 = 0;
    // Last iteration's paused set, and whether an unfiltered worker was idle
    // on the global switch. Two states because there are two kinds of
    // transition: a named queue's, which carries a name, and an unfiltered
    // worker's, which has none to carry.
    let mut paused_queues: Vec<String> = Vec::new();
    let mut idle_on_global_pause = false;
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

        // Pause gate: sits right before the claim, after everything
        // above it, so a job already popped by a previous iteration has
        // long since finished - pausing never interrupts one in flight,
        // it only stops the NEXT claim. `None` means every eligible
        // queue is paused this iteration; behave exactly like an empty
        // poll, without touching the driver.
        let (active, observed_paused) = pause_gate(&connection, &cfg.queues, pausable).await;
        raise_paused_queue_events(&connection, &mut paused_queues, observed_paused).await;
        // An unfiltered worker has no queue names, so its global-pause
        // transition is reported with a `None` queue instead. `paused_queues`
        // stays empty on this path, so the two trackers never double-report.
        let now_idle_unnamed = active.is_none() && cfg.queues.is_empty();
        if now_idle_unnamed != idle_on_global_pause {
            if now_idle_unnamed {
                let _ = EventFacade::dispatch(queue_events::WorkerQueuePaused {
                    connection: connection.clone(),
                    queue: None,
                })
                .await;
            } else {
                let _ = EventFacade::dispatch(queue_events::WorkerQueueResumed {
                    connection: connection.clone(),
                    queue: None,
                })
                .await;
            }
            idle_on_global_pause = now_idle_unnamed;
        }
        let Some(active_queues) = active else {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    exit_with("cancelled", processed, &connection);
                    break ExitReason::Cancelled;
                }
                _ = tokio::time::sleep(cfg.poll_interval) => {}
            }
            continue;
        };

        // Pop OR cancel - whichever happens first. `biased` makes cancel win
        // a tie so a queue under load can still exit promptly.
        let popped = tokio::select! {
            biased;
            _ = shutdown.cancelled() => {
                exit_with("cancelled", processed, &connection);
                break ExitReason::Cancelled;
            }
            res = driver.pop_from(cfg.visibility_timeout, &active_queues) => res,
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

        // Spend the budget *before* running, not only when settling.
        //
        // Every other dead-letter decision happens after the handler
        // returns - which assumes the handler returns. A job that kills
        // its worker (OOM, abort, segfault, or the SIGKILL a supervisor
        // sends when a stop times out) never reaches settlement, so the
        // check at the bottom of this loop never runs for exactly the jobs
        // most in need of it. Counting the reclaimed attempt (which the
        // drivers now do) makes the number climb; without this it climbs
        // forever and nothing acts on it.
        //
        // `>` and not `>=`: `attempts` was just incremented for *this*
        // dispatch, so on the last permitted run it equals `max_tries`.
        // `>=` here would silently cut every configured budget by one.
        if env.attempts > env.max_tries {
            tracing::error!(
                job = %env.job_name,
                id = %env.id,
                attempts = env.attempts,
                max_tries = env.max_tries,
                "queue job exhausted its attempts without ever settling - \
                 dead-lettering before it takes another worker down"
            );
            handle_dead_letter(
                &*driver,
                &res.token,
                &env,
                &connection,
                "attempts exhausted without settlement; the previous workers did not \
                 survive this job",
                false,
                &SettlementDeps::current(),
            )
            .await;
            processed += 1;
            if let Some(max) = cfg.max_jobs
                && processed >= max
            {
                exit_with("max_jobs reached", processed, &connection);
                break ExitReason::MaxJobs;
            }
            continue;
        }

        let identity_pre = queue_events::JobIdentity::from_env(&env, &connection);
        let _ = EventFacade::dispatch(queue_events::JobProcessing {
            job: identity_pre.clone(),
        })
        .await;

        // Laravel checks this in `CallQueuedHandler::call`, after the worker
        // has fired `JobProcessing` and before the middleware pipeline - so a
        // superseded job runs no middleware at all. Same order here. This is a
        // settlement, not a failure: ack, report, move on.
        if envelope_was_superseded(&env).await {
            let _ = EventFacade::dispatch(queue_events::JobDebounced {
                job: identity_pre.clone(),
            })
            .await;
            if let Err(e) = driver.ack(&res.token).await {
                settlement_failure(&*driver, &env, "ack", "debounced", &e);
            }
            tracing::debug!(
                job = %env.job_name,
                id = %env.id,
                "queue job superseded by a newer debounced dispatch"
            );
            processed += 1;
            if let Some(max) = cfg.max_jobs
                && processed >= max
            {
                exit_with("max_jobs reached", processed, &connection);
                break ExitReason::MaxJobs;
            }
            continue;
        }

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

        // Laravel's `finally` sweep (`CallQueuedHandler::dispatchThroughMiddleware`).
        // A middleware that short-circuits means the pipeline core never ran,
        // so the release at processing start never happened either, and the job
        // would sit on its uniqueness lock for the rest of the TTL despite
        // being dropped, dead-lettered, or reported complete by the middleware
        // itself.
        //
        // The guard is Laravel's, and it is about the job's state, not the
        // outcome's severity: sweep everything except `Released`, because a job
        // put back on the queue has not started processing
        // (`! $job->isReleased()`). `TimedOut` splits on that same rule rather
        // than being exempt from it. The timeout above wraps
        // `run_through_middleware`, which is the whole pipeline and not just its
        // core, so a middleware that stalls reaches `TimedOut` with the core
        // never run and the release at dispatch time never issued: the
        // dead-letter sub-arm sweeps, and the retry sub-arm does not, because
        // the envelope is going back on the queue unstarted.
        //
        // Arms reachable with the core already run sweep too. That second
        // release finds no lock this envelope owns and reports `false`, which
        // is what makes it safe to sweep without tracking whether the core ran.
        //
        // The owner-token check comes first so the registry read is skipped
        // entirely for the ordinary job, which never carries one.
        let sweep_unique_lock =
            env.unique_lock_owner.is_some() && job_is_unique_until_processing(&env.job_name);

        match outcome {
            DispatchOutcome::Settled(JobOutcome::Completed) => {
                if sweep_unique_lock {
                    release_unique_lock_if_held(&env).await;
                }
                handle_completed(&*driver, &res.token, &env, &connection, &deps).await;
            }
            DispatchOutcome::Settled(JobOutcome::Released { delay }) => {
                handle_released(&*driver, &res.token, &env, delay, &connection, "middleware").await;
            }
            DispatchOutcome::Settled(JobOutcome::Failed { reason }) => {
                if sweep_unique_lock {
                    release_unique_lock_if_held(&env).await;
                }
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
                if sweep_unique_lock {
                    release_unique_lock_if_held(&env).await;
                }
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
                // reason documented on [`handle_completed`] - acking first
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
                if sweep_unique_lock {
                    release_unique_lock_if_held(&env).await;
                }
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
                    // A stalled middleware times out the whole pipeline, so the
                    // core may never have run and the release at processing
                    // start may never have happened. This envelope is
                    // dead-lettered and will not come back, so a held lock would
                    // block re-dispatch for the rest of `unique_for` on a job
                    // that no longer exists. Owner-scoped, so it costs one
                    // no-op release when the core did run and already released.
                    if sweep_unique_lock {
                        release_unique_lock_if_held(&env).await;
                    }
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

/// Settle a successful run: batch accounting first, then the chain successor
/// and the acknowledgement together via [`QueueDriver::settle`].
///
/// # Terminal settlement (DATA-02)
///
/// Finishing a chained job means enqueuing the next link *and* releasing the
/// job just finished. As two separate operations there is no safe order -
/// ack-first can lose the rest of the chain permanently, push-first can run the
/// successor twice - so the framework hands both to the driver and lets it
/// commit them together. [`DatabaseQueueDriver`](crate::queue::DatabaseQueueDriver)
/// does exactly that, with the reservation-keyed delete acting as a fence: a
/// worker whose visibility expired mid-run commits nothing at all.
///
/// Drivers that cannot settle transactionally answer
/// [`Settled::Unsupported`] and fall through to [`fallback_settle`], which
/// documents the duplicate window that choice accepts.
///
/// # Why batch accounting does not gate the settlement
///
/// A batch repository error is frequently *permanent*:
/// [`PendingBatch::dispatch`](crate::queue::batch::PendingBatch::dispatch)
/// deletes the batch row when a mid-loop push fails, and the envelopes that
/// already landed then get `Err(batch not found)` forever. Refusing to settle
/// on that would spin those orphans on visibility expiry with no exit. So the
/// batch step runs before the settlement - a crash replays it rather than
/// losing it, and replay is harmless because settlement is idempotent per
/// `(batch_id, job_id)` - but its error never holds the reservation.
///
/// Batch accounting is *not* part of the settlement transaction, and
/// deliberately so: the repository is separately installable and may address a
/// different database entirely, which would make the coupling either a lie or
/// a hard constraint on where batch metadata lives. Its own
/// `(batch_id, job_id)` uniqueness is what makes the replay safe.
async fn handle_completed(
    driver: &dyn QueueDriver,
    token: &crate::queue::driver::ReservationToken,
    env: &Envelope,
    connection: &str,
    deps: &SettlementDeps,
) {
    // 1. Build the chain successor, if any, onto the SAME driver that settled
    // this job. The worker is bound to a specific `Arc<dyn QueueDriver>` at
    // `run_worker(driver, ...)`; resolving through `current_driver()` would
    // re-pick whichever driver is registered globally, which differs from the
    // bound one under multi-connection setups (e.g. one worker per connection)
    // and would silently land the next link on the wrong queue.
    let mut follow_ups: Vec<Envelope> = Vec::new();
    if !env.chain_remaining.is_empty() {
        let mut tail = env.chain_remaining.clone();
        let next: ChainLink = tail.remove(0);
        // Derived from this envelope's id, not random: on the non-atomic path
        // this push happens before the ack, so a crash in that window
        // redelivers `env` and runs the push again. A random id made the second
        // push indistinguishable from a legitimate new step. See
        // `ChainLink::to_envelope_after`.
        let mut next_env = next.to_envelope_after(env.id);
        next_env.chain_remaining = tail;
        next_env.batch_id = env.batch_id.clone();
        follow_ups.push(next_env);
    }

    // 2. Notify batch repository (best-effort - see the doc comment). This
    // runs before the settlement so a crash replays it rather than losing it;
    // replay is harmless because settlement is idempotent per `(batch_id,
    // job_id)`.
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

    // 3. Enqueue the successor and drop the reservation - in one transaction
    // where the driver can, push-then-ack where it cannot.
    match driver.settle(token, &follow_ups).await {
        Ok(Settled::Atomically) => {
            tracing::debug!(job = %env.job_name, id = %env.id, "queue job ok");
        }
        Ok(Settled::Stale) => {
            // Our reservation expired and someone else owns this message now.
            // Nothing was enqueued: the successor belongs to whoever holds it.
            tracing::warn!(
                job = %env.job_name,
                id = %env.id,
                driver = driver.name(),
                "queue settlement found the reservation already reclaimed; \
                 nothing enqueued, the current owner will settle it"
            );
        }
        Ok(Settled::Unsupported) => {
            fallback_settle(driver, token, env, follow_ups).await;
        }
        Err(e) => {
            // Leave the reservation intact so visibility expiry redelivers.
            // Nothing committed, so the replay starts from a clean state.
            settlement_failure(driver, env, "settle", "success", &e);
            return;
        }
    }

    // 4. Observation only - these carry no recovery value, so they run after
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

/// Push-then-ack settlement for drivers that answer [`QueueDriver::settle`]
/// with [`Settled::Unsupported`] - Redis, in-memory, and any driver written
/// before the protocol existed.
///
/// # Why the push goes first (DATA-02a)
///
/// Acking first drops the reservation while the follow-up is still unwritten.
/// A crash in that window - and a rolling restart samples it once per in-flight
/// job, so it is not theoretical - leaves the job gone from the queue with its
/// successor never enqueued. The chain then stalls permanently: nothing is left
/// in the queue to retry from, and no operator action recovers it.
///
/// Ordering the push before the ack converts that silent permanent loss into a
/// detectable duplicate. The reservation stays live, visibility expiry
/// redelivers the envelope, and the handler runs a second time. That trade is
/// deliberate and it is safe because duplicate execution is already the
/// framework's delivery contract - see the module header: every production
/// handler must be idempotent. When the duplication is caused by a failing
/// `ack` it is counted by [`METRIC_SETTLEMENT_FAILURES`], so operators can
/// alert on the rate; when it is caused by a failing push it is logged at
/// ERROR with the driver, job and envelope id. Silent loss has neither.
///
/// This is the non-atomic half of the trade, kept only for drivers that cannot
/// do better. A backend whose follow-up write and acknowledgement share a
/// transaction domain should implement [`QueueDriver::settle`] instead, which
/// removes the duplicate window rather than labelling it.
async fn fallback_settle(
    driver: &dyn QueueDriver,
    token: &crate::queue::driver::ReservationToken,
    env: &Envelope,
    follow_ups: Vec<Envelope>,
) {
    for next in follow_ups {
        if let Err(e) = driver.push(next).await {
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
    if let Err(e) = driver.ack(token).await {
        settlement_failure(driver, env, "ack", "success", &e);
    } else {
        tracing::debug!(job = %env.job_name, id = %env.id, "queue job ok");
    }
}

/// Settle a job that asked to be retried later without spending an attempt -
/// a busy `WithoutOverlapping` lock, a throttle, or an explicit
/// `JobOutcome::Released`.
///
/// # Why this is one driver call (DATA-02)
///
/// This used to be push-then-ack: decrement the local attempt counter, push a
/// copy with a later `available_at`, then ack the original. That is only a
/// release on a driver where two envelopes may share an id. On
/// [`DatabaseQueueDriver`](crate::queue::database::DatabaseQueueDriver) the id
/// is the `jobs` primary key, so the push collided with the row still holding
/// the live reservation and came back `UNIQUE constraint failed: jobs.id`. The
/// push error then took the early return below - correct, given the evidence,
/// since a lost push must never be followed by an ack - and the release
/// silently became a no-op: no delay applied, no `JobReleased` event, the job
/// simply parked until visibility expiry redelivered it. Every release on a
/// database-backed queue behaved that way.
///
/// [`QueueDriver::release`] moves the whole operation into the driver, which
/// requeues its own stored copy in place. There is no window in which the
/// message exists twice or not at all, and no attempt-counter arithmetic here
/// for a driver to disagree with.
async fn handle_released(
    driver: &dyn QueueDriver,
    token: &crate::queue::driver::ReservationToken,
    env: &Envelope,
    delay: Duration,
    connection: &str,
    reason: &str,
) {
    // A failing release leaves the reservation intact on every in-tree driver,
    // so visibility expiry redelivers the job rather than dropping it. The
    // release is retried on that delivery.
    if let Err(e) = driver.release(token, env, delay).await {
        settlement_failure(driver, env, "release", "released", &e);
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
/// the queue nor the failed store - permanently and silently gone, with no
/// operator action that brings it back.
///
/// Writing the record before dropping the reservation trades that away for a
/// duplicate: a failing write returns early WITHOUT acking, visibility expiry
/// redelivers the envelope, the handler runs (and presumably fails) again, and
/// the write is retried. Duplicate execution is already the framework's
/// documented delivery contract (see the module header), and the failure is
/// visible - an ERROR log per cycle carrying driver, job and envelope id, plus
/// [`METRIC_SETTLEMENT_FAILURES`] whenever the duplication comes from a failing
/// `ack` rather than a failing write.
///
/// **Operator note:** a store that fails *permanently* - a
/// [`DatabaseFailedJobStore`](crate::queue::DatabaseFailedJobStore) pointed at
/// a missing or unmigrated `failed_jobs` table - now recycles dead-lettered
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
/// `Then` for a cancelled batch - despite a comment on the other copy
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
    // envelope actually died on - `queue:retry` re-pushes the stored
    // envelope, and an operator triaging a dedicated pool filters failed
    // jobs by this column, so writing "default" for a routed job would
    // hide its failures from the very pool that owns them.
    match deps.failed_store.as_ref() {
        Some(store) => {
            if let Err(e) = store
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
        }
        None => {
            // No store bound. This used to fall straight through to the ack
            // below, so a dead-lettered job was deleted with no record
            // anywhere - quieter than the failure case above, which at
            // least leaves the reservation intact. An absent store was
            // treated as more successful than a broken one.
            //
            // The job still has to leave the queue: it is out of attempts,
            // and putting it back is how a poison job becomes immortal. So
            // the envelope goes to the log at ERROR, because a serialised
            // envelope is what `queue:retry` re-pushes - this line is the
            // difference between work that can be recovered by hand and work
            // that silently ceased to exist.
            //
            // `unique_lock_owner` is cleared first. It is the bearer token for
            // an owner-scoped lock release, and a log is readable by a wider
            // audience than the queue store - anyone holding the token can free
            // a dedupe lock a newer dispatch already owns. Re-pushing does not
            // need it: a fresh push takes a fresh lock.
            let mut redacted = env.clone();
            redacted.unique_lock_owner = None;
            let payload = redacted
                .to_json()
                .unwrap_or_else(|e| format!("<envelope could not be serialised: {e}>"));
            tracing::error!(
                job = %env.job_name,
                id = %env.id,
                driver = driver.name(),
                attempts = env.attempts,
                reason = %reason,
                envelope = %payload,
                "queue job dead-lettered with NO failed-jobs store configured - the \
                 envelope is logged here because there is nowhere else to put it. \
                 Bind a FailedJobStore so failures are queryable instead of \
                 grep-able."
            );
        }
    }

    // 2. Notify batch repository of failure (and cancel if !allow_failures).
    // Best-effort - see the doc comment.
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

    // 4. Observation only - never gates the settlement.
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
        ("ack", "debounced") => {
            "queue ack failed for a superseded debounced job; \
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
        ("settle", "success") => {
            "queue terminal settlement failed; nothing was committed and the \
             reservation stays until visibility expiry redelivers the job"
        }
        ("release", "released") => {
            "queue release failed; the requested delay was not applied and \
             the reservation stays until visibility expiry redelivers the job"
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

// `PartialEq`/`Debug` so `terminal_batch_phase` can be asserted on directly -
// the phase choice is a correctness rule (a cancelled batch must never
// report success), and asserting it needs the value, not a side effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BatchPhase {
    Then,
    Catch,
    Finally,
}

pub(crate) async fn fire_batch_callbacks(batch: &crate::queue::batch::Batch, phase: BatchPhase) {
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
    /// a knob that makes the two settlement writes fail - the shape of a
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

    /// No failed-jobs store and no batch repository installed - the wiring a
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
    /// arm used to branch on `failed_jobs` alone and fired `Then` here -
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
            "a cancelled batch fires Catch - Then would tell the caller a \
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
            unique_lock_owner: None,
            debounce_id: None,
            debounce_owner: None,
            batch_id: None,
            chain_remaining: Vec::new(),
        }
    }

    // ---- terminal settlement: delegate first, fall back only if asked ----

    /// A driver that reports whatever [`Settled`] outcome the test wants and
    /// records what it was handed.
    struct SettlingDriver {
        ops: Arc<OpLog>,
        outcome: Settled,
        settled: Mutex<Vec<Vec<Envelope>>>,
    }

    impl SettlingDriver {
        fn new(outcome: Settled) -> Self {
            Self {
                ops: Arc::new(OpLog::default()),
                outcome,
                settled: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl QueueDriver for SettlingDriver {
        async fn push(&self, _env: Envelope) -> Result<(), FrameworkError> {
            self.ops.record("push");
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
            Ok(())
        }
        async fn settle(
            &self,
            _token: &ReservationToken,
            follow_ups: &[Envelope],
        ) -> Result<Settled, FrameworkError> {
            self.ops.record("settle");
            self.settled
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(follow_ups.to_vec());
            Ok(self.outcome)
        }
    }

    /// The chain successor must reach the driver as part of the settlement,
    /// not as a separate push the driver cannot tie to the acknowledgement.
    #[tokio::test]
    async fn a_chain_successor_is_handed_to_the_driver_as_part_of_the_settlement() {
        let driver = SettlingDriver::new(Settled::Atomically);
        let token = ReservationToken(Uuid::new_v4());
        let mut env = fresh_env("Head", 1);
        env.chain_remaining = vec![chain_link("Tail")];

        handle_completed(&driver, &token, &env, "test", &no_deps()).await;

        assert_eq!(driver.ops.count("settle"), 1);
        assert_eq!(
            (driver.ops.count("push"), driver.ops.count("ack")),
            (0, 0),
            "the worker must not also push the successor or ack separately - \
             doing both is the two-step settlement this replaces"
        );

        let handed = driver.settled.lock().unwrap_or_else(|e| e.into_inner());
        let follow_ups = handed.first().expect("settle was called");
        assert_eq!(follow_ups.len(), 1, "exactly the one successor");
        assert_eq!(follow_ups[0].job_name, "Tail");
        assert_eq!(
            follow_ups[0].id,
            crate::queue::chain::next_link_id(env.id),
            "and under the id derived from its predecessor"
        );
    }

    /// A job with no chain still settles through the driver, so the fence
    /// applies to plain acknowledgements too.
    #[tokio::test]
    async fn a_job_with_no_chain_settles_with_no_follow_ups() {
        let driver = SettlingDriver::new(Settled::Atomically);
        let token = ReservationToken(Uuid::new_v4());
        let env = fresh_env("Solo", 1);

        handle_completed(&driver, &token, &env, "test", &no_deps()).await;

        let handed = driver.settled.lock().unwrap_or_else(|e| e.into_inner());
        assert!(
            handed.first().expect("settle was called").is_empty(),
            "no chain means no follow-ups, not a skipped settlement"
        );
    }

    /// A reservation that has been reclaimed must not have its successor
    /// enqueued behind the new owner's back.
    #[tokio::test]
    async fn a_stale_settlement_neither_pushes_nor_acks() {
        let driver = SettlingDriver::new(Settled::Stale);
        let token = ReservationToken(Uuid::new_v4());
        let mut env = fresh_env("Head", 1);
        env.chain_remaining = vec![chain_link("Tail")];

        handle_completed(&driver, &token, &env, "test", &no_deps()).await;

        assert_eq!(
            (driver.ops.count("push"), driver.ops.count("ack")),
            (0, 0),
            "a stale settlement must not be retried as push-then-ack - that \
             would re-open the fork the fence just closed"
        );
    }

    /// Drivers that cannot settle transactionally keep the documented
    /// push-before-ack ordering, including the guarantee that a failed push
    /// leaves the reservation alone.
    #[tokio::test]
    async fn an_unsupported_driver_falls_back_to_push_then_ack() {
        let driver = RecordingDriver::new(false);
        let token = ReservationToken(Uuid::new_v4());
        let mut env = fresh_env("Head", 1);
        env.chain_remaining = vec![chain_link("Tail")];

        handle_completed(&driver, &token, &env, "test", &no_deps()).await;

        assert_eq!(
            driver.ops.ops(),
            vec!["push", "ack"],
            "the successor is enqueued before the reservation is dropped"
        );
    }

    #[tokio::test]
    async fn an_unsupported_driver_that_cannot_push_the_successor_does_not_ack() {
        let driver = RecordingDriver::new(true);
        let token = ReservationToken(Uuid::new_v4());
        let mut env = fresh_env("Head", 1);
        env.chain_remaining = vec![chain_link("Tail")];

        handle_completed(&driver, &token, &env, "test", &no_deps()).await;

        assert_eq!(
            (driver.push_count(), driver.ack_count()),
            (1, 0),
            "a chain whose successor could not be enqueued must stay \
             redeliverable, or the rest of it is lost with nothing to retry from"
        );
    }

    // ---- release: the worker delegates, the default impl still holds -----
    //
    // `RecordingDriver` deliberately does NOT override `QueueDriver::release`,
    // so these two exercise the trait's default push-then-ack fallback - the
    // path every third-party driver written before `release` existed still
    // takes. In-tree drivers override it and are covered by
    // `queue_database::release_*` and `queue_memory::*`.

    #[tokio::test]
    async fn the_default_release_pushes_before_acking_so_a_failed_push_keeps_the_job() {
        // Push fails: the reservation must NOT be acked, so the original
        // survives for visibility-expiry redelivery rather than being lost.
        let driver = RecordingDriver::new(true);
        let token = ReservationToken(Uuid::new_v4());
        let env = fresh_env("J", 1);

        handle_released(
            &driver,
            &token,
            &env,
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
    async fn the_default_release_acks_after_a_successful_push() {
        // Push succeeds: both the re-enqueue and the ack run, and the pushed
        // copy carries the decremented attempt count and shifted availability.
        let driver = RecordingDriver::new(false);
        let token = ReservationToken(Uuid::new_v4());
        let env = fresh_env("J", 2);
        let before = env.available_at;

        handle_released(
            &driver,
            &token,
            &env,
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
            "release does not burn an attempt - the pre-dispatch count is restored"
        );
        assert!(
            copy.available_at > before,
            "the released copy is delayed by the requested duration"
        );
    }

    /// The worker must call [`QueueDriver::release`] rather than open-coding
    /// push-then-ack, or a driver that CAN release atomically never gets the
    /// chance - which is precisely how the database driver ended up answering
    /// every release with a primary-key collision.
    #[tokio::test]
    async fn the_worker_delegates_the_release_to_the_driver() {
        struct AtomicReleaseDriver {
            ops: Arc<OpLog>,
        }

        #[async_trait]
        impl QueueDriver for AtomicReleaseDriver {
            async fn push(&self, _env: Envelope) -> Result<(), FrameworkError> {
                self.ops.record("push");
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
                Ok(())
            }
            async fn release(
                &self,
                _token: &ReservationToken,
                _env: &Envelope,
                _delay: Duration,
            ) -> Result<(), FrameworkError> {
                self.ops.record("release");
                Ok(())
            }
        }

        let ops = Arc::new(OpLog::default());
        let driver = AtomicReleaseDriver { ops: ops.clone() };
        let token = ReservationToken(Uuid::new_v4());
        let env = fresh_env("J", 1);

        handle_released(
            &driver,
            &token,
            &env,
            Duration::from_secs(5),
            "test",
            "middleware",
        )
        .await;

        assert_eq!(ops.count("release"), 1, "the driver's release is used");
        assert_eq!(
            (ops.count("push"), ops.count("ack")),
            (0, 0),
            "and the worker does not also push a copy or drop the reservation \
             behind the driver's back"
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
            "the successor must be enqueued before the reservation is dropped - \
             acking first means a crash in the window loses the chain forever"
        );
        let pushed = driver.pushed.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            pushed.first().expect("next link pushed").job_name,
            "Tail",
            "the pushed envelope is the next chain link"
        );
    }

    /// DATA-02b - the duplicate the pre-ack push ordering deliberately
    /// trades for, made identifiable.
    ///
    /// Settlement pushes the successor before acking, so a crash or a failed
    /// ack in that window redelivers the *same* envelope and runs the push
    /// again. This drives exactly that: settle the same envelope twice, as
    /// visibility expiry would, and assert both pushes carry one id.
    ///
    /// With `Uuid::new_v4()` the two ids differed, which meant no handler,
    /// driver, or outbox could tell a redelivered step from a new one - the
    /// framework's "handlers must be idempotent" contract was unsatisfiable
    /// for chained jobs, because the only identifier a handler is given was
    /// fresh every time.
    #[tokio::test]
    async fn a_redelivered_job_re_pushes_its_successor_under_the_same_id() {
        let driver = RecordingDriver::new(false);
        let token = ReservationToken(Uuid::new_v4());
        let mut env = fresh_env("Head", 1);
        env.chain_remaining = vec![chain_link("Tail")];

        // First settlement, then the redelivery of the very same envelope.
        handle_completed(&driver, &token, &env, "test", &no_deps()).await;
        handle_completed(&driver, &token, &env, "test", &no_deps()).await;

        let pushed = driver.pushed.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(pushed.len(), 2, "the redelivery pushes the successor again");
        assert_eq!(
            pushed[0].id, pushed[1].id,
            "a redelivered chain step must re-push its successor under the id \
             it used before; two ids for one logical step is a duplicate \
             nothing downstream can recognise"
        );
    }

    /// …while two *different* predecessors still yield distinct successors,
    /// so the derivation cannot collapse unrelated chains onto one id.
    #[tokio::test]
    async fn distinct_chain_steps_keep_distinct_successor_ids() {
        let driver = RecordingDriver::new(false);
        let token = ReservationToken(Uuid::new_v4());

        for name in ["HeadA", "HeadB"] {
            let mut env = fresh_env(name, 1);
            env.chain_remaining = vec![chain_link("Tail")];
            handle_completed(&driver, &token, &env, "test", &no_deps()).await;
        }

        let pushed = driver.pushed.lock().unwrap_or_else(|e| e.into_inner());
        assert_ne!(
            pushed[0].id, pushed[1].id,
            "successors of different envelopes must not share an id"
        );
    }

    #[tokio::test]
    async fn completed_chain_push_failure_leaves_the_job_redeliverable() {
        // Failure mode: the follow-up cannot land. The reservation must stay
        // live so visibility expiry redelivers the envelope - a duplicate run
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
             dropped - otherwise a crash strands the batch on pending > 0"
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
            "the failed-jobs record IS the recovery path - it must be durable \
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
        // silently swallowed the job - acked first, record never written, no
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
        // Both follow-ups, in order, ahead of the ack - and a failing batch
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

    #[test]
    fn diff_paused_queues_reports_only_the_transitions() {
        // Nothing changed: no events.
        let (paused, resumed) = diff_paused_queues(&["a".into()], &["a".into()]);
        assert!(paused.is_empty() && resumed.is_empty());

        // One newly paused, in the order `current` lists it.
        let (paused, resumed) = diff_paused_queues(&[], &["a".into(), "b".into()]);
        assert_eq!(paused, vec!["a".to_string(), "b".to_string()]);
        assert!(resumed.is_empty());

        // One resumed while another stays paused.
        let (paused, resumed) = diff_paused_queues(&["a".into(), "b".into()], &["b".into()]);
        assert!(paused.is_empty());
        assert_eq!(resumed, vec!["a".to_string()]);

        // Both directions in one iteration.
        let (paused, resumed) = diff_paused_queues(&["a".into()], &["b".into()]);
        assert_eq!(paused, vec!["b".to_string()]);
        assert_eq!(resumed, vec!["a".to_string()]);
    }
}
