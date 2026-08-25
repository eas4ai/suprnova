//! Failover queue parity (#60950): pushes fall through an ordered driver
//! list, `QueueFailedOver` fires edge-triggered, reads never fail over.
//!
//! The asymmetry is the whole point and every test here is written to pin
//! one half of it. Writes fall through; everything a reservation token
//! could reach - `pop`, `ack`, `nack`, `release`, `settle`, the counters
//! and the listings - stays on the primary, because a token issued by one
//! backend means nothing to another.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use suprnova::queue::driver::Settled;
use suprnova::queue::events::QueueFailedOver;
use suprnova::queue::inspect::InspectedJob;
use suprnova::queue::{
    BackoffSchedule, CURRENT_SCHEMA_VERSION, Envelope, FailoverQueueDriver, MemoryQueueDriver,
    Queue, QueueDriver, Reservation, ReservationToken,
};
use suprnova::{EventFacade, FrameworkError, Job, async_trait};
use uuid::Uuid;

/// A driver whose `push` fails while `broken` is true, or once its accept
/// budget runs out; every other method delegates to a real inner
/// [`MemoryQueueDriver`], so the decorator's read-side delegation is observed
/// against a driver that actually answers counters and listings rather than
/// the trait's `Err` defaults.
struct FlakyDriver {
    inner: MemoryQueueDriver,
    broken: AtomicBool,
    /// How many more pushes this driver accepts before it starts refusing.
    /// [`NEVER_TRIPS`] means "budget is irrelevant, only `broken` decides".
    ///
    /// A budget is what makes a batch fail *partway*, which is the only shape
    /// of failure that can tell per-envelope fall-through apart from wholesale
    /// forwarding: a driver that refuses from the first envelope onwards
    /// accepts nothing either way.
    accepts_remaining: AtomicUsize,
}

/// Sentinel accept budget meaning "never run out".
const NEVER_TRIPS: usize = usize::MAX;

impl FlakyDriver {
    fn new(broken: bool) -> Self {
        Self {
            inner: MemoryQueueDriver::new(),
            broken: AtomicBool::new(broken),
            accepts_remaining: AtomicUsize::new(NEVER_TRIPS),
        }
    }

    /// A healthy driver that accepts exactly `accepts` pushes and refuses
    /// every push after that.
    fn accepting_only(accepts: usize) -> Self {
        Self {
            inner: MemoryQueueDriver::new(),
            broken: AtomicBool::new(false),
            accepts_remaining: AtomicUsize::new(accepts),
        }
    }
}

#[async_trait]
impl QueueDriver for FlakyDriver {
    async fn push(&self, env: Envelope) -> Result<(), FrameworkError> {
        if self.broken.load(Ordering::SeqCst) {
            return Err(FrameworkError::internal("flaky: push refused"));
        }
        let budget = self.accepts_remaining.load(Ordering::SeqCst);
        if budget == 0 {
            return Err(FrameworkError::internal(
                "flaky: push refused (budget spent)",
            ));
        }
        if budget != NEVER_TRIPS {
            self.accepts_remaining.fetch_sub(1, Ordering::SeqCst);
        }
        self.inner.push(env).await
    }

    /// Overridden rather than inherited so this driver's batch semantics are
    /// stated here instead of borrowed from the trait default. The decorator
    /// must never reach it - `FailoverQueueDriver::bulk_push` loops `push` -
    /// and the mid-batch tests below are what prove that.
    async fn bulk_push(&self, envs: Vec<Envelope>) -> Result<(), FrameworkError> {
        for env in envs {
            self.push(env).await?;
        }
        Ok(())
    }

