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
//! `Queue::push` family, [`PendingBatch::dispatch`], [`PendingChain::dispatch`],
//! the failed-job retries, and the worker's push of a chain's next link. A
//! batch is recorded as a [`FakedBatch`] and a chain as a [`FakedChain`], and
//! their jobs are recorded as pushes too, so a test that only cares that a
//! job was queued does not need to know which path queued it.
//!
//! [`QueueFakeGuard::except`] narrows the fake: the job types it names take
//! the real path to the real queue, and only the rest are recorded.
//! [`Queue::push_raw`] payloads are kept apart from typed pushes and read back
//! with [`raw_pushes`] / [`pushed_raw`].
//!
//! A worker can run on the fake: [`QueueFakeGuard::driver`] returns a
//! [`QueueDriver`] that reserves the recorded pushes, and every reservation
//! it hands out is recorded for [`reserved`] and
//! [`Queue::reserved_jobs`](crate::queue::Queue::reserved_jobs).
//!
//! [`PendingBatch::dispatch`]: crate::queue::PendingBatch::dispatch
//! [`PendingChain::dispatch`]: crate::queue::PendingChain::dispatch
//! [`Queue::push_raw`]: crate::queue::Queue::push_raw

use crate::error::FrameworkError;
use crate::queue::driver::{QueueDriver, QueueFilterCapability, Reservation, ReservationToken};
use crate::queue::envelope::{queue_filter, queue_matches};
use crate::queue::{Envelope, EnvelopeError, EnvelopeOverrides, InspectedJob, Job};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
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
    /// The identity [`install_fake`] gave this fake. A driver carries the
    /// identity of the guard that made it, and answers only while the
    /// installed fake has that same identity, so a driver from a dropped
    /// guard never reaches a later fake's jobs.
    identity: u64,
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
    /// The fake's own queue, the one [`QueueFakeGuard::driver`] reserves from:
    /// one entry for each push recorded in `pushed`, in push order.
    live: Vec<LiveJob>,
    /// The envelope of every reservation the fake's driver handed out, in
    /// the order it handed them out, as each envelope was when reserved.
    /// Kept after the job settles, as Laravel's `QueueFake` keeps its
    /// reserved jobs.
    reservations: Vec<Envelope>,
}

/// One job on the fake's own queue.
///
/// Kept apart from the push records: a worker that retries a job changes its
/// attempts and its `available_at`, and the record of what was pushed must
/// not change with it.
struct LiveJob {
    envelope: Envelope,
    /// The reservation holding this job, `None` while it waits for a worker.
    token: Option<ReservationToken>,
}

/// Process-wide serializer: only one test may hold the fake at a time.
static FAKE_SERIAL: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static FAKE: Mutex<Option<FakeStore>> = Mutex::new(None);
/// The identity the next installed fake receives. Bumped once per
/// [`install_fake`], so no two fakes in the process share one.
static NEXT_FAKE_IDENTITY: AtomicU64 = AtomicU64::new(1);

fn lock_fake() -> std::sync::MutexGuard<'static, Option<FakeStore>> {
    FAKE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The error every read of the fake returns once no fake is installed.
fn inactive() -> FrameworkError {
    FrameworkError::internal("Queue::fake() must be active")
}

/// Run `f` on the installed fake's store, or report that none is installed.
fn with_store<T>(f: impl FnOnce(&mut FakeStore) -> T) -> Result<T, FrameworkError> {
    lock_fake().as_mut().map(f).ok_or_else(inactive)
}

/// Run `f` on the installed fake's store only when that fake is the one
/// `identity` names. A driver calls this, never [`with_store`]: a driver
/// whose guard was dropped, or whose fake was replaced by a later
/// [`install_fake`], gets the inactive error instead of the later fake.
fn with_store_of<T>(
    identity: u64,
    f: impl FnOnce(&mut FakeStore) -> T,
) -> Result<T, FrameworkError> {
    lock_fake()
        .as_mut()
        .filter(|store| store.identity == identity)
        .map(f)
        .ok_or_else(inactive)
}

/// Decode one recorded payload as `J`. `record` names what recorded it,
/// `pushed` or `reserved`, so the error says which record failed.
fn decode_recorded<J: Job>(payload: &serde_json::Value, record: &str) -> Result<J, FrameworkError> {
    serde_json::from_value::<J>(payload.clone()).map_err(|e| {
        FrameworkError::internal(format!(
            "a {record} {} does not decode as its job type: {e}",
            J::job_name()
        ))
    })
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
    let id = Uuid::new_v4();
    let queue = overrides
        .queue
        .clone()
        .or_else(|| J::queue().map(str::to_owned));
    let envelope = fake_envelope::<J>(job, id, queue.clone(), available_at, &overrides)?;
    let payload = envelope.payload.clone();
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
        store.live.push(LiveJob {
            envelope,
            token: None,
        });
    }
    Ok(id)
}

