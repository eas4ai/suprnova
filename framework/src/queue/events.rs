//! Queue lifecycle events.
//!
//! Mirrors Laravel 13's `Illuminate\Queue\Events\*`. The worker emits these
//! through the standard [`crate::events::Event`] facade so
//! observers (admin dashboards, custom listeners) can hook in via
//! `Event::listen`. Events carry envelope metadata (not the typed job
//! instance) because the worker is type-erased over JSON payloads.
//!
//! `FrameworkError` doesn't implement `Clone`, so failure events carry the
//! error as a `String` (the formatted display). That's enough for logging
//! and listener-side classification (string prefix / contains checks)
//! without forcing every listener to hold the full error chain.
//!
//! These events are best-effort - `Event::dispatch` with no listeners
//! registered is a no-op `Ok(())`, so workers that emit them in
//! deployments without `Event::init()` pay nothing.

use crate::events::Event;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

/// Snapshot of the envelope's identity, carried by every queue event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobIdentity {
    /// Unique envelope identifier assigned by the driver.
    pub id: Uuid,
    /// Fully-qualified job type name (e.g. `"App\\Jobs\\SendInvoice"`).
    pub job_name: String,
    /// Number of times the worker has dispatched this job, including the current attempt.
    pub attempts: u32,
    /// Maximum dispatch attempts before the worker dead-letters the job.
    pub max_tries: u32,
    /// Driver connection name the envelope lives on.
    pub connection: String,
}

impl JobIdentity {
    pub(crate) fn from_env(env: &crate::queue::Envelope, connection: &str) -> Self {
        Self {
            id: env.id,
            job_name: env.job_name.clone(),
            attempts: env.attempts,
            max_tries: env.max_tries,
            connection: connection.to_string(),
        }
    }
}

/// Fired before the envelope is committed to the driver (sync path of
/// `Queue::push`). Mirrors `Illuminate\Queue\Events\JobQueueing`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobQueueing {
    /// Fully-qualified job type name (e.g. `"App\\Jobs\\SendInvoice"`).
    pub job_name: String,
    /// Driver connection name the envelope is bound for.
    pub connection: String,
}

impl Event for JobQueueing {
    fn event_name() -> &'static str {
        "queue::JobQueueing"
    }
}

/// Fired after the envelope is successfully committed to the driver.
/// Mirrors `Illuminate\Queue\Events\JobQueued`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobQueued {
    /// Unique envelope identifier assigned by the driver.
    pub id: Uuid,
    /// Fully-qualified job type name (e.g. `"App\\Jobs\\SendInvoice"`).
    pub job_name: String,
    /// Driver connection name the envelope was committed to.
    pub connection: String,
}

impl Event for JobQueued {
    fn event_name() -> &'static str {
        "queue::JobQueued"
    }
}

/// Fired when the worker pops an envelope and is about to dispatch it.
/// Mirrors `Illuminate\Queue\Events\JobProcessing`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobProcessing {
    /// Identity of the job about to be dispatched.
    pub job: JobIdentity,
}

impl Event for JobProcessing {
    fn event_name() -> &'static str {
        "queue::JobProcessing"
    }
}

/// Fired after a successful run. Mirrors
/// `Illuminate\Queue\Events\JobProcessed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobProcessed {
    /// Identity of the job that completed successfully.
    pub job: JobIdentity,
}

impl Event for JobProcessed {
    fn event_name() -> &'static str {
        "queue::JobProcessed"
    }
}

/// Fired immediately after a job attempt resolves to a terminal outcome
/// (success / fail / timeout - not retry). Mirrors
/// `Illuminate\Queue\Events\JobAttempted`. Distinct from [`JobProcessed`]:
/// `JobAttempted` fires for every terminal settlement, while
/// `JobProcessed` only fires on a clean success.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobAttempted {
    /// Identity of the job whose attempt just settled.
    pub job: JobIdentity,
}

