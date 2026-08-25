//! `ShouldBeUniqueUntilProcessing` parity: the uniqueness lock is released when
//! processing begins (after the job's middleware pass, before the handler runs),
//! so a long-running job stops blocking re-dispatch the moment it starts
//! executing.
//!
//! Laravel evidence: `Queue/CallQueuedHandler.php:133-159` (release in the
//! pipeline's `->then(...)`, with a `finally` sweep for middleware
//! short-circuits) and `Bus/UniqueLock.php:37-105` (acquire records the lock
//! owner; release is owner-scoped).

use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use suprnova::App;
use suprnova::cache::{CacheStore, InMemoryCache};
use suprnova::idempotency::Idempotency;
use suprnova::queue::driver::QueueDriver;
use suprnova::queue::memory::MemoryQueueDriver;
use suprnova::queue::worker::{WorkerConfig, register_job, run_worker};
use suprnova::queue::{
    Envelope, JobMiddleware, JobMiddlewareNext, JobOutcome, MemoryFailedJobStore, Queue,
};
use suprnova::{FrameworkError, Job, async_trait};
use tokio_util::sync::CancellationToken;

static HANDLED: AtomicUsize = AtomicUsize::new(0);
static RELEASED_HANDLED: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UntilProcessingJob {
    key: String,
}

#[async_trait]
impl Job for UntilProcessingJob {
    fn job_name() -> &'static str {
        "wave5-until-processing"
    }
    fn unique_id(&self) -> Option<String> {
        Some(self.key.clone())
    }
    fn unique_until_processing() -> bool {
        true
    }
    fn unique_for() -> Duration {
        Duration::from_secs(300)
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        HANDLED.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn install_cache() {
    App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
}

fn unique_key(job_name: &str, id: &str) -> String {
    format!("queue-unique:{job_name}:{id}")
}

/// Run the worker until exactly one envelope settles.
///
/// The timeout turns a regression that stops the envelope being popped into a
/// failing test rather than a suite that hangs forever.
async fn work_one(driver: Arc<MemoryQueueDriver>) {
    tokio::time::timeout(
        Duration::from_secs(15),
        run_worker(
            driver,
            WorkerConfig {
                max_jobs: Some(1),
                poll_interval: Duration::from_millis(10),
                ..WorkerConfig::default()
            },
            CancellationToken::new(),
        ),
    )
    .await
    .expect("worker did not settle a job within 15s");
}

#[tokio::test]
#[serial]
async fn push_unique_stamps_the_lock_owner_on_the_envelope() {
    install_cache();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    let pushed = Queue::push_unique(UntilProcessingJob {
        key: "owner".into(),
    })
    .await
    .expect("push_unique");
    assert!(pushed, "first push must win the lock");

    let res = driver
        .pop(Duration::from_secs(60))
        .await
        .expect("pop")
        .expect("a reservation");
    let owner = res.envelope.unique_lock_owner.as_deref().unwrap_or("");
    assert!(
        !owner.is_empty(),
        "push_unique must record the cache lock owner token on the envelope; got none"
    );
}

#[tokio::test]
#[serial]
async fn lock_is_released_when_processing_begins_not_when_it_ends() {
    install_cache();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    register_job::<UntilProcessingJob>();
    HANDLED.store(0, Ordering::SeqCst);

    let first = Queue::push_unique(UntilProcessingJob { key: "k1".into() })
        .await
        .expect("first push");
    assert!(first);
    // While the job is only *queued*, a duplicate is still skipped.
    let dup = Queue::push_unique(UntilProcessingJob { key: "k1".into() })
        .await
        .expect("dup push");
    assert!(!dup, "queued-but-unprocessed job must still dedupe");

    work_one(driver.clone()).await;
    assert_eq!(HANDLED.load(Ordering::SeqCst), 1, "the job ran");

    // The lock was released when processing began, so a re-push wins the lock
    // again even though unique_for (300s) has not elapsed.
    let repush = Queue::push_unique(UntilProcessingJob { key: "k1".into() })
        .await
        .expect("re-push after processing began");
    assert!(
        repush,
        "unique_until_processing must release the lock at processing start; \
         a re-push inside the TTL was still deduped"
    );
}

#[tokio::test]
#[serial]
async fn plain_unique_jobs_keep_ttl_semantics() {
    // Regression pin: a job that does NOT opt in keeps push-time-only dedupe.
    install_cache();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct PlainUnique;
    #[async_trait]
    impl Job for PlainUnique {
        fn job_name() -> &'static str {
            "wave5-plain-unique"
        }
        fn unique_id(&self) -> Option<String> {
            Some("fixed".into())
        }
        async fn handle(self) -> Result<(), FrameworkError> {
            Ok(())
        }
    }
    register_job::<PlainUnique>();

    assert!(Queue::push_unique(PlainUnique).await.expect("push"));
    work_one(driver.clone()).await;

    // Even after the job processed, the TTL window still dedupes.
    assert!(
        !Queue::push_unique(PlainUnique).await.expect("re-push"),
        "a plain unique job must keep TTL-window dedupe after processing"
    );
}

