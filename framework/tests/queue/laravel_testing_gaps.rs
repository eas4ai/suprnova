//! Laravel testing gaps in the queue fake: a captured payload that does not
//! decode is reported, and the fake records the jobs a worker reserves from
//! it (PAR-179).
//!
//! Laravel evidence: `Support/Testing/Fakes/QueueFake.php`
//! (`serializeAndRestore` surfaces a job that does not restore, `reserve`
//! records a reservation, and `reservedJobs` / `allReservedJobs` answer from
//! those records rather than from a real queue).

use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use suprnova::queue::testing::{pushed, reserved, try_pushed, try_reserved};
use suprnova::queue::worker::{WorkerConfig, register_job};
use suprnova::queue::{
    BackoffSchedule, CURRENT_SCHEMA_VERSION, Envelope, MemoryQueueDriver, Queue, QueueDriver,
};
use suprnova::{FrameworkError, Job, WorkerControls, async_trait, run_worker_with_controls};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Invoice {
    id: u32,
}

#[async_trait]
impl Job for Invoice {
    fn job_name() -> &'static str {
        "laravel-testing-gaps-invoice"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// A job on a queue of its own, so a listing filtered by queue can be seen
/// to leave the others out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Report {
    id: u32,
}

#[async_trait]
impl Job for Report {
    fn job_name() -> &'static str {
        "laravel-testing-gaps-report"
    }
    fn queue() -> Option<&'static str> {
        Some("reports")
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// A job whose payload never decodes: `secret` is left out of the serialized
/// form but is required to deserialize it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Lossy {
    id: u32,
    #[serde(skip_serializing)]
    secret: String,
}

#[async_trait]
impl Job for Lossy {
    fn job_name() -> &'static str {
        "laravel-testing-gaps-lossy"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        if self.secret.is_empty() {
            return Err(FrameworkError::internal("the job needs its secret"));
        }
        Ok(())
    }
}

/// How many times [`FailsOnce`] has run.
static FAILS_ONCE_RUNS: AtomicU32 = AtomicU32::new(0);

/// Fails its first run and succeeds on the second, with no delay between.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct FailsOnce {
    id: u32,
}

#[async_trait]
impl Job for FailsOnce {
    fn job_name() -> &'static str {
        "laravel-testing-gaps-fails-once"
    }
    fn max_tries() -> u32 {
        2
    }
    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Fixed { secs: 0 }
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        if FAILS_ONCE_RUNS.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(FrameworkError::internal("the first run fails"));
        }
        Ok(())
    }
}

fn lossy() -> Lossy {
    Lossy {
        id: 1,
        secret: "s".into(),
    }
}

/// The message of the panic `result` holds. Fails the test when `result` did
/// not panic.
fn panic_message<T>(result: std::thread::Result<T>) -> String {
    let payload = match result {
        Ok(_) => panic!("expected the call to panic"),
        Err(payload) => payload,
    };
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_owned();
    }
    String::from("<a panic without a message>")
}

fn fast_worker() -> WorkerConfig {
    WorkerConfig {
        poll_interval: Duration::from_millis(5),
        ..WorkerConfig::default()
    }
}

/// Run a worker over `driver` until it finds the queue empty.
async fn drain(driver: Arc<dyn QueueDriver>) {
    let status = run_worker_with_controls(
        driver,
        fast_worker(),
        WorkerControls {
            stop_when_empty: true,
            ..WorkerControls::default()
        },
        CancellationToken::new(),
    )
    .await
    .expect("the worker runs");
    assert_eq!(status, 0, "the worker stops normally");
}

// ---- PAR-179: a payload that does not decode ----

#[tokio::test]
#[serial]
async fn pushed_fails_naming_the_job_and_the_decode_error() {
    let _fake = Queue::fake();
    Queue::push(lossy()).await.unwrap();

    let message = panic_message(std::panic::catch_unwind(pushed::<Lossy>));
    assert!(
        message.contains("laravel-testing-gaps-lossy"),
        "names the job: {message}"
    );
    assert!(
        message.contains("secret"),
        "names the decode error: {message}"
    );
}

#[tokio::test]
#[serial]
async fn try_pushed_returns_the_decode_error() {
    let _fake = Queue::fake();
    Queue::push(lossy()).await.unwrap();

    let error = try_pushed::<Lossy>()
        .expect_err("a payload that does not decode is an error")
        .to_string();
    assert!(error.contains("laravel-testing-gaps-lossy"), "{error}");
    assert!(error.contains("secret"), "{error}");
}

