//! Every queue lifecycle event, in the order it fires, for every outcome of
//! an attempt on a worker, against Laravel 13.27.
//!
//! Each test records the whole sequence with real listeners and compares it
//! with the sequence Laravel's `Worker::process` raises for the same
//! outcome. The Laravel source each expectation comes from is named on the
//! test.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use suprnova::App;
use suprnova::FrameworkError;
use suprnova::cache::{Cache, CacheStore, InMemoryCache};
use suprnova::events::{EventFacade, Listener};
use suprnova::queue::events::{
    JobAttempted, JobDebounced, JobExceptionOccurred, JobFailed, JobProcessed, JobProcessing,
    JobQueued, JobQueueing, JobReleased, JobReleasedAfterException, JobTimedOut, UniqueJobSkipped,
};
use suprnova::queue::middleware::{JobMiddleware, Next};
use suprnova::queue::worker::{WorkerConfig, register_job, run_worker};
use suprnova::queue::{
    ChainLink, Envelope, FailOnException, Job, JobOutcome, MemoryQueueDriver, Queue, QueueDriver,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// The recorder
// ---------------------------------------------------------------------------

/// One event as it fired: its name, the job it names, and the envelope id
/// where the event carries one.
#[derive(Clone, Debug)]
struct Fired {
    event: &'static str,
    job: String,
    id: Option<Uuid>,
}

static FIRED: Mutex<Vec<Fired>> = Mutex::new(Vec::new());

fn note(event: &'static str, job: &str, id: Option<Uuid>) {
    FIRED.lock().unwrap().push(Fired {
        event,
        job: job.to_owned(),
        id,
    });
}

struct Recorder;

#[async_trait]
impl Listener<JobQueueing> for Recorder {
    async fn handle(&self, e: &JobQueueing) -> Result<(), FrameworkError> {
        note("JobQueueing", &e.job_name, None);
        Ok(())
    }
}
#[async_trait]
impl Listener<JobQueued> for Recorder {
    async fn handle(&self, e: &JobQueued) -> Result<(), FrameworkError> {
        note("JobQueued", &e.job_name, Some(e.id));
        Ok(())
    }
}
#[async_trait]
impl Listener<UniqueJobSkipped> for Recorder {
    async fn handle(&self, e: &UniqueJobSkipped) -> Result<(), FrameworkError> {
        note("UniqueJobSkipped", &e.job_name, None);
        Ok(())
    }
}
#[async_trait]
impl Listener<JobProcessing> for Recorder {
    async fn handle(&self, e: &JobProcessing) -> Result<(), FrameworkError> {
        note("JobProcessing", &e.job.job_name, Some(e.job.id));
        Ok(())
    }
}
#[async_trait]
impl Listener<JobProcessed> for Recorder {
    async fn handle(&self, e: &JobProcessed) -> Result<(), FrameworkError> {
        note("JobProcessed", &e.job.job_name, Some(e.job.id));
        Ok(())
    }
}
#[async_trait]
impl Listener<JobExceptionOccurred> for Recorder {
    async fn handle(&self, e: &JobExceptionOccurred) -> Result<(), FrameworkError> {
        note("JobExceptionOccurred", &e.job.job_name, Some(e.job.id));
        Ok(())
    }
}
#[async_trait]
impl Listener<JobFailed> for Recorder {
    async fn handle(&self, e: &JobFailed) -> Result<(), FrameworkError> {
        note("JobFailed", &e.job.job_name, Some(e.job.id));
        Ok(())
    }
}
#[async_trait]
impl Listener<JobReleased> for Recorder {
    async fn handle(&self, e: &JobReleased) -> Result<(), FrameworkError> {
        note("JobReleased", &e.job.job_name, Some(e.job.id));
        Ok(())
    }
}
#[async_trait]
impl Listener<JobReleasedAfterException> for Recorder {
    async fn handle(&self, e: &JobReleasedAfterException) -> Result<(), FrameworkError> {
        note("JobReleasedAfterException", &e.job.job_name, Some(e.job.id));
        Ok(())
    }
}
#[async_trait]
impl Listener<JobAttempted> for Recorder {
    async fn handle(&self, e: &JobAttempted) -> Result<(), FrameworkError> {
        note("JobAttempted", &e.job.job_name, Some(e.job.id));
        Ok(())
    }
}
#[async_trait]
impl Listener<JobDebounced> for Recorder {
    async fn handle(&self, e: &JobDebounced) -> Result<(), FrameworkError> {
        note("JobDebounced", &e.job.job_name, Some(e.job.id));
        Ok(())
    }
}
#[async_trait]
impl Listener<JobTimedOut> for Recorder {
    async fn handle(&self, e: &JobTimedOut) -> Result<(), FrameworkError> {
        note("JobTimedOut", &e.job.job_name, Some(e.job.id));
        Ok(())
    }
}

/// Listens to every lifecycle event until dropped.
struct Recording;

impl Drop for Recording {
    fn drop(&mut self) {
        EventFacade::forget::<JobQueueing>();
        EventFacade::forget::<JobQueued>();
        EventFacade::forget::<UniqueJobSkipped>();
        EventFacade::forget::<JobProcessing>();
        EventFacade::forget::<JobProcessed>();
        EventFacade::forget::<JobExceptionOccurred>();
        EventFacade::forget::<JobFailed>();
        EventFacade::forget::<JobReleased>();
        EventFacade::forget::<JobReleasedAfterException>();
        EventFacade::forget::<JobAttempted>();
        EventFacade::forget::<JobDebounced>();
        EventFacade::forget::<JobTimedOut>();
    }
}

async fn record() -> Recording {
    FIRED.lock().unwrap().clear();
    let r = Arc::new(Recorder);
    EventFacade::listen::<JobQueueing, _>(r.clone()).await;
    EventFacade::listen::<JobQueued, _>(r.clone()).await;
    EventFacade::listen::<UniqueJobSkipped, _>(r.clone()).await;
    EventFacade::listen::<JobProcessing, _>(r.clone()).await;
    EventFacade::listen::<JobProcessed, _>(r.clone()).await;
    EventFacade::listen::<JobExceptionOccurred, _>(r.clone()).await;
    EventFacade::listen::<JobFailed, _>(r.clone()).await;
    EventFacade::listen::<JobReleased, _>(r.clone()).await;
    EventFacade::listen::<JobReleasedAfterException, _>(r.clone()).await;
    EventFacade::listen::<JobAttempted, _>(r.clone()).await;
    EventFacade::listen::<JobDebounced, _>(r.clone()).await;
    EventFacade::listen::<JobTimedOut, _>(r).await;
    Recording
}

/// The events that named `job`, in order.
fn fired(job: &str) -> Vec<&'static str> {
    FIRED
        .lock()
        .unwrap()
        .iter()
        .filter(|fired| fired.job == job)
        .map(|fired| fired.event)
        .collect()
}

