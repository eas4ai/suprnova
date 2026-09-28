//! `Queue::fake()` - installs an in-memory recorder that captures
//! dispatched jobs without running them.
//!
//! `install_fake()` acquires a process-wide serialization mutex for the
//! lifetime of the returned `QueueFakeGuard`. This prevents parallel tests
//! from clobbering each other's fake store.
//!
//! Recorded pushes carry their `available_at` so tests can assert delayed
//! dispatch timestamps through [`pushed_with_available_at`] /
//! [`assert_pushed_later`] without leaving the fake surface.

use crate::error::FrameworkError;
use crate::queue::{EnvelopeOverrides, InspectedJob, Job};
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use std::any::TypeId;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use uuid::Uuid;

/// One captured push: the envelope id the fake assigned, the serialized
/// job payload, and the `available_at` the facade dispatched with.
/// `Queue::push` records `Utc::now()`; the `*_later` variants record the
/// explicit timestamp.
///
/// The id exists so a test can join a captured push to the `JobQueued`
/// event a listener saw - the real path stamps one per envelope, and the
/// fake would otherwise be the only enqueue in the framework without an
/// identity.
#[derive(Clone)]
struct FakePush {
    id: Uuid,
    /// `J::job_name()` at record time - carried so a listing that spans
    /// every job type (see [`pending_jobs`] / [`delayed_jobs`]) can name
    /// what it captured without knowing `J`.
    job_name: &'static str,
    /// The queue this push would have been routed to:
    /// `overrides.queue.clone().or_else(|| J::queue().map(str::to_owned))`.
    /// Never consults [`Queue::route`](crate::queue::Queue::route) -
    /// routing resolution doesn't run under the fake.
    queue: Option<String>,
    payload: serde_json::Value,
    available_at: DateTime<Utc>,
    /// Per-push [`EnvelopeOverrides`] as declared to
    /// [`Queue::push_with`](crate::queue::Queue::push_with) /
    /// [`Queue::later_with`](crate::queue::Queue::later_with).
    /// `EnvelopeOverrides::default()` for every other entry point
    /// (`push`, `push_later`, `bulk`, `push_unique`, …), none of which
    /// take one.
    overrides: EnvelopeOverrides,
}

impl FakePush {
    /// Project this recorded push as an [`InspectedJob`]. `attempts` is
    /// always `0` (nothing runs under the fake, so nothing is ever
    /// retried) and `created_at` is always `None` - the fake never records
    /// a dispatch timestamp distinct from `available_at`, so there is
    /// nothing honest to report there; use [`pushed_with_available_at`] if
    /// the scheduled time matters to your test.
    fn to_inspected(&self) -> InspectedJob {
        InspectedJob {
            id: Some(self.id),
            queue: self.queue.clone(),
            name: self.job_name.to_string(),
            attempts: 0,
            payload: self.payload.clone(),
            created_at: None,
        }
    }
}

#[derive(Default)]
struct FakeStore {
    pushed: HashMap<TypeId, Vec<FakePush>>,
}

/// Process-wide serializer: only one test may hold the fake at a time.
static FAKE_SERIAL: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static FAKE: Mutex<Option<FakeStore>> = Mutex::new(None);

fn lock_fake() -> std::sync::MutexGuard<'static, Option<FakeStore>> {
    FAKE.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn is_active() -> bool {
    lock_fake().is_some()
}

pub(crate) fn record<J: Job>(job: &J, available_at: DateTime<Utc>) -> Result<Uuid, FrameworkError> {
    record_with_overrides::<J>(job, available_at, EnvelopeOverrides::default())
}

/// Like [`record`], but also captures the [`EnvelopeOverrides`] a
/// [`Queue::push_with`](crate::queue::Queue::push_with) /
/// [`Queue::later_with`](crate::queue::Queue::later_with) caller declared.
/// Without this, a push's queue/connection/timeout/etc overrides were
/// silently dropped under the fake, making a `push_with` caller
/// indistinguishable from a plain `push` - see [`pushed_with_overrides`].
pub(crate) fn record_with_overrides<J: Job>(
    job: &J,
    available_at: DateTime<Utc>,
    overrides: EnvelopeOverrides,
) -> Result<Uuid, FrameworkError> {
    let payload =
        serde_json::to_value(job).map_err(|e| FrameworkError::internal(format!("encode: {e}")))?;
    let id = Uuid::new_v4();
    let queue = overrides
        .queue
        .clone()
        .or_else(|| J::queue().map(str::to_owned));
    let mut g = lock_fake();
    if let Some(store) = g.as_mut() {
        store
            .pushed
            .entry(TypeId::of::<J>())
            .or_default()
            .push(FakePush {
                id,
                job_name: J::job_name(),
                queue,
                payload,
                available_at,
                overrides,
            });
    }
    Ok(id)
}

