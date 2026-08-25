//! Queued batch tests: dispatch, progress tracking, cancellation, callbacks.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use suprnova::App;
use suprnova::cache::{Cache, CacheStore, InMemoryCache};
use suprnova::error::FrameworkError;
use suprnova::queue::batch::register_callback;
use suprnova::queue::{
    Batch, BatchCallback, Job, MemoryBatchRepository, MemoryQueueDriver, Queue,
    worker::{WorkerConfig, register_job, run_worker},
};
use tokio_util::sync::CancellationToken;

fn cache_init() {
    if !Cache::is_initialized() {
        App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    }
}

static BATCHED_RUNS: AtomicU32 = AtomicU32::new(0);

#[derive(Serialize, Deserialize, Clone)]
struct BatchedJob {
    n: u32,
}

#[async_trait]
impl Job for BatchedJob {
    fn job_name() -> &'static str {
        "queue_batches::BatchedJob"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        BATCHED_RUNS.fetch_add(self.n, Ordering::SeqCst);
        Ok(())
    }
}

static CALLBACK_HITS: AtomicU32 = AtomicU32::new(0);

struct CountingCallback {
    name: &'static str,
}

#[async_trait]
impl BatchCallback for CountingCallback {
    fn name(&self) -> &'static str {
        self.name
    }
    async fn handle(&self, _batch: Batch, _err: Option<String>) -> Result<(), FrameworkError> {
        CALLBACK_HITS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn batch_dispatches_every_job_and_fires_then_finally() {
    cache_init();
    BATCHED_RUNS.store(0, Ordering::SeqCst);
    CALLBACK_HITS.store(0, Ordering::SeqCst);
    register_job::<BatchedJob>();
    Queue::set_batch_repository(Arc::new(MemoryBatchRepository::new()));
    register_callback(Arc::new(CountingCallback { name: "then-cb" }));
    register_callback(Arc::new(CountingCallback { name: "finally-cb" }));

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    let batch_id = Queue::batch()
        .name("test-batch")
        .add(BatchedJob { n: 1 })
        .add(BatchedJob { n: 2 })
        .add(BatchedJob { n: 4 })
        .then("then-cb")
        .finally("finally-cb")
        .dispatch()
        .await
        .unwrap();

    // Drain the queue.
    let cfg = WorkerConfig {
        visibility_timeout: Duration::from_secs(5),
        poll_interval: Duration::from_millis(5),
        max_jobs: Some(3),
        queues: Vec::new(),
    };
    let cancel = CancellationToken::new();
    run_worker(driver.clone(), cfg, cancel).await;

    assert_eq!(BATCHED_RUNS.load(Ordering::SeqCst), 7);
    let repo = Queue::batch_repository().unwrap();
    let snap = repo.find(&batch_id).await.unwrap().unwrap();
    assert_eq!(snap.pending_jobs, 0);
    assert_eq!(snap.failed_jobs, 0);
    assert!(snap.finished());
    // Both callbacks fired once.
    assert_eq!(CALLBACK_HITS.load(Ordering::SeqCst), 2);
}

#[derive(Serialize, Deserialize, Clone)]
struct FailingJob;

#[async_trait]
impl Job for FailingJob {
    fn job_name() -> &'static str {
        "queue_batches::FailingJob"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("nope"))
    }
    fn max_tries() -> u32 {
        1
    }
}

#[tokio::test]
#[serial]
async fn batch_records_failure_and_cancels_when_allow_failures_off() {
    cache_init();
    register_job::<FailingJob>();
    Queue::set_batch_repository(Arc::new(MemoryBatchRepository::new()));

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    let batch_id = Queue::batch()
        .name("fail-batch")
        .add(FailingJob)
        .dispatch()
        .await
        .unwrap();

    let cfg = WorkerConfig {
        visibility_timeout: Duration::from_secs(5),
        poll_interval: Duration::from_millis(5),
        max_jobs: Some(1),
        queues: Vec::new(),
    };
    let cancel = CancellationToken::new();
    run_worker(driver.clone(), cfg, cancel).await;

    let repo = Queue::batch_repository().unwrap();
    let snap = repo.find(&batch_id).await.unwrap().unwrap();
    assert_eq!(snap.failed_jobs, 1);
    assert!(snap.cancelled());
}