#[tokio::test]
#[serial]
async fn try_pushed_returns_every_push_that_decodes() {
    let _fake = Queue::fake();
    assert!(try_pushed::<Invoice>().unwrap().is_empty());

    Queue::push(Invoice { id: 1 }).await.unwrap();
    Queue::push(Invoice { id: 2 }).await.unwrap();

    let expected = vec![Invoice { id: 1 }, Invoice { id: 2 }];
    assert_eq!(try_pushed::<Invoice>().unwrap(), expected);
    assert_eq!(pushed::<Invoice>(), expected);
}

// ---- PAR-179: the jobs a worker reserves from the fake ----

#[tokio::test]
#[serial]
async fn a_worker_reserves_from_the_fake_and_reserved_jobs_answers_its_record() {
    register_job::<Invoice>();
    // A real driver holding a reservation of its own: under the fake,
    // `Queue::reserved_jobs` must not read it.
    let real = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(real.clone());
    real.push(report_envelope()).await.unwrap();
    real.pop(Duration::from_secs(60))
        .await
        .unwrap()
        .expect("the real driver reserves its envelope");

    let fake = Queue::fake();
    Queue::push(Invoice { id: 7 }).await.unwrap();
    drain(fake.driver()).await;

    let records = Queue::reserved_jobs(None).await.unwrap();
    assert_eq!(records.len(), 1, "one reservation: {records:?}");
    assert_eq!(records[0].name, "laravel-testing-gaps-invoice");
    assert_eq!(records[0].payload["id"], 7);
    assert_eq!(reserved::<Invoice>(), vec![Invoice { id: 7 }]);
    assert_eq!(try_reserved::<Invoice>().unwrap(), vec![Invoice { id: 7 }]);
    assert_eq!(
        pushed::<Invoice>(),
        vec![Invoice { id: 7 }],
        "the push record stays after the worker ran the job"
    );
}

#[tokio::test]
#[serial]
async fn reserved_jobs_under_the_fake_filters_by_queue() {
    let fake = Queue::fake();
    Queue::push(Invoice { id: 1 }).await.unwrap();
    Queue::push(Report { id: 2 }).await.unwrap();
    let driver = fake.driver();
    driver.pop(Duration::from_secs(60)).await.unwrap().unwrap();
    driver.pop(Duration::from_secs(60)).await.unwrap().unwrap();

    let reports = Queue::reserved_jobs(Some("reports")).await.unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].name, "laravel-testing-gaps-report");
    let defaults = Queue::reserved_jobs(Some("default")).await.unwrap();
    assert_eq!(defaults.len(), 1);
    assert_eq!(defaults[0].name, "laravel-testing-gaps-invoice");
    assert_eq!(Queue::reserved_jobs(None).await.unwrap().len(), 2);
    assert_eq!(reserved::<Report>(), vec![Report { id: 2 }]);
}

#[tokio::test]
#[serial]
async fn a_worker_on_the_fake_claims_only_the_queues_it_names() {
    let fake = Queue::fake();
    Queue::push(Invoice { id: 1 }).await.unwrap();
    Queue::push(Report { id: 2 }).await.unwrap();

    let only_reports = fake
        .driver()
        .pop_from(Duration::from_secs(60), &["reports".to_owned()])
        .await
        .unwrap()
        .expect("the report is claimable");
    assert_eq!(
        only_reports.envelope.job_name,
        "laravel-testing-gaps-report"
    );
    assert!(
        fake.driver()
            .pop_from(Duration::from_secs(60), &["reports".to_owned()])
            .await
            .unwrap()
            .is_none(),
        "a reserved job is not claimed twice, and the invoice is not on `reports`"
    );
    assert!(reserved::<Invoice>().is_empty());
}

#[tokio::test]
#[serial]
async fn a_delayed_push_is_not_reserved_before_it_is_due() {
    let fake = Queue::fake();
    Queue::later(Duration::from_secs(3600), Invoice { id: 1 })
        .await
        .unwrap();

    assert!(
        fake.driver()
            .pop(Duration::from_secs(60))
            .await
            .unwrap()
            .is_none()
    );
    assert!(Queue::reserved_jobs(None).await.unwrap().is_empty());
    assert!(reserved::<Invoice>().is_empty());
}