/// Install the queue fake for the current test.
///
/// The returned `QueueFakeGuard` holds a process-wide serialization lock,
/// preventing parallel tests from running simultaneously and interfering
/// with each other's store. It also clears the store on drop.
pub fn install_fake() -> QueueFakeGuard {
    let serial = FAKE_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    *lock_fake() = Some(FakeStore::default());
    QueueFakeGuard { _serial: serial }
}

/// RAII guard returned by [`install_fake`]. Holds the process-wide
/// serialization lock and clears the fake store on drop.
pub struct QueueFakeGuard {
    _serial: MutexGuard<'static, ()>,
}

impl Drop for QueueFakeGuard {
    fn drop(&mut self) {
        // Use unwrap_or_else so a poisoned mutex from a test failure never
        // causes a double-panic (which would abort the process).
        *lock_fake() = None;
    }
}

/// Assert at least one captured push of `J` satisfies `pred`. Panics
/// when no match is found.
pub fn assert_pushed<J: Job>(pred: impl Fn(&J) -> bool) {
    let g = lock_fake();
    let store = g.as_ref().expect("Queue::fake() must be active");
    let bucket = store.pushed.get(&TypeId::of::<J>());
    let count = bucket
        .map(|b| {
            b.iter()
                .filter_map(|p| serde_json::from_value::<J>(p.payload.clone()).ok())
                .filter(|j| pred(j))
                .count()
        })
        .unwrap_or(0);
    assert!(count > 0, "expected at least one pushed {}", J::job_name());
}