    /// A sentinel answer, not a delegation. The memory driver has no
    /// transactional settlement, so delegating would return the same
    /// `Unsupported` the trait default gives and the settle test would pass
    /// whether or not the decorator forwards at all.
    async fn settle(
        &self,
        _token: &ReservationToken,
        _follow_ups: &[Envelope],
    ) -> Result<Settled, FrameworkError> {
        Ok(Settled::Stale)
    }
    async fn pop(&self, vt: Duration) -> Result<Option<Reservation>, FrameworkError> {
        self.inner.pop(vt).await
    }
    async fn pop_from(
        &self,
        vt: Duration,
        queues: &[String],
    ) -> Result<Option<Reservation>, FrameworkError> {
        self.inner.pop_from(vt, queues).await
    }
    async fn ack(&self, t: &ReservationToken) -> Result<(), FrameworkError> {
        self.inner.ack(t).await
    }
    async fn nack(&self, t: &ReservationToken, d: Duration) -> Result<(), FrameworkError> {
        self.inner.nack(t, d).await
    }
    async fn release(
        &self,
        t: &ReservationToken,
        env: &Envelope,
        d: Duration,
    ) -> Result<(), FrameworkError> {
        self.inner.release(t, env, d).await
    }
    async fn size(&self) -> Result<u64, FrameworkError> {
        self.inner.size().await
    }
    async fn pending_size(&self) -> Result<u64, FrameworkError> {
        self.inner.pending_size().await
    }
    async fn delayed_size(&self) -> Result<u64, FrameworkError> {
        self.inner.delayed_size().await
    }
    async fn reserved_size(&self) -> Result<u64, FrameworkError> {
        self.inner.reserved_size().await
    }
    async fn pending_jobs(&self, queue: Option<&str>) -> Result<Vec<InspectedJob>, FrameworkError> {
        self.inner.pending_jobs(queue).await
    }
    async fn delayed_jobs(&self, queue: Option<&str>) -> Result<Vec<InspectedJob>, FrameworkError> {
        self.inner.delayed_jobs(queue).await
    }
    async fn reserved_jobs(
        &self,
        queue: Option<&str>,
    ) -> Result<Vec<InspectedJob>, FrameworkError> {
        self.inner.reserved_jobs(queue).await
    }
    async fn clear(&self) -> Result<u64, FrameworkError> {
        self.inner.clear().await
    }
    fn name(&self) -> &'static str {
        "flaky"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FailoverJob;

#[async_trait]
impl Job for FailoverJob {
    fn job_name() -> &'static str {
        "wave5-failover"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

fn build(
    primary_broken: bool,
) -> (
    Arc<FlakyDriver>,
    Arc<MemoryQueueDriver>,
    Arc<FailoverQueueDriver>,
) {
    let primary = Arc::new(FlakyDriver::new(primary_broken));
    let fallback = Arc::new(MemoryQueueDriver::new());
    let failover = Arc::new(
        FailoverQueueDriver::new(vec![
            (
                "primary".to_string(),
                primary.clone() as Arc<dyn QueueDriver>,
            ),
            (
                "fallback".to_string(),
                fallback.clone() as Arc<dyn QueueDriver>,
            ),
        ])
        .expect("two drivers"),
    );
    (primary, fallback, failover)
}

/// An envelope with an explicit `available_at`, for the bulk-delay test.
fn env_at(available_at: chrono::DateTime<Utc>) -> Envelope {
    Envelope {
        schema_version: CURRENT_SCHEMA_VERSION,
        id: Uuid::new_v4(),
        job_name: FailoverJob::job_name().into(),
        queue: None,
        payload: serde_json::json!({}),
        dispatched_at: Utc::now(),
        available_at,
        attempts: 0,
        max_tries: 3,
        backoff: BackoffSchedule::default(),
        timeout_secs: None,
        fail_on_timeout: false,
        idempotency_key: None,
        unique_lock_owner: None,
        batch_id: None,
        chain_remaining: Vec::new(),
    }
}

#[tokio::test]
#[serial]
async fn push_falls_through_to_the_next_driver() {
    let (_p, fallback, failover) = build(true);
    Queue::set_driver(failover);
    Queue::push(FailoverJob)
        .await
        .expect("push must succeed via fallback");
    assert!(
        fallback
            .pop(Duration::from_secs(1))
            .await
            .expect("pop")
            .is_some(),
        "the job must land on the fallback driver"
    );
}

#[tokio::test]
#[serial]
async fn a_healthy_primary_keeps_every_push_and_fires_nothing() {
    let (primary, fallback, failover) = build(false);
    Queue::set_driver(failover);
    let _events = EventFacade::fake();

    Queue::push(FailoverJob).await.expect("push");

    assert_eq!(
        primary.size().await.expect("primary size"),
        1,
        "a healthy primary must keep the push"
    );
    assert_eq!(
        fallback.size().await.expect("fallback size"),
        0,
        "the fallback must never see a push the primary accepted"
    );
    assert!(
        suprnova::events::dispatched::<QueueFailedOver>(|_| true).is_empty(),
        "no failover happened, so no QueueFailedOver may fire"
    );
}

#[tokio::test]
#[serial]
async fn failed_over_event_is_edge_triggered() {
    let (_p, _f, failover) = build(true);
    Queue::set_driver(failover);
    let _events = EventFacade::fake();

    Queue::push(FailoverJob).await.expect("push 1");
    Queue::push(FailoverJob).await.expect("push 2");

    // One QueueFailedOver for the primary's transition into failure, not one
    // per push - the second push finds "primary" already in the failing set.
    let events = suprnova::events::dispatched::<QueueFailedOver>(|e| {
        e.connection == "primary" && e.job_name == "wave5-failover"
    });
    assert_eq!(
        events.len(),
        1,
        "the event is edge-triggered: one per transition into failure, not one per push"
    );
    assert!(
        events[0].exception.contains("push refused"),
        "the event must carry the failing connection's own error, got {:?}",
        events[0].exception
    );
}

#[tokio::test]
#[serial]
async fn recovery_rearms_the_event() {
    let (primary, _f, failover) = build(true);
    Queue::set_driver(failover);
    let _events = EventFacade::fake();

    Queue::push(FailoverJob).await.expect("push while broken");
    primary.broken.store(false, Ordering::SeqCst);
    Queue::push(FailoverJob).await.expect("push after recovery"); // success resets state
    primary.broken.store(true, Ordering::SeqCst);
    Queue::push(FailoverJob).await.expect("push broken again");
    // A fourth push while still broken pins the *edge*, not just the count:
    // without it a driver that fired on every failed push would also land on
    // two and pass for the wrong reason.
    Queue::push(FailoverJob).await.expect("push still broken");

    let events = suprnova::events::dispatched::<QueueFailedOver>(|_| true);
    assert_eq!(
        events.len(),
        2,
        "one event per transition into failure: the first outage, then the second"
    );
}

#[tokio::test]
#[serial]
async fn all_drivers_failing_returns_the_last_error() {
    let a = Arc::new(FlakyDriver::new(true));
    let b = Arc::new(FlakyDriver::new(true));
    let failover = FailoverQueueDriver::new(vec![
        ("a".into(), a as Arc<dyn QueueDriver>),
        ("b".into(), b as Arc<dyn QueueDriver>),
    ])
    .expect("build");
    Queue::set_driver(Arc::new(failover));
    let err = Queue::push(FailoverJob)
        .await
        .expect_err("all backends down");
    assert!(
        err.to_string().contains("push refused"),
        "the caller must see a backend's own error, got {err}"
    );
}

#[tokio::test]
#[serial]
async fn every_failing_connection_reports_itself_once() {
    let a = Arc::new(FlakyDriver::new(true));
    let b = Arc::new(FlakyDriver::new(true));
    let failover = FailoverQueueDriver::new(vec![
        ("a".into(), a as Arc<dyn QueueDriver>),
        ("b".into(), b as Arc<dyn QueueDriver>),
    ])
    .expect("build");
    Queue::set_driver(Arc::new(failover));
    let _events = EventFacade::fake();

    let _ = Queue::push(FailoverJob).await;
    let _ = Queue::push(FailoverJob).await;

    let labels: Vec<String> = suprnova::events::dispatched::<QueueFailedOver>(|_| true)
        .into_iter()
        .map(|e| e.connection)
        .collect();
    assert_eq!(
        labels,
        vec!["a".to_string(), "b".to_string()],
        "both connections enter failure on the first push and stay quiet on the second"
    );
}

#[tokio::test]
#[serial]
async fn reads_never_fail_over() {
    // Jobs pushed while the primary is broken land on the fallback; every
    // read goes to the PRIMARY only, so it sees nothing. That asymmetry is
    // the contract - a reservation token is meaningful only to the driver
    // that issued it.
    let (_p, fallback, failover) = build(true);
    Queue::set_driver(failover.clone());
    Queue::push(FailoverJob).await.expect("push");

    assert_eq!(
        fallback.size().await.expect("fallback size"),
        1,
        "precondition: the job really is on the fallback"
    );
    assert!(
        failover
            .pop(Duration::from_secs(1))
            .await
            .expect("pop")
            .is_none(),
        "pop must delegate to the primary only, never the fallback"
    );
    assert!(
        failover
            .pop_from(Duration::from_secs(1), &["default".to_string()])
            .await
            .expect("pop_from")
            .is_none(),
        "pop_from must delegate to the primary only"
    );
    assert_eq!(
        failover.size().await.expect("size"),
        0,
        "size = primary only"
    );
    assert_eq!(
        failover.pending_size().await.expect("pending_size"),
        0,
        "pending_size = primary only"
    );
    assert_eq!(
        failover.delayed_size().await.expect("delayed_size"),
        0,
        "delayed_size = primary only"
    );
    assert_eq!(
        failover.reserved_size().await.expect("reserved_size"),
        0,
        "reserved_size = primary only"
    );
    assert!(
        failover
            .pending_jobs(None)
            .await
            .expect("pending_jobs")
            .is_empty(),
        "pending_jobs = primary only"
    );
    assert!(
        failover
            .delayed_jobs(None)
            .await
            .expect("delayed_jobs")
            .is_empty(),
        "delayed_jobs = primary only"
    );
    assert!(
        failover
            .reserved_jobs(None)
            .await
            .expect("reserved_jobs")
            .is_empty(),
        "reserved_jobs = primary only"
    );
    assert_eq!(
        failover.clear().await.expect("clear"),
        0,
        "clear = primary only; the fallback's backlog survives"
    );
    assert_eq!(
        fallback.size().await.expect("fallback size after clear"),
        1,
        "clear must not reach into a connection whose lifecycle we do not own"
    );
}

#[tokio::test]
#[serial]
async fn lifecycle_calls_reach_the_primary_that_issued_the_token() {
    // The primary is healthy here, so the reservation exists on it; ack via
    // the decorator must settle that reservation and nothing else.
    let (primary, _f, failover) = build(false);
    Queue::set_driver(failover.clone());
    Queue::push(FailoverJob).await.expect("push");

    let res = failover
        .pop(Duration::from_secs(30))
        .await
        .expect("pop")
        .expect("a reservation");
    assert_eq!(
        primary.reserved_size().await.expect("reserved"),
        1,
        "the primary holds the reservation"
    );
    failover.ack(&res.token).await.expect("ack");
    assert_eq!(
        primary.size().await.expect("size after ack"),
        0,
        "ack must settle the primary's reservation"
    );
}

#[tokio::test]
#[serial]
async fn settle_reports_the_primary_answer() {
    let (_p, _f, failover) = build(false);
    // `FlakyDriver::settle` answers `Stale`, which neither the trait default
    // nor the fallback memory driver ever produces. Asserting on it is what
    // makes this test fail if the decorator stops forwarding and lets the
    // trait's own `Unsupported` default answer instead.
    let settled = failover
        .settle(&ReservationToken(Uuid::new_v4()), &[])
        .await
        .expect("settle");
    assert_eq!(
        settled,
        Settled::Stale,
        "settle must report the primary's answer, not the decorator's default"
    );
}

#[tokio::test]
#[serial]
async fn bulk_push_preserves_each_envelopes_own_delay() {
    // Laravel #60950: `FailoverQueue::bulk` looped the batch wholesale and
    // lost each job's delay. Suprnova resolves the delay onto the envelope
    // before the driver sees it, so the per-envelope loop is what keeps it.
    //
    // The primary accepts one envelope and then refuses, so the batch fails
    // partway. A wholesale-forwarding decorator would re-push the accepted
    // envelope onto the fallback too and land `fallback.pending_size() == 2`.
    let primary = Arc::new(FlakyDriver::accepting_only(1));
    let fallback = Arc::new(MemoryQueueDriver::new());
    let failover = FailoverQueueDriver::new(vec![
        (
            "primary".to_string(),
            primary.clone() as Arc<dyn QueueDriver>,
        ),
        (
            "fallback".to_string(),
            fallback.clone() as Arc<dyn QueueDriver>,
        ),
    ])
    .expect("build");

    let now = Utc::now();
    let envs = vec![
        env_at(now),
        env_at(now + chrono::Duration::seconds(600)),
        env_at(now),
    ];
    failover.bulk_push(envs).await.expect("bulk push");

    assert_eq!(
        primary.pending_size().await.expect("primary pending"),
        1,
        "the primary accepted exactly one envelope before its budget ran out"
    );
    assert_eq!(
        fallback.pending_size().await.expect("fallback pending"),
        1,
        "only the refused immediate envelope falls through; re-pushing the \
         accepted one would make this 2"
    );
    assert_eq!(
        fallback.delayed_size().await.expect("fallback delayed"),
        1,
        "the delayed envelope must keep its own available_at, not the batch's"
    );
    assert_eq!(
        primary.size().await.expect("primary") + fallback.size().await.expect("fallback"),
        3,
        "three envelopes in, three envelopes out - no duplicates"
    );
}

#[tokio::test]
#[serial]
async fn bulk_push_does_not_re_push_what_the_primary_accepted() {
    // The load-bearing case: one batch, and the primary dies *inside* it.
    // Per-envelope fall-through leaves two envelopes, one on each connection.
    // A decorator that forwarded the whole batch to the fallback after the
    // primary's `bulk_push` returned `Err` would leave three, because the
    // envelope the primary already wrote would be pushed again.
    let primary = Arc::new(FlakyDriver::accepting_only(1));
    let fallback = Arc::new(MemoryQueueDriver::new());
    let failover = FailoverQueueDriver::new(vec![
        (
            "primary".to_string(),
            primary.clone() as Arc<dyn QueueDriver>,
        ),
        (
            "fallback".to_string(),
            fallback.clone() as Arc<dyn QueueDriver>,
        ),
    ])
    .expect("build");

    let now = Utc::now();
    failover
        .bulk_push(vec![env_at(now), env_at(now)])
        .await
        .expect("the batch fails partway and finishes on the fallback");

    assert_eq!(
        primary.size().await.expect("primary size"),
        1,
        "the primary keeps the one envelope it accepted"
    );
    assert_eq!(
        fallback.size().await.expect("fallback size"),
        1,
        "only the refused envelope reaches the fallback; wholesale forwarding \
         would put both there"
    );
    assert_eq!(
        primary.size().await.expect("primary") + fallback.size().await.expect("fallback"),
        2,
        "two envelopes in, two envelopes out - wholesale forwarding yields 3"
    );
}

#[tokio::test]
#[serial]
async fn the_decorator_names_itself_failover() {
    let (_p, _f, failover) = build(false);
    assert_eq!(failover.name(), "failover");
}

#[test]
fn empty_driver_list_is_rejected() {
    assert!(
        FailoverQueueDriver::new(vec![]).is_err(),
        "a failover connection with nothing to fail over to is a misconfiguration"
    );
}
