//! The tenth parity round's queue clauses of PAR-111: every failed job
//! attempt, a timed-out one included, is reported through `Exceptions`,
//! and an error the application names with `dont_retry` or
//! `dont_retry_when` fails the job at once. Each test is named after the
//! falsifier clause it observes.
//!
//! # Isolation
//!
//! The `Exceptions` registry, the queue driver and the job registry are
//! process-wide. Every test here runs `#[serial]`, holds the binary's env
//! lock, and empties the `Exceptions` registry at its start and end
//! ([`Isolated`]). Its callbacks match error types no other test uses.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serial_test::serial;
use suprnova::events::{EventFacade, dispatched};
use suprnova::queue::events::JobFailed;
use suprnova::queue::worker::{WorkerConfig, register_job};
use suprnova::queue::{FailOnException, JobMiddleware};
use suprnova::{
    BackoffSchedule, Exceptions, FrameworkError, Job, MemoryQueueDriver, Queue, QueueDriver,
    TimeoutExceeded, WorkerControls, async_trait, run_worker_with_controls,
};
use tokio_util::sync::CancellationToken;

/// Empties the `Exceptions` registry when made and when dropped, and holds
/// the env lock so no other test of this binary that registers callbacks
/// overlaps.
struct Isolated {
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

impl Isolated {
    async fn new() -> Self {
        let lock = crate::env_lock::lock_env_async().await;
        Exceptions::reset();
        Self { _lock: lock }
    }
}

impl Drop for Isolated {
    fn drop(&mut self) {
        Exceptions::reset();
    }
}

/// An error type only this file uses.
#[derive(Debug)]
struct CardDeclined(u32);

impl std::fmt::Display for CardDeclined {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "card declined for order {}", self.0)
    }
}

impl std::error::Error for CardDeclined {}

/// How many times [`ChargeCard`] ran.
static CHARGES: AtomicU32 = AtomicU32::new(0);

/// Fails every attempt with [`CardDeclined`] when `declined`, else with an
/// internal error; three tries, no backoff.
#[derive(serde::Serialize, serde::Deserialize)]
struct ChargeCard {
    order: u32,
    declined: bool,
}

#[async_trait]
impl Job for ChargeCard {
    fn job_name() -> &'static str {
        "laravel_http_gaps::ChargeCard"
    }
    fn max_tries() -> u32 {
        3
    }
    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Fixed { secs: 0 }
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        CHARGES.fetch_add(1, Ordering::SeqCst);
        if self.declined {
            Err(FrameworkError::from_external(CardDeclined(self.order)))
        } else {
            Err(FrameworkError::internal(format!(
                "the gateway timed out for order {}",
                self.order
            )))
        }
    }
}

/// Fails with an error `FailOnException` turns into a failed job.
#[derive(serde::Serialize, serde::Deserialize)]
struct RefundCard {
    order: u32,
}

#[async_trait]
impl Job for RefundCard {
    fn job_name() -> &'static str {
        "laravel_http_gaps::RefundCard"
    }
    fn max_tries() -> u32 {
        3
    }
    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Fixed { secs: 0 }
    }
    fn middleware() -> Vec<Arc<dyn JobMiddleware>> {
        vec![Arc::new(FailOnException::new(|error: &FrameworkError| {
            error
                .external_source()
                .is_some_and(|source| source.is::<CardDeclined>())
        }))]
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Err(FrameworkError::from_external(CardDeclined(self.order)))
    }
}

/// How many attempts of [`SlowCharge`] started.
static SLOW_STARTS: AtomicU32 = AtomicU32::new(0);

/// Sleeps past its one-second per-attempt timeout on every attempt; two
/// tries, no backoff. The budget is whole seconds because the envelope
/// stores `timeout_secs`.
#[derive(serde::Serialize, serde::Deserialize)]
struct SlowCharge {
    order: u32,
}

#[async_trait]
impl Job for SlowCharge {
    fn job_name() -> &'static str {
        "laravel_http_gaps::SlowCharge"
    }
    fn max_tries() -> u32 {
        2
    }
    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Fixed { secs: 0 }
    }
    fn timeout() -> Option<Duration> {
        Some(Duration::from_secs(1))
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        SLOW_STARTS.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(30)).await;
        Ok(())
    }
}

