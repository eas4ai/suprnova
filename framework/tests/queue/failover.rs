//! Failover queue parity (#60950): pushes fall through an ordered driver
//! list, `QueueFailedOver` fires edge-triggered, and reads drain every driver.
//!
//! Writes fall through in configured order. Read-side state is aggregated,
//! pops rotate fairly, and aggregate-issued reservation aliases route lifecycle
//! calls back to the exact driver and inner token that issued the reservation.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use suprnova::queue::driver::{QueueFilterCapability, Settled};
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
/// [`MemoryQueueDriver`], so the decorator's aggregate read and lifecycle
/// behavior is observed against a driver that actually answers counters and
/// listings rather than the trait's `Err` defaults.
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
    fn queue_filter_capability(&self) -> QueueFilterCapability {
        QueueFilterCapability::Supported
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

fn colliding_inner_token() -> ReservationToken {
    ReservationToken(Uuid::from_u128(0xfeed_face_cafe_beef))
}

struct DefaultDeadlineDriver;

#[async_trait]
impl QueueDriver for DefaultDeadlineDriver {
    async fn push(&self, _env: Envelope) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("default-deadline test driver"))
    }

    async fn pop(
        &self,
        _visibility_timeout: Duration,
    ) -> Result<Option<Reservation>, FrameworkError> {
        Ok(None)
    }

    async fn ack(&self, _token: &ReservationToken) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn nack(
        &self,
        _token: &ReservationToken,
        _requeue_delay: Duration,
    ) -> Result<(), FrameworkError> {
        Ok(())
    }

    fn name(&self) -> &'static str {
        "default-deadline"
    }
}

#[derive(Clone, Copy)]
enum ClearOutcome {
    Success(u64),
    Failure(&'static str),
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum LifecycleOperation {
    Ack,
    Nack,
    Release,
    Settle,
}

#[derive(Clone, Copy)]
enum DeadlineOutcome {
    Fallback,
    Immediate,
    After(Duration),
}

/// One deterministic backend script covers collision, lease, barrier,
/// aggregate-error, clear, and lifecycle-routing contracts without a family
/// of nearly identical one-off drivers.
struct ScriptedDriver {
    next: tokio::sync::Mutex<VecDeque<Envelope>>,
    pop_error: Option<&'static str>,
    clear_outcome: ClearOutcome,
    deadlines: Mutex<VecDeque<DeadlineOutcome>>,
    block_pop: bool,
    block_clear: bool,
    block_ack: bool,
    pop_entered: tokio::sync::Notify,
    release_pop: tokio::sync::Notify,
    clear_entered: tokio::sync::Notify,
    release_clear: tokio::sync::Notify,
    ack_entered: tokio::sync::Notify,
    release_ack: tokio::sync::Notify,
    active_envelope: tokio::sync::Mutex<Option<Uuid>>,
    acknowledged_envelopes: Mutex<Vec<Uuid>>,
    clear_calls: AtomicUsize,
    acknowledgements: AtomicUsize,
    lifecycle_failures: Mutex<HashSet<LifecycleOperation>>,
    lifecycle_attempts: Mutex<Vec<LifecycleOperation>>,
}

impl ScriptedDriver {
    fn new(envelopes: impl IntoIterator<Item = Envelope>) -> Self {
        Self::operation(envelopes, None, ClearOutcome::Success(0))
    }

    fn operation(
        envelopes: impl IntoIterator<Item = Envelope>,
        pop_error: Option<&'static str>,
        clear_outcome: ClearOutcome,
    ) -> Self {
        Self {
            next: tokio::sync::Mutex::new(envelopes.into_iter().collect()),
            pop_error,
            clear_outcome,
            deadlines: Mutex::new(VecDeque::new()),
            block_pop: false,
            block_clear: false,
            block_ack: false,
            pop_entered: tokio::sync::Notify::new(),
            release_pop: tokio::sync::Notify::new(),
            clear_entered: tokio::sync::Notify::new(),
            release_clear: tokio::sync::Notify::new(),
            ack_entered: tokio::sync::Notify::new(),
            release_ack: tokio::sync::Notify::new(),
            active_envelope: tokio::sync::Mutex::new(None),
            acknowledged_envelopes: Mutex::new(Vec::new()),
            clear_calls: AtomicUsize::new(0),
            acknowledgements: AtomicUsize::new(0),
            lifecycle_failures: Mutex::new(HashSet::new()),
            lifecycle_attempts: Mutex::new(Vec::new()),
        }
    }

    fn with_deadlines(mut self, deadlines: impl IntoIterator<Item = DeadlineOutcome>) -> Self {
        self.deadlines = Mutex::new(deadlines.into_iter().collect());
        self
    }

    fn blocking_pop(mut self) -> Self {
        self.block_pop = true;
        self
    }

    fn blocking_clear_and_ack(mut self) -> Self {
        self.block_clear = true;
        self.block_ack = true;
        self
    }

    fn failing_once(mut self, operations: impl IntoIterator<Item = LifecycleOperation>) -> Self {
        self.lifecycle_failures = Mutex::new(operations.into_iter().collect());
        self
    }

    fn record_lifecycle(&self, operation: LifecycleOperation) -> Result<(), FrameworkError> {
        self.lifecycle_attempts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(operation);
        if self
            .lifecycle_failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&operation)
        {
            Err(FrameworkError::internal("scripted lifecycle failure"))
        } else {
            Ok(())
        }
    }

    fn lifecycle_attempts(&self, operation: LifecycleOperation) -> usize {
        self.lifecycle_attempts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|attempt| **attempt == operation)
            .count()
    }