/// The events that named envelope `id`, in order.
fn fired_for(id: Uuid) -> Vec<&'static str> {
    FIRED
        .lock()
        .unwrap()
        .iter()
        .filter(|fired| fired.id == Some(id))
        .map(|fired| fired.event)
        .collect()
}

/// Run a worker on `driver` until it has settled one attempt.
async fn work_one(driver: Arc<MemoryQueueDriver>) {
    let cfg = WorkerConfig {
        visibility_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(5),
        max_jobs: Some(1),
        queues: Vec::new(),
    };
    tokio::time::timeout(
        Duration::from_secs(20),
        run_worker(driver, cfg, CancellationToken::new()),
    )
    .await
    .expect("the worker settled one attempt");
}

fn worker() -> Arc<MemoryQueueDriver> {
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    driver
}

fn cache_init() {
    if !Cache::is_initialized() {
        App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    }
}

// ---------------------------------------------------------------------------
// Jobs and middleware
// ---------------------------------------------------------------------------

macro_rules! job {
    ($ty:ident, $name:literal, { $($body:tt)* }) => {
        #[derive(Serialize, Deserialize, Clone)]
        struct $ty;

        #[async_trait]
        impl Job for $ty {
            fn job_name() -> &'static str {
                $name
            }
            $($body)*
        }
    };
}

/// Settles every attempt with `outcome`, without running the handler.
struct Settle(fn() -> JobOutcome);