/// A fresh memory driver holding `jobs`, with every job type of this file
/// registered.
async fn queue_with<J: Job>(jobs: Vec<J>) -> Arc<MemoryQueueDriver> {
    CHARGES.store(0, Ordering::SeqCst);
    SLOW_STARTS.store(0, Ordering::SeqCst);
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    register_job::<ChargeCard>();
    register_job::<RefundCard>();
    register_job::<SlowCharge>();
    for job in jobs {
        Queue::push(job)
            .await
            .expect("the memory driver takes the push");
    }
    driver
}

/// Run a worker until the queue is empty.
async fn drain(driver: Arc<MemoryQueueDriver>) {
    drain_with_tries(driver, None).await;
}

/// Run a worker until the queue is empty, with `tries` overriding each
/// job's own attempt budget as `queue:work --tries` does.
async fn drain_with_tries(driver: Arc<MemoryQueueDriver>, tries: Option<u32>) {
    let exit = tokio::time::timeout(
        Duration::from_secs(10),
        run_worker_with_controls(
            driver,
            WorkerConfig {
                poll_interval: Duration::from_millis(5),
                ..WorkerConfig::default()
            },
            WorkerControls {
                stop_when_empty: true,
                tries,
                ..WorkerControls::default()
            },
            CancellationToken::new(),
        ),
    )
    .await
    .expect("the worker drains the queue within 10 seconds")
    .expect("the worker runs");
    assert_eq!(exit, 0);
}

/// Record each [`CardDeclined`] a callback receives.
fn record_declines() -> Arc<Mutex<Vec<String>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    Exceptions::reportable(move |error: &CardDeclined| {
        record
            .lock()
            .expect("the list is not poisoned")
            .push(error.to_string());
    });
    seen
}

fn seen(list: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    list.lock().expect("the list is not poisoned").clone()
}

/// Record each [`TimeoutExceeded`] of [`SlowCharge`] a callback receives,
/// as `<job name> after <budget>`.
fn record_timeouts() -> Arc<Mutex<Vec<String>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    Exceptions::reportable(move |error: &TimeoutExceeded| {
        if error.job_name == SlowCharge::job_name() {
            record
                .lock()
                .expect("the list is not poisoned")
                .push(format!("{} after {:?}", error.job_name, error.timeout));
        }
    });
    seen
}

const SLOW_CHARGE_TIMED_OUT: &str = "laravel_http_gaps::SlowCharge after 1s";

#[tokio::test]
#[serial]
async fn every_failed_attempt_of_a_queued_job_reaches_the_callbacks() {
    let _isolated = Isolated::new().await;
    let declines = record_declines();
    let _events = EventFacade::fake();
    let driver = queue_with(vec![ChargeCard {
        order: 11,
        declined: true,
    }])
    .await;

    drain(driver.clone()).await;

    assert_eq!(CHARGES.load(Ordering::SeqCst), 3, "three tries");
    assert_eq!(
        seen(&declines),
        [
            "card declined for order 11",
            "card declined for order 11",
            "card declined for order 11"
        ],
        "each failed attempt, the retried ones and the last, is reported"
    );
    assert_eq!(dispatched::<JobFailed>(|_| true).len(), 1);
    assert_eq!(driver.size(None).await.expect("size"), 0);
}

#[tokio::test]
#[serial]
async fn a_job_failed_by_fail_on_exception_reaches_the_callbacks() {
    let _isolated = Isolated::new().await;
    let declines = record_declines();
    let _events = EventFacade::fake();
    let driver = queue_with(vec![RefundCard { order: 12 }]).await;

    drain(driver.clone()).await;

    assert_eq!(seen(&declines), ["card declined for order 12"]);
    assert_eq!(dispatched::<JobFailed>(|_| true).len(), 1);
}

#[tokio::test]
#[serial]
async fn a_job_failing_with_an_error_dont_retry_when_accepts_fails_without_another_attempt() {
    let _isolated = Isolated::new().await;
    Exceptions::dont_retry_when(|error: &FrameworkError| {
        error.to_string().starts_with("card declined for order")
    });
    let _events = EventFacade::fake();
    let driver = queue_with(vec![ChargeCard {
        order: 13,
        declined: true,
    }])
    .await;

    drain(driver.clone()).await;

    assert_eq!(
        CHARGES.load(Ordering::SeqCst),
        1,
        "the job fails on its first attempt, with two tries left"
    );
    let failed = dispatched::<JobFailed>(|_| true);
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].exception, "card declined for order 13");
    assert_eq!(driver.size(None).await.expect("size"), 0);
    assert_eq!(driver.delayed_size(None).await.expect("delayed size"), 0);
}