    fn acknowledgements(&self) -> usize {
        self.acknowledgements.load(Ordering::SeqCst)
    }

    async fn enqueue(&self, envelope: Envelope) {
        self.next.lock().await.push_back(envelope);
    }

    fn acknowledged_envelopes(&self) -> Vec<Uuid> {
        self.acknowledged_envelopes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl QueueDriver for ScriptedDriver {
    async fn push(&self, _env: Envelope) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("scripted driver refuses pushes"))
    }

    async fn pop(
        &self,
        _visibility_timeout: Duration,
    ) -> Result<Option<Reservation>, FrameworkError> {
        if let Some(error) = self.pop_error {
            return Err(FrameworkError::internal(error));
        }
        if self.block_pop {
            self.pop_entered.notify_one();
            self.release_pop.notified().await;
        }
        let envelope = self.next.lock().await.pop_front();
        if let Some(envelope) = &envelope {
            self.active_envelope.lock().await.replace(envelope.id);
        }
        Ok(envelope.map(|envelope| Reservation {
            token: colliding_inner_token(),
            envelope,
        }))
    }

    fn queue_filter_capability(&self) -> QueueFilterCapability {
        QueueFilterCapability::Unsupported
    }

    fn reservation_deadline(
        &self,
        _token: &ReservationToken,
        fallback_deadline: Instant,
    ) -> Instant {
        match self
            .deadlines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
            .unwrap_or(DeadlineOutcome::Fallback)
        {
            DeadlineOutcome::Fallback => fallback_deadline,
            DeadlineOutcome::Immediate => Instant::now(),
            DeadlineOutcome::After(duration) => Instant::now() + duration,
        }
    }

    async fn ack(&self, _token: &ReservationToken) -> Result<(), FrameworkError> {
        self.record_lifecycle(LifecycleOperation::Ack)?;
        if self.block_ack {
            self.ack_entered.notify_one();
            self.release_ack.notified().await;
        }
        if let Some(envelope_id) = self.active_envelope.lock().await.take() {
            self.acknowledged_envelopes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(envelope_id);
        }
        self.acknowledgements.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn nack(
        &self,
        _token: &ReservationToken,
        _requeue_delay: Duration,
    ) -> Result<(), FrameworkError> {
        self.record_lifecycle(LifecycleOperation::Nack)
    }

    async fn release(
        &self,
        _token: &ReservationToken,
        _env: &Envelope,
        _delay: Duration,
    ) -> Result<(), FrameworkError> {
        self.record_lifecycle(LifecycleOperation::Release)
    }

    async fn settle(
        &self,
        _token: &ReservationToken,
        _follow_ups: &[Envelope],
    ) -> Result<Settled, FrameworkError> {
        self.record_lifecycle(LifecycleOperation::Settle)?;
        Ok(Settled::Atomically)
    }

    async fn clear(&self) -> Result<u64, FrameworkError> {
        self.clear_entered.notify_one();
        if self.block_clear {
            self.release_clear.notified().await;
        }
        self.clear_calls.fetch_add(1, Ordering::SeqCst);
        match self.clear_outcome {
            ClearOutcome::Success(count) => {
                self.active_envelope.lock().await.take();
                Ok(count)
            }
            ClearOutcome::Failure(error) => Err(FrameworkError::internal(error)),
        }
    }

    fn name(&self) -> &'static str {
        "scripted"
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
        debounce_id: None,
        debounce_owner: None,
        batch_id: None,
        chain_remaining: Vec::new(),
    }
}

fn tagged_env(tag: &str) -> Envelope {
    let mut envelope = env_at(Utc::now());
    envelope.payload = serde_json::json!({ "tag": tag });
    envelope
}

