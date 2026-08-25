//! CI-04 - fault injection for the queue worker's settlement paths.
//!
//! The worker's happy path is well covered. What was not covered is what
//! happens when the *broker* misbehaves: an ack that fails after the job
//! already ran, a nack that never lands, a lease that expires mid-flight, a
//! bulk push that dies halfway. Those paths are where at-least-once
//! delivery either holds or quietly turns into at-most-once, and until now
//! nothing exercised them.
//!
//! `worker.rs`'s `settlement_failure` already enumerates seven
//! `(operation, outcome)` pairs it can hit. That taxonomy is a map of
//! reachable states, and every one of them was reachable only in
//! production. These tests make them reachable in the suite.
//!
//! The faults are injected through a driver decorator rather than by
//! sleeping or racing, so the tests are deterministic: `FaultDriver` wraps
//! any real `QueueDriver` and fails the operation you name, on the call you
//! name, in the way you name.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use suprnova::queue::driver::{QueueDriver, Reservation, ReservationToken};
use suprnova::queue::memory::MemoryQueueDriver;
use suprnova::queue::worker::{WorkerConfig, register_job, run_worker};
use suprnova::queue::{BackoffSchedule, Queue};
use suprnova::queue::{CURRENT_SCHEMA_VERSION, Envelope};
use suprnova::{FrameworkError, Job, async_trait};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// The fault-injecting driver
// ---------------------------------------------------------------------------

/// How a faulted operation should behave.
///
/// The distinction matters enormously and is the whole reason this enum
/// exists rather than a bare `bool`:
///
/// - [`Fault::AfterEffect`] models the *uncertainty* case - the broker
///   carried out the request and the acknowledgement was lost on the way
///   back. The state changed; the caller does not know it.
/// - [`Fault::BeforeEffect`] models a request that never landed at all.
///
/// A system that is correct under one and not the other is not correct.
/// `ack` is the interesting one: an ack lost on the return path leaves a
/// job that ran, completed, and will be delivered again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    /// Apply the operation to the inner driver, then report an error.
    AfterEffect,
    /// Report an error without touching the inner driver.
    BeforeEffect,
}

/// Which call of an operation to fail (1-based), and how.
#[derive(Clone, Copy, Debug)]
struct FaultPlan {
    /// 1-based ordinal of the call to fail. `1` fails the first call.
    nth: u32,
    fault: Fault,
}

/// Wraps a real driver and injects failures into named operations.
///
/// Deliberately a decorator over a *real* driver rather than a hand-rolled
/// stub: the point is to prove the worker's behaviour against genuine queue
/// semantics with one operation perturbed, not against a mock that agrees
/// with whatever the test expects.
struct FaultDriver {
    inner: Arc<dyn QueueDriver>,
    ack_plan: Mutex<Option<FaultPlan>>,
    nack_plan: Mutex<Option<FaultPlan>>,
    push_plan: Mutex<Option<FaultPlan>>,
    ack_calls: AtomicU32,
    nack_calls: AtomicU32,
    push_calls: AtomicU32,
    /// Every token the worker successfully acked, in order. Lets a test
    /// assert settlement actually happened rather than inferring it.
    acked: Mutex<Vec<ReservationToken>>,
}

impl FaultDriver {
    fn new(inner: Arc<dyn QueueDriver>) -> Self {
        Self {
            inner,
            ack_plan: Mutex::new(None),
            nack_plan: Mutex::new(None),
            push_plan: Mutex::new(None),
            ack_calls: AtomicU32::new(0),
            nack_calls: AtomicU32::new(0),
            push_calls: AtomicU32::new(0),
            acked: Mutex::new(Vec::new()),
        }
    }

    fn fail_ack(self: &Arc<Self>, nth: u32, fault: Fault) {
        *self.ack_plan.lock().unwrap() = Some(FaultPlan { nth, fault });
    }

    fn fail_nack(self: &Arc<Self>, nth: u32, fault: Fault) {
        *self.nack_plan.lock().unwrap() = Some(FaultPlan { nth, fault });
    }

    fn fail_push(self: &Arc<Self>, nth: u32, fault: Fault) {
        *self.push_plan.lock().unwrap() = Some(FaultPlan { nth, fault });
    }

    fn ack_count(&self) -> u32 {
        self.ack_calls.load(Ordering::SeqCst)
    }

    fn nack_count(&self) -> u32 {
        self.nack_calls.load(Ordering::SeqCst)
    }

    fn acked_tokens(&self) -> Vec<ReservationToken> {
        self.acked.lock().unwrap().clone()
    }

