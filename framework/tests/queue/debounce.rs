//! Debounced jobs and debounced queued listeners.
//!
//! Debouncing keeps the LAST dispatch of a burst, where `push_unique` keeps the
//! first, so the failures that matter are the two directions of "which dispatch
//! survives": a superseded envelope that runs anyway (the burst was not
//! collapsed), and a current envelope that is dropped as superseded (work
//! silently lost). Every test below pins one of those, plus the max-wait escape
//! hatch, the fail-open rule, and the mutual-exclusion refusal.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serial_test::serial;
use suprnova::App;
use suprnova::cache::{Cache, CacheStore, InMemoryCache};
use suprnova::events::{DebouncedListener, Event, EventFacade, dispatched_count};
use suprnova::queue::driver::{QueueDriver, Reservation, ReservationToken};
use suprnova::queue::events::JobDebounced;
use suprnova::queue::memory::MemoryQueueDriver;
use suprnova::queue::worker::{WorkerConfig, register_job, run_worker};
use suprnova::queue::{DebounceOptions, Job, Queue};
use suprnova::testing::TestContainer;
use suprnova::{FrameworkError, async_trait};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

fn cache_init() {
    if !Cache::is_initialized() {
        App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    }
}

fn worker_cfg() -> WorkerConfig {
    WorkerConfig {
        visibility_timeout: Duration::from_secs(30),
        poll_interval: Duration::from_millis(5),
        max_jobs: None,
        queues: Vec::new(),
    }
}

