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
//!
//! Every path that writes to the driver records here instead: the
//! `Queue::push` family, [`PendingBatch::dispatch`], [`PendingChain::dispatch`]
//! and the failed-job retries. A batch is recorded as a [`FakedBatch`] and a
//! chain as a [`FakedChain`], and their jobs are recorded as pushes too, so a
//! test that only cares that a job was queued does not need to know which
//! path queued it.
//!
//! [`QueueFakeGuard::except`] narrows the fake: the job types it names take
//! the real path to the real queue, and only the rest are recorded.
//! [`Queue::push_raw`] payloads are kept apart from typed pushes and read back
//! with [`raw_pushes`] / [`pushed_raw`].
//!
//! [`PendingBatch::dispatch`]: crate::queue::PendingBatch::dispatch
//! [`PendingChain::dispatch`]: crate::queue::PendingChain::dispatch
//! [`Queue::push_raw`]: crate::queue::Queue::push_raw

use crate::error::FrameworkError;
use crate::queue::{Envelope, EnvelopeError, EnvelopeOverrides, InspectedJob, Job};
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard};
use uuid::Uuid;

/// One captured push: the envelope id the fake assigned, the serialized
/// job payload, and the `available_at` the facade dispatched with.
/// `Queue::push` records `suprnova::clock::now()`; the `*_later` variants record the
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
    /// what it captured without knowing `J`. Owned, because a batch member
    /// or a chain head arrives as a built envelope whose name is a `String`.
    job_name: String,
    /// The queue this push would have been routed to:
    /// `overrides.queue.clone().or_else(|| J::queue().map(str::to_owned))`,
    /// or the envelope's own queue for a push that arrived already built.
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
    /// The job name of every link chained after this push, in order. Empty
    /// for every push except the head of a chain of two or more jobs, which
    /// is what [`assert_pushed_without_chain`] reads.
    chained: Vec<String>,
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
            name: self.job_name.clone(),
            attempts: 0,
            payload: self.payload.clone(),
            created_at: None,
        }
    }
}

#[derive(Default)]
struct FakeStore {
    /// Keyed by `Job::job_name()`, not by the job's type: a batch member, a
    /// chain head and a retried failed job reach the facade as built
    /// envelopes, with the job type erased. The name is the one identity
    /// both a typed push and a built envelope carry, and `Job::job_name`
    /// is documented as unique per job type.
    pushed: HashMap<String, Vec<FakePush>>,
    batches: Vec<FakedBatch>,
    chains: Vec<FakedChain>,
    /// Payloads handed to [`Queue::push_raw`](crate::queue::Queue::push_raw),
    /// in push order. Kept apart from `pushed` because a raw push names no
    /// job type, as Laravel's `QueueFake::$rawPushes` is kept apart from its
    /// `$jobs`.
    raw: Vec<RawPush>,
    /// `Job::job_name()`s that [`QueueFakeGuard::except`] sends to the real
    /// queue instead of recording.
    except: HashSet<String>,
}

/// Process-wide serializer: only one test may hold the fake at a time.
static FAKE_SERIAL: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static FAKE: Mutex<Option<FakeStore>> = Mutex::new(None);

fn lock_fake() -> std::sync::MutexGuard<'static, Option<FakeStore>> {
    FAKE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether the fake is installed at all, whatever it excepts. The paths that
/// name no job type ask this: a raw push, and the driver lookup a retry of
/// every failed job does up front.
pub(crate) fn is_active() -> bool {
    lock_fake().is_some()
}

/// Whether the fake records a push of the job named `job_name`: it is
/// installed, and [`QueueFakeGuard::except`] does not name the job. Every
/// path that pushes a job asks this rather than [`is_active`], so an excepted
/// job takes the same real path it would take without the fake.
pub(crate) fn fakes(job_name: &str) -> bool {
    lock_fake()
        .as_ref()
        .is_some_and(|store| !store.except.contains(job_name))
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
            .entry(J::job_name().to_owned())
            .or_default()
            .push(FakePush {
                id,
                job_name: J::job_name().to_owned(),
                queue,
                payload,
                available_at,
                overrides,
                chained: Vec::new(),
            });
    }
    Ok(id)
}