#[tokio::test]
#[serial]
async fn a_job_the_worker_retries_is_reserved_again() {
    register_job::<FailsOnce>();
    FAILS_ONCE_RUNS.store(0, Ordering::SeqCst);
    let fake = Queue::fake();
    Queue::push(FailsOnce { id: 3 }).await.unwrap();
    drain(fake.driver()).await;

    assert_eq!(FAILS_ONCE_RUNS.load(Ordering::SeqCst), 2, "ran twice");
    let records = Queue::reserved_jobs(None).await.unwrap();
    assert_eq!(records.len(), 2, "one record per reservation: {records:?}");
    assert_eq!(records[0].attempts, 0);
    assert_eq!(
        records[1].attempts, 1,
        "the retry carries the spent attempt"
    );
    assert_eq!(
        reserved::<FailsOnce>(),
        vec![FailsOnce { id: 3 }, FailsOnce { id: 3 }]
    );
}

#[tokio::test]
#[serial]
async fn reserved_fails_naming_the_job_when_a_reserved_payload_does_not_decode() {
    let fake = Queue::fake();
    Queue::push(lossy()).await.unwrap();
    fake.driver()
        .pop(Duration::from_secs(60))
        .await
        .unwrap()
        .expect("the fake reserves the push");

    let error = try_reserved::<Lossy>()
        .expect_err("a payload that does not decode is an error")
        .to_string();
    assert!(error.contains("laravel-testing-gaps-lossy"), "{error}");
    assert!(error.contains("secret"), "{error}");

    let message = panic_message(std::panic::catch_unwind(reserved::<Lossy>));
    assert!(message.contains("laravel-testing-gaps-lossy"), "{message}");
    assert!(message.contains("secret"), "{message}");
}

#[tokio::test]
#[serial]
async fn the_fake_driver_refuses_once_the_fake_is_gone() {
    let fake = Queue::fake();
    let driver = fake.driver();
    drop(fake);

    let error = driver
        .pop(Duration::from_secs(60))
        .await
        .expect_err("no fake, nothing to reserve from");
    assert!(error.to_string().contains("Queue::fake()"), "{error}");
}

#[tokio::test]
#[serial]
async fn a_driver_from_a_dropped_guard_never_touches_a_later_fake() {
    let first_fake = Queue::fake();
    let old_driver = first_fake.driver();
    drop(first_fake);

    let second_fake = Queue::fake();
    Queue::push(Invoice { id: 4 }).await.unwrap();

    let error = old_driver
        .pop(Duration::from_secs(60))
        .await
        .expect_err("a driver from a dropped guard is inactive");
    assert!(error.to_string().contains("Queue::fake()"), "{error}");
    assert!(
        old_driver
            .push(report_envelope())
            .await
            .expect_err("a push through a dropped guard's driver is refused")
            .to_string()
            .contains("Queue::fake()")
    );
    assert!(
        Queue::reserved_jobs(None).await.unwrap().is_empty(),
        "the later fake's jobs stay unreserved"
    );
    assert!(reserved::<Invoice>().is_empty());
    assert_eq!(
        pushed::<Report>(),
        Vec::<Report>::new(),
        "a push through the old driver is not recorded by the later fake"
    );

    second_fake
        .driver()
        .pop(Duration::from_secs(60))
        .await
        .unwrap()
        .expect("the later fake's own driver reserves its job");
    assert_eq!(reserved::<Invoice>(), vec![Invoice { id: 4 }]);
}

/// An envelope for the real driver, of a job the fake never sees.
fn report_envelope() -> Envelope {
    Envelope {
        schema_version: CURRENT_SCHEMA_VERSION,
        id: Uuid::new_v4(),
        job_name: "laravel-testing-gaps-report".into(),
        queue: Some("reports".into()),
        payload: serde_json::json!({ "id": 9 }),
        dispatched_at: chrono::Utc::now(),
        available_at: chrono::Utc::now(),
        attempts: 0,
        max_tries: 3,
        backoff: BackoffSchedule::default(),
        timeout_secs: None,
        fail_on_timeout: false,
        idempotency_key: None,
        message_group: None,
        deduplication_id: None,
        unique_lock_owner: None,
        debounce_id: None,
        debounce_owner: None,
        batch_id: None,
        chain_remaining: Vec::new(),
        context: None,
    }
}