/// Middleware that releases every job back onto the queue without ever
/// reaching the handler - Laravel's `$job->isReleased()` case.
struct AlwaysRelease;

#[async_trait]
impl JobMiddleware for AlwaysRelease {
    async fn handle(
        &self,
        _env: Envelope,
        _next: JobMiddlewareNext,
    ) -> Result<JobOutcome, FrameworkError> {
        Ok(JobOutcome::Released {
            delay: Duration::from_secs(30),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReleasedByMiddlewareJob;

#[async_trait]
impl Job for ReleasedByMiddlewareJob {
    fn job_name() -> &'static str {
        "wave5-until-processing-released"
    }
    fn unique_id(&self) -> Option<String> {
        Some("released".into())
    }
    fn unique_until_processing() -> bool {
        true
    }
    fn middleware() -> Vec<Arc<dyn JobMiddleware>> {
        vec![Arc::new(AlwaysRelease)]
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        RELEASED_HANDLED.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn a_job_released_by_middleware_keeps_its_lock() {
    install_cache();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    register_job::<ReleasedByMiddlewareJob>();
    RELEASED_HANDLED.store(0, Ordering::SeqCst);

    assert!(
        Queue::push_unique(ReleasedByMiddlewareJob)
            .await
            .expect("push")
    );
    work_one(driver.clone()).await;
    assert_eq!(
        RELEASED_HANDLED.load(Ordering::SeqCst),
        0,
        "the middleware short-circuited, so the handler never ran"
    );

    assert!(
        !Queue::push_unique(ReleasedByMiddlewareJob)
            .await
            .expect("re-push"),
        "a job released back onto the queue has not started processing, so it \
         must keep its uniqueness lock"
    );
}

#[tokio::test]
#[serial]
async fn a_redelivered_attempt_never_releases_a_newer_dispatchs_lock() {
    install_cache();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    register_job::<UntilProcessingJob>();
    HANDLED.store(0, Ordering::SeqCst);

    // Dispatch A wins the lock.
    assert!(
        Queue::push_unique(UntilProcessingJob { key: "k2".into() })
            .await
            .expect("dispatch A")
    );
    let a = driver
        .pop(Duration::from_secs(60))
        .await
        .expect("pop A")
        .expect("A reserved");
    let owner_a = a
        .envelope
        .unique_lock_owner
        .clone()
        .expect("A must carry an owner token");
    driver.ack(&a.token).await.expect("ack A");

    // A starts processing, which releases A's lock owner-scoped.
    let key = unique_key(UntilProcessingJob::job_name(), "k2");
    assert!(
        Idempotency::release_owned(&key, &owner_a)
            .await
            .expect("release A"),
        "the owner that took the lock must be able to release it"
    );

    // Dispatch B now holds the lock, and its envelope is reserved (held, not
    // acked) so the worker below cannot claim it.
    assert!(
        Queue::push_unique(UntilProcessingJob { key: "k2".into() })
            .await
            .expect("dispatch B")
    );
    let b = driver
        .pop(Duration::from_secs(300))
        .await
        .expect("pop B")
        .expect("B reserved");
    let owner_b = b
        .envelope
        .unique_lock_owner
        .clone()
        .expect("B must carry an owner token");
    assert_ne!(owner_a, owner_b, "a second acquisition mints a fresh token");

    // A is redelivered, still carrying A's now-stale owner token. The worker
    // bumps `attempts` to 2 on pop, which is Laravel's retry case.
    let mut redelivered = a.envelope.clone();
    redelivered.attempts = 1;
    driver.push(redelivered).await.expect("redeliver A");

    work_one(driver.clone()).await;
    assert_eq!(HANDLED.load(Ordering::SeqCst), 1, "the redelivery ran");

    assert!(
        !Queue::push_unique(UntilProcessingJob { key: "k2".into() })
            .await
            .expect("dispatch C"),
        "the redelivered attempt released owner-scoped with a stale token, so \
         dispatch B's lock must still be held"
    );
}

#[tokio::test]
#[serial]
async fn an_envelope_without_a_recorded_owner_keeps_ttl_semantics() {
    // Envelopes serialized before `unique_lock_owner` existed deserialize it as
    // `None`. There is no owner token to release and Suprnova has no
    // force-release, so the TTL stays the only release - exactly the behaviour
    // that shipped before this feature.
    install_cache();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    register_job::<UntilProcessingJob>();
    HANDLED.store(0, Ordering::SeqCst);

    assert!(
        Queue::push_unique(UntilProcessingJob { key: "k3".into() })
            .await
            .expect("push")
    );
    let res = driver
        .pop(Duration::from_secs(60))
        .await
        .expect("pop")
        .expect("reserved");
    driver.ack(&res.token).await.expect("ack");

    let mut legacy = res.envelope.clone();
    legacy.unique_lock_owner = None;
    driver.push(legacy).await.expect("push legacy envelope");

    work_one(driver.clone()).await;
    assert_eq!(HANDLED.load(Ordering::SeqCst), 1, "the job ran");

    assert!(
        !Queue::push_unique(UntilProcessingJob { key: "k3".into() })
            .await
            .expect("re-push"),
        "with no owner token there is nothing to release owner-scoped, so the \
         TTL still gates re-dispatch"
    );
}

// ---------------------------------------------------------------------------
// The `finally` sweep.
//
// A middleware that short-circuits means the pipeline core never ran, so the
// release at processing start never happened. Every short-circuit except
// `Released` still has to give the lock up, because the job is never going to
// process - Laravel guards its `finally` on `! $job->isReleased()`, not on the
// severity of the outcome.
// ---------------------------------------------------------------------------

static SHORT_CIRCUIT_HANDLED: AtomicUsize = AtomicUsize::new(0);

/// How a short-circuiting middleware settles the job without ever calling the
/// handler.
enum ShortCircuit {
    Deleted,
    Failed,
    Completed,
}

struct ShortCircuitMiddleware(ShortCircuit);

#[async_trait]
impl JobMiddleware for ShortCircuitMiddleware {
    async fn handle(
        &self,
        _env: Envelope,
        _next: JobMiddlewareNext,
    ) -> Result<JobOutcome, FrameworkError> {
        Ok(match self.0 {
            ShortCircuit::Deleted => JobOutcome::Deleted,
            ShortCircuit::Failed => JobOutcome::Failed {
                reason: "short-circuited by middleware".into(),
            },
            ShortCircuit::Completed => JobOutcome::Completed,
        })
    }
}

/// One opted-in unique job per short-circuit outcome. Each needs its own
/// `job_name` and its own `middleware()`, so the boilerplate is generated
/// rather than written three times.
macro_rules! short_circuit_job {
    ($ty:ident, $job_name:literal, $unique_id:literal, $variant:ident) => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        struct $ty;

        #[async_trait]
        impl Job for $ty {
            fn job_name() -> &'static str {
                $job_name
            }
            fn unique_id(&self) -> Option<String> {
                Some($unique_id.into())
            }
            fn unique_until_processing() -> bool {
                true
            }
            fn middleware() -> Vec<Arc<dyn JobMiddleware>> {
                vec![Arc::new(ShortCircuitMiddleware(ShortCircuit::$variant))]
            }
            async fn handle(self) -> Result<(), FrameworkError> {
                SHORT_CIRCUIT_HANDLED.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }
    };
}

short_circuit_job!(DroppedJob, "wave5-sweep-deleted", "sweep-deleted", Deleted);
short_circuit_job!(
    DeadLetteredJob,
    "wave5-sweep-failed",
    "sweep-failed",
    Failed
);
short_circuit_job!(
    CompletedByMiddlewareJob,
    "wave5-sweep-completed",
    "sweep-completed",
    Completed
);

/// Push the job, let the worker settle it once, and assert the sweep freed the
/// lock so a re-push inside `unique_for` wins it again.
async fn assert_sweep_releases_lock<J: Job>(make: impl Fn() -> J, settled_as: &str) {
    install_cache();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    register_job::<J>();
    SHORT_CIRCUIT_HANDLED.store(0, Ordering::SeqCst);

    assert!(
        Queue::push_unique(make()).await.expect("push"),
        "the first push must win the lock"
    );
    work_one(driver.clone()).await;
    assert_eq!(
        SHORT_CIRCUIT_HANDLED.load(Ordering::SeqCst),
        0,
        "the middleware short-circuited, so the handler never ran and the \
         release at processing start never happened"
    );

    assert!(
        Queue::push_unique(make()).await.expect("re-push"),
        "a job the middleware settled as {settled_as} is never going to \
         process, so the sweep must release its uniqueness lock"
    );
}

#[tokio::test]
#[serial]
async fn a_job_dropped_by_middleware_gives_up_its_lock() {
    assert_sweep_releases_lock(|| DroppedJob, "deleted").await;
}

#[tokio::test]
#[serial]
async fn a_job_dead_lettered_by_middleware_gives_up_its_lock() {
    // A dead-letter with no store bound logs the whole envelope at ERROR;
    // bind one so the settlement takes its normal path.
    Queue::set_failed_store(Arc::new(MemoryFailedJobStore::new()));
    assert_sweep_releases_lock(|| DeadLetteredJob, "failed").await;
}

#[tokio::test]
#[serial]
async fn a_job_completed_by_middleware_gives_up_its_lock() {
    // The one short-circuit that is not a failure. The lock still has to go:
    // the handler never ran, so nothing released it at processing start, and
    // the job is settled and gone.
    assert_sweep_releases_lock(|| CompletedByMiddlewareJob, "completed").await;
}