/// All captured pushes of `J` with their `available_at`. Use this in tests
/// that need to assert delayed-dispatch timestamps (e.g. that
/// `Queue::push_later(job, t)` recorded `t`, not `now`).
pub fn pushed_with_available_at<J: Job>() -> Vec<(J, DateTime<Utc>)> {
    let g = lock_fake();
    let store = g.as_ref().expect("Queue::fake() must be active");
    store
        .pushed
        .get(&TypeId::of::<J>())
        .map(|b| {
            b.iter()
                .filter_map(|p| {
                    serde_json::from_value::<J>(p.payload.clone())
                        .ok()
                        .map(|j| (j, p.available_at))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Like [`assert_pushed`] but receives `(job, available_at)` so tests can
/// pin the scheduled timestamp.
pub fn assert_pushed_later<J: Job>(pred: impl Fn(&J, DateTime<Utc>) -> bool) {
    let g = lock_fake();
    let store = g.as_ref().expect("Queue::fake() must be active");
    let count = store
        .pushed
        .get(&TypeId::of::<J>())
        .map(|b| {
            b.iter()
                .filter_map(|p| {
                    serde_json::from_value::<J>(p.payload.clone())
                        .ok()
                        .map(|j| (j, p.available_at))
                })
                .filter(|(j, t)| pred(j, *t))
                .count()
        })
        .unwrap_or(0);
    assert!(
        count > 0,
        "expected at least one pushed {} matching (job, available_at)",
        J::job_name()
    );
}

/// All captured pushes of `J` deserialized back into the typed payload.
pub fn pushed<J: Job>() -> Vec<J> {
    let g = lock_fake();
    let store = g.as_ref().expect("Queue::fake() must be active");
    store
        .pushed
        .get(&TypeId::of::<J>())
        .map(|b| {
            b.iter()
                .filter_map(|p| serde_json::from_value::<J>(p.payload.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// All captured pushes of `J` paired with the envelope id the fake
/// assigned. Mirrors Laravel's `QueueFake` stamping a `uuid` on every
/// inspected job.
///
/// Use it to join what the fake captured to what a listener saw:
/// `Queue::push` under the fake dispatches the same
/// [`JobQueued`](crate::queue::events::JobQueued) a real driver push
/// would, carrying this id.
pub fn pushed_with_id<J: Job>() -> Vec<(J, Uuid)> {
    let g = lock_fake();
    let store = g.as_ref().expect("Queue::fake() must be active");
    store
        .pushed
        .get(&TypeId::of::<J>())
        .map(|b| {
            b.iter()
                .filter_map(|p| {
                    serde_json::from_value::<J>(p.payload.clone())
                        .ok()
                        .map(|j| (j, p.id))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// All captured pushes of `J` paired with the [`EnvelopeOverrides`] the
/// caller declared via
/// [`Queue::push_with`](crate::queue::Queue::push_with) /
/// [`Queue::later_with`](crate::queue::Queue::later_with). Every other
/// entry point records `EnvelopeOverrides::default()`, since none of them
/// take one - that default is indistinguishable from "no override was
/// declared", the same way a bare `Queue::push` would read.
///
/// Use [`assert_pushed_on_queue`] / [`assert_pushed_on_connection`] for the
/// common case of asserting a single field; use this directly for anything
/// else the envelope overlay carries (timeout, backoff, max_tries,
/// fail_on_timeout).
pub fn pushed_with_overrides<J: Job>() -> Vec<(J, EnvelopeOverrides)> {
    let g = lock_fake();
    let store = g.as_ref().expect("Queue::fake() must be active");
    store
        .pushed
        .get(&TypeId::of::<J>())
        .map(|b| {
            b.iter()
                .filter_map(|p| {
                    serde_json::from_value::<J>(p.payload.clone())
                        .ok()
                        .map(|j| (j, p.overrides.clone()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Assert at least one captured push of `J` declared `queue` via
/// [`EnvelopeOverrides`] (i.e. was pushed through
/// [`Queue::push_with`](crate::queue::Queue::push_with) /
/// [`Queue::later_with`](crate::queue::Queue::later_with) with
/// `overrides.queue == Some(queue)`). Panics with every captured override
/// set if no match is found.
///
/// Mirrors [`MailFake::assert_queued_on`](crate::mail::MailFake::assert_queued_on)
/// so the two fakes read alike; unlike that method, this checks the
/// declared override rather than a fully resolved queue name, since
/// [`Queue::route`](crate::queue::Queue::route) / [`Job::queue`]
/// resolution never runs under the fake.
pub fn assert_pushed_on_queue<J: Job>(queue: &str) {
    let entries = pushed_with_overrides::<J>();
    let matching = entries
        .iter()
        .filter(|(_, o)| o.queue.as_deref() == Some(queue))
        .count();
    assert!(
        matching > 0,
        "expected at least one pushed {} with EnvelopeOverrides.queue == {:?}; \
         captured {} push(es) with overrides: {:#?}",
        J::job_name(),
        queue,
        entries.len(),
        entries.iter().map(|(_, o)| o).collect::<Vec<_>>()
    );
}

/// Assert at least one captured push of `J` declared `connection` via
/// [`EnvelopeOverrides`] (i.e. was pushed through
/// [`Queue::push_with`](crate::queue::Queue::push_with) /
/// [`Queue::later_with`](crate::queue::Queue::later_with) with
/// `overrides.connection == Some(connection)`). Panics with every captured
/// override set if no match is found.
pub fn assert_pushed_on_connection<J: Job>(connection: &str) {
    let entries = pushed_with_overrides::<J>();
    let matching = entries
        .iter()
        .filter(|(_, o)| o.connection.as_deref() == Some(connection))
        .count();
    assert!(
        matching > 0,
        "expected at least one pushed {} with EnvelopeOverrides.connection == {:?}; \
         captured {} push(es) with overrides: {:#?}",
        J::job_name(),
        connection,
        entries.len(),
        entries.iter().map(|(_, o)| o).collect::<Vec<_>>()
    );
}

/// Every recorded push, across every job type, whose `available_at <= now`,
/// projected as [`InspectedJob`]. The fake's stand-in for
/// [`QueueDriver::pending_jobs`](crate::queue::driver::QueueDriver::pending_jobs) -
/// `attempts` is always `0` and `created_at` is always `None`, since nothing
/// runs (and so nothing is ever retried) under the fake, and the fake never
/// records a dispatch timestamp separate from `available_at`.
///
/// Unlike [`pushed`], this is not generic over `J`: `InspectedJob` is
/// already type-erased (`name` + `payload`), so it aggregates across every
/// job type the fake has recorded, matching what a real driver's listing
/// would return.
pub fn pending_jobs() -> Vec<InspectedJob> {
    let g = lock_fake();
    let store = g.as_ref().expect("Queue::fake() must be active");
    let now = Utc::now();
    store
        .pushed
        .values()
        .flatten()
        .filter(|p| p.available_at <= now)
        .map(FakePush::to_inspected)
        .collect()
}

/// Every recorded push, across every job type, whose `available_at > now`.
/// The fake's stand-in for
/// [`QueueDriver::delayed_jobs`](crate::queue::driver::QueueDriver::delayed_jobs).
/// See [`pending_jobs`] for the projection caveats.
pub fn delayed_jobs() -> Vec<InspectedJob> {
    let g = lock_fake();
    let store = g.as_ref().expect("Queue::fake() must be active");
    let now = Utc::now();
    store
        .pushed
        .values()
        .flatten()
        .filter(|p| p.available_at > now)
        .map(FakePush::to_inspected)
        .collect()
}