impl Event for JobAttempted {
    fn event_name() -> &'static str {
        "queue::JobAttempted"
    }
}

/// Fired when a job throws and the worker is about to decide retry vs
/// dead-letter. Mirrors `Illuminate\Queue\Events\JobExceptionOccurred`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobExceptionOccurred {
    /// Identity of the job that threw.
    pub job: JobIdentity,
    /// Formatted display of the error that was raised.
    pub exception: String,
}

impl Event for JobExceptionOccurred {
    fn event_name() -> &'static str {
        "queue::JobExceptionOccurred"
    }
}

/// Fired when the worker dead-letters a job (max_tries exhausted, fatal
/// timeout, manual fail). Mirrors `Illuminate\Queue\Events\JobFailed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobFailed {
    /// Identity of the job that was dead-lettered.
    pub job: JobIdentity,
    /// Formatted display of the final error that caused the failure.
    pub exception: String,
}

impl Event for JobFailed {
    fn event_name() -> &'static str {
        "queue::JobFailed"
    }
}

/// Fired after the worker re-enqueues a failed job (not on release via
/// middleware, which uses [`JobReleased`] instead). Mirrors
/// `Illuminate\Queue\Events\JobReleasedAfterException`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobReleasedAfterException {
    /// Identity of the job being retried.
    pub job: JobIdentity,
    /// Formatted display of the error that triggered the back-off.
    pub exception: String,
    /// Computed back-off in seconds before the next attempt.
    pub delay_secs: u64,
}

impl Event for JobReleasedAfterException {
    fn event_name() -> &'static str {
        "queue::JobReleasedAfterException"
    }
}

/// Fired when middleware (or manual `release(delay)`) re-enqueues a job
/// **without** counting it as a failed attempt. Distinct from
/// [`JobReleasedAfterException`] - the original Laravel split, kept here
/// so listeners can distinguish "back-off after error" from "retry later
/// because lock/throttle was busy".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobReleased {
    /// Identity of the job that was released back to the queue.
    pub job: JobIdentity,
    /// Delay in seconds before the job becomes eligible for re-claim.
    pub delay_secs: u64,
    /// Reason supplied by the middleware (e.g. `"rate_limited"`, `"locked"`).
    pub reason: String,
}

impl Event for JobReleased {
    fn event_name() -> &'static str {
        "queue::JobReleased"
    }
}

/// Fired when a job times out during dispatch. Mirrors
/// `Illuminate\Queue\Events\JobTimedOut`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobTimedOut {
    /// Identity of the job that exceeded its timeout.
    pub job: JobIdentity,
    /// Timeout budget the job blew past.
    pub timeout: Duration,
}

impl Event for JobTimedOut {
    fn event_name() -> &'static str {
        "queue::JobTimedOut"
    }
}

/// Fired every iteration of the worker loop, after pop+dispatch settles.
/// Mirrors `Illuminate\Queue\Events\Looping`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Looping {
    /// Driver connection name the worker just polled.
    pub connection: String,
}

impl Event for Looping {
    fn event_name() -> &'static str {
        "queue::Looping"
    }
}

/// Fired once when [`run_worker`](crate::queue::worker::run_worker) starts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStarting {
    /// Driver connection name the worker is starting on.
    pub connection: String,
}

impl Event for WorkerStarting {
    fn event_name() -> &'static str {
        "queue::WorkerStarting"
    }
}

/// Fired once when [`run_worker`](crate::queue::worker::run_worker) exits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStopping {
    /// Driver connection name the worker was draining.
    pub connection: String,
    /// Total jobs the worker settled before exiting.
    pub processed: u64,
}

impl Event for WorkerStopping {
    fn event_name() -> &'static str {
        "queue::WorkerStopping"
    }
}

/// Fired when a `Queue::restart()` signal causes a running worker to exit
/// cleanly without claiming additional work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerInterrupted {
    /// Driver connection name the worker was draining.
    pub connection: String,
    /// Total jobs the worker settled before honoring the restart signal.
    pub processed: u64,
}

