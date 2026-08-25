//! `InspectedJob` - the DTO returned by the queue inspection API
//! (`pending_jobs` / `delayed_jobs` / `reserved_jobs`).
//!
//! Mirrors Laravel's `Illuminate\Queue\Jobs\InspectedJob`. See the
//! "Inspecting queues" section of `manual/queues.md` for the divergences
//! from Laravel's shape.

use crate::queue::envelope::Envelope;
use chrono::{DateTime, Utc};
use serde::Serialize;

/// One envelope as seen by [`QueueDriver::pending_jobs`](crate::queue::driver::QueueDriver::pending_jobs)
/// / `delayed_jobs` / `reserved_jobs`.
///
/// `id` and `created_at` are `Option` because not every source can supply
/// them:
///
/// - The database driver's listing methods decode each row's
///   `envelope_json`; a row whose JSON fails to parse is still reported
///   (rather than dropped, which would hide a poison job from an operator)
///   with `id: None` and a `payload` that flags it as unparseable, since
///   there is no envelope to recover an id or a dispatch timestamp from.
/// - `Queue::fake()`'s projection (`queue::testing::pending_jobs` /
///   `delayed_jobs`) never records a dispatch timestamp distinct from
///   `available_at`, so `created_at` is always `None` there; `id` is still
///   populated because the fake stamps one on every recorded push.
#[derive(Debug, Clone, Serialize)]
pub struct InspectedJob {
    /// Envelope id, when the source could recover one.
    pub id: Option<uuid::Uuid>,
    /// Queue the envelope was routed to. `None` means the driver's default
    /// queue - the same convention [`Envelope::queue`] uses.
    pub queue: Option<String>,
    /// Job type name (`Job::job_name()`).
    pub name: String,
    /// Delivery attempts recorded against this envelope. Always `0` under
    /// `Queue::fake()`, since nothing ever runs (and therefore nothing is
    /// ever retried) under the fake.
    pub attempts: u32,
    /// Typed handler payload as JSON.
    pub payload: serde_json::Value,
    /// When the envelope was first pushed, when known.
    pub created_at: Option<DateTime<Utc>>,
}

impl InspectedJob {
    /// Build from a fully-decoded [`Envelope`] - the common case for every
    /// driver that stores or reconstructs the envelope in full (memory,
    /// a successfully-parsed database row, Redis).
    pub fn from_envelope(env: &Envelope) -> Self {
        Self {
            id: Some(env.id),
            queue: env.queue.clone(),
            name: env.job_name.clone(),
            attempts: env.attempts,
            payload: env.payload.clone(),
            created_at: Some(env.dispatched_at),
        }
    }
}
