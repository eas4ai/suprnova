//! Queue pause / resume - `Queue::pause` / `resume` / `pause_all` /
//! `resume_all` / `is_paused` / `paused_queues`, and the worker's claim
//! gate that gives them teeth.
//!
//! The CLI-level "exits non-zero without a queue and without `--all`"
//! contract is proven separately, inline in `framework/src/app/mod.rs`'s
//! `queue_pause_target_tests` module (beside `migrate_fresh_gate_tests`,
//! the same file's existing precedent for this) - the function it guards
//! is a private free function, invisible to this integration-test crate,
//! and its process-exit paths cannot be exercised in-process at all.
//! Every other "Proves" bullet for this task is a named test below.

use serial_test::serial;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use suprnova::App;
use suprnova::cache::{Cache, CacheStore, InMemoryCache};
use suprnova::events::{EventFacade, assert_dispatched_once, dispatched_count};
use suprnova::queue::events::{
    QueuePaused, QueueResumed, QueuesPaused, QueuesResumed, WorkerQueuePaused, WorkerQueueResumed,
};
use suprnova::queue::{
    MemoryQueueDriver, Queue,
    worker::{WorkerConfig, register_job, run_worker},
};
use suprnova::{FrameworkError, Job, async_trait};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

fn cache_init() {
    if !Cache::is_initialized() {
        App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    }
}