/// The envelope a worker reserving from the fake receives for a typed push:
/// the envelope the real path builds, with `J`'s declarations, the push's
/// overrides and the caller's context, under the id and the queue the fake
/// recorded. The queue is the fake's because routing does not run under the
/// fake, so the worker sees the job where a test sees it.
fn fake_envelope<J: Job>(
    job: &J,
    id: Uuid,
    queue: Option<String>,
    available_at: DateTime<Utc>,
    overrides: &EnvelopeOverrides,
) -> Result<Envelope, FrameworkError> {
    let mut envelope =
        super::build_envelope_on::<J>(job, available_at, "", crate::context::Context::dehydrate())?;
    super::apply_overrides(&mut envelope, overrides, "");
    envelope.id = id;
    envelope.queue = queue;
    Ok(envelope)
}

/// Record a push that reached the facade as a built envelope: a batch
/// member, the head of a chain, the next link a worker dispatches after it,
/// a retried failed job.
///
/// It lands in the same store a typed push does, so [`pushed`] and
/// [`assert_pushed`] see it. Laravel's `QueueFake` does the same: a batch
/// reaches it through `bulk`, and each job is then an ordinary recorded
/// push.
pub(crate) fn record_envelope(env: &Envelope) {
    if let Some(store) = lock_fake().as_mut() {
        record_envelope_in(store, env);
    }
}

/// [`record_envelope`] on a store the caller already holds.
fn record_envelope_in(store: &mut FakeStore, env: &Envelope) {
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
    store.live.push(LiveJob {
        envelope: env.clone(),
        token: None,
    });
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
    let identity = NEXT_FAKE_IDENTITY.fetch_add(1, Ordering::Relaxed);
    *lock_fake() = Some(FakeStore {
        identity,
        ..FakeStore::default()
    });
    QueueFakeGuard {
        _serial: serial,
        identity,
    }
}

/// Remove every connection registered with
/// [`Queue::register_connection`](crate::queue::Queue::register_connection),
/// and every setting made with
/// [`Queue::set_connection_after_commit`](crate::queue::Queue::set_connection_after_commit).
///
/// The connections are process-wide. A test that registers one calls this
/// when it is done, so a connection it registered, or a setting it gave a
/// connection, does not change what a connection name means for the next
/// test.
pub fn forget_connections() {
    crate::queue::connections::clear();
}

/// RAII guard returned by [`install_fake`]. Holds the process-wide
/// serialization lock and clears the fake store on drop.
pub struct QueueFakeGuard {
    _serial: MutexGuard<'static, ()>,
    /// The identity of the fake this guard installed; see [`FakeStore`].
    identity: u64,
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
    /// queue. A chain whose first job is excepted reaches the real queue,
    /// and the worker that runs it dispatches each later link through the
    /// fake, so a link is recorded unless `except` names it, as in Laravel.
    /// A [`Queue::push_raw`] is always recorded: a raw payload is not a job
    /// type, as in Laravel.
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

    /// A [`QueueDriver`] over this fake, for a worker to reserve from. Pass it
    /// to [`run_worker`](crate::queue::worker::run_worker) or
    /// [`run_worker_with_controls`](crate::queue::worker::run_worker_with_controls)
    /// to run the jobs the fake recorded, and read the reservations back with
    /// [`reserved`] or [`Queue::reserved_jobs`](crate::queue::Queue::reserved_jobs).
    /// It stands in for Laravel's `QueueFake::reserve`, with a real worker
    /// making the reservations.
    ///
    /// The driver reserves the recorded pushes in push order, each once it is
    /// due, and filters by queue as the memory driver does. Each envelope is
    /// the one the real path builds, the pusher's context included. A
    /// reservation stays until the worker settles it: there is no visibility
    /// timeout. `ack` takes the job off the fake's queue, `nack` and `release`
    /// put it back, and `nack` adds an attempt. A push to the driver is
    /// recorded as a push. A raw push is kept apart and is never reserved.
    /// The push records never change, so [`pushed`] still lists a job after a
    /// worker ran it.
    ///
    /// The fake records every reservation, settled or not, and keeps the
    /// records until the guard drops. Every method of the driver returns an
    /// error once the guard is dropped, and once a later
    /// [`Queue::fake`](crate::queue::Queue::fake) replaces this fake: the
    /// driver works only on the fake that made it, so a worker that outlives
    /// its guard cannot reserve another test's jobs.
    pub fn driver(&self) -> Arc<dyn QueueDriver> {
        Arc::new(FakeQueueDriver {
            identity: self.identity,
        })
    }
}

