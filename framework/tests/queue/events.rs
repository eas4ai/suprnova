//! Worker emits lifecycle events through Event::dispatch.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use suprnova::error::FrameworkError;
use suprnova::events::dispatched;
use suprnova::events::{EventFacade, Listener};
use suprnova::queue::events::{JobAttempted, JobFailed, JobTimedOut};
use suprnova::queue::events::{JobProcessed, JobProcessing, JobQueued, WorkerStarting};
use suprnova::queue::{
    Job, MemoryQueueDriver, Queue, QueueDriver,
    worker::{WorkerConfig, register_job, run_worker},
};
use tokio_util::sync::CancellationToken;

static EV_QUEUED: AtomicU32 = AtomicU32::new(0);
static EV_PROCESSING: AtomicU32 = AtomicU32::new(0);
static EV_PROCESSED: AtomicU32 = AtomicU32::new(0);
static EV_STARTING: AtomicU32 = AtomicU32::new(0);

struct CountQueued;
#[async_trait]
impl Listener<JobQueued> for CountQueued {
    async fn handle(&self, _e: &JobQueued) -> Result<(), FrameworkError> {
        EV_QUEUED.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
struct CountProcessing;
#[async_trait]
impl Listener<JobProcessing> for CountProcessing {
    async fn handle(&self, _e: &JobProcessing) -> Result<(), FrameworkError> {
        EV_PROCESSING.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
struct CountProcessed;
#[async_trait]
impl Listener<JobProcessed> for CountProcessed {
    async fn handle(&self, _e: &JobProcessed) -> Result<(), FrameworkError> {
        EV_PROCESSED.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
struct CountStarting;
#[async_trait]
impl Listener<WorkerStarting> for CountStarting {
    async fn handle(&self, _e: &WorkerStarting) -> Result<(), FrameworkError> {
        EV_STARTING.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct EventJob;

#[async_trait]
impl Job for EventJob {
    fn job_name() -> &'static str {
        "queue_events::EventJob"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn worker_emits_lifecycle_events() {
    EV_QUEUED.store(0, Ordering::SeqCst);
    EV_PROCESSING.store(0, Ordering::SeqCst);
    EV_PROCESSED.store(0, Ordering::SeqCst);
    EV_STARTING.store(0, Ordering::SeqCst);

    register_job::<EventJob>();
    EventFacade::listen::<JobQueued, _>(Arc::new(CountQueued)).await;
    EventFacade::listen::<JobProcessing, _>(Arc::new(CountProcessing)).await;
    EventFacade::listen::<JobProcessed, _>(Arc::new(CountProcessed)).await;
    EventFacade::listen::<WorkerStarting, _>(Arc::new(CountStarting)).await;

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::push(EventJob).await.unwrap();

    let cfg = WorkerConfig {
        visibility_timeout: Duration::from_secs(5),
        poll_interval: Duration::from_millis(5),
        max_jobs: Some(1),
        queues: Vec::new(),
    };
    let cancel = CancellationToken::new();
    run_worker(driver, cfg, cancel).await;

    assert_eq!(EV_QUEUED.load(Ordering::SeqCst), 1);
    assert_eq!(EV_PROCESSING.load(Ordering::SeqCst), 1);
    assert_eq!(EV_PROCESSED.load(Ordering::SeqCst), 1);
    assert_eq!(EV_STARTING.load(Ordering::SeqCst), 1);
}

// ---- JobTimedOut carries the budget it blew (Laravel 13.25 #61060) --------
//
// The `timeout` field has existed since the event was added and was
// never asserted anywhere. Laravel added the same field in 13.25; this
// pins ours so a refactor of the worker's timeout plumbing can't quietly
// drop it.

#[derive(Serialize, Deserialize, Clone)]
struct SlowJob;

#[async_trait]
impl Job for SlowJob {
    fn job_name() -> &'static str {
        "queue_events::SlowJob"
    }
    fn timeout() -> Option<Duration> {
        // Whole seconds: the envelope stores `timeout_secs`, so a
        // sub-second budget would round to zero and prove nothing.
        Some(Duration::from_secs(1))
    }
    fn fail_on_timeout() -> bool {
        // Dead-letter on the first timeout so the worker settles once
        // and the test doesn't pay for a retry.
        true
    }
    fn max_tries() -> u32 {
        1
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        tokio::time::sleep(Duration::from_secs(30)).await;
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn job_timed_out_event_carries_the_jobs_timeout_budget() {
    register_job::<SlowJob>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    let _events = EventFacade::fake();
    Queue::push(SlowJob).await.unwrap();

    let cfg = WorkerConfig {
        visibility_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(5),
        max_jobs: Some(1),
        queues: Vec::new(),
    };
    run_worker(driver, cfg, CancellationToken::new()).await;

    let timed_out = dispatched::<JobTimedOut>(|_| true);
    assert_eq!(timed_out.len(), 1, "one settlement, one JobTimedOut");
    assert_eq!(
        timed_out[0].timeout,
        Duration::from_secs(1),
        "the event must report the budget the job declared, not a default"
    );
    assert_eq!(timed_out[0].job.job_name, "queue_events::SlowJob");
    // DRIVERS-054: a timed-out attempt that settles terminally is an attempt.
    let attempted = dispatched::<JobAttempted>(|_| true);
    assert_eq!(
        attempted.len(),
        1,
        "a dead-lettered timeout must fire JobAttempted"
    );
    assert_eq!(attempted[0].job.job_name, "queue_events::SlowJob");
}

// ---- JobAttempted fires for every terminal settlement (DRIVERS-054) -------

#[derive(Serialize, Deserialize, Clone)]
struct AlwaysFailsJob;

#[async_trait]
impl Job for AlwaysFailsJob {
    fn job_name() -> &'static str {
        "queue_events::AlwaysFailsJob"
    }
    fn max_tries() -> u32 {
        1
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("this job always fails"))
    }
}

#[tokio::test]
#[serial]
async fn job_attempted_fires_when_a_job_fails_terminally() {
    register_job::<AlwaysFailsJob>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    let _events = EventFacade::fake();
    Queue::push(AlwaysFailsJob).await.unwrap();

    let cfg = WorkerConfig {
        visibility_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(5),
        max_jobs: Some(1),
        queues: Vec::new(),
    };
    run_worker(driver, cfg, CancellationToken::new()).await;

    assert_eq!(dispatched::<JobFailed>(|_| true).len(), 1, "dead-lettered");
    let attempted = dispatched::<JobAttempted>(|_| true);
    assert_eq!(
        attempted.len(),
        1,
        "a terminal failure is a settled attempt and must fire JobAttempted"
    );
    assert_eq!(attempted[0].job.job_name, "queue_events::AlwaysFailsJob");
}

/// Settles every attempt as deleted, without running the handler.
struct DropTheJob;

#[async_trait]
impl suprnova::queue::middleware::JobMiddleware for DropTheJob {
    async fn handle(
        &self,
        _env: suprnova::queue::Envelope,
        _next: suprnova::queue::middleware::Next,
    ) -> Result<suprnova::queue::JobOutcome, FrameworkError> {
        Ok(suprnova::queue::JobOutcome::Deleted)
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct DeletedByMiddlewareJob;

#[async_trait]
impl Job for DeletedByMiddlewareJob {
    fn job_name() -> &'static str {
        "queue_events::DeletedByMiddlewareJob"
    }
    fn middleware() -> Vec<Arc<dyn suprnova::queue::middleware::JobMiddleware>> {
        vec![Arc::new(DropTheJob)]
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// Sol review of DRIVERS-054: a job that middleware deletes is acknowledged
/// and gone, a terminal settlement like any other, but the worker logged it
/// without firing JobAttempted.
#[tokio::test]
#[serial]
async fn job_attempted_fires_when_middleware_deletes_the_job() {
    register_job::<DeletedByMiddlewareJob>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    let _events = EventFacade::fake();
    Queue::push(DeletedByMiddlewareJob).await.unwrap();

    let cfg = WorkerConfig {
        visibility_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(5),
        max_jobs: Some(1),
        queues: Vec::new(),
    };
    run_worker(driver.clone(), cfg, CancellationToken::new()).await;

    assert_eq!(driver.size().await.unwrap(), 0, "the deleted job is gone");
    let attempted = dispatched::<JobAttempted>(|_| true);
    assert_eq!(
        attempted.len(),
        1,
        "a deletion is a settled attempt and must fire JobAttempted"
    );
    assert_eq!(
        attempted[0].job.job_name,
        "queue_events::DeletedByMiddlewareJob"
    );
}