/// SAFETY: env mutation is process-global; `#[serial]` keeps these tests
/// from racing each other or any other `#[serial]` test in this binary.
fn set_env(key: &str, value: Option<&str>) {
    unsafe {
        match value {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
}

fn default_worker_cfg(queues: Vec<String>) -> WorkerConfig {
    WorkerConfig {
        visibility_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(5),
        max_jobs: None,
        queues,
    }
}

// ============================================================================
// Global pause
// ============================================================================

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct GlobalPauseJob;
static GLOBAL_PAUSE_RUNS: AtomicU32 = AtomicU32::new(0);

#[async_trait]
impl Job for GlobalPauseJob {
    fn job_name() -> &'static str {
        "queue_pause::GlobalPauseJob"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        GLOBAL_PAUSE_RUNS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn global_pause_stops_every_claim_and_resume_all_restarts_draining() {
    cache_init();
    Queue::resume_all().await.unwrap(); // defensive: undo any leftover state
    GLOBAL_PAUSE_RUNS.store(0, Ordering::SeqCst);
    register_job::<GlobalPauseJob>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    Queue::pause_all().await.unwrap();
    Queue::push(GlobalPauseJob).await.unwrap();

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        default_worker_cfg(Vec::new()),
        CancellationToken::new(),
    ));

    // Bounded window, not a correctness-by-duration guess: give the worker
    // plenty of loop iterations to (wrongly) claim the job if the gate
    // didn't hold, then assert it didn't.
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        GLOBAL_PAUSE_RUNS.load(Ordering::SeqCst),
        0,
        "a globally paused worker must claim nothing"
    );

    Queue::resume_all().await.unwrap();
    for _ in 0..300 {
        if GLOBAL_PAUSE_RUNS.load(Ordering::SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    handle.abort();
    assert_eq!(
        GLOBAL_PAUSE_RUNS.load(Ordering::SeqCst),
        1,
        "resume_all must let the worker claim again"
    );
}

// ============================================================================
// Per-queue pause
// ============================================================================

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct QueueAJob;
static QUEUE_A_RUNS: AtomicU32 = AtomicU32::new(0);

#[async_trait]
impl Job for QueueAJob {
    fn job_name() -> &'static str {
        "queue_pause::QueueAJob"
    }
    fn queue() -> Option<&'static str> {
        Some("pause_test_a")
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        QUEUE_A_RUNS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct QueueBJob;
static QUEUE_B_RUNS: AtomicU32 = AtomicU32::new(0);

#[async_trait]
impl Job for QueueBJob {
    fn job_name() -> &'static str {
        "queue_pause::QueueBJob"
    }
    fn queue() -> Option<&'static str> {
        Some("pause_test_b")
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        QUEUE_B_RUNS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn per_queue_pause_leaves_other_named_queues_draining() {
    cache_init();
    Queue::resume_all().await.unwrap(); // defensive
    QUEUE_A_RUNS.store(0, Ordering::SeqCst);
    QUEUE_B_RUNS.store(0, Ordering::SeqCst);
    register_job::<QueueAJob>();
    register_job::<QueueBJob>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    let connection = Queue::connection_name();
    Queue::resume(&connection, "pause_test_a").await.unwrap(); // defensive

    Queue::pause(&connection, "pause_test_a").await.unwrap();
    Queue::push(QueueAJob).await.unwrap();
    Queue::push(QueueBJob).await.unwrap();

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        default_worker_cfg(vec!["pause_test_a".to_string(), "pause_test_b".to_string()]),
        CancellationToken::new(),
    ));

    for _ in 0..300 {
        if QUEUE_B_RUNS.load(Ordering::SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        QUEUE_B_RUNS.load(Ordering::SeqCst),
        1,
        "the unpaused named queue must keep draining"
    );
    assert_eq!(
        QUEUE_A_RUNS.load(Ordering::SeqCst),
        0,
        "the paused named queue must not be claimed"
    );

    Queue::resume(&connection, "pause_test_a").await.unwrap();
    for _ in 0..300 {
        if QUEUE_A_RUNS.load(Ordering::SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    handle.abort();
    assert_eq!(
        QUEUE_A_RUNS.load(Ordering::SeqCst),
        1,
        "resuming the specific queue must let it drain too"
    );
}

// ============================================================================
// resume_all leaves a per-queue pause in place (pure facade, no worker)
// ============================================================================

#[tokio::test]
#[serial]
async fn resume_all_does_not_clear_a_per_queue_pause() {
    cache_init();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver);
    let connection = Queue::connection_name();

    Queue::pause(&connection, "pause_test_billing")
        .await
        .unwrap();
    assert!(
        Queue::is_paused(&connection, "pause_test_billing")
            .await
            .unwrap()
    );

    Queue::resume_all().await.unwrap();

    assert!(
        Queue::is_paused(&connection, "pause_test_billing")
            .await
            .unwrap(),
        "resume_all must not clear a per-queue pause - Laravel semantics"
    );
    assert_eq!(
        Queue::paused_queues(
            &connection,
            &[
                "pause_test_billing".to_string(),
                "pause_test_other".to_string(),
            ],
        )
        .await
        .unwrap(),
        vec!["pause_test_billing".to_string()],
        "only the queue paused individually should come back"
    );

    Queue::resume(&connection, "pause_test_billing")
        .await
        .unwrap();
    assert!(
        !Queue::is_paused(&connection, "pause_test_billing")
            .await
            .unwrap()
    );
}

// ============================================================================
// Exactly one event per call, under Event::fake()
// ============================================================================

#[tokio::test]
#[serial]
async fn pause_and_resume_each_dispatch_exactly_one_event() {
    cache_init();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver);
    let connection = Queue::connection_name();

    let _fake = EventFacade::fake();

    Queue::pause_all().await.unwrap();
    Queue::resume_all().await.unwrap();
    Queue::pause(&connection, "pause_test_events")
        .await
        .unwrap();
    Queue::resume(&connection, "pause_test_events")
        .await
        .unwrap();

    assert_dispatched_once::<QueuesPaused>();
    assert_dispatched_once::<QueuesResumed>();
    assert_dispatched_once::<QueuePaused>();
    assert_dispatched_once::<QueueResumed>();
}

// ============================================================================
// An in-flight job completes after pause takes effect
// ============================================================================

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct BlockingJob;

fn blocking_started() -> &'static Notify {
    static N: OnceLock<Notify> = OnceLock::new();
    N.get_or_init(Notify::new)
}
fn blocking_proceed() -> &'static Notify {
    static N: OnceLock<Notify> = OnceLock::new();
    N.get_or_init(Notify::new)
}
fn blocking_done() -> &'static Notify {
    static N: OnceLock<Notify> = OnceLock::new();
    N.get_or_init(Notify::new)
}