/// The driver [`QueueFakeGuard::driver`] returns. It holds only the identity
/// of the guard that made it; every method works on the installed fake's
/// store while that fake has the same identity, and fails otherwise.
struct FakeQueueDriver {
    identity: u64,
}

impl FakeQueueDriver {
    /// Run `f` on the store of the fake this driver was made for.
    fn with_store<T>(&self, f: impl FnOnce(&mut FakeStore) -> T) -> Result<T, FrameworkError> {
        with_store_of(self.identity, f)
    }

    /// Put the job reserved under `token` back on the fake's queue, due after
    /// `delay`, adding an attempt when `consume_attempt` is set. An unknown
    /// token is ignored, as the trait asks of `nack` and `release`.
    fn requeue(
        &self,
        token: &ReservationToken,
        delay: Duration,
        consume_attempt: bool,
    ) -> Result<(), FrameworkError> {
        let available_at = crate::queue::driver::available_after(delay)?;
        self.with_store(|store| {
            if let Some(job) = store
                .live
                .iter_mut()
                .find(|job| job.token.as_ref() == Some(token))
            {
                if consume_attempt {
                    job.envelope.attempts += 1;
                }
                job.envelope.available_at = available_at;
                job.token = None;
            }
        })
    }
}

#[async_trait]
impl QueueDriver for FakeQueueDriver {
    async fn push(&self, env: Envelope) -> Result<(), FrameworkError> {
        self.with_store(|store| record_envelope_in(store, &env))
    }

    async fn pop(
        &self,
        visibility_timeout: Duration,
    ) -> Result<Option<Reservation>, FrameworkError> {
        self.pop_from(visibility_timeout, &[]).await
    }

    fn queue_filter_capability(&self) -> QueueFilterCapability {
        QueueFilterCapability::Supported
    }

    async fn pop_from(
        &self,
        _visibility_timeout: Duration,
        queues: &[String],
    ) -> Result<Option<Reservation>, FrameworkError> {
        let now = crate::clock::now();
        self.with_store(|store| {
            let job = store.live.iter_mut().find(|job| {
                job.token.is_none()
                    && job.envelope.available_at <= now
                    && queue_matches(job.envelope.queue.as_deref(), queues)
            })?;
            let token = ReservationToken(Uuid::new_v4());
            job.token = Some(token.clone());
            let envelope = job.envelope.clone();
            store.reservations.push(envelope.clone());
            Some(Reservation { envelope, token })
        })
    }

    async fn ack(&self, token: &ReservationToken) -> Result<(), FrameworkError> {
        self.with_store(|store| {
            store.live.retain(|job| job.token.as_ref() != Some(token));
        })
    }

    async fn nack(
        &self,
        token: &ReservationToken,
        requeue_delay: Duration,
    ) -> Result<(), FrameworkError> {
        self.requeue(token, requeue_delay, true)
    }

    async fn release(
        &self,
        token: &ReservationToken,
        _env: &Envelope,
        delay: Duration,
    ) -> Result<(), FrameworkError> {
        // The fake's copy still holds the attempts it was reserved with: the
        // worker bumps only its own envelope.
        self.requeue(token, delay, false)
    }