#[tokio::test]
#[serial]
async fn a_job_failing_with_a_dont_retry_type_fails_without_another_attempt() {
    let _isolated = Isolated::new().await;
    Exceptions::dont_retry::<CardDeclined>();
    let declines = record_declines();
    let _events = EventFacade::fake();
    let driver = queue_with(vec![ChargeCard {
        order: 14,
        declined: true,
    }])
    .await;

    drain(driver.clone()).await;

    assert_eq!(CHARGES.load(Ordering::SeqCst), 1);
    assert_eq!(dispatched::<JobFailed>(|_| true).len(), 1);
    assert_eq!(
        seen(&declines),
        ["card declined for order 14"],
        "the attempt that ends the retries is reported too"
    );
}

#[tokio::test]
#[serial]
async fn an_error_no_dont_retry_entry_names_is_retried_until_its_tries_are_spent() {
    let _isolated = Isolated::new().await;
    Exceptions::dont_retry::<CardDeclined>();
    let _events = EventFacade::fake();
    let driver = queue_with(vec![ChargeCard {
        order: 15,
        declined: false,
    }])
    .await;

    drain(driver.clone()).await;

    assert_eq!(
        CHARGES.load(Ordering::SeqCst),
        3,
        "a gateway timeout is not a declined card, so it runs all three tries"
    );
    assert_eq!(dispatched::<JobFailed>(|_| true).len(), 1);
}

#[tokio::test]
#[serial]
async fn a_timed_out_attempt_with_tries_left_is_retried_and_reaches_the_callbacks() {
    let _isolated = Isolated::new().await;
    let timeouts = record_timeouts();
    let _events = EventFacade::fake();
    let driver = queue_with(vec![SlowCharge { order: 21 }]).await;

    drain(driver.clone()).await;

    assert_eq!(
        SLOW_STARTS.load(Ordering::SeqCst),
        2,
        "the first timeout leaves a try, so the job runs again"
    );
    assert_eq!(
        seen(&timeouts),
        [SLOW_CHARGE_TIMED_OUT, SLOW_CHARGE_TIMED_OUT],
        "each timed-out attempt, the retried one and the last, is reported"
    );
    assert_eq!(dispatched::<JobFailed>(|_| true).len(), 1);
    assert_eq!(driver.size(None).await.expect("size"), 0);
}

#[tokio::test]
#[serial]
async fn a_timed_out_attempt_with_no_tries_left_is_dead_lettered_and_reaches_the_callbacks() {
    let _isolated = Isolated::new().await;
    let timeouts = record_timeouts();
    let _events = EventFacade::fake();
    let driver = queue_with(vec![SlowCharge { order: 22 }]).await;

    drain_with_tries(driver.clone(), Some(1)).await;

    assert_eq!(SLOW_STARTS.load(Ordering::SeqCst), 1);
    let failed = dispatched::<JobFailed>(|_| true);
    assert_eq!(failed.len(), 1, "the one timeout spends the only try");
    assert_eq!(
        seen(&timeouts),
        [SLOW_CHARGE_TIMED_OUT],
        "the timeout that dead-letters the job is reported"
    );
    assert_eq!(driver.size(None).await.expect("size"), 0);
}

#[tokio::test]
#[serial]
async fn a_timed_out_attempt_dont_retry_when_accepts_fails_without_another_attempt() {
    let _isolated = Isolated::new().await;
    Exceptions::dont_retry_when(|error: &FrameworkError| {
        error
            .external_source()
            .is_some_and(|source| source.is::<TimeoutExceeded>())
    });
    let timeouts = record_timeouts();
    let _events = EventFacade::fake();
    let driver = queue_with(vec![SlowCharge { order: 23 }]).await;

    drain(driver.clone()).await;

    assert_eq!(
        SLOW_STARTS.load(Ordering::SeqCst),
        1,
        "the job fails on its first timeout, with a try left"
    );
    assert_eq!(dispatched::<JobFailed>(|_| true).len(), 1);
    assert_eq!(seen(&timeouts), [SLOW_CHARGE_TIMED_OUT]);
    assert_eq!(driver.size(None).await.expect("size"), 0);
    assert_eq!(driver.delayed_size(None).await.expect("delayed size"), 0);
}
