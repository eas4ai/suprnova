//! `Queue::push_unique` enqueue gating.
//!
//! The dedupe key is `queue-unique:<job_name>:<id>`; a second `push_unique`
//! for the same key within `Job::unique_for()` returns `Ok(false)` and does
//! NOT publish a second envelope to the driver. The TTL test below uses a
//! short-lived `unique_for` so the second call escapes the dedupe window
//! without sleeping for minutes.

use serde::{Deserialize, Serialize};
use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;
use suprnova::App;
use suprnova::cache::{CacheStore, InMemoryCache};
use suprnova::queue::Queue;
use suprnova::queue::driver::QueueDriver;
use suprnova::queue::memory::MemoryQueueDriver;
use suprnova::{FrameworkError, Job, async_trait};

#[derive(Serialize, Deserialize, Clone)]
struct UniqueJob {
    id: u32,
}

#[async_trait]
impl Job for UniqueJob {
    fn job_name() -> &'static str {
        "UniqueJob"
    }
    fn unique_id(&self) -> Option<String> {
        Some(self.id.to_string())
    }
    fn unique_for() -> Duration {
        // Long enough to test "second push is rejected" reliably; the TTL
        // test below uses a different short-lived job to avoid sleeping.
        Duration::from_secs(60)
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct ShortTtlJob {
    id: u32,
}

#[async_trait]
impl Job for ShortTtlJob {
    fn job_name() -> &'static str {
        "ShortTtlJob"
    }
    fn unique_id(&self) -> Option<String> {
        Some(self.id.to_string())
    }
    fn unique_for() -> Duration {
        Duration::from_millis(700)
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct NoUniqueIdJob;

#[async_trait]
impl Job for NoUniqueIdJob {
    fn job_name() -> &'static str {
        "NoUniqueIdJob"
    }
    // Inherits the default `unique_id() -> None`.
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

async fn install_memory_drivers() {
    // `Cache::bootstrap` is `pub(crate)` because it reads `CacheConfig` from
    // env; in tests we bind the in-memory store directly so the dedupe
    // lock has a backing store without depending on env state.
    App::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    Queue::set_driver(Arc::new(MemoryQueueDriver::new()));
}

async fn pop_all(driver: &Arc<dyn QueueDriver>) -> usize {
    let mut n = 0;
    while let Some(res) = driver.pop(Duration::from_millis(50)).await.unwrap() {
        driver.ack(&res.token).await.unwrap();
        n += 1;
    }
    n
}

#[tokio::test]
#[serial]
async fn push_unique_suppresses_a_duplicate_within_the_window() {
    install_memory_drivers().await;
    // Pre-cleanup: a prior test may have left an envelope in the registered
    // driver (we re-install per-test, but the test order isn't guaranteed).
    let drv = Queue::driver().unwrap();
    let _ = pop_all(&drv).await;

    let first = Queue::push_unique(UniqueJob { id: 1 }).await.unwrap();
    assert!(first, "first push must enqueue (Fresh)");

    let second = Queue::push_unique(UniqueJob { id: 1 }).await.unwrap();
    assert!(
        !second,
        "second push within unique_for must be suppressed (Duplicate)"
    );

    let drained = pop_all(&drv).await;
    assert_eq!(
        drained, 1,
        "exactly one envelope was published to the driver"
    );
}

#[tokio::test]
#[serial]
async fn push_unique_lets_different_ids_through() {
    install_memory_drivers().await;
    let drv = Queue::driver().unwrap();
    let _ = pop_all(&drv).await;

    assert!(Queue::push_unique(UniqueJob { id: 10 }).await.unwrap());
    assert!(Queue::push_unique(UniqueJob { id: 11 }).await.unwrap());
    let drained = pop_all(&drv).await;
    assert_eq!(drained, 2, "different unique_ids enqueue independently");
}

#[tokio::test]
#[serial]
async fn push_unique_re_enqueues_after_ttl_expires() {
    install_memory_drivers().await;
    let drv = Queue::driver().unwrap();
    let _ = pop_all(&drv).await;

    assert!(Queue::push_unique(ShortTtlJob { id: 1 }).await.unwrap());

    // Within the 700ms window — still a duplicate.
    assert!(!Queue::push_unique(ShortTtlJob { id: 1 }).await.unwrap());

    // Past the window — the dedupe key has expired so a fresh push lands.
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(Queue::push_unique(ShortTtlJob { id: 1 }).await.unwrap());

    let drained = pop_all(&drv).await;
    assert_eq!(drained, 2, "two envelopes after the TTL window elapses");
}

#[tokio::test]
#[serial]
async fn push_unique_errors_when_unique_id_returns_none() {
    install_memory_drivers().await;
    let err = Queue::push_unique(NoUniqueIdJob).await.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("unique_id"),
        "error must name the missing trait method: {msg}"
    );
}

#[tokio::test]
#[serial]
async fn push_unique_populates_envelope_idempotency_key() {
    install_memory_drivers().await;
    let drv = Queue::driver().unwrap();
    let _ = pop_all(&drv).await;

    assert!(Queue::push_unique(UniqueJob { id: 42 }).await.unwrap());

    let res = drv
        .pop(Duration::from_millis(50))
        .await
        .unwrap()
        .expect("envelope present");
    assert_eq!(
        res.envelope.idempotency_key.as_deref(),
        Some("42"),
        "envelope must carry the unique_id for log correlation"
    );
    drv.ack(&res.token).await.unwrap();
}

// ---------------------------------------------------------------------------
// A lost dedupe lease is still a pushed job
// ---------------------------------------------------------------------------
//
// `Idempotency::commit_on_success` returns `FreshUnfenced` when the body ran
// to completion but the lock's lease was lost partway through. For
// `push_unique` the body IS the driver push, so `FreshUnfenced` means the
// envelope is on the queue and only the uniqueness claim is unproven.
// Reporting `false` — which this API documents as "suppressed as a
// duplicate" — tells the caller the opposite of what happened.

use std::sync::atomic::{AtomicU32, Ordering};
use suprnova::queue::Envelope;
use suprnova::queue::driver::{Reservation, ReservationToken};
use tokio::sync::Notify;

/// A cache whose `refresh_lock` always reports the token as no longer ours,
/// and signals `refreshed` at the moment it does. Everything else delegates
/// to a real in-memory store, so only the lease renewal misbehaves, and the
/// signal is what makes the fault deterministic: `SlowPushDriver::push`
/// below waits on it directly instead of racing a sleep duration against
/// the renewal interval.
struct LeaseLostCache {
    inner: InMemoryCache,
    refreshed: Arc<Notify>,
}

impl LeaseLostCache {
    fn new(refreshed: Arc<Notify>) -> Self {
        Self {
            inner: InMemoryCache::new(),
            refreshed,
        }
    }
}

#[async_trait]
impl CacheStore for LeaseLostCache {
    async fn get_raw(&self, key: &str) -> Result<Option<String>, FrameworkError> {
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
    async fn touch(&self, key: &str, ttl: Duration) -> Result<bool, FrameworkError> {
        self.inner.touch(key, ttl).await
    }
    async fn refresh_lock(
        &self,
        _key: &str,
        _token: &str,
        _ttl: Duration,
    ) -> Result<bool, FrameworkError> {
        // `Ok(false)` is the *definite* loss signal: the token no longer
        // matches, so somebody else holds this lock right now. An `Err`
        // would only be a backend blip, which the lease loop deliberately
        // rides out.
        //
        // Notifying here forces the ordering the test depends on: the push
        // below cannot complete until this refresh has actually run, so
        // "the lease was lost while the push was still in flight" is a
        // guaranteed precondition, not a race between two durations.
        self.refreshed.notify_one();
        Ok(false)
    }
}

/// A driver whose `push` blocks until `LeaseLostCache::refresh_lock` above
/// has fired, then completes. `Idempotency::commit_on_success` polls the
/// body and the lease-renewal task with a biased `select!` that always
/// checks the body first, so the renewal task only gets to run refresh_lock
/// while the body is still pending — which `push` guarantees here by
/// waiting on the same signal `refresh_lock` sends.
struct SlowPushDriver {
    inner: MemoryQueueDriver,
    pushes: AtomicU32,
    refreshed: Arc<Notify>,
}

impl SlowPushDriver {
    fn new(refreshed: Arc<Notify>) -> Self {
        Self {
            inner: MemoryQueueDriver::new(),
            pushes: AtomicU32::new(0),
            refreshed,
        }
    }
}

#[async_trait]
impl QueueDriver for SlowPushDriver {
    async fn push(&self, env: Envelope) -> Result<(), FrameworkError> {
        // A generous timeout turns "the timing assumption broke" into a
        // loud test failure instead of a hung test suite, without weakening
        // the forced ordering above.
        tokio::time::timeout(Duration::from_secs(5), self.refreshed.notified())
            .await
            .map_err(|_| {
                FrameworkError::internal(
                    "lease-renewal refresh_lock never fired before the timeout; \
                     the ordering this test depends on no longer holds",
                )
            })?;
        self.pushes.fetch_add(1, Ordering::SeqCst);
        self.inner.push(env).await
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
        self.inner.ack(token).await
    }
    async fn nack(
        &self,
        token: &ReservationToken,
        requeue_delay: Duration,
    ) -> Result<(), FrameworkError> {
        self.inner.nack(token, requeue_delay).await
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct UnfencedJob {
    id: u32,
}

#[async_trait]
impl Job for UnfencedJob {
    fn job_name() -> &'static str {
        "UnfencedJob"
    }
    fn unique_id(&self) -> Option<String> {
        Some(self.id.to_string())
    }
    fn unique_for() -> Duration {
        // The lease renewal interval is `ttl / 3`, floored at 50ms, so 150ms
        // keeps the first refresh (and this test) fast. The push no longer
        // needs to outlast that interval — `SlowPushDriver::push` waits on
        // the refresh signal directly — so this value only needs to be
        // short, not tuned against a race.
        Duration::from_millis(150)
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[tokio::test]
#[serial]
async fn push_unique_reports_true_when_the_lease_is_lost_mid_push() {
    let refreshed = Arc::new(Notify::new());
    App::bind::<dyn CacheStore>(Arc::new(LeaseLostCache::new(refreshed.clone())));
    Queue::set_driver(Arc::new(SlowPushDriver::new(refreshed)));
    let drv = Queue::driver().unwrap();
    let _ = pop_all(&drv).await;

    let pushed = Queue::push_unique(UnfencedJob { id: 7 })
        .await
        .expect("the push itself succeeded, so the call must not error");

    assert!(
        pushed,
        "the envelope WAS published to the driver. FreshUnfenced means the \
         dedupe lease was lost, not that the push was suppressed — reporting \
         false tells the caller a job that is about to run was skipped"
    );

    let drained = pop_all(&drv).await;
    assert_eq!(
        drained, 1,
        "exactly one envelope reached the driver; this fix is about what the \
         boolean says, not about pushing twice"
    );
}