    /// Resolve the plan for this call: returns the fault to apply, if any.
    fn decide(plan: &Mutex<Option<FaultPlan>>, calls: &AtomicU32) -> Option<Fault> {
        let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
        let plan = plan.lock().unwrap();
        match *plan {
            Some(p) if p.nth == n => Some(p.fault),
            _ => None,
        }
    }
}

#[async_trait]
impl QueueDriver for FaultDriver {
    async fn push(&self, env: Envelope) -> Result<(), FrameworkError> {
        match Self::decide(&self.push_plan, &self.push_calls) {
            Some(Fault::BeforeEffect) => {
                Err(FrameworkError::internal("injected: push never landed"))
            }
            Some(Fault::AfterEffect) => {
                self.inner.push(env).await?;
                Err(FrameworkError::internal("injected: push landed, ack lost"))
            }
            None => self.inner.push(env).await,
        }
    }

    async fn pop(
        &self,
        visibility_timeout: Duration,
    ) -> Result<Option<Reservation>, FrameworkError> {
        self.inner.pop(visibility_timeout).await
    }

    async fn pop_from(
        &self,
        visibility_timeout: Duration,
        queues: &[String],
    ) -> Result<Option<Reservation>, FrameworkError> {
        self.inner.pop_from(visibility_timeout, queues).await
    }

    async fn ack(&self, token: &ReservationToken) -> Result<(), FrameworkError> {
        match Self::decide(&self.ack_plan, &self.ack_calls) {
            Some(Fault::BeforeEffect) => Err(FrameworkError::internal(
                "injected: ack never reached the broker",
            )),
            Some(Fault::AfterEffect) => {
                self.inner.ack(token).await?;
                self.acked.lock().unwrap().push(token.clone());
                Err(FrameworkError::internal(
                    "injected: ack applied, response lost",
                ))
            }
            None => {
                let r = self.inner.ack(token).await;
                if r.is_ok() {
                    self.acked.lock().unwrap().push(token.clone());
                }
                r
            }
        }
    }

    async fn nack(
        &self,
        token: &ReservationToken,
        requeue_delay: Duration,
    ) -> Result<(), FrameworkError> {
        match Self::decide(&self.nack_plan, &self.nack_calls) {
            Some(Fault::BeforeEffect) => Err(FrameworkError::internal(
                "injected: nack never reached the broker",
            )),
            Some(Fault::AfterEffect) => {
                self.inner.nack(token, requeue_delay).await?;
                Err(FrameworkError::internal(
                    "injected: nack applied, response lost",
                ))
            }
            None => self.inner.nack(token, requeue_delay).await,
        }
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

    async fn clear(&self) -> Result<u64, FrameworkError> {
        self.inner.clear().await
    }

    fn name(&self) -> &'static str {
        "fault"
    }
}

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

static RUNS: AtomicU32 = AtomicU32::new(0);

/// Records every execution so a test can distinguish "ran once" from "ran
/// again after a lost ack".
#[derive(Serialize, Deserialize, Debug, Clone)]
struct CountingJob {
    id: u32,
}

#[async_trait]
impl Job for CountingJob {
    fn job_name() -> &'static str {
        "CiFourCountingJob"
    }
    fn max_tries() -> u32 {
        3
    }
    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Fixed { secs: 0 }
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        RUNS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

static FAIL_RUNS: AtomicU32 = AtomicU32::new(0);

/// Always fails, so the worker takes the nack / dead-letter path.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct AlwaysFailingJob {
    id: u32,
}

#[async_trait]
impl Job for AlwaysFailingJob {
    fn job_name() -> &'static str {
        "CiFourAlwaysFailingJob"
    }
    fn max_tries() -> u32 {
        2
    }
    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Fixed { secs: 0 }
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        FAIL_RUNS.fetch_add(1, Ordering::SeqCst);
        Err(FrameworkError::internal("always fails"))
    }
}

/// Build an envelope the same way the other queue tests do - by literal,
/// since `Envelope` has no constructor and the fields are the contract.
fn env(name: &str, payload: serde_json::Value) -> Envelope {
    Envelope {
        schema_version: CURRENT_SCHEMA_VERSION,
        id: Uuid::new_v4(),
        job_name: name.into(),
        queue: None,
        payload,
        dispatched_at: Utc::now(),
        available_at: Utc::now(),
        attempts: 0,
        max_tries: 3,
        backoff: BackoffSchedule::default(),
        timeout_secs: None,
        fail_on_timeout: false,
        idempotency_key: None,
        batch_id: None,
        chain_remaining: Vec::new(),
    }
}