#[derive(Serialize, Deserialize, Clone)]
struct CancelAwareJob {
    n: u32,
}

#[async_trait]
impl Job for CancelAwareJob {
    fn job_name() -> &'static str {
        "queue_batches::CancelAwareJob"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        if self.n == 1 {
            Err(FrameworkError::internal("first job fails"))
        } else {
            BATCHED_RUNS.fetch_add(self.n, Ordering::SeqCst);
            Ok(())
        }
    }
    fn max_tries() -> u32 {
        1
    }
    fn middleware() -> Vec<Arc<dyn suprnova::queue::JobMiddleware>> {
        vec![Arc::new(suprnova::queue::SkipIfBatchCancelled)]
    }
}

// Lights up the M37 fix. A two-job `allow_failures` batch where the
// failing job dead-letters FIRST and the succeeding job settles LAST.
// Before the fix, `handle_completed` fired `Then` unconditionally on
// pending==0, so the late-success path declared the batch a success even
// though a prior job had failed. The correct callback is `Catch` because
// `failed_jobs > 0` at settlement time.
#[derive(Serialize, Deserialize, Clone)]
struct M37FailingThen {
    succeed: bool,
}

#[async_trait]
impl Job for M37FailingThen {
    fn job_name() -> &'static str {
        "queue_batches::M37FailingThen"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        if self.succeed {
            Ok(())
        } else {
            Err(FrameworkError::internal("planned failure"))
        }
    }
    fn max_tries() -> u32 {
        1
    }
}

struct LabeledCallback {
    name: &'static str,
    hits: Arc<AtomicU32>,
}

