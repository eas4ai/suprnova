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
async fn a_push_that_fails_after_arming_lets_the_window_lapse() {
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

    // The second dispatch arms the window - overwriting the first dispatch's
    // token - and then fails to enqueue anything to carry it.
    Queue::set_driver(Arc::new(RefusingQueueDriver));
    Queue::push(SyncOrder {
        order_id: 900,
        revision: 2,
    })
    .await
    .expect_err("the driver refused the write");

    assert!(
        Cache::get::<String>("queue-debounce:queue_debounce::SyncOrder:900")
            .await
            .expect("cache")
            .is_none(),
        "a window whose envelope never reached the queue must lapse, not stand as \
         an owner nothing can satisfy"
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
    let stamp_key = "queue-debounce:queue_debounce::CompactLedger::first_dispatched_at";
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
    key.ends_with(":first_dispatched_at")
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

/// The owner token is written before the max-wait bookkeeping runs, so an
/// arming that fails halfway would otherwise leave a token in the cache that no
/// envelope carries - and every earlier envelope of the burst, whose own push
/// returned `Ok`, would be dropped at the worker as superseded by a dispatch
/// that never completed. Only jobs declaring `max_debounce_wait` reach that
/// bookkeeping at all, which is the manual's headline example.
#[tokio::test]
#[serial]
async fn an_arming_that_fails_halfway_hands_the_window_back() {
    cache_init();
    COMPACT_RUNS.store(0, Ordering::SeqCst);
    register_job::<CompactLedger>();
    Cache::forget("queue-debounce:queue_debounce::CompactLedger::first_dispatched_at")
        .await
        .expect("cache");

    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());

    // A arms cleanly and is enqueued. Its push returned Ok, so its work is
    // owed.
    Queue::push(CompactLedger).await.expect("first");

    {
        // B writes its owner token over A's, then fails reading the timestamp
        // key.
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
        "an arming that could not complete must hand its window back, so the \
         envelope already on the queue still runs"
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

/// Handing a window back has to be owner-checked, or the cleanup becomes the
/// opposite bug: a dispatch whose write fails slowly tears down a window a
/// newer dispatch has since armed and filled, and the whole burst un-collapses.
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

    // C arms over B's token and is enqueued. B's cleanup must not touch it.
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
        "an unconditional cleanup would delete the live owner token, and every \
         queued envelope of the burst would fail open and run"
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