fn worker_config(max_jobs: Option<u64>) -> WorkerConfig {
    WorkerConfig {
        visibility_timeout: Duration::from_secs(60),
        poll_interval: Duration::from_millis(5),
        max_jobs,
        queues: Vec::new(),
    }
}

/// Drive the worker until it has settled `max_jobs` jobs, with a hard
/// deadline so a hang fails loudly instead of hanging the suite.
async fn run_until_done(driver: Arc<dyn QueueDriver>, max_jobs: u64) {
    let handle = tokio::spawn(run_worker(
        driver,
        worker_config(Some(max_jobs)),
        CancellationToken::new(),
    ));
    match tokio::time::timeout(Duration::from_secs(10), handle).await {
        Ok(joined) => joined.expect("worker task panicked"),
        Err(_) => panic!("worker did not settle {max_jobs} job(s) within 10s"),
    }
}

// ---------------------------------------------------------------------------
// ACK uncertainty
// ---------------------------------------------------------------------------

/// A lost ack response must not lose the job's completion.
///
/// The broker applied the ack; only the reply vanished. The worker cannot
/// tell this apart from "the ack never landed", so the one thing it must
/// not do is treat the job as failed and retry it as a *failure* - the job
/// succeeded. `settlement_failure("ack", "success")` documents the intent
/// ("job may be re-delivered (at-least-once)"); this pins it.
#[tokio::test]
#[serial]
async fn a_lost_ack_response_does_not_retry_a_job_that_succeeded() {
    RUNS.store(0, Ordering::SeqCst);

    let inner = Arc::new(MemoryQueueDriver::new());
    let driver = Arc::new(FaultDriver::new(inner.clone()));
    Queue::set_driver(driver.clone());
    register_job::<CountingJob>();

    Queue::push(CountingJob { id: 1 }).await.unwrap();

    // The ack lands at the broker; the response is lost.
    driver.fail_ack(1, Fault::AfterEffect);

    run_until_done(driver.clone(), 1).await;

    assert_eq!(
        RUNS.load(Ordering::SeqCst),
        1,
        "the job ran once and succeeded; a lost ack RESPONSE must not cause \
         a re-run, because the ack itself was applied"
    );
    assert_eq!(driver.ack_count(), 1, "the worker acked exactly once");
    assert_eq!(
        inner.size().await.unwrap(),
        0,
        "the ack reached the inner driver, so nothing may remain queued - \
         if this fails, a lost ack response silently converts at-least-once \
         into an infinite redelivery loop"
    );
}

/// An ack that never reached the broker leaves the message reserved, and
/// the worker must not pretend otherwise.
///
/// This is the genuinely at-least-once case: the job ran, the broker never
/// learned it, so the message is still there and will be redelivered when
/// the lease lapses. The invariant under test is narrower than "it works" -
/// it is that the worker does not *consume* the failure silently and leave
/// the queue in a state it misreports.
#[tokio::test]
#[serial]
async fn an_ack_that_never_landed_leaves_the_message_for_redelivery() {
    RUNS.store(0, Ordering::SeqCst);

    let inner = Arc::new(MemoryQueueDriver::new());
    let driver = Arc::new(FaultDriver::new(inner.clone()));
    Queue::set_driver(driver.clone());
    register_job::<CountingJob>();

    Queue::push(CountingJob { id: 2 }).await.unwrap();
    driver.fail_ack(1, Fault::BeforeEffect);

    run_until_done(driver.clone(), 1).await;

    assert_eq!(RUNS.load(Ordering::SeqCst), 1, "the job ran exactly once");
    assert_eq!(driver.ack_count(), 1);
    assert!(
        driver.acked_tokens().is_empty(),
        "no ack reached the broker, so none may be recorded"
    );
    assert_eq!(
        inner.size().await.unwrap(),
        1,
        "the message is still held by the broker - this is the at-least-once \
         contract, and a worker that dropped it here would be at-most-once"
    );
}

// ---------------------------------------------------------------------------
// Nack failure
// ---------------------------------------------------------------------------