    fn name(&self) -> &'static str {
        "fake"
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

/// All captured pushes of `J` deserialized back into the typed payload, in
/// push order.
///
/// # Panics
///
/// Panics when the fake is not installed, and when a payload recorded under
/// `J`'s [`Job::job_name`] does not decode as `J`, with a message that names
/// the job and the decode error. Leaving such a payload out would let a test
/// pass over the very push it meant to check, and
/// [`assert_not_pushed`] pass while it is there. Laravel's
/// `QueueFake::serializeAndRestore` surfaces a job that does not restore the
/// same way. [`try_pushed`] returns the error instead.
pub fn pushed<J: Job>() -> Vec<J> {
    try_pushed::<J>().unwrap_or_else(|e| panic!("{e}"))
}

/// [`pushed`], returning an error instead of panicking.
///
/// # Errors
///
/// Returns [`FrameworkError`] when the fake is not installed, and, naming the
/// job and the decode error, when a payload recorded under `J`'s name does not
/// decode as `J`.
pub fn try_pushed<J: Job>() -> Result<Vec<J>, FrameworkError> {
    let payloads = with_store(|store| {
        store
            .pushed
            .get(J::job_name())
            .map(|b| b.iter().map(|p| p.payload.clone()).collect::<Vec<_>>())
            .unwrap_or_default()
    })?;
    payloads
        .iter()
        .map(|payload| decode_recorded::<J>(payload, "pushed"))
        .collect()
}

/// Every job of type `J` a worker reserved from the fake's driver, in the
/// order it reserved them, one entry for each reservation: a job retried
/// once is listed twice. Mirrors the reserved jobs Laravel's `QueueFake`
/// records; [`Queue::reserved_jobs`](crate::queue::Queue::reserved_jobs)
/// lists the same records across every job type.
///
/// # Panics
///
/// Panics when the fake is not installed, and, naming the job and the decode
/// error, when a reserved payload does not decode as `J`, as [`pushed`] does.
/// [`try_reserved`] returns the error instead.
pub fn reserved<J: Job>() -> Vec<J> {
    try_reserved::<J>().unwrap_or_else(|e| panic!("{e}"))
}

/// [`reserved`], returning an error instead of panicking.
///
/// # Errors
///
/// As [`try_pushed`], for the reserved records.
pub fn try_reserved<J: Job>() -> Result<Vec<J>, FrameworkError> {
    let payloads = with_store(|store| {
        store
            .reservations
            .iter()
            .filter(|env| env.job_name == J::job_name())
            .map(|env| env.payload.clone())
            .collect::<Vec<_>>()
    })?;
    payloads
        .iter()
        .map(|payload| decode_recorded::<J>(payload, "reserved"))
        .collect()
}

/// The reservations the fake recorded, on `queue` or on every queue for
/// `None`, as [`InspectedJob`]s. What
/// [`Queue::reserved_jobs`](crate::queue::Queue::reserved_jobs) answers
/// while the fake is installed. Each carries the id, the attempts and the
/// dispatch time of the envelope the worker reserved.
pub(crate) fn reservations(queue: Option<&str>) -> Result<Vec<InspectedJob>, FrameworkError> {
    let filter = queue_filter(queue);
    with_store(|store| {
        store
            .reservations
            .iter()
            .filter(|env| queue_matches(env.queue.as_deref(), &filter))
            .map(InspectedJob::from_envelope)
            .collect()
    })
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

/// Assert a push on `queue` satisfies `pred`, so the queue and payload belong to one job.
/// The queue is the per-push override or the job's declared queue. Panics when none matches.
pub fn assert_pushed_on_queue<J: Job>(queue: &str, pred: impl Fn(&J) -> bool) {
    let pushes = {
        let g = lock_fake();
        let store = g.as_ref().expect("Queue::fake() must be active");
        store.pushed.get(J::job_name()).cloned().unwrap_or_default()
    };
    let matching = pushes
        .iter()
        .filter(|push| push.queue.as_deref() == Some(queue))
        .filter_map(|push| serde_json::from_value::<J>(push.payload.clone()).ok())
        .any(|job| pred(&job));
    assert!(
        matching,
        "expected a pushed {} on {:?} matching the filter (EnvelopeOverrides.queue or Job::queue)",
        J::job_name(),
        queue
    );
}

/// Assert no captured push satisfies `pred`, so another job of the type can still be present.
/// Panics when the fake is inactive or a matching job was pushed.
pub fn assert_not_pushed<J: Job>(pred: impl Fn(&J) -> bool) {
    assert!(
        !pushed::<J>().iter().any(pred),
        "expected no pushed {} matching the filter",
        J::job_name()
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
/// [`QueueDriver::pending_jobs`] -
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
/// [`QueueDriver::delayed_jobs`].
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

/// Assert an unchained push satisfies `pred`, so you can match its job data. Mirrors
/// Laravel's `Queue::assertPushedWithoutChain`.
///
/// A job pushed on its own has no chain, and so do a batch member, a retried
/// job without a chain and a chain of one job. The head of a chain of two or
/// more jobs carries the rest of the chain, so a push of `J` that is only
/// ever such a head fails this. Panics when `J` was not pushed at all, and
/// when no unchained typed push satisfies the filter. Raw payloads do not match.
///
/// Use [`assert_chained`] to assert on the chain itself.
pub fn assert_pushed_without_chain<J: Job>(pred: impl Fn(&J) -> bool) {
    let pushes = {
        let g = lock_fake();
        let store = g.as_ref().expect("Queue::fake() must be active");
        store.pushed.get(J::job_name()).cloned().unwrap_or_default()
    };
    assert!(
        !pushes.is_empty(),
        "expected at least one pushed {}",
        J::job_name()
    );
    let matches = pushes
        .iter()
        .filter(|push| push.chained.is_empty())
        .filter_map(|push| serde_json::from_value::<J>(push.payload.clone()).ok())
        .any(|job| pred(&job));
    assert!(
        matches,
        "expected a pushed {} without a chain matching the filter",
        J::job_name()
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