/// Record a push that reached the facade as a built envelope: a batch
/// member, the head of a chain, a retried failed job.
///
/// It lands in the same store a typed push does, so [`pushed`] and
/// [`assert_pushed`] see it. Laravel's `QueueFake` does the same: a batch
/// reaches it through `bulk`, and each job is then an ordinary recorded
/// push.
pub(crate) fn record_envelope(env: &Envelope) {
    let mut g = lock_fake();
    if let Some(store) = g.as_mut() {
        store
            .pushed
            .entry(env.job_name.clone())
            .or_default()
            .push(FakePush {
                id: env.id,
                job_name: env.job_name.clone(),
                queue: env.queue.clone(),
                payload: env.payload.clone(),
                available_at: env.available_at,
                overrides: EnvelopeOverrides::default(),
                chained: env
                    .chain_remaining
                    .iter()
                    .map(|link| link.job_name.clone())
                    .collect(),
            });
    }
}

/// Record a raw push in place of writing it to the driver. `payload` and
/// `queue` are kept exactly as the caller passed them, as Laravel's
/// `QueueFake::pushRaw` keeps them.
pub(crate) fn record_raw(payload: &str, queue: Option<&str>) {
    let mut g = lock_fake();
    if let Some(store) = g.as_mut() {
        store.raw.push(RawPush {
            payload: payload.to_owned(),
            queue: queue.map(str::to_owned),
        });
    }
}

/// Record a batch in place of dispatching it. Only the batch is recorded
/// here: the dispatch records each job the fake records as a push itself
/// (see [`record_envelope`]), because a job that
/// [`QueueFakeGuard::except`] names goes to the real queue instead.
pub(crate) fn record_batch(id: &str, name: &str, envelopes: &[Envelope]) {
    let mut g = lock_fake();
    if let Some(store) = g.as_mut() {
        store.batches.push(FakedBatch {
            id: id.to_owned(),
            name: name.to_owned(),
            jobs: envelopes.iter().map(InspectedJob::from_envelope).collect(),
        });
    }
}

/// Record a chain in place of dispatching it. `head` is the envelope the
/// driver would have received, with the rest of the chain on its
/// `chain_remaining`. The head is recorded as a push as well, because it is
/// the one job of the chain that is on the queue at dispatch.
pub(crate) fn record_chain(head: &Envelope) {
    record_envelope(head);
    let links = std::iter::once(InspectedJob::from_envelope(head))
        .chain(head.chain_remaining.iter().map(|link| InspectedJob {
            id: None,
            queue: link.queue.clone(),
            name: link.job_name.clone(),
            attempts: 0,
            payload: link.payload.clone(),
            created_at: None,
        }))
        .collect();
    let mut g = lock_fake();
    if let Some(store) = g.as_mut() {
        store.chains.push(FakedChain { links });
    }
}

/// A batch the queue fake recorded in place of dispatching it.
#[derive(Debug, Clone)]
pub struct FakedBatch {
    /// The id [`PendingBatch::dispatch`](crate::queue::PendingBatch::dispatch)
    /// returned to its caller.
    pub id: String,
    /// The name the batch was built with.
    pub name: String,
    /// The jobs of the batch, in the order they were added.
    pub jobs: Vec<InspectedJob>,
}

impl FakedBatch {
    /// The jobs of type `J` in this batch, decoded, in the order they were
    /// added. Jobs of another type are left out.
    pub fn jobs_of<J: Job>(&self) -> Vec<J> {
        self.jobs
            .iter()
            .filter(|job| job.name == J::job_name())
            .filter_map(|job| serde_json::from_value::<J>(job.payload.clone()).ok())
            .collect()
    }
}

/// A chain the queue fake recorded in place of dispatching it.
#[derive(Debug, Clone)]
pub struct FakedChain {
    /// The links of the chain, head first. Only the head carries an `id`
    /// and a `created_at`: a later link has no envelope until the link
    /// before it completes.
    pub links: Vec<InspectedJob>,
}