#[async_trait]
impl Job for BlockingJob {
    fn job_name() -> &'static str {
        "queue_pause::BlockingJob"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        blocking_started().notify_one();
        blocking_proceed().notified().await;
        blocking_done().notify_one();
        Ok(())
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct QuickJob;
static QUICK_JOB_RUNS: AtomicU32 = AtomicU32::new(0);

#[async_trait]
impl Job for QuickJob {
    fn job_name() -> &'static str {
        "queue_pause::QuickJob"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        QUICK_JOB_RUNS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn in_flight_job_completes_after_pause_and_the_next_job_waits_for_resume() {
    cache_init();
    Queue::resume_all().await.unwrap(); // defensive
    QUICK_JOB_RUNS.store(0, Ordering::SeqCst);
    register_job::<BlockingJob>();
    register_job::<QuickJob>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    Queue::push(BlockingJob).await.unwrap();

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        default_worker_cfg(Vec::new()),
        CancellationToken::new(),
    ));

    // Deterministic: wait for the handler to actually start running, not a
    // timing guess.
    tokio::time::timeout(Duration::from_secs(2), blocking_started().notified())
        .await
        .expect("BlockingJob must start within 2s");

    // Pause WHILE the job is in flight, then push a second job that must
    // not be claimed until resume.
    Queue::pause_all().await.unwrap();
    Queue::push(QuickJob).await.unwrap();

    // Let the in-flight job finish.
    blocking_proceed().notify_one();
    tokio::time::timeout(Duration::from_secs(2), blocking_done().notified())
        .await
        .expect("the in-flight job must complete despite the pause");

    // Bounded window: give the worker a fair chance to (wrongly) claim
    // QuickJob if the gate didn't hold.
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        QUICK_JOB_RUNS.load(Ordering::SeqCst),
        0,
        "the queue is globally paused; QuickJob must not be claimed yet"
    );

    Queue::resume_all().await.unwrap();
    for _ in 0..300 {
        if QUICK_JOB_RUNS.load(Ordering::SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    handle.abort();
    assert_eq!(
        QUICK_JOB_RUNS.load(Ordering::SeqCst),
        1,
        "QuickJob must run once the pause is lifted"
    );
}

// ============================================================================
// QUEUE_PAUSABLE=false - the worker ignores pause signals entirely
// ============================================================================

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct UnpausableJob;
static UNPAUSABLE_RUNS: AtomicU32 = AtomicU32::new(0);

#[async_trait]
impl Job for UnpausableJob {
    fn job_name() -> &'static str {
        "queue_pause::UnpausableJob"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        UNPAUSABLE_RUNS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn pausable_false_worker_ignores_the_global_pause_signal() {
    cache_init();
    Queue::resume_all().await.unwrap(); // defensive
    UNPAUSABLE_RUNS.store(0, Ordering::SeqCst);
    register_job::<UnpausableJob>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::pause_all().await.unwrap();
    Queue::push(UnpausableJob).await.unwrap();

    set_env("QUEUE_PAUSABLE", Some("false"));
    let mut cfg = default_worker_cfg(Vec::new());
    cfg.max_jobs = Some(1); // the worker exits cleanly once it settles one job
    let handle = tokio::spawn(run_worker(driver.clone(), cfg, CancellationToken::new()));

    let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    set_env("QUEUE_PAUSABLE", None);
    Queue::resume_all().await.unwrap();

    assert!(
        result.is_ok(),
        "an unpausable worker must not stall waiting for a pause it ignores"
    );
    assert_eq!(
        UNPAUSABLE_RUNS.load(Ordering::SeqCst),
        1,
        "QUEUE_PAUSABLE=false must make the worker ignore the global pause"
    );
}

// ============================================================================
// Worker-side pause/resume events (Laravel #61142)
// ============================================================================

#[tokio::test]
#[serial]
async fn a_worker_emits_one_paused_and_one_resumed_event_per_named_queue() {
    cache_init();
    Queue::resume_all().await.unwrap(); // defensive
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    let connection = Queue::connection_name();
    Queue::resume(&connection, "pause_evt_a").await.unwrap(); // defensive

    let _fake = EventFacade::fake();

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        default_worker_cfg(vec!["pause_evt_a".to_string(), "pause_evt_b".to_string()]),
        CancellationToken::new(),
    ));

    // Let the worker observe an unpaused world first, so the pause below is a
    // transition rather than the initial state.
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        dispatched_count::<WorkerQueuePaused>(|_| true),
        0,
        "nothing is paused yet"
    );

    Queue::pause(&connection, "pause_evt_a").await.unwrap();
    for _ in 0..60 {
        if dispatched_count::<WorkerQueuePaused>(|_| true) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Many loop iterations pass while the queue stays paused; the event must
    // fire on the transition, not on every one of them.
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        dispatched_count::<WorkerQueuePaused>(|e| e.queue.as_deref() == Some("pause_evt_a")),
        1,
        "exactly one WorkerQueuePaused per transition, not one per loop"
    );
    assert_eq!(
        dispatched_count::<WorkerQueuePaused>(|e| e.queue.as_deref() == Some("pause_evt_b")),
        0,
        "the queue that was never paused must not be reported"
    );

    Queue::resume(&connection, "pause_evt_a").await.unwrap();
    for _ in 0..60 {
        if dispatched_count::<WorkerQueueResumed>(|_| true) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    handle.abort();
    assert_eq!(
        dispatched_count::<WorkerQueueResumed>(|e| e.queue.as_deref() == Some("pause_evt_a")),
        1,
        "and exactly one WorkerQueueResumed on the way back"
    );

    Queue::resume(&connection, "pause_evt_a").await.unwrap(); // leave the world clean
}