/// A failed nack must not lose the job or crash the worker.
///
/// `settlement_failure("nack", "retry")` warns that the reservation "may be
/// redelivered after visibility expiry without bumped attempts". The
/// dangerous reading of that is a job that retries forever because attempts
/// never advance. This test pins the part that is actually guaranteed: the
/// worker survives, and the message is not silently dropped.
#[tokio::test]
#[serial]
async fn a_failed_nack_keeps_the_job_rather_than_dropping_it() {
    FAIL_RUNS.store(0, Ordering::SeqCst);

    let inner = Arc::new(MemoryQueueDriver::new());
    let driver = Arc::new(FaultDriver::new(inner.clone()));
    Queue::set_driver(driver.clone());
    register_job::<AlwaysFailingJob>();

    Queue::push(AlwaysFailingJob { id: 3 }).await.unwrap();
    driver.fail_nack(1, Fault::BeforeEffect);

    run_until_done(driver.clone(), 1).await;

    assert_eq!(FAIL_RUNS.load(Ordering::SeqCst), 1, "the job ran once");
    assert_eq!(driver.nack_count(), 1, "the worker attempted one nack");
    assert_eq!(
        inner.size().await.unwrap(),
        1,
        "the nack never landed, so the broker still holds the message; \
         dropping it here would lose a job that has retries left"
    );
    // `size()` alone cannot tell this apart from a *successful* nack - the
    // message counts either way. `reserved_size()` is what distinguishes
    // them: a nack that landed returns the message to the visible set,
    // while one that never landed leaves it reserved until the lease
    // lapses. That difference is the whole content of
    // `settlement_failure("nack", "retry")`, so it is what to assert.
    assert_eq!(
        inner.reserved_size().await.unwrap(),
        1,
        "the message must still be RESERVED, not requeued - the worker must \
         not act as though a nack it never confirmed had taken effect"
    );
    assert_eq!(
        inner.pending_size().await.unwrap(),
        0,
        "and it must not be visible for another worker yet; that only \
         happens when the lease expires"
    );
}

/// The counterpart to the test above: when the nack *does* land, the
/// message goes back to the visible set with its attempt count bumped.
///
/// Present so the fault case above is provably measuring the fault. Two
/// tests that read the same numbers prove nothing; these read different
/// ones from the same accessors.
#[tokio::test]
#[serial]
async fn a_successful_nack_returns_the_message_to_the_visible_set() {
    FAIL_RUNS.store(0, Ordering::SeqCst);

    let inner = Arc::new(MemoryQueueDriver::new());
    let driver = Arc::new(FaultDriver::new(inner.clone()));
    Queue::set_driver(driver.clone());
    register_job::<AlwaysFailingJob>();

    Queue::push(AlwaysFailingJob { id: 4 }).await.unwrap();
    // No fault injected this time.

    run_until_done(driver.clone(), 1).await;

    assert_eq!(driver.nack_count(), 1);
    assert_eq!(
        inner.reserved_size().await.unwrap(),
        0,
        "a nack that landed releases the reservation"
    );
    assert_eq!(
        inner.pending_size().await.unwrap(),
        1,
        "and makes the message available again for the retry"
    );
}

// ---------------------------------------------------------------------------
// Partial dispatch
// ---------------------------------------------------------------------------

/// A bulk push that dies partway must not silently report success, and the
/// items that did land must still be there.
///
/// The default `bulk_push` pushes serially, so a mid-list failure leaves a
/// partially-populated queue. The contract that matters to a caller is that
/// the error is not swallowed - a caller that believes all ten landed when
/// three did has no way to recover.
#[tokio::test]
#[serial]
async fn a_partial_bulk_push_reports_the_failure_and_keeps_what_landed() {
    let inner = Arc::new(MemoryQueueDriver::new());
    let driver = Arc::new(FaultDriver::new(inner.clone()));
    Queue::set_driver(driver.clone());
    register_job::<CountingJob>();

    // Fail the 3rd push outright, so items 1 and 2 land and 3 does not.
    driver.fail_push(3, Fault::BeforeEffect);

    let envelopes: Vec<Envelope> = (0..5)
        .map(|i| env("CiFourCountingJob", serde_json::json!({ "id": i })))
        .collect();

    let result = driver.bulk_push(envelopes).await;

    assert!(
        result.is_err(),
        "a bulk push that could not place every item must report an error; \
         swallowing it leaves the caller believing work is queued that is not"
    );
    assert_eq!(
        inner.size().await.unwrap(),
        2,
        "the two pushes that preceded the failure landed and must remain - \
         bulk_push is not transactional and must not pretend to roll back"
    );
}

// ---------------------------------------------------------------------------
// Redelivery / idempotent settlement
// ---------------------------------------------------------------------------