impl FakedChain {
    /// The job name of every link, head first.
    pub fn job_names(&self) -> Vec<&str> {
        self.links.iter().map(|link| link.name.as_str()).collect()
    }

    /// The link at `index`, decoded as `J`. `None` when the chain is
    /// shorter than that, or when the link there is not a `J`.
    pub fn link<J: Job>(&self, index: usize) -> Option<J> {
        let link = self.links.get(index)?;
        if link.name != J::job_name() {
            return None;
        }
        serde_json::from_value::<J>(link.payload.clone()).ok()
    }
}

/// A payload the queue fake recorded in place of a
/// [`Queue::push_raw`](crate::queue::Queue::push_raw). Mirrors one entry of
/// Laravel's `QueueFake::rawPushes()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPush {
    /// The payload exactly as it was passed: the JSON wire form of an
    /// [`Envelope`].
    pub payload: String,
    /// The queue the push named, `None` when it named none.
    pub queue: Option<String>,
}

impl RawPush {
    /// Decode the payload, so a test can match on the job name or the job's
    /// data rather than on the JSON text. `push_raw` refuses a payload that
    /// does not decode, so a recorded one always does.
    pub fn envelope(&self) -> Result<Envelope, EnvelopeError> {
        Envelope::from_json(&self.payload)
    }
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

/// Remove every connection registered with
/// [`Queue::register_connection`](crate::queue::Queue::register_connection).
///
/// The connections are process-wide. A test that registers one calls this
/// when it is done, so a connection it registered does not change what a
/// connection name means for the next test.
pub fn forget_connections() {
    crate::queue::connections::clear();
}

/// RAII guard returned by [`install_fake`]. Holds the process-wide
/// serialization lock and clears the fake store on drop.
pub struct QueueFakeGuard {
    _serial: MutexGuard<'static, ()>,
}

impl QueueFakeGuard {
    /// Send the jobs named in `job_names` to the real queue and record every
    /// other job. Each entry is a `Job::job_name()`. Mirrors Laravel's
    /// `Queue::fake()->except($jobs)`; [`Queue::fake_except`] is the
    /// one-call spelling.
    ///
    /// An excepted job takes the path it takes without the fake: it resolves
    /// its connection, reaches the driver, emits its lifecycle events, and
    /// fails where a real push fails. Calling `except` again adds to the
    /// names already excepted.
    ///
    /// A batch is still recorded, and only its excepted jobs reach the real
    /// queue. A chain follows its first job, because the worker that runs
    /// that job dispatches the rest. A [`Queue::push_raw`] is always
    /// recorded: a raw payload is not a job type, as in Laravel.
    ///
    /// [`Queue::fake_except`]: crate::queue::Queue::fake_except
    /// [`Queue::push_raw`]: crate::queue::Queue::push_raw
    pub fn except(self, job_names: &[&str]) -> Self {
        let mut g = lock_fake();
        if let Some(store) = g.as_mut() {
            store
                .except
                .extend(job_names.iter().map(|name| (*name).to_owned()));
        }
        drop(g);
        self
    }
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
    let bucket = store.pushed.get(J::job_name());
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
        .get(J::job_name())
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
        .get(J::job_name())
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
        .get(J::job_name())
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
        .get(J::job_name())
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
        .get(J::job_name())
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
    let now = crate::clock::now();
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
    let now = crate::clock::now();
    store
        .pushed
        .values()
        .flatten()
        .filter(|p| p.available_at > now)
        .map(FakePush::to_inspected)
        .collect()
}

/// Every batch recorded so far, in dispatch order.
pub fn batched() -> Vec<FakedBatch> {
    let g = lock_fake();
    let store = g.as_ref().expect("Queue::fake() must be active");
    store.batches.clone()
}

/// Assert at least one recorded batch satisfies `pred`. Panics with the
/// name and size of every recorded batch when none does. Mirrors Laravel's
/// `Bus::assertBatched`.
pub fn assert_batched(pred: impl Fn(&FakedBatch) -> bool) {
    let batches = batched();
    assert!(
        batches.iter().any(pred),
        "expected at least one batch to match; recorded {} batch(es): {:?}",
        batches.len(),
        batches
            .iter()
            .map(|b| (b.name.as_str(), b.jobs.len()))
            .collect::<Vec<_>>()
    );
}

/// Assert exactly `expected` batches were recorded. Mirrors Laravel's
/// `Bus::assertBatchCount`.
pub fn assert_batch_count(expected: usize) {
    let recorded = batched().len();
    assert_eq!(
        recorded, expected,
        "expected {expected} batch(es), recorded {recorded}"
    );
}

/// Assert no batch was recorded. Mirrors Laravel's
/// `Bus::assertNothingBatched`.
pub fn assert_nothing_batched() {
    assert_batch_count(0);
}

/// Every chain recorded so far, in dispatch order.
pub fn chained() -> Vec<FakedChain> {
    let g = lock_fake();
    let store = g.as_ref().expect("Queue::fake() must be active");
    store.chains.clone()
}

/// Assert at least one recorded chain is made of exactly these jobs, in
/// this order. Each entry is a `Job::job_name()`. Panics with the job names
/// of every recorded chain when none matches. Mirrors Laravel's
/// `Bus::assertChained`.
///
/// Use [`chained`] with [`FakedChain::link`] to assert on a link's payload.
pub fn assert_chained(job_names: &[&str]) {
    let chains = chained();
    assert!(
        chains.iter().any(|chain| chain.job_names() == job_names),
        "expected a chain of {:?}; recorded {} chain(s): {:?}",
        job_names,
        chains.len(),
        chains.iter().map(FakedChain::job_names).collect::<Vec<_>>()
    );
}

/// Assert no chain was recorded.
pub fn assert_nothing_chained() {
    let chains = chained();
    assert!(
        chains.is_empty(),
        "expected no chain, recorded {}: {:?}",
        chains.len(),
        chains.iter().map(FakedChain::job_names).collect::<Vec<_>>()
    );
}

/// Assert at least one captured push of `J` carried no chain. Mirrors
/// Laravel's `Queue::assertPushedWithoutChain`.
///
/// A job pushed on its own has no chain, and so do a batch member, a retried
/// job without a chain and a chain of one job. The head of a chain of two or
/// more jobs carries the rest of the chain, so a push of `J` that is only
/// ever such a head fails this. Panics when `J` was not pushed at all, and
/// when every push of it carried a chain, naming the chains it carried.
///
/// Use [`assert_chained`] to assert on the chain itself.
pub fn assert_pushed_without_chain<J: Job>() {
    let chains: Vec<Vec<String>> = {
        let g = lock_fake();
        let store = g.as_ref().expect("Queue::fake() must be active");
        store
            .pushed
            .get(J::job_name())
            .map(|pushes| pushes.iter().map(|push| push.chained.clone()).collect())
            .unwrap_or_default()
    };
    assert!(
        !chains.is_empty(),
        "expected at least one pushed {}",
        J::job_name()
    );
    assert!(
        chains.iter().any(Vec::is_empty),
        "expected a pushed {} without a chain; every push of it carried one: {:?}",
        J::job_name(),
        chains
    );
}

/// Every [`Queue::push_raw`](crate::queue::Queue::push_raw) recorded so far,
/// in push order. Mirrors Laravel's `Queue::rawPushes()`.
///
/// Raw pushes are kept apart from typed pushes: [`pushed`] and
/// [`assert_pushed`] never see them, and these never see a typed push.
pub fn raw_pushes() -> Vec<RawPush> {
    let g = lock_fake();
    let store = g.as_ref().expect("Queue::fake() must be active");
    store.raw.clone()
}

/// The recorded raw pushes `pred` accepts, in push order. Mirrors Laravel's
/// `Queue::pushedRaw($callback)`; pass `|_| true` for all of them.
///
/// [`RawPush::envelope`] decodes the payload when the predicate needs the
/// job name or the job's data.
pub fn pushed_raw(pred: impl Fn(&RawPush) -> bool) -> Vec<RawPush> {
    raw_pushes().into_iter().filter(|raw| pred(raw)).collect()
}