#[test]
fn default_reservation_deadline_uses_the_pre_pop_fallback() {
    let driver = DefaultDeadlineDriver;
    let fallback_deadline = Instant::now() + Duration::from_secs(30);

    assert_eq!(
        driver.reservation_deadline(&colliding_inner_token(), fallback_deadline),
        fallback_deadline,
        "backward-compatible drivers must inherit the conservative pre-pop deadline"
    );
}

#[tokio::test]
#[serial]
async fn delayed_pop_uses_the_issuers_longer_authoritative_deadline() {
    let issuer = Arc::new(
        ScriptedDriver::new([tagged_env("delayed-deadline")])
            .blocking_pop()
            .with_deadlines([DeadlineOutcome::After(Duration::from_secs(30))]),
    );
    let failover = Arc::new(
        FailoverQueueDriver::new(vec![(
            "issuer".into(),
            issuer.clone() as Arc<dyn QueueDriver>,
        )])
        .expect("one driver"),
    );
    let aggregate = failover.clone();
    let pop = tokio::spawn(async move {
        aggregate
            .pop(Duration::ZERO)
            .await
            .expect("aggregate pop")
            .expect("reservation")
    });

    issuer.pop_entered.notified().await;
    issuer.release_pop.notify_one();
    let reservation = pop.await.expect("pop task");
    failover
        .ack(&reservation.token)
        .await
        .expect("aggregate ack");
    assert_eq!(
        issuer.acknowledgements.load(Ordering::SeqCst),
        1,
        "the aggregate must retain the alias for the issuer's longer lease"
    );
}