#[async_trait]
impl BatchCallback for LabeledCallback {
    fn name(&self) -> &'static str {
        self.name
    }
    async fn handle(&self, _batch: Batch, _err: Option<String>) -> Result<(), FrameworkError> {
        self.hits.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn allow_failures_batch_with_late_success_fires_catch_not_then() {
    cache_init();
    register_job::<M37FailingThen>();
    Queue::set_batch_repository(Arc::new(MemoryBatchRepository::new()));

    let then_hits = Arc::new(AtomicU32::new(0));
    let catch_hits = Arc::new(AtomicU32::new(0));
    let finally_hits = Arc::new(AtomicU32::new(0));
    register_callback(Arc::new(LabeledCallback {
        name: "m37-then",
        hits: then_hits.clone(),
    }));
    register_callback(Arc::new(LabeledCallback {
        name: "m37-catch",
        hits: catch_hits.clone(),
    }));
    register_callback(Arc::new(LabeledCallback {
        name: "m37-finally",
        hits: finally_hits.clone(),
    }));

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    // FIFO drain order: failing first, succeeding last. The succeeding
    // settlement drives `pending_jobs` to 0 - that's the M37 path.
    let batch_id = Queue::batch()
        .name("m37-late-success")
        .add(M37FailingThen { succeed: false })
        .add(M37FailingThen { succeed: true })
        .allow_failures()
        .then("m37-then")
        .catch("m37-catch")
        .finally("m37-finally")
        .dispatch()
        .await
        .unwrap();

    let cfg = WorkerConfig {
        visibility_timeout: Duration::from_secs(5),
        poll_interval: Duration::from_millis(5),
        max_jobs: Some(2),
        queues: Vec::new(),
    };
    let cancel = CancellationToken::new();
    run_worker(driver.clone(), cfg, cancel).await;

    let repo = Queue::batch_repository().unwrap();
    let snap = repo.find(&batch_id).await.unwrap().unwrap();
    assert_eq!(snap.failed_jobs, 1, "one job must have dead-lettered");
    assert_eq!(snap.pending_jobs, 0, "batch must have fully drained");
    assert!(snap.finished(), "batch must be marked finished");
    assert!(
        !snap.cancelled(),
        "allow_failures should keep the batch un-cancelled"
    );

    assert_eq!(
        then_hits.load(Ordering::SeqCst),
        0,
        "Then must NOT fire when a prior job failed (M37)"
    );
    assert_eq!(
        catch_hits.load(Ordering::SeqCst),
        1,
        "Catch must fire because failed_jobs > 0"
    );
    assert_eq!(
        finally_hits.load(Ordering::SeqCst),
        1,
        "Finally must always fire on settle"
    );
}

#[tokio::test]
#[serial]
async fn multi_job_batch_fires_finally_even_when_first_fails_and_rest_skipped() {
    cache_init();
    BATCHED_RUNS.store(0, Ordering::SeqCst);
    CALLBACK_HITS.store(0, Ordering::SeqCst);
    register_job::<CancelAwareJob>();
    Queue::set_batch_repository(Arc::new(MemoryBatchRepository::new()));
    register_callback(Arc::new(CountingCallback {
        name: "multi-finally",
    }));

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    let batch_id = Queue::batch()
        .name("multi-fail")
        .add(CancelAwareJob { n: 1 })
        .add(CancelAwareJob { n: 2 })
        .add(CancelAwareJob { n: 4 })
        .finally("multi-finally")
        .dispatch()
        .await
        .unwrap();

    // Drain. Job 1 dead-letters, cancels batch. Jobs 2-3 settle via
    // `SkipIfBatchCancelled` → `Deleted`.
    let cfg = WorkerConfig {
        visibility_timeout: Duration::from_secs(5),
        poll_interval: Duration::from_millis(5),
        max_jobs: Some(3),
        queues: Vec::new(),
    };
    let cancel = CancellationToken::new();
    run_worker(driver.clone(), cfg, cancel).await;

    let repo = Queue::batch_repository().unwrap();
    let snap = repo.find(&batch_id).await.unwrap().unwrap();
    assert_eq!(
        snap.pending_jobs, 0,
        "every batched job must decrement pending_jobs (cancelled or not)"
    );
    assert!(snap.finished(), "batch should be marked finished");
    // The cancelled-skip path counts as a deleted settlement, so the
    // expected runs total is only the failed job's side effects (none) -
    // no successful runs because the rest were skipped.
    assert_eq!(BATCHED_RUNS.load(Ordering::SeqCst), 0);
    // `finally` MUST fire even when the batch was cancelled mid-flight.
    assert!(
        CALLBACK_HITS.load(Ordering::SeqCst) >= 1,
        "finally callback must run after cancellation"
    );
}

// Lights up the M38 fix. A driver whose Nth push fails leaves the batch
// row half-populated; before the fix that row sat with
// `pending_jobs == total_jobs > 0` forever and callbacks never fired. The
// fixed dispatcher rolls the batch row back so the partial enqueue can't
// strand the queue.
mod m38_partial_push {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use suprnova::queue::driver::{Reservation, ReservationToken};
    use suprnova::queue::{Envelope, QueueDriver};

    /// Wraps the in-memory driver and fails `push` after `fail_after`
    /// successful calls.
    struct FailAfterDriver {
        inner: Arc<MemoryQueueDriver>,
        fail_after: usize,
        pushed: AtomicUsize,
    }

    impl FailAfterDriver {
        fn new(fail_after: usize) -> Self {
            Self {
                inner: Arc::new(MemoryQueueDriver::new()),
                fail_after,
                pushed: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl QueueDriver for FailAfterDriver {
        async fn push(&self, env: Envelope) -> Result<(), FrameworkError> {
            let n = self.pushed.fetch_add(1, Ordering::SeqCst);
            if n >= self.fail_after {
                return Err(FrameworkError::internal("simulated push failure"));
            }
            self.inner.push(env).await
        }
        async fn pop(&self, t: std::time::Duration) -> Result<Option<Reservation>, FrameworkError> {
            self.inner.pop(t).await
        }
        async fn ack(&self, t: &ReservationToken) -> Result<(), FrameworkError> {
            self.inner.ack(t).await
        }
        async fn nack(
            &self,
            t: &ReservationToken,
            d: std::time::Duration,
        ) -> Result<(), FrameworkError> {
            self.inner.nack(t, d).await
        }
        fn name(&self) -> &'static str {
            "fail-after"
        }
    }

    #[derive(Serialize, Deserialize, Clone)]
    struct M38Job;

    #[async_trait]
    impl Job for M38Job {
        fn job_name() -> &'static str {
            "queue_batches::M38Job"
        }
        async fn handle(self) -> Result<(), FrameworkError> {
            Ok(())
        }
    }
    #[tokio::test]
    #[serial]
    async fn a_partial_push_settles_the_batch_instead_of_deleting_it() {
        cache_init();
        register_job::<M38Job>();
        Queue::set_batch_repository(Arc::new(MemoryBatchRepository::new()));

        // Fail on the third push of a five-job batch.
        let driver = Arc::new(FailAfterDriver::new(2));
        Queue::set_driver(driver.clone());

        let result = Queue::batch()
            .name("m38-partial")
            .add(M38Job)
            .add(M38Job)
            .add(M38Job)
            .add(M38Job)
            .add(M38Job)
            .dispatch()
            .await;

        assert!(
            result.is_err(),
            "partial push must surface the driver error"
        );

        // `dispatch` errored, so the caller never got the id - but the two
        // envelopes that DID land carry it, which is exactly the position a
        // worker is in. That is the whole problem with deleting the batch row:
        // these envelopes are real, they are in the queue, and they are
        // stamped with a batch id.
        let first = driver
            .inner
            .pop(Duration::from_secs(60))
            .await
            .unwrap()
            .expect("the first job was pushed before the failure");
        let second = driver
            .inner
            .pop(Duration::from_secs(60))
            .await
            .unwrap()
            .expect("and so was the second");
        let batch_id = first
            .envelope
            .batch_id
            .clone()
            .expect("a batched envelope carries its batch id");

        let repo = Queue::batch_repository().unwrap();
        let batch = repo
            .find(&batch_id)
            .await
            .unwrap()
            .expect("the batch must still exist - deleting it orphans these two");

        assert_eq!(
            batch.total_jobs, 5,
            "the batch still counts what was asked for"
        );
        assert_eq!(
            batch.failed_jobs, 3,
            "the three that never reached the queue are recorded as failures"
        );
        assert_eq!(
            batch.pending_jobs, 2,
            "and only the two actually queued are still outstanding"
        );
        assert!(
            batch.cancelled(),
            "the batch is cancelled so SkipIfBatchCancelled drops the rest"
        );

        // The regression this replaces: settling the queued jobs used to
        // return `Err(batch not found)` on every delivery, forever.
        repo.record_successful_job(&batch_id, first.envelope.id)
            .await
            .expect("a job that was actually dispatched can settle");
        let last = repo
            .record_successful_job(&batch_id, second.envelope.id)
            .await
            .expect("and so can the second");
        assert_eq!(
            last.pending_jobs, 0,
            "which lets the batch reach zero and fire its terminal callbacks"
        );
    }

    /// When the very first push fails there is no worker left to drive the
    /// final settlement, so `dispatch` has to fire the terminal callbacks
    /// itself or they never run at all.
    #[tokio::test]
    #[serial]
    async fn a_batch_that_never_dispatched_still_fires_its_catch_callback() {
        cache_init();
        register_job::<M38Job>();
        Queue::set_batch_repository(Arc::new(MemoryBatchRepository::new()));
        register_callback(Arc::new(RecordingCallback("m38-catch")));
        register_callback(Arc::new(RecordingCallback("m38-finally")));
        CATCH_FIRED.store(0, Ordering::SeqCst);
        FINALLY_FIRED.store(0, Ordering::SeqCst);

        // Fail on the very first push.
        Queue::set_driver(Arc::new(FailAfterDriver::new(0)));

        let result = Queue::batch()
            .name("m38-none")
            .add(M38Job)
            .add(M38Job)
            .catch("m38-catch")
            .finally("m38-finally")
            .dispatch()
            .await;

        assert!(result.is_err());
        assert_eq!(
            CATCH_FIRED.load(Ordering::SeqCst),
            1,
            "a batch where nothing was dispatched still fires catch"
        );
        assert_eq!(
            FINALLY_FIRED.load(Ordering::SeqCst),
            1,
            "and finally, exactly once each"
        );
    }

    static CATCH_FIRED: AtomicUsize = AtomicUsize::new(0);
    static FINALLY_FIRED: AtomicUsize = AtomicUsize::new(0);

    struct RecordingCallback(&'static str);

    #[async_trait]
    impl suprnova::queue::BatchCallback for RecordingCallback {
        fn name(&self) -> &'static str {
            self.0
        }
        async fn handle(
            &self,
            _batch: suprnova::queue::Batch,
            _error: Option<String>,
        ) -> Result<(), FrameworkError> {
            if self.0 == "m38-catch" {
                CATCH_FIRED.fetch_add(1, Ordering::SeqCst);
            } else {
                FINALLY_FIRED.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }
}