#[tokio::test]
#[serial]
async fn a_queue_already_paused_at_worker_start_is_reported_once() {
    cache_init();
    Queue::resume_all().await.unwrap(); // defensive
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    let connection = Queue::connection_name();
    Queue::pause(&connection, "pause_evt_start").await.unwrap();

    let _fake = EventFacade::fake();

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        default_worker_cfg(vec!["pause_evt_start".to_string()]),
        CancellationToken::new(),
    ));

    // Far more loop iterations than events we expect.
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    handle.abort();

    assert_eq!(
        dispatched_count::<WorkerQueuePaused>(|e| e.queue.as_deref() == Some("pause_evt_start")),
        1,
        "a worker that starts into a paused queue reports it once, not once per poll"
    );
    assert_eq!(
        dispatched_count::<WorkerQueueResumed>(|_| true),
        0,
        "nothing resumed"
    );

    Queue::resume(&connection, "pause_evt_start").await.unwrap();
}

#[tokio::test]
#[serial]
async fn an_unfiltered_worker_reports_a_global_pause_without_a_queue_name() {
    cache_init();
    Queue::resume_all().await.unwrap(); // defensive
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    let _fake = EventFacade::fake();

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        default_worker_cfg(Vec::new()),
        CancellationToken::new(),
    ));
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    Queue::pause_all().await.unwrap();
    for _ in 0..60 {
        if dispatched_count::<WorkerQueuePaused>(|_| true) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        dispatched_count::<WorkerQueuePaused>(|e| e.queue.is_none()),
        1,
        "an unfiltered worker has no queue names to report, so the event carries None"
    );

    Queue::resume_all().await.unwrap();
    for _ in 0..60 {
        if dispatched_count::<WorkerQueueResumed>(|_| true) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    handle.abort();
    assert_eq!(
        dispatched_count::<WorkerQueueResumed>(|e| e.queue.is_none()),
        1,
        "and one resume on the way back"
    );
}