#[tokio::test]
#[serial]
async fn reused_inner_token_cannot_be_reached_through_an_expired_outer_alias() {
    let issuer = Arc::new(
        ScriptedDriver::new([tagged_env("first"), tagged_env("second")]).with_deadlines([
            DeadlineOutcome::Immediate,
            DeadlineOutcome::After(Duration::from_secs(30)),
        ]),
    );
    let failover = FailoverQueueDriver::new(vec![(
        "issuer".into(),
        issuer.clone() as Arc<dyn QueueDriver>,
    )])
    .expect("one driver");

    let first = failover
        .pop(Duration::from_secs(30))
        .await
        .expect("first aggregate pop")
        .expect("first reservation");
    let second = failover
        .pop(Duration::from_secs(30))
        .await
        .expect("second aggregate pop")
        .expect("second reservation");
    assert_ne!(
        first.token, second.token,
        "outer aliases must remain unique"
    );

    failover
        .ack(&first.token)
        .await
        .expect("expired alias ack is a no-op");
    assert_eq!(
        issuer.acknowledgements.load(Ordering::SeqCst),
        0,
        "the expired alias must not act on the reused inner token"
    );
    failover.ack(&second.token).await.expect("live alias ack");
    assert_eq!(issuer.acknowledgements.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
#[serial]
async fn clear_waits_for_pop_alias_publication_then_invalidates_the_alias() {
    let issuer = Arc::new(
        ScriptedDriver::operation([tagged_env("barrier")], None, ClearOutcome::Success(1))
            .blocking_pop(),
    );
    let failover = Arc::new(
        FailoverQueueDriver::new(vec![(
            "issuer".into(),
            issuer.clone() as Arc<dyn QueueDriver>,
        )])
        .expect("one driver"),
    );

    let aggregate = failover.clone();
    let pop = tokio::spawn(async move {
        aggregate
            .pop(Duration::from_secs(30))
            .await
            .expect("aggregate pop")
            .expect("reservation")
    });
    issuer.pop_entered.notified().await;

    let clear_started = Arc::new(tokio::sync::Notify::new());
    let aggregate = failover.clone();
    let started = clear_started.clone();
    let clear = tokio::spawn(async move {
        started.notify_one();
        aggregate.clear().await
    });
    clear_started.notified().await;
    assert!(
        tokio::time::timeout(Duration::from_secs(1), issuer.clear_entered.notified())
            .await
            .is_err(),
        "clear must not enter the backend while pop has not published its alias"
    );

    issuer.release_pop.notify_one();
    let reservation = pop.await.expect("pop task");
    assert_eq!(
        clear.await.expect("clear task").expect("aggregate clear"),
        1
    );
    failover
        .ack(&reservation.token)
        .await
        .expect("cleared alias ack is idempotent");
    assert_eq!(
        issuer.acknowledgements.load(Ordering::SeqCst),
        0,
        "successful clear must invalidate the alias published by pop"
    );
}

#[tokio::test(start_paused = true)]
#[serial]
async fn cleared_alias_cannot_ack_a_reused_inner_token_after_waiting_on_the_gate() {
    let old_envelope = tagged_env("old");
    let new_envelope = tagged_env("new");
    let new_id = new_envelope.id;
    let issuer = Arc::new(
        ScriptedDriver::operation([old_envelope], None, ClearOutcome::Success(1))
            .blocking_clear_and_ack(),
    );
    let failover = Arc::new(
        FailoverQueueDriver::new(vec![(
            "issuer".into(),
            issuer.clone() as Arc<dyn QueueDriver>,
        )])
        .expect("one driver"),
    );
    let old = failover
        .pop(Duration::from_secs(30))
        .await
        .expect("old aggregate pop")
        .expect("old reservation");

    let aggregate = failover.clone();
    let clear = tokio::spawn(async move { aggregate.clear().await });
    issuer.clear_entered.notified().await;

    let ack_started = Arc::new(tokio::sync::Notify::new());
    let aggregate = failover.clone();
    let started = ack_started.clone();
    let old_token = old.token.clone();
    let old_ack = tokio::spawn(async move {
        started.notify_one();
        aggregate.ack(&old_token).await
    });
    ack_started.notified().await;
    let _entered_backend_before_clear =
        tokio::time::timeout(Duration::from_secs(1), issuer.ack_entered.notified()).await;

    issuer.release_clear.notify_one();
    assert_eq!(
        clear.await.expect("clear task").expect("aggregate clear"),
        1
    );
    issuer.enqueue(new_envelope).await;
    let new = failover
        .pop(Duration::from_secs(30))
        .await
        .expect("post-clear aggregate pop")
        .expect("new reservation reusing the fixed inner token");
    assert_eq!(new.envelope.id, new_id);
    assert_ne!(old.token, new.token, "aggregate aliases remain distinct");

    issuer.release_ack.notify_one();
    old_ack
        .await
        .expect("old ack task")
        .expect("stale aggregate ack is idempotent");
    assert!(
        issuer.acknowledged_envelopes().is_empty(),
        "the old aggregate alias must not acknowledge the new reservation"
    );
}

#[tokio::test]
#[serial]
async fn empty_pop_reports_every_failed_connection_in_configured_order() {
    let first = Arc::new(ScriptedDriver::operation(
        None,
        Some("first pop failed"),
        ClearOutcome::Success(0),
    ));
    let second = Arc::new(ScriptedDriver::operation(
        None,
        Some("second pop failed"),
        ClearOutcome::Success(0),
    ));
    let third = Arc::new(ScriptedDriver::operation(
        None,
        Some("third pop failed"),
        ClearOutcome::Success(0),
    ));
    let failover = FailoverQueueDriver::new(vec![
        ("first".into(), first as Arc<dyn QueueDriver>),
        ("second".into(), second as Arc<dyn QueueDriver>),
        ("third".into(), third as Arc<dyn QueueDriver>),
    ])
    .expect("three drivers");

    let message = failover
        .pop(Duration::from_secs(30))
        .await
        .expect_err("all failed pops must be aggregated")
        .to_string();
    let first_at = message.find("first").expect("first label");
    let second_at = message.find("second").expect("second label");
    let third_at = message.find("third").expect("third label");
    assert!(
        first_at < second_at && second_at < third_at,
        "pop failures must retain configured order: {message}"
    );
}

#[tokio::test]
#[serial]
async fn clear_reports_all_failures_and_invalidates_only_successful_connections() {
    let first = Arc::new(ScriptedDriver::operation(
        Some(tagged_env("first")),
        None,
        ClearOutcome::Failure("first clear failed"),
    ));
    let middle = Arc::new(ScriptedDriver::operation(
        Some(tagged_env("middle")),
        None,
        ClearOutcome::Success(1),
    ));
    let last = Arc::new(ScriptedDriver::operation(
        Some(tagged_env("last")),
        None,
        ClearOutcome::Failure("last clear failed"),
    ));
    let failover = FailoverQueueDriver::new(vec![
        ("first".into(), first.clone() as Arc<dyn QueueDriver>),
        ("middle".into(), middle.clone() as Arc<dyn QueueDriver>),
        ("last".into(), last.clone() as Arc<dyn QueueDriver>),
    ])
    .expect("three drivers");

    let mut aliases = Vec::new();
    for _ in 0..3 {
        let reservation = failover
            .pop(Duration::from_secs(30))
            .await
            .expect("aggregate pop")
            .expect("reservation");
        aliases.push((
            reservation.envelope.payload["tag"]
                .as_str()
                .expect("tag")
                .to_owned(),
            reservation.token,
        ));
    }

    let message = failover
        .clear()
        .await
        .expect_err("two failed clears must be reported")
        .to_string();
    let first_at = message.find("first").expect("first failed label");
    let last_at = message.find("last").expect("last failed label");
    assert!(
        first_at < last_at,
        "clear failures must retain order: {message}"
    );
    assert_eq!(middle.clear_calls.load(Ordering::SeqCst), 1);

    for (_, alias) in &aliases {
        failover.ack(alias).await.expect("post-clear ack");
    }
    assert_eq!(first.acknowledgements.load(Ordering::SeqCst), 1);
    assert_eq!(
        middle.acknowledgements.load(Ordering::SeqCst),
        0,
        "successful clear must invalidate only the middle alias"
    );
    assert_eq!(last.acknowledgements.load(Ordering::SeqCst), 1);
}

#[tokio::test]
#[serial]
async fn clear_count_overflow_is_labeled_and_does_not_skip_later_connections() {
    let first = Arc::new(ScriptedDriver::operation(
        None,
        None,
        ClearOutcome::Success(u64::MAX),
    ));
    let second = Arc::new(ScriptedDriver::operation(
        None,
        None,
        ClearOutcome::Success(1),
    ));
    let third = Arc::new(ScriptedDriver::operation(
        None,
        None,
        ClearOutcome::Success(0),
    ));
    let failover = FailoverQueueDriver::new(vec![
        ("first".into(), first as Arc<dyn QueueDriver>),
        ("second".into(), second.clone() as Arc<dyn QueueDriver>),
        ("third".into(), third.clone() as Arc<dyn QueueDriver>),
    ])
    .expect("three drivers");

    let message = failover
        .clear()
        .await
        .expect_err("the aggregate count must not wrap")
        .to_string();
    assert!(
        message.contains("second") && message.contains("overflow"),
        "the overflow must identify the connection whose count crossed the limit: {message}"
    );
    assert_eq!(second.clear_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        third.clear_calls.load(Ordering::SeqCst),
        1,
        "count overflow must not skip later clear attempts"
    );
}

#[tokio::test]
#[serial]
async fn failed_lifecycle_calls_retain_their_aliases_for_retry() {
    let issuer = Arc::new(
        ScriptedDriver::operation(
            [
                tagged_env("ack"),
                tagged_env("nack"),
                tagged_env("release"),
                tagged_env("settle"),
            ],
            None,
            ClearOutcome::Success(0),
        )
        .failing_once([
            LifecycleOperation::Ack,
            LifecycleOperation::Nack,
            LifecycleOperation::Release,
            LifecycleOperation::Settle,
        ]),
    );
    let failover = FailoverQueueDriver::new(vec![(
        "issuer".into(),
        issuer.clone() as Arc<dyn QueueDriver>,
    )])
    .expect("one driver");

    let ack = failover
        .pop(Duration::from_secs(30))
        .await
        .expect("ack pop")
        .expect("ack reservation");
    failover.ack(&ack.token).await.expect_err("first ack fails");
    failover.ack(&ack.token).await.expect("retried ack");

    let nack = failover
        .pop(Duration::from_secs(30))
        .await
        .expect("nack pop")
        .expect("nack reservation");
    failover
        .nack(&nack.token, Duration::ZERO)
        .await
        .expect_err("first nack fails");
    failover
        .nack(&nack.token, Duration::ZERO)
        .await
        .expect("retried nack");

    let release = failover
        .pop(Duration::from_secs(30))
        .await
        .expect("release pop")
        .expect("release reservation");
    failover
        .release(&release.token, &release.envelope, Duration::ZERO)
        .await
        .expect_err("first release fails");
    failover
        .release(&release.token, &release.envelope, Duration::ZERO)
        .await
        .expect("retried release");

    let settle = failover
        .pop(Duration::from_secs(30))
        .await
        .expect("settle pop")
        .expect("settle reservation");
    failover
        .settle(&settle.token, &[])
        .await
        .expect_err("first settle fails");
    assert_eq!(
        failover
            .settle(&settle.token, &[])
            .await
            .expect("retried settle"),
        Settled::Atomically
    );

    for operation in [
        LifecycleOperation::Ack,
        LifecycleOperation::Nack,
        LifecycleOperation::Release,
        LifecycleOperation::Settle,
    ] {
        assert_eq!(
            issuer.lifecycle_attempts(operation),
            2,
            "the failed call must leave its aggregate alias routable"
        );
    }
}

#[tokio::test]
#[serial]
async fn unknown_ack_nack_and_release_aliases_are_idempotent_no_ops() {
    let issuer = Arc::new(ScriptedDriver::operation(
        std::iter::empty(),
        None,
        ClearOutcome::Success(0),
    ));
    let failover = FailoverQueueDriver::new(vec![(
        "issuer".into(),
        issuer.clone() as Arc<dyn QueueDriver>,
    )])
    .expect("one driver");
    let unknown = ReservationToken(Uuid::from_u128(0xbaad_f00d_baad_f00d));

    failover.ack(&unknown).await.expect("unknown ack");
    failover
        .nack(&unknown, Duration::ZERO)
        .await
        .expect("unknown nack");
    failover
        .release(&unknown, &tagged_env("unknown"), Duration::ZERO)
        .await
        .expect("unknown release");

    assert_eq!(issuer.lifecycle_attempts(LifecycleOperation::Ack), 0);
    assert_eq!(issuer.lifecycle_attempts(LifecycleOperation::Nack), 0);
    assert_eq!(issuer.lifecycle_attempts(LifecycleOperation::Release), 0);
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
async fn failed_over_job_is_drained_and_settled_by_the_failover_driver() {
    let (_primary, fallback, failover) = build(true);
    Queue::set_driver(failover.clone());
    Queue::push(FailoverJob)
        .await
        .expect("push must succeed via fallback");
    assert_eq!(
        fallback.pending_size().await.expect("fallback pending"),
        1,
        "the failed-over job must be pending on the fallback"
    );

    let queues = ["default".to_owned()];
    let reservation = failover
        .pop_from(Duration::from_secs(1), &queues)
        .await
        .expect("aggregate filtered pop")
        .expect("the aggregate must drain the fallback");
    assert_eq!(
        failover
            .settle(&reservation.token, &[])
            .await
            .expect("aggregate settle"),
        Settled::Unsupported,
        "settle must report the fallback driver's answer"
    );

    failover
        .ack(&reservation.token)
        .await
        .expect("aggregate ack");
    assert_eq!(
        fallback.pending_size().await.expect("pending after ack"),
        0,
        "ack must not put the fallback reservation back into pending"
    );
    assert_eq!(
        fallback.reserved_size().await.expect("reserved after ack"),
        0,
        "ack must remove the fallback reservation"
    );
}

#[tokio::test]
#[serial]
async fn filtered_pop_rejects_unsupported_connection_before_draining_capable_fallback() {
    let unsupported = Arc::new(ScriptedDriver::new([tagged_env("unsupported")]));
    let capable = Arc::new(MemoryQueueDriver::new());
    capable
        .push(tagged_env("capable"))
        .await
        .expect("capable push");
    let failover = FailoverQueueDriver::new(vec![
        ("unsupported".into(), unsupported as Arc<dyn QueueDriver>),
        ("capable".into(), capable.clone() as Arc<dyn QueueDriver>),
    ])
    .expect("two drivers");

    let queues = ["default".to_owned()];
    let error = failover
        .pop_from(Duration::from_secs(30), &queues)
        .await
        .expect_err("filter support must be preflighted for every connection");
    assert!(
        error.to_string().contains("unsupported"),
        "the error must identify the incapable connection: {error}"
    );
    assert_eq!(
        capable.pending_size().await.expect("capable pending"),
        1,
        "preflight failure must happen before a capable fallback is drained"
    );
}

#[tokio::test]
#[serial]
async fn unknown_filter_error_stops_before_polling_a_later_supported_connection() {
    let capable = Arc::new(MemoryQueueDriver::new());
    capable
        .push(tagged_env("capable"))
        .await
        .expect("capable push");
    let failover = FailoverQueueDriver::new(vec![
        (
            "unknown".into(),
            Arc::new(DefaultDeadlineDriver) as Arc<dyn QueueDriver>,
        ),
        ("capable".into(), capable.clone() as Arc<dyn QueueDriver>),
    ])
    .expect("two drivers");

    let error = failover
        .pop_from(Duration::from_secs(30), &["default".to_owned()])
        .await
        .expect_err("an unknown driver's filter error must stop this poll");
    assert!(error.to_string().contains("unknown"));
    assert_eq!(
        capable.pending_size().await.expect("capable pending"),
        1,
        "later connections must not be drained after an unknown filter implementation errors"
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
async fn aggregate_counters_listings_and_clear_span_every_driver_in_configured_order() {
    let (primary, fallback, failover) = build(true);
    failover
        .push(tagged_env("fallback"))
        .await
        .expect("fallback push");
    primary.broken.store(false, Ordering::SeqCst);
    failover
        .push(tagged_env("primary"))
        .await
        .expect("primary push");

    assert_eq!(primary.pending_size().await.expect("primary pending"), 1);
    assert_eq!(fallback.pending_size().await.expect("fallback pending"), 1);
    assert_eq!(
        failover.size().await.expect("size"),
        2,
        "size must include both configured drivers"
    );
    assert_eq!(
        failover.pending_size().await.expect("pending_size"),
        2,
        "pending_size must include both configured drivers"
    );
    assert_eq!(
        failover.delayed_size().await.expect("delayed_size"),
        0,
        "neither driver contains a delayed job"
    );
    assert_eq!(
        failover.reserved_size().await.expect("reserved_size"),
        0,
        "neither driver contains a reservation"
    );

    let pending = failover.pending_jobs(None).await.expect("pending_jobs");
    let tags: Vec<Option<&str>> = pending
        .iter()
        .map(|job| job.payload["tag"].as_str())
        .collect();
    assert_eq!(
        tags,
        vec![Some("primary"), Some("fallback")],
        "listings must concatenate drivers in configured order"
    );
    assert!(
        failover
            .delayed_jobs(None)
            .await
            .expect("delayed_jobs")
            .is_empty(),
        "the aggregate delayed listing must be empty"
    );
    assert!(
        failover
            .reserved_jobs(None)
            .await
            .expect("reserved_jobs")
            .is_empty(),
        "the aggregate reserved listing must be empty"
    );
    assert_eq!(
        failover.clear().await.expect("clear"),
        2,
        "clear must report the sum removed from every driver"
    );
    assert_eq!(
        primary.size().await.expect("primary size after clear"),
        0,
        "clear must empty the primary"
    );
    assert_eq!(
        fallback.size().await.expect("fallback size after clear"),
        0,
        "clear must empty the fallback"
    );
    assert_eq!(
        failover.size().await.expect("aggregate size after clear"),
        0,
        "the aggregate must be empty after clear"
    );
}

#[tokio::test]
#[serial]
async fn lifecycle_calls_reach_the_primary_that_issued_the_token() {
    // The primary is healthy here, so the reservation exists on it; the
    // aggregate-issued alias must route ack back to that exact reservation.
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
async fn fallback_nack_routes_to_the_driver_that_issued_the_token() {
    let (_primary, fallback, failover) = build(true);
    failover
        .push(tagged_env("fallback-nack"))
        .await
        .expect("fallback push");
    let reservation = failover
        .pop(Duration::from_secs(30))
        .await
        .expect("aggregate pop")
        .expect("fallback reservation");
    let id = reservation.envelope.id;
    let attempts = reservation.envelope.attempts;

    failover
        .nack(&reservation.token, Duration::ZERO)
        .await
        .expect("aggregate nack");
    assert_eq!(fallback.pending_size().await.expect("pending"), 1);
    assert_eq!(fallback.reserved_size().await.expect("reserved"), 0);

    let retried = failover
        .pop(Duration::from_secs(30))
        .await
        .expect("aggregate retry pop")
        .expect("retried fallback reservation");
    assert_eq!(retried.envelope.id, id);
    assert_eq!(
        retried.envelope.attempts,
        attempts + 1,
        "nack must burn one attempt on the issuing fallback"
    );
    failover.ack(&retried.token).await.expect("cleanup ack");
}

#[tokio::test]
#[serial]
async fn fallback_release_routes_to_the_driver_that_issued_the_token() {
    let (_primary, fallback, failover) = build(true);
    failover
        .push(tagged_env("fallback-release"))
        .await
        .expect("fallback push");
    let reservation = failover
        .pop(Duration::from_secs(30))
        .await
        .expect("aggregate pop")
        .expect("fallback reservation");
    let id = reservation.envelope.id;
    let attempts = reservation.envelope.attempts;

    failover
        .release(&reservation.token, &reservation.envelope, Duration::ZERO)
        .await
        .expect("aggregate release");
    assert_eq!(fallback.pending_size().await.expect("pending"), 1);
    assert_eq!(fallback.reserved_size().await.expect("reserved"), 0);

    let released = failover
        .pop(Duration::from_secs(30))
        .await
        .expect("aggregate release pop")
        .expect("released fallback reservation");
    assert_eq!(released.envelope.id, id);
    assert_eq!(
        released.envelope.attempts, attempts,
        "release must preserve attempts on the issuing fallback"
    );
    failover.ack(&released.token).await.expect("cleanup ack");
}

#[tokio::test]
#[serial]
async fn settle_reports_the_primary_answer() {
    let (primary, _fallback, failover) = build(false);
    failover
        .push(tagged_env("primary-settle"))
        .await
        .expect("primary push");
    let reservation = failover
        .pop(Duration::from_secs(30))
        .await
        .expect("aggregate pop")
        .expect("primary reservation");
    assert_eq!(primary.reserved_size().await.expect("reserved"), 1);

    // `FlakyDriver::settle` answers `Stale`, which neither the trait default
    // nor the fallback memory driver ever produces. Asserting on it is what
    // proves the aggregate routes its public alias back to the primary.
    let settled = failover
        .settle(&reservation.token, &[])
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
async fn unknown_alias_settle_is_stale_never_unsupported() {
    let first = Arc::new(MemoryQueueDriver::new());
    let second = Arc::new(MemoryQueueDriver::new());
    let failover = FailoverQueueDriver::new(vec![
        ("first".into(), first as Arc<dyn QueueDriver>),
        ("second".into(), second as Arc<dyn QueueDriver>),
    ])
    .expect("two drivers");

    assert_eq!(
        failover
            .settle(
                &ReservationToken(Uuid::from_u128(0xdead_beef_dead_beef)),
                &[],
            )
            .await
            .expect("unknown settle"),
        Settled::Stale,
        "an unknown aggregate alias must not inherit a backend's Unsupported answer"
    );
}

#[tokio::test]
#[serial]
async fn expired_alias_is_stale_and_cannot_reach_its_former_issuer() {
    let issuer = Arc::new(ScriptedDriver::new([tagged_env("expired")]));
    let failover = FailoverQueueDriver::new(vec![(
        "issuer".into(),
        issuer.clone() as Arc<dyn QueueDriver>,
    )])
    .expect("one driver");
    let reservation = failover
        .pop(Duration::ZERO)
        .await
        .expect("aggregate pop")
        .expect("reservation");

    assert_eq!(
        failover
            .settle(&reservation.token, &[])
            .await
            .expect("expired settle"),
        Settled::Stale,
        "an alias at its lease deadline must not expose the issuer's Unsupported answer"
    );
    failover
        .ack(&reservation.token)
        .await
        .expect("expired ack is idempotent");
    assert_eq!(
        issuer.acknowledgements(),
        0,
        "an expired alias must never reach a possibly reused inner token"
    );
}

#[tokio::test]
#[serial]
async fn recovered_busy_primary_cannot_starve_the_fallback() {
    let (primary, _fallback, failover) = build(true);
    failover
        .push(tagged_env("fallback"))
        .await
        .expect("fallback push");
    primary.broken.store(false, Ordering::SeqCst);
    failover
        .push(tagged_env("primary-one"))
        .await
        .expect("first primary push");
    failover
        .push(tagged_env("primary-two"))
        .await
        .expect("second primary push");

    let first = failover
        .pop(Duration::from_secs(30))
        .await
        .expect("first aggregate pop")
        .expect("first reservation");
    let second = failover
        .pop(Duration::from_secs(30))
        .await
        .expect("second aggregate pop")
        .expect("second reservation");
    let popped_tags = [
        first.envelope.payload["tag"].as_str(),
        second.envelope.payload["tag"].as_str(),
    ];
    assert!(
        popped_tags.contains(&Some("fallback")),
        "two fair pops must visit the fallback while the primary remains busy; got {popped_tags:?}"
    );
    assert_eq!(
        primary.pending_size().await.expect("primary pending"),
        1,
        "the primary must still be busy when the fallback is selected"
    );

    failover.ack(&first.token).await.expect("first cleanup ack");
    failover
        .ack(&second.token)
        .await
        .expect("second cleanup ack");
}

#[tokio::test]
#[serial]
async fn colliding_inner_tokens_receive_distinct_aliases_and_route_to_their_issuers() {
    let first = Arc::new(ScriptedDriver::new([tagged_env("first")]));
    let second = Arc::new(ScriptedDriver::new([tagged_env("second")]));
    let failover = FailoverQueueDriver::new(vec![
        ("first".into(), first.clone() as Arc<dyn QueueDriver>),
        ("second".into(), second.clone() as Arc<dyn QueueDriver>),
    ])
    .expect("two drivers");

    let first_reservation = failover
        .pop(Duration::from_secs(30))
        .await
        .expect("first aggregate pop")
        .expect("first reservation");
    let second_reservation = failover
        .pop(Duration::from_secs(30))
        .await
        .expect("second aggregate pop")
        .expect("second reservation");
    let first_issuer = first_reservation.envelope.payload["tag"]
        .as_str()
        .expect("first issuer tag");
    let second_issuer = second_reservation.envelope.payload["tag"]
        .as_str()
        .expect("second issuer tag");
    assert_ne!(
        first_issuer, second_issuer,
        "the two pops must come from different issuers"
    );
    assert_ne!(
        first_reservation.token, second_reservation.token,
        "equal backend tokens must receive distinct public aliases"
    );

    failover
        .ack(&first_reservation.token)
        .await
        .expect("first alias ack");
    let counts_after_first = (first.acknowledgements(), second.acknowledgements());
    let expected_after_first = if first_issuer == "first" {
        (1, 0)
    } else {
        (0, 1)
    };
    assert_eq!(
        counts_after_first, expected_after_first,
        "the first alias must reach only its issuer"
    );

    failover
        .ack(&second_reservation.token)
        .await
        .expect("second alias ack");
    assert_eq!(first.acknowledgements(), 1);
    assert_eq!(second.acknowledgements(), 1);
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