/// Settling the same token twice must be harmless.
///
/// The trait requires it in so many words - "Drivers MUST be tolerant of
/// unknown / already-acked tokens (idempotent)" - because a worker that
/// retries a lost ack will present the same token again. An implementation
/// that errors on the second ack turns a recoverable blip into a stuck job.
/// Nothing tested this on any driver.
#[tokio::test]
#[serial]
async fn settling_a_token_twice_is_idempotent_on_every_path() {
    let driver = MemoryQueueDriver::new();
    driver
        .push(env("CiFourCountingJob", serde_json::json!({ "id": 9 })))
        .await
        .unwrap();

    let res = driver
        .pop(Duration::from_secs(60))
        .await
        .unwrap()
        .expect("a message was pushed, so one must pop");

    driver.ack(&res.token).await.expect("first ack succeeds");
    driver
        .ack(&res.token)
        .await
        .expect("a repeated ack must be a no-op, not an error");
    driver
        .nack(&res.token, Duration::from_secs(0))
        .await
        .expect("nacking an already-acked token must be a no-op, not an error");

    assert_eq!(
        driver.size().await.unwrap(),
        0,
        "the redundant nack must not resurrect an acked message"
    );

    // An entirely unknown token is the other half of the same contract.
    let unknown = ReservationToken(uuid::Uuid::new_v4());
    driver
        .ack(&unknown)
        .await
        .expect("acking an unknown token must be a no-op");
    driver
        .nack(&unknown, Duration::from_secs(0))
        .await
        .expect("nacking an unknown token must be a no-op");
    assert_eq!(
        driver.size().await.unwrap(),
        0,
        "settling an unknown token must not invent a message"
    );
}

// ---------------------------------------------------------------------------
// Lease loss
// ---------------------------------------------------------------------------

/// When a lease expires while the job is still running, the message becomes
/// available again and a second worker may take it - at-least-once in its
/// rawest form.
///
/// Driven by an expired visibility timeout rather than a sleep race, so it
/// is deterministic. The invariant is that the redelivered message is the
/// same envelope with its identity intact: a consumer's idempotency key is
/// `envelope.id`, and if redelivery minted a new id every consumer's
/// dedupe would silently fail open.
#[tokio::test]
#[serial]
async fn a_lapsed_lease_redelivers_the_same_envelope_identity() {
    let driver = MemoryQueueDriver::new();
    driver
        .push(env("CiFourCountingJob", serde_json::json!({ "id": 11 })))
        .await
        .unwrap();

    // Reserve with a lease that has already lapsed by the time we look.
    let first = driver
        .pop(Duration::from_millis(1))
        .await
        .unwrap()
        .expect("the pushed message must pop");
    tokio::time::sleep(Duration::from_millis(30)).await;

    let second = driver
        .pop(Duration::from_secs(60))
        .await
        .unwrap()
        .expect("once the lease lapses the message must become available again");

    assert_eq!(
        first.envelope.id, second.envelope.id,
        "redelivery must preserve the envelope id - consumers dedupe on it, \
         so a fresh id would make every idempotency check fail open"
    );
    assert_ne!(
        first.token, second.token,
        "each reservation is a distinct lease and must carry its own token, \
         or the stale worker could settle the new holder's message"
    );

    // The stale holder settling its lapsed lease must not remove the
    // message the *current* holder is working on.
    driver
        .ack(&first.token)
        .await
        .expect("a stale ack must not error");
    assert_eq!(
        driver.size().await.unwrap(),
        1,
        "a stale worker's ack must not delete a message that has since been \
         re-reserved by someone else - that is how a job silently vanishes"
    );

    driver.ack(&second.token).await.unwrap();
    assert_eq!(driver.size().await.unwrap(), 0);
}

/// Tokens must be unique per reservation across the driver's lifetime.
///
/// Cheap to state, and the thing every "stale worker" guard rests on. If a
/// token were reused, a stale settle would land on the wrong message.
#[tokio::test]
#[serial]
async fn reservation_tokens_are_never_reused() {
    let driver = MemoryQueueDriver::new();
    let mut seen: HashSet<ReservationToken> = HashSet::new();

    for i in 0..8 {
        driver
            .push(env("CiFourCountingJob", serde_json::json!({ "id": i })))
            .await
            .unwrap();
    }

    for _ in 0..8 {
        let res = driver
            .pop(Duration::from_secs(60))
            .await
            .unwrap()
            .expect("eight were pushed, so eight must pop");
        assert!(
            seen.insert(res.token.clone()),
            "reservation token {:?} was issued twice; a stale settle would \
             then land on an unrelated message",
            res.token
        );
    }
}