impl Event for WorkerInterrupted {
    fn event_name() -> &'static str {
        "queue::WorkerInterrupted"
    }
}

/// Fired when [`Queue::push_unique`](crate::queue::Queue::push_unique)
/// (or one of its `*_later` / `*_at` siblings) suppressed an enqueue
/// because an identical `(job_name, unique_id)` was already in flight
/// within the job's [`Job::unique_for`](crate::queue::Job::unique_for)
/// window. Mirrors `Illuminate\Queue\Events\UniqueJobSkipped`.
///
/// The suppression is the feature working, not a failure - the return
/// value stays `Ok(false)`. The event exists because a silent suppression
/// is invisible: without it, "the job never ran" and "the job was
/// deduped" look identical from outside the process.
///
/// Carries the composed identity rather than the typed job: the dedupe
/// decision is made before an envelope exists, so there is no envelope id
/// to report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniqueJobSkipped {
    /// Fully-qualified job type name (`J::job_name()`).
    pub job_name: String,
    /// The `Job::unique_id(&self)` value whose window was still open.
    pub unique_id: String,
    /// Driver connection name the push was routed to.
    pub connection: String,
}

impl Event for UniqueJobSkipped {
    fn event_name() -> &'static str {
        "queue::UniqueJobSkipped"
    }
}

/// Fired by [`Queue::pause_all`](crate::queue::Queue::pause_all). A marker
/// with no fields. Mirrors Laravel's
/// `Illuminate\Queue\Events\QueuesPaused`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuesPaused;

impl Event for QueuesPaused {
    fn event_name() -> &'static str {
        "queue::QueuesPaused"
    }
}

/// Fired by [`Queue::resume_all`](crate::queue::Queue::resume_all). A
/// marker with no fields. Mirrors Laravel's
/// `Illuminate\Queue\Events\QueuesResumed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuesResumed;

impl Event for QueuesResumed {
    fn event_name() -> &'static str {
        "queue::QueuesResumed"
    }
}

/// Fired by [`Queue::pause`](crate::queue::Queue::pause). Mirrors
/// Laravel's `Illuminate\Queue\Events\QueuePaused` (minus its optional
/// `$ttl` - Suprnova has no `pauseFor` equivalent).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuePaused {
    /// Connection name the pause applies to.
    pub connection: String,
    /// Queue name that was paused.
    pub queue: String,
}

impl Event for QueuePaused {
    fn event_name() -> &'static str {
        "queue::QueuePaused"
    }
}

/// Fired by [`Queue::resume`](crate::queue::Queue::resume). Mirrors
/// Laravel's `Illuminate\Queue\Events\QueueResumed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueResumed {
    /// Connection name the resume applies to.
    pub connection: String,
    /// Queue name that was resumed.
    pub queue: String,
}

impl Event for QueueResumed {
    fn event_name() -> &'static str {
        "queue::QueueResumed"
    }
}

/// A push failed over from one connection to the next in a
/// [`FailoverQueueDriver`](crate::queue::FailoverQueueDriver)'s list.
///
/// Edge-triggered: it fires when a connection *enters* failure, not on every
/// rejected push, and re-arms when the connection recovers. A queue whose
/// primary has been down for an hour therefore produces one alert, not one
/// per dispatch - which is what makes it usable as an alerting signal rather
/// than a log firehose.
///
/// `connection` is the configured label of the connection that failed, not
/// the one that eventually accepted the job. Mirrors Laravel's
/// `Illuminate\Queue\Events\QueueFailedOver`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueFailedOver {
    /// The configured label of the failing connection (e.g. `"redis"`).
    pub connection: String,
    /// The job that was being pushed.
    pub job_name: String,
    /// Display form of the error the failing connection returned.
    pub exception: String,
}

impl Event for QueueFailedOver {
    fn event_name() -> &'static str {
        "queue::QueueFailedOver"
    }
}
