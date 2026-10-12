//! Typed queue errors mirroring Laravel 13's queue exception classes.
//!
//! These name the cause of a failed attempt (timeout, max-attempts
//! exhausted, manual fail) so callers can match on it rather than on a
//! message. The worker wraps a timed-out attempt's [`TimeoutExceeded`] with
//! `FrameworkError::from_external`, and reports it through `Exceptions` like
//! any failed attempt: a `reportable` callback taking `&TimeoutExceeded`
//! receives it, and `Exceptions::dont_retry::<TimeoutExceeded>()` ends the
//! retries of a job that times out (PAR-111).

use std::time::Duration;
use thiserror::Error;

/// Thrown by the worker when a job exhausts its `max_tries` budget. Mirrors
/// `Illuminate\Queue\MaxAttemptsExceededException`. Carries the job name and
/// the attempt count for the failed-job record.
#[derive(Debug, Clone, Error)]
#[error("queue job '{job_name}' exhausted max_tries after {attempts} attempts: {reason}")]
pub struct MaxAttemptsExceeded {
    /// Fully-qualified job type name.
    pub job_name: String,
    /// Total dispatch attempts the worker exhausted.
    pub attempts: u32,
    /// Formatted display of the final failure cause.
    pub reason: String,
}

/// Thrown when a job's per-attempt `timeout()` budget is exceeded. Mirrors
/// `Illuminate\Queue\TimeoutExceededException`.
#[derive(Debug, Clone, Error)]
#[error("queue job '{job_name}' exceeded its per-attempt timeout of {timeout:?}")]
pub struct TimeoutExceeded {
    /// Fully-qualified job type name.
    pub job_name: String,
    /// Timeout budget the attempt blew past.
    pub timeout: Duration,
}

/// Thrown when a job middleware (or the handler itself) manually marked the
/// job as failed via `JobContext::fail`. Mirrors
/// `Illuminate\Queue\ManuallyFailedException`.
#[derive(Debug, Clone, Error)]
#[error("queue job '{job_name}' was manually failed: {reason}")]
pub struct ManuallyFailed {
    /// Fully-qualified job type name.
    pub job_name: String,
    /// Operator-supplied failure reason.
    pub reason: String,
}