/// Spin until `done` reports true, then keep spinning for a grace period so a
/// broken supersession check has every chance to run the envelopes it should
/// have dropped.
async fn settle(done: impl Fn() -> bool) {
    for _ in 0..200 {
        if done() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// ---------------------------------------------------------------------------
// Push side: the burst collapses onto the last dispatch
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct SyncOrder {
    order_id: u32,
    revision: u32,
}
static SYNC_ORDER_RUNS: AtomicU32 = AtomicU32::new(0);
static SYNC_ORDER_LAST_REVISION: AtomicU32 = AtomicU32::new(0);

#[async_trait]
impl Job for SyncOrder {
    fn job_name() -> &'static str {
        "queue_debounce::SyncOrder"
    }
    fn debounce_for() -> Option<Duration> {
        Some(Duration::from_millis(120))
    }
    fn debounce_id(&self) -> Option<String> {
        Some(self.order_id.to_string())
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        SYNC_ORDER_RUNS.fetch_add(1, Ordering::SeqCst);
        SYNC_ORDER_LAST_REVISION.store(self.revision, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn a_burst_of_dispatches_runs_once_and_keeps_the_last_one() {
    cache_init();
    SYNC_ORDER_RUNS.store(0, Ordering::SeqCst);
    SYNC_ORDER_LAST_REVISION.store(0, Ordering::SeqCst);
    register_job::<SyncOrder>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    for revision in 1..=5 {
        Queue::push(SyncOrder {
            order_id: 7,
            revision,
        })
        .await
        .expect("push");
    }
    assert_eq!(
        driver.size().await.expect("size"),
        5,
        "every dispatch is enqueued; debouncing is settled at the worker, not by \
         suppressing the push"
    );

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| SYNC_ORDER_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();

    assert_eq!(
        SYNC_ORDER_RUNS.load(Ordering::SeqCst),
        1,
        "a burst of five must collapse into one run"
    );
    assert_eq!(
        SYNC_ORDER_LAST_REVISION.load(Ordering::SeqCst),
        5,
        "and the run must be the LAST dispatch, not the first"
    );
}

#[tokio::test]
#[serial]
async fn different_debounce_ids_debounce_independently() {
    cache_init();
    SYNC_ORDER_RUNS.store(0, Ordering::SeqCst);
    register_job::<SyncOrder>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    for order_id in 101..=103 {
        Queue::push(SyncOrder {
            order_id,
            revision: 1,
        })
        .await
        .expect("push");
    }

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| SYNC_ORDER_RUNS.load(Ordering::SeqCst) >= 3).await;
    handle.abort();
    assert_eq!(
        SYNC_ORDER_RUNS.load(Ordering::SeqCst),
        3,
        "three orders are three independent windows, not one shared one"
    );
}

#[tokio::test]
#[serial]
async fn call_site_options_outrank_what_the_job_declares() {
    cache_init();
    SYNC_ORDER_RUNS.store(0, Ordering::SeqCst);
    SYNC_ORDER_LAST_REVISION.store(0, Ordering::SeqCst);
    register_job::<SyncOrder>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    // `SyncOrder` keys its window on `order_id`, so these two dispatches would
    // debounce independently. The call site says otherwise, and the call site
    // wins: one shared window, one run.
    for order_id in 1..=2 {
        Queue::push_debounced(
            SyncOrder {
                order_id,
                revision: order_id,
            },
            DebounceOptions::new(Duration::from_millis(120)).id("call-site"),
        )
        .await
        .expect("push_debounced");
    }

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| SYNC_ORDER_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();
    assert_eq!(
        SYNC_ORDER_RUNS.load(Ordering::SeqCst),
        1,
        "the options' id replaces Job::debounce_id, so both dispatches share one \
         window"
    );
    assert_eq!(
        SYNC_ORDER_LAST_REVISION.load(Ordering::SeqCst),
        2,
        "and the survivor is still the last dispatch"
    );
}

// ---------------------------------------------------------------------------
// Fail open: only a positively different owner drops an envelope
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn a_lapsed_window_runs_the_job_rather_than_dropping_it() {
    cache_init();
    SYNC_ORDER_RUNS.store(0, Ordering::SeqCst);
    register_job::<SyncOrder>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::push(SyncOrder {
        order_id: 404,
        revision: 1,
    })
    .await
    .expect("push");

    // Exactly what an eviction or a TTL expiry leaves behind: an envelope
    // carrying a token, and no token in the cache to compare it against.
    Cache::forget("queue-debounce:queue_debounce::SyncOrder:404")
        .await
        .expect("forget");

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| SYNC_ORDER_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();
    assert_eq!(
        SYNC_ORDER_RUNS.load(Ordering::SeqCst),
        1,
        "a missing owner token is not evidence that somebody else owns the window, \
         so the job runs rather than being silently discarded"
    );
}

/// A driver that refuses every write, for proving that a push which arms a
/// window and then fails does not leave the window naming an owner that never
/// reached the queue.
struct RefusingQueueDriver;

#[async_trait]
impl QueueDriver for RefusingQueueDriver {
    async fn push(&self, _env: suprnova::queue::Envelope) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("driver refused the write"))
    }
    async fn pop(&self, _vt: Duration) -> Result<Option<Reservation>, FrameworkError> {
        Ok(None)
    }
    async fn ack(&self, _token: &ReservationToken) -> Result<(), FrameworkError> {
        Ok(())
    }
    async fn nack(
        &self,
        _token: &ReservationToken,
        _delay: Duration,
    ) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn a_push_that_fails_after_arming_leaves_the_window_to_the_queued_dispatch() {
    cache_init();
    SYNC_ORDER_RUNS.store(0, Ordering::SeqCst);
    SYNC_ORDER_LAST_REVISION.store(0, Ordering::SeqCst);
    register_job::<SyncOrder>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::push(SyncOrder {
        order_id: 900,
        revision: 1,
    })
    .await
    .expect("first");
    let window = Cache::get::<String>("queue-debounce:queue_debounce::SyncOrder:900")
        .await
        .expect("cache");

    // The second dispatch arms the window and then fails to enqueue anything
    // to carry it.
    Queue::set_driver(Arc::new(RefusingQueueDriver));
    Queue::push(SyncOrder {
        order_id: 900,
        revision: 2,
    })
    .await
    .expect_err("the driver refused the write");

    assert_eq!(
        Cache::get::<String>("queue-debounce:queue_debounce::SyncOrder:900")
            .await
            .expect("cache"),
        window,
        "a dispatch whose envelope never reached the queue must not name the \
         window's owner: nothing could ever satisfy it"
    );

    Queue::set_driver(driver.clone());
    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| SYNC_ORDER_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();

    assert_eq!(
        SYNC_ORDER_RUNS.load(Ordering::SeqCst),
        1,
        "the first dispatch is still queued and must still run: its own push \
         reported success"
    );
    assert_eq!(
        SYNC_ORDER_LAST_REVISION.load(Ordering::SeqCst),
        1,
        "the survivor is the dispatch that actually made it onto the queue"
    );
}

// ---------------------------------------------------------------------------
// The superseded envelope is dropped, and says so
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct ReportSupersession {
    dispatch: u32,
}
static SUPERSESSION_RUNS: AtomicU32 = AtomicU32::new(0);
static SUPERSESSION_SURVIVOR: AtomicU32 = AtomicU32::new(0);

#[async_trait]
impl Job for ReportSupersession {
    fn job_name() -> &'static str {
        "queue_debounce::ReportSupersession"
    }
    fn debounce_for() -> Option<Duration> {
        Some(Duration::from_millis(120))
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        SUPERSESSION_RUNS.fetch_add(1, Ordering::SeqCst);
        SUPERSESSION_SURVIVOR.store(self.dispatch, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn a_superseded_envelope_is_dropped_and_reports_it() {
    cache_init();
    SUPERSESSION_RUNS.store(0, Ordering::SeqCst);
    SUPERSESSION_SURVIVOR.store(0, Ordering::SeqCst);
    register_job::<ReportSupersession>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    let _fake = EventFacade::fake();
    Queue::push(ReportSupersession { dispatch: 1 })
        .await
        .expect("first");
    Queue::push(ReportSupersession { dispatch: 2 })
        .await
        .expect("second");

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| SUPERSESSION_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();

    assert_eq!(
        SUPERSESSION_RUNS.load(Ordering::SeqCst),
        1,
        "the second dispatch supersedes the first"
    );
    assert_eq!(
        SUPERSESSION_SURVIVOR.load(Ordering::SeqCst),
        2,
        "and the survivor is the SECOND dispatch: a comparison that dropped the \
         current envelope and ran the stale one would also leave one run behind"
    );
    assert_eq!(
        dispatched_count::<JobDebounced>(|e| e.job.job_name == "queue_debounce::ReportSupersession"),
        1,
        "a dropped envelope is reported, not swallowed: exactly one JobDebounced \
         for the one envelope that was superseded"
    );
    assert_eq!(
        driver.size().await.expect("size"),
        0,
        "the superseded envelope is acknowledged, not left to be redelivered"
    );
    // Sol review of DRIVERS-054: the dropped envelope is a settled attempt
    // too, as it is in Laravel, where the worker fires JobAttempted for it.
    assert_eq!(
        dispatched_count::<suprnova::queue::events::JobAttempted>(
            |e| e.job.job_name == "queue_debounce::ReportSupersession"
        ),
        2,
        "both envelopes settled: the one that ran and the one that was dropped"
    );
}

// ---------------------------------------------------------------------------
// Max wait: a continuous burst cannot defer the run forever
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct RollUpMetrics;
static ROLLUP_RUNS: AtomicU32 = AtomicU32::new(0);

#[async_trait]
impl Job for RollUpMetrics {
    fn job_name() -> &'static str {
        "queue_debounce::RollUpMetrics"
    }
    fn debounce_for() -> Option<Duration> {
        Some(Duration::from_secs(3600)) // never elapses inside this test
    }
    fn max_debounce_wait() -> Option<Duration> {
        Some(Duration::from_secs(0)) // every dispatch after the first is overdue
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        ROLLUP_RUNS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn max_wait_forces_a_run_that_the_window_alone_would_defer() {
    cache_init();
    ROLLUP_RUNS.store(0, Ordering::SeqCst);
    register_job::<RollUpMetrics>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    // First dispatch stamps the burst's start and waits out the (hour-long)
    // window. The second finds the max wait already exceeded and is queued with
    // no delay at all.
    Queue::push(RollUpMetrics).await.expect("first");
    Queue::push(RollUpMetrics).await.expect("second");

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| ROLLUP_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();
    assert_eq!(
        ROLLUP_RUNS.load(Ordering::SeqCst),
        1,
        "max_wait must let the deferred work through instead of holding it for \
         the full window"
    );
}

#[derive(Serialize, Deserialize, Clone)]
struct CompactLedger;
static COMPACT_RUNS: AtomicU32 = AtomicU32::new(0);

#[async_trait]
impl Job for CompactLedger {
    fn job_name() -> &'static str {
        "queue_debounce::CompactLedger"
    }
    fn debounce_for() -> Option<Duration> {
        Some(Duration::from_millis(120))
    }
    fn max_debounce_wait() -> Option<Duration> {
        // Generous on purpose: this job reaches the worker by the ORDINARY
        // debounce path, with max wait never exceeded. That is the path the
        // window reset has to cover.
        Some(Duration::from_secs(600))
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        COMPACT_RUNS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Laravel #61281: the max-wait window restarts at every actual run, not only
/// when max wait fired. Asserted at the cache layer because the observable
/// consequence - a later burst measuring its window from a previous burst's
/// first dispatch - takes ten wall-clock minutes to reproduce end to end.
#[tokio::test]
#[serial]
async fn an_actual_run_clears_the_first_dispatch_stamp() {
    cache_init();
    COMPACT_RUNS.store(0, Ordering::SeqCst);
    register_job::<CompactLedger>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    let stamp_key = "queue-debounce-first-dispatched:queue-debounce:queue_debounce::CompactLedger:";
    Cache::forget(stamp_key).await.expect("cache");

    Queue::push(CompactLedger).await.expect("first");
    Queue::push(CompactLedger).await.expect("second");

    assert!(
        Cache::get::<i64>(stamp_key).await.expect("cache").is_some(),
        "control: the burst stamped its first dispatch, and the ordinary path \
         left the stamp in place"
    );

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| COMPACT_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();

    assert_eq!(
        COMPACT_RUNS.load(Ordering::SeqCst),
        1,
        "the burst still collapses to one run"
    );
    assert!(
        Cache::get::<i64>(stamp_key).await.expect("cache").is_none(),
        "every actual run starts a fresh max-wait window, so the next burst \
         measures from its own first dispatch"
    );
}

// ---------------------------------------------------------------------------
// Failure mode: debounce and uniqueness cannot both be declared
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct ConfusedJob;

#[async_trait]
impl Job for ConfusedJob {
    fn job_name() -> &'static str {
        "queue_debounce::ConfusedJob"
    }
    fn debounce_for() -> Option<Duration> {
        Some(Duration::from_millis(50))
    }
    fn unique_id(&self) -> Option<String> {
        Some("only-one".to_string())
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn declaring_both_debounce_and_uniqueness_is_refused() {
    cache_init();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    let err = Queue::push(ConfusedJob)
        .await
        .expect_err("the two mechanisms disagree about which dispatch survives");
    let message = err.to_string();
    assert!(
        message.contains("debounce_for") && message.contains("unique_id"),
        "the error must name both declarations so the fix is obvious: {message}"
    );
    assert_eq!(
        driver.size().await.expect("size"),
        0,
        "nothing may be enqueued when the declarations conflict"
    );
}

#[tokio::test]
#[serial]
async fn push_unique_refuses_a_debounced_job_too() {
    cache_init();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    // The conflict is in the declarations, not in which entry point was
    // called: reaching for `push_unique` must not quietly demote a declared
    // debounce window to nothing.
    let err = Queue::push_unique(ConfusedJob)
        .await
        .expect_err("the declarations still conflict");
    let message = err.to_string();
    assert!(
        message.contains("debounce_for") && message.contains("unique_id"),
        "the error must name both declarations: {message}"
    );
    assert_eq!(
        driver.size().await.expect("size"),
        0,
        "nothing may be enqueued when the declarations conflict"
    );
}

// ---------------------------------------------------------------------------
// Failure mode: the cache is unreachable
// ---------------------------------------------------------------------------

/// A cache store whose every operation fails, for proving that a debounce that
/// cannot be armed fails the push instead of enqueueing an envelope no worker
/// can judge.
struct BrokenCache;

fn broken() -> FrameworkError {
    FrameworkError::internal("cache store unreachable")
}

#[async_trait]
impl CacheStore for BrokenCache {
    async fn get_raw(&self, _key: &str) -> Result<Option<String>, FrameworkError> {
        Err(broken())
    }
    async fn put_raw(
        &self,
        _key: &str,
        _value: &str,
        _ttl: Option<Duration>,
    ) -> Result<(), FrameworkError> {
        Err(broken())
    }
    async fn has(&self, _key: &str) -> Result<bool, FrameworkError> {
        Err(broken())
    }
    async fn forget(&self, _key: &str) -> Result<bool, FrameworkError> {
        Err(broken())
    }
    async fn flush(&self) -> Result<(), FrameworkError> {
        Err(broken())
    }
    async fn increment(&self, _key: &str, _amount: i64) -> Result<i64, FrameworkError> {
        Err(broken())
    }
    async fn decrement(&self, _key: &str, _amount: i64) -> Result<i64, FrameworkError> {
        Err(broken())
    }
    async fn tagged_put_raw(
        &self,
        _tags: &[&str],
        _key: &str,
        _value: &str,
        _ttl: Option<Duration>,
    ) -> Result<(), FrameworkError> {
        Err(broken())
    }
    async fn flush_tags(&self, _tags: &[&str]) -> Result<(), FrameworkError> {
        Err(broken())
    }
    async fn acquire_lock(
        &self,
        _key: &str,
        _ttl: Duration,
    ) -> Result<Option<String>, FrameworkError> {
        Err(broken())
    }
    async fn release_lock(&self, _key: &str, _token: &str) -> Result<bool, FrameworkError> {
        Err(broken())
    }
    async fn refresh_lock(
        &self,
        _key: &str,
        _token: &str,
        _ttl: Duration,
    ) -> Result<bool, FrameworkError> {
        Err(broken())
    }
    async fn touch(&self, _key: &str, _ttl: Duration) -> Result<bool, FrameworkError> {
        Err(broken())
    }
}

#[tokio::test]
#[serial]
async fn a_cache_failure_fails_the_push_instead_of_enqueueing_an_unjudgeable_job() {
    cache_init();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    let _container = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(BrokenCache));

    let err = Queue::push(SyncOrder {
        order_id: 500,
        revision: 1,
    })
    .await
    .expect_err("a window that cannot be armed is not a window");
    assert!(
        err.to_string().contains("cache store unreachable"),
        "the caller sees the cache error rather than a silent success: {err}"
    );
    assert_eq!(
        driver.size().await.expect("size"),
        0,
        "an envelope with no armed window would be judged against a key nothing \
         wrote; the push fails instead"
    );
}

// ---------------------------------------------------------------------------
// Arming is all-or-nothing: a half-armed window must not outlive its dispatch
// ---------------------------------------------------------------------------

/// Delegates to the real cache except for the debounce timestamp key, whose
/// reads fail. That is the shape of a Redis blip mid-arming, and also of a
/// stamp key holding a value that will not deserialize as an `i64` - which
/// fails deterministically on every push.
struct StampBrokenCache {
    inner: Arc<dyn CacheStore>,
}

fn is_stamp(key: &str) -> bool {
    key.starts_with("queue-debounce-first-dispatched:")
}

#[async_trait]
impl CacheStore for StampBrokenCache {
    async fn get_raw(&self, key: &str) -> Result<Option<String>, FrameworkError> {
        if is_stamp(key) {
            return Err(FrameworkError::internal("cache store unreachable"));
        }
        self.inner.get_raw(key).await
    }
    async fn put_raw(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), FrameworkError> {
        self.inner.put_raw(key, value, ttl).await
    }
    fn default_ttl(&self) -> Option<Duration> {
        self.inner.default_ttl()
    }
    async fn has(&self, key: &str) -> Result<bool, FrameworkError> {
        self.inner.has(key).await
    }
    async fn forget(&self, key: &str) -> Result<bool, FrameworkError> {
        self.inner.forget(key).await
    }
    async fn flush(&self) -> Result<(), FrameworkError> {
        self.inner.flush().await
    }
    async fn increment(&self, key: &str, amount: i64) -> Result<i64, FrameworkError> {
        self.inner.increment(key, amount).await
    }
    async fn decrement(&self, key: &str, amount: i64) -> Result<i64, FrameworkError> {
        self.inner.decrement(key, amount).await
    }
    async fn tagged_put_raw(
        &self,
        tags: &[&str],
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), FrameworkError> {
        self.inner.tagged_put_raw(tags, key, value, ttl).await
    }
    async fn flush_tags(&self, tags: &[&str]) -> Result<(), FrameworkError> {
        self.inner.flush_tags(tags).await
    }
    async fn acquire_lock(
        &self,
        key: &str,
        ttl: Duration,
    ) -> Result<Option<String>, FrameworkError> {
        self.inner.acquire_lock(key, ttl).await
    }
    async fn release_lock(&self, key: &str, token: &str) -> Result<bool, FrameworkError> {
        self.inner.release_lock(key, token).await
    }
    async fn refresh_lock(
        &self,
        key: &str,
        token: &str,
        ttl: Duration,
    ) -> Result<bool, FrameworkError> {
        self.inner.refresh_lock(key, token, ttl).await
    }
    async fn touch(&self, key: &str, ttl: Duration) -> Result<bool, FrameworkError> {
        self.inner.touch(key, ttl).await
    }
}

/// An arming that fails halfway, in the max-wait bookkeeping, must leave no
/// token in the cache that no envelope carries - or every earlier envelope of
/// the burst, whose own push returned `Ok`, would be dropped at the worker as
/// superseded by a dispatch that never completed. Only jobs declaring
/// `max_debounce_wait` reach that bookkeeping at all, which is the manual's
/// headline example.
#[tokio::test]
#[serial]
async fn an_arming_that_fails_halfway_hands_the_window_back() {
    cache_init();
    COMPACT_RUNS.store(0, Ordering::SeqCst);
    register_job::<CompactLedger>();
    Cache::forget("queue-debounce-first-dispatched:queue-debounce:queue_debounce::CompactLedger:")
        .await
        .expect("cache");

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    // A arms cleanly and is enqueued. Its push returned Ok, so its work is
    // owed.
    Queue::push(CompactLedger).await.expect("first");

    {
        // B arms, then fails reading the timestamp key.
        let real = Cache::store().expect("cache store");
        let _container = TestContainer::fake();
        TestContainer::bind::<dyn CacheStore>(Arc::new(StampBrokenCache { inner: real }));
        Queue::push(CompactLedger)
            .await
            .expect_err("the arming could not complete");
    }

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| COMPACT_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();

    assert_eq!(
        COMPACT_RUNS.load(Ordering::SeqCst),
        1,
        "an arming that could not complete must leave the window to the \
         envelope already on the queue, which still runs"
    );
}

/// A driver that parks inside `push` until released, then fails - so a slow
/// failing write can be interleaved with a newer dispatch that arms the same
/// window and enqueues successfully.
struct GatedFailingQueueDriver {
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

#[async_trait]
impl QueueDriver for GatedFailingQueueDriver {
    async fn push(&self, _env: suprnova::queue::Envelope) -> Result<(), FrameworkError> {
        self.entered.notify_one();
        self.release.notified().await;
        Err(FrameworkError::internal("driver refused the write"))
    }
    async fn pop(&self, _vt: Duration) -> Result<Option<Reservation>, FrameworkError> {
        Ok(None)
    }
    async fn ack(&self, _token: &ReservationToken) -> Result<(), FrameworkError> {
        Ok(())
    }
    async fn nack(
        &self,
        _token: &ReservationToken,
        _delay: Duration,
    ) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// A dispatch whose write fails slowly must leave alone a window a newer
/// dispatch has since armed and filled, or the whole burst un-collapses.
#[tokio::test]
#[serial]
async fn a_failed_push_never_tears_down_a_newer_dispatch_window() {
    cache_init();
    SYNC_ORDER_RUNS.store(0, Ordering::SeqCst);
    SYNC_ORDER_LAST_REVISION.store(0, Ordering::SeqCst);
    register_job::<SyncOrder>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    // A arms and is enqueued.
    Queue::push(SyncOrder {
        order_id: 800,
        revision: 1,
    })
    .await
    .expect("first");

    // B arms, then parks inside the driver write that will fail.
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    Queue::set_driver(Arc::new(GatedFailingQueueDriver {
        entered: entered.clone(),
        release: release.clone(),
    }));
    let parked = tokio::spawn(async {
        Queue::push(SyncOrder {
            order_id: 800,
            revision: 2,
        })
        .await
    });
    entered.notified().await;

    // C arms and is enqueued while B's write is still failing. B's failure
    // must not touch C's window.
    Queue::set_driver(driver.clone());
    Queue::push(SyncOrder {
        order_id: 800,
        revision: 3,
    })
    .await
    .expect("third");

    release.notify_one();
    parked
        .await
        .expect("join")
        .expect_err("B's driver write failed");

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| SYNC_ORDER_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();

    assert_eq!(
        SYNC_ORDER_RUNS.load(Ordering::SeqCst),
        1,
        "a failed dispatch that deleted the live owner token would let every \
         queued envelope of the burst fail open and run"
    );
    assert_eq!(
        SYNC_ORDER_LAST_REVISION.load(Ordering::SeqCst),
        3,
        "the survivor is the newest dispatch that actually reached the queue"
    );
}

// ---------------------------------------------------------------------------
// The fake must not hide a conflict that is a bug in the job
// ---------------------------------------------------------------------------

/// Declares uniqueness only, with no declarative `debounce_for` override.
/// Pushing it through [`Queue::push_debounced`] with call-site options is the
/// only way the two mechanisms collide for this job, which isolates the
/// options form of the conflict from `ConfusedJob`, which conflicts through
/// the declarative form alone.
#[derive(Serialize, Deserialize, Clone)]
struct UniqueOnlyJob;

#[async_trait]
impl Job for UniqueOnlyJob {
    fn job_name() -> &'static str {
        "queue_debounce::UniqueOnlyJob"
    }
    fn unique_id(&self) -> Option<String> {
        Some("only-one".to_string())
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn the_fake_refuses_a_job_declaring_both_too() {
    let _fake = suprnova::queue::testing::install_fake();

    let err = Queue::push(ConfusedJob)
        .await
        .expect_err("the declarations conflict whether or not a driver is wired");
    assert!(
        err.to_string().contains("debounce_for") && err.to_string().contains("unique_id"),
        "the error must name both declarations: {err}"
    );

    let err = Queue::push_unique(ConfusedJob)
        .await
        .expect_err("and through the unique entry point too");
    assert!(
        err.to_string().contains("debounce_for") && err.to_string().contains("unique_id"),
        "the error must name both declarations: {err}"
    );

    // The options form conflicts the same way, even though this job declares
    // no `debounce_for` at all: the window comes from the call site instead
    // of the job, and the fake must refuse it exactly as production does
    // rather than reporting `Ok` because there was no cache to write to.
    let err = Queue::push_debounced(
        UniqueOnlyJob,
        DebounceOptions::new(Duration::from_millis(50)),
    )
    .await
    .expect_err("call-site debounce options conflict with a declared unique_id too");
    assert!(
        err.to_string().contains("debounce_for") && err.to_string().contains("unique_id"),
        "the error must name both declarations: {err}"
    );
}

// ---------------------------------------------------------------------------
// Chains and batches refuse a debounced job rather than silently ignoring it
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct ChainedDebouncedJob;

#[async_trait]
impl Job for ChainedDebouncedJob {
    fn job_name() -> &'static str {
        "queue_debounce::ChainedDebouncedJob"
    }
    fn debounce_for() -> Option<Duration> {
        Some(Duration::from_millis(50))
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn a_chain_refuses_a_debounced_link() {
    cache_init();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver);
    let err = Queue::chain()
        .add(ChainedDebouncedJob)
        .expect_err("a dropped link would strand the rest of the chain");
    assert!(err.to_string().contains("debounce"));
}

#[tokio::test]
#[serial]
async fn a_batch_refuses_a_debounced_job_at_dispatch() {
    cache_init();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver);
    let err = Queue::batch()
        .add(ChainedDebouncedJob)
        .dispatch()
        .await
        .expect_err("a dropped job would leave pending_jobs above zero forever");
    assert!(err.to_string().contains("debounce"));
}

// ---------------------------------------------------------------------------
// The listener tier
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct OrderUpdated {
    order_id: u32,
}

impl Event for OrderUpdated {
    fn event_name() -> &'static str {
        "queue_debounce::OrderUpdated"
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct ReindexOrder {
    order_id: u32,
}
static REINDEX_RUNS: AtomicU32 = AtomicU32::new(0);

#[async_trait]
impl Job for ReindexOrder {
    fn job_name() -> &'static str {
        "queue_debounce::ReindexOrder"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        REINDEX_RUNS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn a_debounced_listener_collapses_a_burst_of_events() {
    cache_init();
    REINDEX_RUNS.store(0, Ordering::SeqCst);
    register_job::<ReindexOrder>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    // The job itself declares no debounce; the window is the listener
    // registration's decision, and the key comes from the event.
    EventFacade::listen::<OrderUpdated, _>(Arc::new(
        DebouncedListener::<OrderUpdated, ReindexOrder>::new(Duration::from_millis(120), |e| {
            ReindexOrder {
                order_id: e.order_id,
            }
        })
        .keyed_by(|e| e.order_id.to_string()),
    ))
    .await;

    for _ in 0..4 {
        EventFacade::dispatch(OrderUpdated { order_id: 55 })
            .await
            .expect("dispatch");
    }

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| REINDEX_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();
    assert_eq!(
        REINDEX_RUNS.load(Ordering::SeqCst),
        1,
        "four events on one order must reindex once"
    );
}

// ---------------------------------------------------------------------------
// The window names only a dispatch that reached the queue (DRIVERS-050, -051)
// ---------------------------------------------------------------------------

/// Gates `JobQueueing` while `GATE_QUEUEING` is set: the dispatch that emits
/// it parks after arming its window and before its driver write.
static GATE_QUEUEING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct ParkQueueing {
    entered: Arc<Notify>,
}

#[async_trait]
impl suprnova::events::Listener<suprnova::queue::events::JobQueueing> for ParkQueueing {
    async fn handle(
        &self,
        _event: &suprnova::queue::events::JobQueueing,
    ) -> Result<(), FrameworkError> {
        if GATE_QUEUEING.load(Ordering::SeqCst) {
            self.entered.notify_one();
            std::future::pending::<()>().await;
        }
        Ok(())
    }
}

/// DRIVERS-051: the owner token was written when the window was armed, before
/// the push. A dispatch cancelled between the two - a client disconnect drops
/// an HTTP handler's future - left a token no envelope carries, and the
/// worker dropped the earlier, successfully queued envelope as superseded.
#[tokio::test]
#[serial]
async fn a_dispatch_cancelled_before_its_push_does_not_supersede_queued_work() {
    cache_init();
    SYNC_ORDER_RUNS.store(0, Ordering::SeqCst);
    SYNC_ORDER_LAST_REVISION.store(0, Ordering::SeqCst);
    register_job::<SyncOrder>();
    GATE_QUEUEING.store(false, Ordering::SeqCst);
    let entered = Arc::new(Notify::new());
    EventFacade::listen::<suprnova::queue::events::JobQueueing, _>(Arc::new(ParkQueueing {
        entered: entered.clone(),
    }))
    .await;

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::push(SyncOrder {
        order_id: 610,
        revision: 1,
    })
    .await
    .expect("the first dispatch is queued");

    GATE_QUEUEING.store(true, Ordering::SeqCst);
    let cancelled = tokio::spawn(async {
        Queue::push(SyncOrder {
            order_id: 610,
            revision: 2,
        })
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .expect("the second dispatch armed its window and parked before its push");
    cancelled.abort();
    assert!(cancelled.await.expect_err("aborted").is_cancelled());
    GATE_QUEUEING.store(false, Ordering::SeqCst);
    EventFacade::forget::<suprnova::queue::events::JobQueueing>();

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| SYNC_ORDER_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();
    assert_eq!(
        SYNC_ORDER_RUNS.load(Ordering::SeqCst),
        1,
        "the queued dispatch was dropped as superseded by one that never reached \
         the queue"
    );
    assert_eq!(SYNC_ORDER_LAST_REVISION.load(Ordering::SeqCst), 1);
}

/// DRIVERS-051, as recorded: A is queued and available; B arms and parks in a
/// driver write that will fail; a worker pops A meanwhile. A must run, since
/// B's push never succeeds.
#[tokio::test]
#[serial]
async fn queued_work_runs_while_a_newer_dispatch_is_still_pushing() {
    cache_init();
    SYNC_ORDER_RUNS.store(0, Ordering::SeqCst);
    SYNC_ORDER_LAST_REVISION.store(0, Ordering::SeqCst);
    register_job::<SyncOrder>();
    let options = || DebounceOptions::new(Duration::ZERO).id("611");

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::push_debounced(
        SyncOrder {
            order_id: 611,
            revision: 1,
        },
        options(),
    )
    .await
    .expect("A is queued and available at once");

    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    Queue::set_driver(Arc::new(GatedFailingQueueDriver {
        entered: entered.clone(),
        release: release.clone(),
    }));
    let parked = tokio::spawn(async move {
        Queue::push_debounced(
            SyncOrder {
                order_id: 611,
                revision: 2,
            },
            options(),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .expect("B armed its window and is inside its driver write");

    Queue::set_driver(driver.clone());
    let cfg = WorkerConfig {
        max_jobs: Some(1),
        ..worker_cfg()
    };
    tokio::time::timeout(
        Duration::from_secs(10),
        run_worker(driver.clone(), cfg, CancellationToken::new()),
    )
    .await
    .expect("the worker settled A");
    release.notify_one();
    parked
        .await
        .expect("join")
        .expect_err("B's driver write failed");

    assert_eq!(
        SYNC_ORDER_RUNS.load(Ordering::SeqCst),
        1,
        "A was acknowledged as superseded by B, whose push then failed: neither ran"
    );
    assert_eq!(SYNC_ORDER_LAST_REVISION.load(Ordering::SeqCst), 1);
}

/// Delegates to the in-memory cache, records the TTL of every write to a
/// first-dispatch stamp, and can park the first `forget` of one key.
struct ObservedCache {
    inner: InMemoryCache,
    stamp_ttls: std::sync::Mutex<Vec<Option<Duration>>>,
    gated_key: Option<String>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
    gate_used: std::sync::atomic::AtomicBool,
}

impl ObservedCache {
    fn new(gated_key: Option<&str>) -> Self {
        Self {
            inner: InMemoryCache::new(),
            stamp_ttls: std::sync::Mutex::new(Vec::new()),
            gated_key: gated_key.map(str::to_owned),
            entered: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
            gate_used: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl CacheStore for ObservedCache {
    async fn get_raw(&self, key: &str) -> Result<Option<String>, FrameworkError> {
        self.inner.get_raw(key).await
    }
    async fn put_raw(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), FrameworkError> {
        if is_stamp(key) {
            self.stamp_ttls.lock().unwrap().push(ttl);
        }
        self.inner.put_raw(key, value, ttl).await
    }
    async fn has(&self, key: &str) -> Result<bool, FrameworkError> {
        self.inner.has(key).await
    }
    async fn forget(&self, key: &str) -> Result<bool, FrameworkError> {
        if self.gated_key.as_deref() == Some(key) && !self.gate_used.swap(true, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.inner.forget(key).await
    }
    async fn flush(&self) -> Result<(), FrameworkError> {
        self.inner.flush().await
    }
    async fn increment(&self, key: &str, amount: i64) -> Result<i64, FrameworkError> {
        self.inner.increment(key, amount).await
    }
    async fn decrement(&self, key: &str, amount: i64) -> Result<i64, FrameworkError> {
        self.inner.decrement(key, amount).await
    }
    async fn tagged_put_raw(
        &self,
        tags: &[&str],
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), FrameworkError> {
        self.inner.tagged_put_raw(tags, key, value, ttl).await
    }
    async fn flush_tags(&self, tags: &[&str]) -> Result<(), FrameworkError> {
        self.inner.flush_tags(tags).await
    }
    async fn acquire_lock(
        &self,
        key: &str,
        ttl: Duration,
    ) -> Result<Option<String>, FrameworkError> {
        self.inner.acquire_lock(key, ttl).await
    }
    async fn release_lock(&self, key: &str, token: &str) -> Result<bool, FrameworkError> {
        self.inner.release_lock(key, token).await
    }
    async fn refresh_lock(
        &self,
        key: &str,
        token: &str,
        ttl: Duration,
    ) -> Result<bool, FrameworkError> {
        self.inner.refresh_lock(key, token, ttl).await
    }
    async fn touch(&self, key: &str, ttl: Duration) -> Result<bool, FrameworkError> {
        self.inner.touch(key, ttl).await
    }
}

/// DRIVERS-050: handing a failed dispatch's window back read the owner, then
/// forgot the key in a second step. A newer dispatch that armed and queued in
/// between lost its token, so the burst's earlier envelope failed open and
/// ran beside the newest one.
#[tokio::test]
#[serial]
async fn a_failed_dispatch_cannot_clear_a_window_a_newer_one_armed_meanwhile() {
    SYNC_ORDER_RUNS.store(0, Ordering::SeqCst);
    SYNC_ORDER_LAST_REVISION.store(0, Ordering::SeqCst);
    register_job::<SyncOrder>();
    let key = "queue-debounce:queue_debounce::SyncOrder:612";
    let cache = Arc::new(ObservedCache::new(Some(key)));
    let (entered, release) = (cache.entered.clone(), cache.release.clone());
    let _container = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(cache);

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::push(SyncOrder {
        order_id: 612,
        revision: 1,
    })
    .await
    .expect("first");

    Queue::set_driver(Arc::new(RefusingQueueDriver));
    let mut failing = tokio::spawn(async {
        Queue::push(SyncOrder {
            order_id: 612,
            revision: 2,
        })
        .await
    });
    // A dispatch that hands its window back parks inside that cleanup; one
    // that never claimed the window has nothing to clean up and just fails.
    let cleanup_parked = tokio::select! {
        () = entered.notified() => true,
        result = &mut failing => {
            result.expect("join").expect_err("the driver refused the write");
            false
        }
    };

    Queue::set_driver(driver.clone());
    Queue::push(SyncOrder {
        order_id: 612,
        revision: 3,
    })
    .await
    .expect("third");
    if cleanup_parked {
        release.notify_one();
        failing
            .await
            .expect("join")
            .expect_err("the driver refused the write");
    }

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| SYNC_ORDER_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();
    assert_eq!(
        SYNC_ORDER_RUNS.load(Ordering::SeqCst),
        1,
        "the failed dispatch's cleanup deleted the newer dispatch's window, so the \
         burst ran twice"
    );
    assert_eq!(SYNC_ORDER_LAST_REVISION.load(Ordering::SeqCst), 3);
}

#[derive(Serialize, Deserialize, Clone)]
struct LongBurstJob;

#[async_trait]
impl Job for LongBurstJob {
    fn job_name() -> &'static str {
        "queue_debounce::LongBurstJob"
    }
    fn debounce_for() -> Option<Duration> {
        Some(Duration::from_secs(5))
    }
    fn max_debounce_wait() -> Option<Duration> {
        Some(Duration::from_secs(600))
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// DRIVERS-052: the first-dispatch stamp lived `max(window * 10, 300s)`, so a
/// 600-second max wait outlasted it. A continuous burst renewed its owner
/// token on every dispatch while the stamp expired and was re-created, and
/// the forced run never came.
#[tokio::test]
#[serial]
async fn the_first_dispatch_stamp_outlives_the_max_wait() {
    let cache = Arc::new(ObservedCache::new(None));
    let _container = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(cache.clone());
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    Queue::push(LongBurstJob).await.expect("push");

    let ttls = cache.stamp_ttls.lock().unwrap().clone();
    assert_eq!(ttls.len(), 1, "the first dispatch stamps the burst");
    let ttl = ttls[0].expect("the stamp has a TTL");
    assert!(
        ttl > Duration::from_secs(600),
        "the stamp expires after {ttl:?}, before the 600-second max wait can be \
         measured against it"
    );
}

// ---------------------------------------------------------------------------
// Queue::bulk honors a declared window (DRIVERS-064)
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn bulk_collapses_a_debounced_burst_onto_its_last_job() {
    cache_init();
    SYNC_ORDER_RUNS.store(0, Ordering::SeqCst);
    SYNC_ORDER_LAST_REVISION.store(0, Ordering::SeqCst);
    register_job::<SyncOrder>();

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::bulk(
        (1..=3)
            .map(|revision| SyncOrder {
                order_id: 613,
                revision,
            })
            .collect(),
    )
    .await
    .expect("bulk");

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| SYNC_ORDER_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();
    assert_eq!(
        SYNC_ORDER_RUNS.load(Ordering::SeqCst),
        1,
        "a debounced job pushed in bulk ran every copy"
    );
    assert_eq!(SYNC_ORDER_LAST_REVISION.load(Ordering::SeqCst), 3);
}

#[tokio::test]
#[serial]
async fn bulk_refuses_a_job_declaring_debounce_and_uniqueness() {
    cache_init();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    let err = Queue::bulk(vec![ConfusedJob, ConfusedJob])
        .await
        .expect_err("bulk must refuse the conflicting declarations too");
    assert!(
        err.to_string().contains("debounce_for") && err.to_string().contains("unique_id"),
        "{err}"
    );
    assert_eq!(driver.size().await.expect("size"), 0);
}

// ---------------------------------------------------------------------------
// Overlapping dispatches that all succeed keep the one that armed last
// ---------------------------------------------------------------------------

/// Delegates to a memory driver, except that `bulk_push` parks until released:
/// a bulk can be held after it armed its windows and before it claims them.
struct GatedBulkDriver {
    inner: Arc<MemoryQueueDriver>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

#[async_trait]
impl QueueDriver for GatedBulkDriver {
    async fn push(&self, env: suprnova::queue::Envelope) -> Result<(), FrameworkError> {
        self.inner.push(env).await
    }
    async fn bulk_push(&self, envs: Vec<suprnova::queue::Envelope>) -> Result<(), FrameworkError> {
        self.entered.notify_one();
        self.release.notified().await;
        self.inner.bulk_push(envs).await
    }
    async fn pop(&self, vt: Duration) -> Result<Option<Reservation>, FrameworkError> {
        self.inner.pop(vt).await
    }
    async fn ack(&self, token: &ReservationToken) -> Result<(), FrameworkError> {
        self.inner.ack(token).await
    }
    async fn nack(&self, token: &ReservationToken, delay: Duration) -> Result<(), FrameworkError> {
        self.inner.nack(token, delay).await
    }
}

/// Sol review: places were derived from the claim a dispatch read, so they
/// were not reserved. Bulk A armed places 1 to 3 and stalled before its
/// claim; push B, newer, read the same claim, took place 1 and claimed it; A
/// then claimed place 3, and the worker dropped B's payload, the newest, as
/// superseded by A's older one.
#[tokio::test]
#[serial]
async fn a_push_made_while_a_bulk_is_claiming_still_wins() {
    cache_init();
    SYNC_ORDER_RUNS.store(0, Ordering::SeqCst);
    SYNC_ORDER_LAST_REVISION.store(0, Ordering::SeqCst);
    register_job::<SyncOrder>();

    let driver = Arc::new(MemoryQueueDriver::new());
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    Queue::set_driver(Arc::new(GatedBulkDriver {
        inner: driver.clone(),
        entered: entered.clone(),
        release: release.clone(),
    }));
    let bulk = tokio::spawn(Queue::bulk(
        (1..=3)
            .map(|revision| SyncOrder {
                order_id: 650,
                revision,
            })
            .collect(),
    ));
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .expect("A armed its three jobs and is inside its write");

    Queue::set_driver(driver.clone());
    Queue::push(SyncOrder {
        order_id: 650,
        revision: 4,
    })
    .await
    .expect("B is queued and claims the window");
    release.notify_one();
    bulk.await.expect("join").expect("A is queued");

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| SYNC_ORDER_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();
    assert_eq!(
        SYNC_ORDER_LAST_REVISION.load(Ordering::SeqCst),
        4,
        "B armed after every job of A, so B's payload is the one that runs"
    );
    assert_eq!(
        SYNC_ORDER_RUNS.load(Ordering::SeqCst),
        1,
        "and A, whose claim came last but from an earlier place, runs nothing"
    );
}

/// Parks the first `JobQueueing` after `armed` is set: that dispatch has
/// armed its window and not yet written its envelope.
struct HoldQueueing {
    armed: Arc<std::sync::atomic::AtomicBool>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

#[async_trait]
impl suprnova::events::Listener<suprnova::queue::events::JobQueueing> for HoldQueueing {
    async fn handle(
        &self,
        _event: &suprnova::queue::events::JobQueueing,
    ) -> Result<(), FrameworkError> {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        Ok(())
    }
}

/// Sol review: two pushes that overlap read the same claim and took the same
/// place, so their random ids decided which payload ran. P1 arms and parks
/// before its write; P2 arms after it, writes and claims; then P1 writes and
/// claims. P2 armed last, so P2 is the one that runs.
#[tokio::test]
#[serial]
async fn overlapping_pushes_keep_the_one_that_armed_last() {
    cache_init();
    SYNC_ORDER_RUNS.store(0, Ordering::SeqCst);
    SYNC_ORDER_LAST_REVISION.store(0, Ordering::SeqCst);
    register_job::<SyncOrder>();
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    EventFacade::listen::<suprnova::queue::events::JobQueueing, _>(Arc::new(HoldQueueing {
        armed: armed.clone(),
        entered: entered.clone(),
        release: release.clone(),
    }))
    .await;

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    let first = tokio::spawn(Queue::push(SyncOrder {
        order_id: 651,
        revision: 1,
    }));
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .expect("P1 armed its window and parked before its write");
    Queue::push(SyncOrder {
        order_id: 651,
        revision: 2,
    })
    .await
    .expect("P2 is queued and claims the window");
    release.notify_one();
    first.await.expect("join").expect("P1 is queued");
    EventFacade::forget::<suprnova::queue::events::JobQueueing>();

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| SYNC_ORDER_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();
    assert_eq!(
        SYNC_ORDER_RUNS.load(Ordering::SeqCst),
        1,
        "the two dispatches collapse into one run"
    );
    assert_eq!(
        SYNC_ORDER_LAST_REVISION.load(Ordering::SeqCst),
        2,
        "and the run is P2's, the dispatch that armed last"
    );
}

// ---------------------------------------------------------------------------
// Max wait reached inside one bulk forces the run the bulk claims
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct BulkRollUp {
    revision: u32,
}
static BULK_ROLLUP_RUNS: AtomicU32 = AtomicU32::new(0);
static BULK_ROLLUP_LAST: AtomicU32 = AtomicU32::new(0);

#[async_trait]
impl Job for BulkRollUp {
    fn job_name() -> &'static str {
        "queue_debounce::BulkRollUp"
    }
    fn debounce_for() -> Option<Duration> {
        Some(Duration::from_secs(3600)) // never elapses inside this test
    }
    fn max_debounce_wait() -> Option<Duration> {
        Some(Duration::from_secs(0)) // every dispatch after the first is overdue
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        BULK_ROLLUP_RUNS.fetch_add(1, Ordering::SeqCst);
        BULK_ROLLUP_LAST.store(self.revision, Ordering::SeqCst);
        Ok(())
    }
}

/// Sol review: max wait fired on the bulk's second job, which went out at
/// once and cleared the burst's stamp. The third job stamped the burst
/// again, took the ordinary hour-long delay and claimed the window, so the
/// forced run was dropped as superseded and the burst waited after all. Bulks
/// sent faster than the window could defeat max wait forever.
#[tokio::test]
#[serial]
async fn max_wait_reached_inside_a_bulk_runs_the_bulks_last_job_at_once() {
    cache_init();
    BULK_ROLLUP_RUNS.store(0, Ordering::SeqCst);
    BULK_ROLLUP_LAST.store(0, Ordering::SeqCst);
    register_job::<BulkRollUp>();
    Cache::forget("queue-debounce-first-dispatched:queue-debounce:queue_debounce::BulkRollUp:")
        .await
        .expect("cache");

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::bulk((1..=3).map(|revision| BulkRollUp { revision }).collect())
        .await
        .expect("bulk");

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| BULK_ROLLUP_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();
    assert_eq!(
        BULK_ROLLUP_RUNS.load(Ordering::SeqCst),
        1,
        "max wait fired inside the bulk, so the bulk runs now, not in an hour"
    );
    assert_eq!(
        BULK_ROLLUP_LAST.load(Ordering::SeqCst),
        3,
        "and the forced run is the bulk's last job"
    );
}

// ---------------------------------------------------------------------------
// One id's first-dispatch stamp is never another id's owner key
// ---------------------------------------------------------------------------

/// The stamp of id `660` lived at its owner key plus `:first_dispatched_at`,
/// which is the owner key of id `660:first_dispatched_at`. The stamp's number
/// and the other id's token then shared one key, and whichever was written
/// second failed the other id's dispatches.
#[tokio::test]
#[serial]
async fn a_debounce_id_never_shares_a_key_with_another_ids_stamp() {
    cache_init();
    register_job::<SyncOrder>();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    let owner_key = "queue-debounce:queue_debounce::SyncOrder:660:first_dispatched_at";

    Queue::push_debounced(
        SyncOrder {
            order_id: 660,
            revision: 1,
        },
        DebounceOptions::new(Duration::from_millis(120)).id("660:first_dispatched_at"),
    )
    .await
    .expect("the id ending in `:first_dispatched_at` is queued");
    let token = Cache::get::<String>(owner_key)
        .await
        .expect("cache")
        .expect("and has claimed its window");

    Queue::push_debounced(
        SyncOrder {
            order_id: 660,
            revision: 2,
        },
        DebounceOptions::new(Duration::from_millis(120))
            .max_wait(Duration::from_secs(600))
            .id("660"),
    )
    .await
    .expect("id 660 stamps its burst without reading the other id's token as a stamp");
    assert_eq!(
        Cache::get::<String>(owner_key).await.expect("cache"),
        Some(token),
        "id 660's stamp did not overwrite the other id's token"
    );
}

/// A stamp left at that key, by id `661` before stamps moved or by an
/// upgrade, must not fail the dispatches of id `661:first_dispatched_at` for
/// as long as the stamp lives: it is not a token, so the window is unowned.
#[tokio::test]
#[serial]
async fn a_stamp_left_at_an_owner_key_does_not_block_that_id() {
    cache_init();
    SYNC_ORDER_RUNS.store(0, Ordering::SeqCst);
    SYNC_ORDER_LAST_REVISION.store(0, Ordering::SeqCst);
    register_job::<SyncOrder>();
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Cache::put(
        "queue-debounce:queue_debounce::SyncOrder:661:first_dispatched_at",
        &suprnova::clock::now().timestamp(),
        Some(Duration::from_secs(900)),
    )
    .await
    .expect("cache");

    Queue::push_debounced(
        SyncOrder {
            order_id: 661,
            revision: 1,
        },
        DebounceOptions::new(Duration::from_millis(120)).id("661:first_dispatched_at"),
    )
    .await
    .expect("a stamp at the owner key leaves the window unowned");

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| SYNC_ORDER_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();
    assert_eq!(SYNC_ORDER_RUNS.load(Ordering::SeqCst), 1);
    assert_eq!(SYNC_ORDER_LAST_REVISION.load(Ordering::SeqCst), 1);
}

// ---------------------------------------------------------------------------
// The place counter on the Redis cache store
// ---------------------------------------------------------------------------

/// Where the Redis test runs: `REDIS_TEST_URL`, which the gate sets to its
/// own database, or else a database no other suite on this machine uses.
fn debounce_redis_url() -> String {
    std::env::var("REDIS_TEST_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/7".to_owned())
}

/// The place counter against Redis: `Cache::increment` reserves each place,
/// `Cache::touch` gives the counter a TTL, and a counter that lapsed while a
/// claim lives continues past that claim rather than restarting below it.
#[tokio::test]
#[serial]
#[ignore = "needs Redis: REDIS_TEST_URL, or redis://127.0.0.1:6379/7"]
async fn redis_place_counter_reserves_each_place_once_and_expires() {
    use redis::AsyncCommands;
    use suprnova::cache::{CacheConfig, CacheDriver, RedisCache};

    SYNC_ORDER_RUNS.store(0, Ordering::SeqCst);
    SYNC_ORDER_LAST_REVISION.store(0, Ordering::SeqCst);
    register_job::<SyncOrder>();
    let url = debounce_redis_url();
    // A prefix of this run's own: no flush, and nothing another run wrote.
    let prefix = format!("debounce-place-test-{}:", uuid::Uuid::new_v4().simple());
    let store = RedisCache::connect(&CacheConfig {
        driver: CacheDriver::Redis,
        url: url.clone(),
        prefix: prefix.clone(),
        default_ttl: 0,
        sweep_interval: 0,
    })
    .await
    .expect("connect to the test Redis");
    let _container = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(store));
    let mut redis =
        redis::aio::ConnectionManager::new(redis::Client::open(url.as_str()).expect("a Redis URL"))
            .await
            .expect("a direct Redis connection");
    // A run that failed before its cleanup left its keys, the counter
    // without a TTL among them when the TTL is what failed. Only this test
    // writes under this prefix.
    let leftovers: Vec<String> = redis
        .keys("debounce-place-test-*")
        .await
        .expect("list earlier runs' keys");
    if !leftovers.is_empty() {
        let _: i64 = redis
            .del(leftovers)
            .await
            .expect("remove earlier runs' keys");
    }
    let counter =
        format!("{prefix}queue-debounce-place:queue-debounce:queue_debounce::SyncOrder:670");
    let owner = format!("{prefix}queue-debounce:queue_debounce::SyncOrder:670");
    let owner_place = |token: String| {
        let token: String = serde_json::from_str(&token).expect("a JSON string token");
        token
            .split_once(':')
            .and_then(|(place, _)| place.parse::<u64>().ok())
            .expect("a placed token")
    };

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    for revision in 1..=3 {
        Queue::push(SyncOrder {
            order_id: 670,
            revision,
        })
        .await
        .expect("push");
    }
    let reserved: i64 = redis.get(&counter).await.expect("read the counter");
    assert_eq!(reserved, 3, "three dispatches, three places");
    let ttl: i64 = redis.pttl(&counter).await.expect("read the counter's TTL");
    assert!(
        ttl > 0 && ttl <= 300_000,
        "the counter expires with the window's token TTL, not never: {ttl} ms"
    );
    let claimed: String = redis.get(&owner).await.expect("read the claim");
    assert_eq!(
        owner_place(claimed),
        3,
        "the last dispatch holds the window"
    );

    // The counter lapses while the claim still lives.
    let _: i64 = redis.del(&counter).await.expect("drop the counter");
    Queue::push(SyncOrder {
        order_id: 670,
        revision: 4,
    })
    .await
    .expect("push after the counter lapsed");
    let claimed: String = redis.get(&owner).await.expect("read the claim");
    assert_eq!(
        owner_place(claimed),
        4,
        "a restarted counter moves past the live claim instead of reusing place 1"
    );

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| SYNC_ORDER_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();
    let keys: Vec<String> = redis
        .keys(format!("{prefix}*"))
        .await
        .expect("list this run's keys");
    if !keys.is_empty() {
        let _: i64 = redis.del(keys).await.expect("remove this run's keys");
    }
    assert_eq!(
        SYNC_ORDER_RUNS.load(Ordering::SeqCst),
        1,
        "four dispatches against Redis collapse into one run"
    );
    assert_eq!(SYNC_ORDER_LAST_REVISION.load(Ordering::SeqCst), 4);
}

// ---------------------------------------------------------------------------
// A dropped envelope reports itself as Laravel's worker does
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct OrderedSupersession {
    dispatch: u32,
}
static ORDERED_RUNS: AtomicU32 = AtomicU32::new(0);

#[async_trait]
impl Job for OrderedSupersession {
    fn job_name() -> &'static str {
        "queue_debounce::OrderedSupersession"
    }
    fn debounce_for() -> Option<Duration> {
        Some(Duration::from_millis(120))
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        ORDERED_RUNS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// The lifecycle events of `OrderedSupersession`, by envelope, in the order
/// the worker fired them.
static ORDERED_FIRED: std::sync::Mutex<Vec<(uuid::Uuid, &'static str)>> =
    std::sync::Mutex::new(Vec::new());

struct RecordOrder;

fn record_order(job: &suprnova::queue::events::JobIdentity, name: &'static str) {
    if job.job_name == "queue_debounce::OrderedSupersession" {
        ORDERED_FIRED.lock().unwrap().push((job.id, name));
    }
}

#[async_trait]
impl suprnova::events::Listener<JobDebounced> for RecordOrder {
    async fn handle(&self, event: &JobDebounced) -> Result<(), FrameworkError> {
        record_order(&event.job, "debounced");
        Ok(())
    }
}

#[async_trait]
impl suprnova::events::Listener<suprnova::queue::events::JobProcessed> for RecordOrder {
    async fn handle(
        &self,
        event: &suprnova::queue::events::JobProcessed,
    ) -> Result<(), FrameworkError> {
        record_order(&event.job, "processed");
        Ok(())
    }
}

#[async_trait]
impl suprnova::events::Listener<suprnova::queue::events::JobAttempted> for RecordOrder {
    async fn handle(
        &self,
        event: &suprnova::queue::events::JobAttempted,
    ) -> Result<(), FrameworkError> {
        record_order(&event.job, "attempted");
        Ok(())
    }
}

/// Laravel deletes a superseded debounced job inside its pipeline, which
/// then returns without throwing, so the worker fires JobDebounced, then
/// JobProcessed, then JobAttempted for it. The worker fired no JobProcessed.
#[tokio::test]
#[serial]
async fn a_dropped_envelope_fires_debounced_processed_and_attempted_in_that_order() {
    cache_init();
    ORDERED_RUNS.store(0, Ordering::SeqCst);
    ORDERED_FIRED.lock().unwrap().clear();
    register_job::<OrderedSupersession>();
    EventFacade::listen::<JobDebounced, _>(Arc::new(RecordOrder)).await;
    EventFacade::listen::<suprnova::queue::events::JobProcessed, _>(Arc::new(RecordOrder)).await;
    EventFacade::listen::<suprnova::queue::events::JobAttempted, _>(Arc::new(RecordOrder)).await;

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    Queue::push(OrderedSupersession { dispatch: 1 })
        .await
        .expect("first");
    Queue::push(OrderedSupersession { dispatch: 2 })
        .await
        .expect("second");

    let handle = tokio::spawn(run_worker(
        driver.clone(),
        worker_cfg(),
        CancellationToken::new(),
    ));
    settle(|| ORDERED_RUNS.load(Ordering::SeqCst) > 0).await;
    handle.abort();
    EventFacade::forget::<JobDebounced>();
    EventFacade::forget::<suprnova::queue::events::JobProcessed>();
    EventFacade::forget::<suprnova::queue::events::JobAttempted>();

    let fired = ORDERED_FIRED.lock().unwrap().clone();
    let of = |id: uuid::Uuid| -> Vec<&'static str> {
        fired
            .iter()
            .filter(|(fired_for, _)| *fired_for == id)
            .map(|(_, name)| *name)
            .collect()
    };
    let dropped = fired
        .iter()
        .find(|(_, name)| *name == "debounced")
        .map(|(id, _)| *id)
        .expect("one envelope was dropped as superseded");
    let survivor = fired
        .iter()
        .map(|(id, _)| *id)
        .find(|id| *id != dropped)
        .expect("and one ran");
    assert_eq!(
        of(dropped),
        ["debounced", "processed", "attempted"],
        "the dropped envelope reports as Laravel's worker does"
    );
    assert_eq!(of(survivor), ["processed", "attempted"]);
}