#[async_trait]
impl JobMiddleware for Settle {
    async fn handle(&self, _env: Envelope, _next: Next) -> Result<JobOutcome, FrameworkError> {
        Ok((self.0)())
    }
}

fn release() -> JobOutcome {
    JobOutcome::Released {
        delay: Duration::from_secs(60),
    }
}
fn delete() -> JobOutcome {
    JobOutcome::Deleted
}
fn fail() -> JobOutcome {
    JobOutcome::Failed {
        reason: "the middleware failed the job".into(),
    }
}

fn boom() -> Result<(), FrameworkError> {
    Err(FrameworkError::internal("boom"))
}

job!(Succeeds, "lifecycle::Succeeds", {
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
});
job!(FailsWithRetriesLeft, "lifecycle::FailsWithRetriesLeft", {
    fn max_tries() -> u32 {
        2
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        boom()
    }
});
job!(FailsOnItsLastAttempt, "lifecycle::FailsOnItsLastAttempt", {
    fn max_tries() -> u32 {
        1
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        boom()
    }
});
job!(Panics, "lifecycle::Panics", {
    fn max_tries() -> u32 {
        1
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        panic!("the handler panicked");
    }
});
job!(
    TimesOutWithRetriesLeft,
    "lifecycle::TimesOutWithRetriesLeft",
    {
        fn max_tries() -> u32 {
            2
        }
        fn timeout() -> Option<Duration> {
            Some(Duration::from_secs(1))
        }
        async fn handle(self) -> Result<(), FrameworkError> {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(())
        }
    }
);
job!(TimesOutAndFails, "lifecycle::TimesOutAndFails", {
    fn max_tries() -> u32 {
        3
    }
    fn timeout() -> Option<Duration> {
        Some(Duration::from_secs(1))
    }
    fn fail_on_timeout() -> bool {
        true
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        tokio::time::sleep(Duration::from_secs(30)).await;
        Ok(())
    }
});
job!(ReleasedByMiddleware, "lifecycle::ReleasedByMiddleware", {
    fn middleware() -> Vec<Arc<dyn JobMiddleware>> {
        vec![Arc::new(Settle(release))]
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
});
job!(DeletedByMiddleware, "lifecycle::DeletedByMiddleware", {
    fn middleware() -> Vec<Arc<dyn JobMiddleware>> {
        vec![Arc::new(Settle(delete))]
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
});
job!(FailedByMiddleware, "lifecycle::FailedByMiddleware", {
    fn middleware() -> Vec<Arc<dyn JobMiddleware>> {
        vec![Arc::new(Settle(fail))]
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
});
job!(FailedOnException, "lifecycle::FailedOnException", {
    fn max_tries() -> u32 {
        3
    }
    fn middleware() -> Vec<Arc<dyn JobMiddleware>> {
        vec![Arc::new(FailOnException::new(|_| true))]
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        boom()
    }
});
job!(Debounced, "lifecycle::Debounced", {
    fn debounce_for() -> Option<Duration> {
        Some(Duration::from_millis(50))
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
});
job!(OutOfAttempts, "lifecycle::OutOfAttempts", {
    fn max_tries() -> u32 {
        1
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
});

// ---------------------------------------------------------------------------
// The worker
// ---------------------------------------------------------------------------

/// `Queue::enqueueUsing` (Queue.php:385-388), then `Worker::process`:
/// JobProcessing (Worker.php:580), JobProcessed (:595), JobAttempted (:607).
#[tokio::test]
#[serial]
async fn worker_success() {
    register_job::<Succeeds>();
    let driver = worker();
    let _recording = record().await;
    Queue::push(Succeeds).await.unwrap();
    work_one(driver).await;
    assert_eq!(
        fired("lifecycle::Succeeds"),
        [
            "JobQueueing",
            "JobQueued",
            "JobProcessing",
            "JobProcessed",
            "JobAttempted"
        ]
    );
}

/// `handleJobException`: JobExceptionOccurred (Worker.php:644), the release
/// and JobReleasedAfterException (:653-656) in its `finally`, then
/// JobAttempted in `process`'s `finally` (:607).
#[tokio::test]
#[serial]
async fn worker_retry_after_an_error() {
    register_job::<FailsWithRetriesLeft>();
    let driver = worker();
    let _recording = record().await;
    Queue::push(FailsWithRetriesLeft).await.unwrap();
    work_one(driver).await;
    assert_eq!(
        fired("lifecycle::FailsWithRetriesLeft"),
        [
            "JobQueueing",
            "JobQueued",
            "JobProcessing",
            "JobExceptionOccurred",
            "JobReleasedAfterException",
            "JobAttempted"
        ]
    );
}

/// `markJobAsFailedIfWillExceedMaxAttempts` fails the job first, JobFailed
/// from `Job::fail` (Jobs/Job.php:221), then JobExceptionOccurred
/// (Worker.php:644); no release for a failed job; JobAttempted (:607).
#[tokio::test]
#[serial]
async fn worker_error_on_the_last_attempt() {
    register_job::<FailsOnItsLastAttempt>();
    let driver = worker();
    let _recording = record().await;
    Queue::push(FailsOnItsLastAttempt).await.unwrap();
    work_one(driver).await;
    assert_eq!(
        fired("lifecycle::FailsOnItsLastAttempt"),
        [
            "JobQueueing",
            "JobQueued",
            "JobProcessing",
            "JobFailed",
            "JobExceptionOccurred",
            "JobAttempted"
        ]
    );
}

/// A panic is a thrown error to the worker: the same sequence as an error on
/// the last attempt.
#[tokio::test]
#[serial]
async fn worker_panic() {
    register_job::<Panics>();
    let driver = worker();
    let _recording = record().await;
    Queue::push(Panics).await.unwrap();
    work_one(driver).await;
    assert_eq!(
        fired("lifecycle::Panics"),
        [
            "JobQueueing",
            "JobQueued",
            "JobProcessing",
            "JobFailed",
            "JobExceptionOccurred",
            "JobAttempted"
        ]
    );
}

/// The alarm handler (Worker.php:307-326): nothing fails the job, so only
/// JobTimedOut (:321), and the worker process is killed before `process`'s
/// `finally` runs, so no JobAttempted.
#[tokio::test]
#[serial]
async fn worker_timeout_with_attempts_left() {
    register_job::<TimesOutWithRetriesLeft>();
    let driver = worker();
    let _recording = record().await;
    Queue::push(TimesOutWithRetriesLeft).await.unwrap();
    work_one(driver).await;
    assert_eq!(
        fired("lifecycle::TimesOutWithRetriesLeft"),
        ["JobQueueing", "JobQueued", "JobProcessing", "JobTimedOut"]
    );
}

/// The alarm handler with `failOnTimeout`: `markJobAsFailedIfItShouldFailOnTimeout`
/// (Worker.php:317) fails the job, JobFailed, before JobTimedOut (:321); then
/// the kill, so no JobAttempted.
#[tokio::test]
#[serial]
async fn worker_timeout_that_fails_the_job() {
    register_job::<TimesOutAndFails>();
    let driver = worker();
    let _recording = record().await;
    Queue::push(TimesOutAndFails).await.unwrap();
    work_one(driver).await;
    assert_eq!(
        fired("lifecycle::TimesOutAndFails"),
        [
            "JobQueueing",
            "JobQueued",
            "JobProcessing",
            "JobFailed",
            "JobTimedOut"
        ]
    );
}

/// The pipeline returns: JobProcessed (Worker.php:595), JobReleased because
/// the job was released (:597-601), JobAttempted (:607).
#[tokio::test]
#[serial]
async fn worker_release_from_middleware() {
    register_job::<ReleasedByMiddleware>();
    let driver = worker();
    let _recording = record().await;
    Queue::push(ReleasedByMiddleware).await.unwrap();
    work_one(driver).await;
    assert_eq!(
        fired("lifecycle::ReleasedByMiddleware"),
        [
            "JobQueueing",
            "JobQueued",
            "JobProcessing",
            "JobProcessed",
            "JobReleased",
            "JobAttempted"
        ]
    );
}

/// The pipeline returns: JobProcessed (Worker.php:595), JobAttempted (:607).
#[tokio::test]
#[serial]
async fn worker_delete_from_middleware() {
    register_job::<DeletedByMiddleware>();
    let driver = worker();
    let _recording = record().await;
    Queue::push(DeletedByMiddleware).await.unwrap();
    work_one(driver).await;
    assert_eq!(
        fired("lifecycle::DeletedByMiddleware"),
        [
            "JobQueueing",
            "JobQueued",
            "JobProcessing",
            "JobProcessed",
            "JobAttempted"
        ]
    );
}

/// `$job->fail()` inside the pipeline raises JobFailed (Jobs/Job.php:221);
/// the pipeline then returns, so JobProcessed (Worker.php:595) and
/// JobAttempted (:607) follow.
#[tokio::test]
#[serial]
async fn worker_fail_from_middleware_without_an_error() {
    register_job::<FailedByMiddleware>();
    let driver = worker();
    let _recording = record().await;
    Queue::push(FailedByMiddleware).await.unwrap();
    work_one(driver).await;
    assert_eq!(
        fired("lifecycle::FailedByMiddleware"),
        [
            "JobQueueing",
            "JobQueued",
            "JobProcessing",
            "JobFailed",
            "JobProcessed",
            "JobAttempted"
        ]
    );
}

/// `FailOnException` fails the job, JobFailed, and rethrows
/// (Middleware/FailOnException.php:59-62): `handleJobException` then raises
/// JobExceptionOccurred (Worker.php:644), releases nothing for a failed job,
/// and JobAttempted follows (:607).
#[tokio::test]
#[serial]
async fn worker_fail_on_exception() {
    register_job::<FailedOnException>();
    let driver = worker();
    let _recording = record().await;
    Queue::push(FailedOnException).await.unwrap();
    work_one(driver).await;
    assert_eq!(
        fired("lifecycle::FailedOnException"),
        [
            "JobQueueing",
            "JobQueued",
            "JobProcessing",
            "JobFailed",
            "JobExceptionOccurred",
            "JobAttempted"
        ]
    );
}

/// `CallQueuedHandler::call` deletes a superseded job, raising JobDebounced
/// (CallQueuedHandler.php:296), and returns: JobProcessed (Worker.php:595),
/// JobAttempted (:607).
#[tokio::test]
#[serial]
async fn worker_superseded_drop() {
    cache_init();
    register_job::<Debounced>();
    let driver = worker();
    let _recording = record().await;
    Queue::push(Debounced).await.unwrap();
    Queue::push(Debounced).await.unwrap();
    let first = FIRED
        .lock()
        .unwrap()
        .iter()
        .find(|fired| fired.event == "JobQueued")
        .and_then(|fired| fired.id)
        .expect("the first dispatch was queued");
    let cfg = WorkerConfig {
        visibility_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(5),
        max_jobs: Some(2),
        queues: Vec::new(),
    };
    tokio::time::timeout(
        Duration::from_secs(20),
        run_worker(driver, cfg, CancellationToken::new()),
    )
    .await
    .expect("the worker settled both envelopes");
    assert_eq!(
        fired_for(first),
        [
            "JobQueued",
            "JobProcessing",
            "JobDebounced",
            "JobProcessed",
            "JobAttempted"
        ]
    );
}

/// `process` raises JobProcessing (Worker.php:580) before
/// `markJobAsFailedIfAlreadyExceedsMaxAttempts` (:582) fails the job,
/// JobFailed, and throws; `handleJobException` raises JobExceptionOccurred
/// (:644) and JobAttempted follows (:607).
#[tokio::test]
#[serial]
async fn worker_max_attempts_exceeded() {
    register_job::<OutOfAttempts>();
    let driver = worker();
    let _recording = record().await;
    let mut env = ChainLink::from_job(OutOfAttempts).unwrap().to_envelope();
    env.max_tries = 1;
    env.attempts = 1;
    driver.push(env).await.unwrap();
    work_one(driver).await;
    assert_eq!(
        fired("lifecycle::OutOfAttempts"),
        [
            "JobProcessing",
            "JobFailed",
            "JobExceptionOccurred",
            "JobAttempted"
        ]
    );
}
