use serial_test::serial;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use suprnova::cache::InMemoryCache;
use suprnova::cache::store::CacheStore;
use suprnova::container::App;
use suprnova::idempotency::{Idempotency, Idempotent, Replay};

static RAN: AtomicU32 = AtomicU32::new(0);

fn install_memory_cache() {
    let store: Arc<dyn CacheStore> = Arc::new(InMemoryCache::with_prefix("idem:"));
    App::bind::<dyn CacheStore>(store);
}

#[tokio::test]
#[serial]
async fn first_call_runs_body_subsequent_call_is_duplicate() {
    RAN.store(0, Ordering::SeqCst);
    install_memory_cache();

    let r1: Idempotent<i32> = Idempotency::once("k-1", Duration::from_secs(60), || async {
        RAN.fetch_add(1, Ordering::SeqCst);
        Ok(42_i32)
    })
    .await
    .unwrap();
    assert!(matches!(r1, Idempotent::Fresh(42)));
    assert_eq!(RAN.load(Ordering::SeqCst), 1);

    let r2: Idempotent<i32> = Idempotency::once("k-1", Duration::from_secs(60), || async {
        RAN.fetch_add(1, Ordering::SeqCst);
        Ok(99_i32)
    })
    .await
    .unwrap();
    assert!(matches!(r2, Idempotent::Duplicate));
    assert_eq!(
        RAN.load(Ordering::SeqCst),
        1,
        "body must not run for duplicate key"
    );
}

#[tokio::test]
#[serial]
async fn key_expires_after_ttl() {
    install_memory_cache();
    let _ = Idempotency::once::<_, _, ()>("k-2", Duration::from_millis(50), || async { Ok(()) })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let r = Idempotency::once::<_, _, i32>("k-2", Duration::from_secs(5), || async { Ok(7) })
        .await
        .unwrap();
    assert!(matches!(r, Idempotent::Fresh(7)));
}

#[tokio::test]
#[serial]
async fn once_consumes_the_window_even_when_body_errors() {
    RAN.store(0, Ordering::SeqCst);
    install_memory_cache();

    // First caller's body errors. `once` does NOT release on error - the TTL
    // is the dedupe window regardless of outcome (this is the contract that
    // distinguishes it from `commit_on_success`).
    let r1 = Idempotency::once::<_, _, i32>("once-err", Duration::from_secs(60), || async {
        RAN.fetch_add(1, Ordering::SeqCst);
        Err(suprnova::FrameworkError::internal("boom"))
    })
    .await;
    assert!(r1.is_err());

    // Second caller within the window: the lock is still held → Duplicate, and
    // the body does NOT run again.
    let r2: Idempotent<i32> = Idempotency::once("once-err", Duration::from_secs(60), || async {
        RAN.fetch_add(1, Ordering::SeqCst);
        Ok(7)
    })
    .await
    .unwrap();
    assert_eq!(r2, Idempotent::Duplicate);
    assert_eq!(
        RAN.load(Ordering::SeqCst),
        1,
        "once must not re-run after a failed predecessor; use commit_on_success to allow retry"
    );
}

#[tokio::test]
#[serial]
async fn commit_on_success_releases_lock_when_body_errors() {
    RAN.store(0, Ordering::SeqCst);
    install_memory_cache();

    // First call - body returns Err, lock must be released.
    let r1 =
        Idempotency::commit_on_success::<_, _, i32>("cos-1", Duration::from_secs(60), || async {
            RAN.fetch_add(1, Ordering::SeqCst);
            Err(suprnova::FrameworkError::internal("synthetic"))
        })
        .await;
    assert!(r1.is_err());
    assert_eq!(RAN.load(Ordering::SeqCst), 1);

    // Second call - lock was released, so body runs again.
    let r2: Idempotent<i32> =
        Idempotency::commit_on_success("cos-1", Duration::from_secs(60), || async {
            RAN.fetch_add(1, Ordering::SeqCst);
            Ok(99)
        })
        .await
        .unwrap();
    assert!(matches!(r2, Idempotent::Fresh(99)));
    assert_eq!(
        RAN.load(Ordering::SeqCst),
        2,
        "body must run after a failed predecessor releases the lock"
    );
}

#[tokio::test]
#[serial]
async fn commit_on_success_keeps_lock_when_body_succeeds() {
    RAN.store(0, Ordering::SeqCst);
    install_memory_cache();

    let r1: Idempotent<i32> =
        Idempotency::commit_on_success("cos-2", Duration::from_secs(60), || async {
            RAN.fetch_add(1, Ordering::SeqCst);
            Ok(42)
        })
        .await
        .unwrap();
    assert!(matches!(r1, Idempotent::Fresh(42)));
    assert_eq!(RAN.load(Ordering::SeqCst), 1);

    // Duplicate caller after success - still Duplicate.
    let r2: Idempotent<i32> =
        Idempotency::commit_on_success("cos-2", Duration::from_secs(60), || async {
            RAN.fetch_add(1, Ordering::SeqCst);
            Ok(99)
        })
        .await
        .unwrap();
    assert!(matches!(r2, Idempotent::Duplicate));
    assert_eq!(
        RAN.load(Ordering::SeqCst),
        1,
        "body must not run for duplicate after success"
    );
}

#[tokio::test]
#[serial]
async fn remember_records_result_and_replays_it_to_duplicates() {
    RAN.store(0, Ordering::SeqCst);
    install_memory_cache();

    let r1: Replay<String> = Idempotency::remember("rem-1", Duration::from_secs(60), || async {
        RAN.fetch_add(1, Ordering::SeqCst);
        Ok("hello".to_string())
    })
    .await
    .unwrap();
    assert_eq!(r1, Replay::Fresh("hello".to_string()));

    // Duplicate: a different body value must NOT run; the recorded result replays.
    let r2: Replay<String> = Idempotency::remember("rem-1", Duration::from_secs(60), || async {
        RAN.fetch_add(1, Ordering::SeqCst);
        Ok("world".to_string())
    })
    .await
    .unwrap();
    assert_eq!(r2, Replay::Replayed("hello".to_string()));
    assert_eq!(
        RAN.load(Ordering::SeqCst),
        1,
        "replay must not run the body"
    );
}

#[tokio::test]
#[serial]
async fn remember_error_does_not_replay_and_is_retryable() {
    RAN.store(0, Ordering::SeqCst);
    install_memory_cache();

    // First call errors - nothing is recorded and the lock is released.
    let r1 = Idempotency::remember::<_, _, i32>("rem-err", Duration::from_secs(60), || async {
        RAN.fetch_add(1, Ordering::SeqCst);
        Err(suprnova::FrameworkError::internal("boom"))
    })
    .await;
    assert!(r1.is_err());

    // Second call re-enters (lock was released) and succeeds.
    let r2: Replay<i32> = Idempotency::remember("rem-err", Duration::from_secs(60), || async {
        RAN.fetch_add(1, Ordering::SeqCst);
        Ok(42)
    })
    .await
    .unwrap();
    assert_eq!(r2, Replay::Fresh(42));

    // Third call replays the recorded success.
    let r3: Replay<i32> = Idempotency::remember("rem-err", Duration::from_secs(60), || async {
        RAN.fetch_add(1, Ordering::SeqCst);
        Ok(0)
    })
    .await
    .unwrap();
    assert_eq!(r3, Replay::Replayed(42));
    assert_eq!(
        RAN.load(Ordering::SeqCst),
        2,
        "body runs once on retry-after-error and never again on replay"
    );
}

#[tokio::test]
#[serial]
async fn remember_returns_in_progress_for_concurrent_duplicate() {
    install_memory_cache();

    // `inside_body` fires once caller 1 is executing the body (lock held, no
    // result recorded yet); `release_body` lets caller 1 finish.
    let inside_body = Arc::new(tokio::sync::Notify::new());
    let inside_body_tx = inside_body.clone();
    let release_body = Arc::new(tokio::sync::Notify::new());
    let release_body_rx = release_body.clone();

    let caller1 = tokio::spawn(async move {
        Idempotency::remember::<_, _, i32>("inprog", Duration::from_secs(60), || async move {
            inside_body_tx.notify_one();
            release_body_rx.notified().await;
            Ok(7)
        })
        .await
    });

    // Wait until caller 1 is inside the body, then race a duplicate in.
    inside_body.notified().await;
    let r2: Replay<i32> =
        Idempotency::remember("inprog", Duration::from_secs(60), || async { Ok(99) })
            .await
            .unwrap();
    assert_eq!(
        r2,
        Replay::InProgress,
        "duplicate arriving before the original records a result must be InProgress"
    );

    // Let caller 1 finish and record its result.
    release_body.notify_one();
    let r1 = caller1.await.unwrap().unwrap();
    assert_eq!(r1, Replay::Fresh(7));

    // A later caller now replays the recorded result.
    let r3: Replay<i32> =
        Idempotency::remember("inprog", Duration::from_secs(60), || async { Ok(0) })
            .await
            .unwrap();
    assert_eq!(r3, Replay::Replayed(7));
}

#[tokio::test]
#[serial]
async fn long_body_keeps_lock_alive_so_a_late_duplicate_does_not_double_execute() {
    RAN.store(0, Ordering::SeqCst);
    install_memory_cache();

    let inside_body = Arc::new(tokio::sync::Notify::new());
    let inside_body_tx = inside_body.clone();
    let release_body = Arc::new(tokio::sync::Notify::new());
    let release_body_rx = release_body.clone();

    // Caller 1 holds a 200ms-TTL lock but blocks in the body well past it.
    // Without lease renewal the lock would expire at ~200ms and a later caller
    // would acquire it and run the body a second time.
    let caller1 = tokio::spawn(async move {
        Idempotency::once::<_, _, i32>("watchdog", Duration::from_millis(200), || async move {
            RAN.fetch_add(1, Ordering::SeqCst);
            inside_body_tx.notify_one();
            release_body_rx.notified().await;
            Ok(1)
        })
        .await
    });

    inside_body.notified().await;
    // Wait 2.5x the original TTL: a non-renewed lock would already be gone.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let r2: Idempotent<i32> = Idempotency::once("watchdog", Duration::from_millis(200), || async {
        RAN.fetch_add(1, Ordering::SeqCst);
        Ok(2)
    })
    .await
    .unwrap();
    assert_eq!(
        r2,
        Idempotent::Duplicate,
        "lease renewal must keep the lock alive past the original TTL while the body runs"
    );

    release_body.notify_one();
    let r1 = caller1.await.unwrap().unwrap();
    assert_eq!(r1, Idempotent::Fresh(1));
    assert_eq!(
        RAN.load(Ordering::SeqCst),
        1,
        "body must execute exactly once despite the body outliving the TTL"
    );
}

/// A cache whose `release_lock` always fails, leaving every other operation
/// delegated to a real in-memory backend. Proves the release path is handled
/// (logged, not panicked, body error preserved) when the backend cannot
/// acknowledge a release.
struct FailingReleaseCache(InMemoryCache);

#[async_trait::async_trait]
impl CacheStore for FailingReleaseCache {
    async fn get_raw(&self, key: &str) -> Result<Option<String>, suprnova::FrameworkError> {
        self.0.get_raw(key).await
    }
    async fn put_raw(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), suprnova::FrameworkError> {
        self.0.put_raw(key, value, ttl).await
    }
    fn default_ttl(&self) -> Option<Duration> {
        self.0.default_ttl()
    }
    async fn has(&self, key: &str) -> Result<bool, suprnova::FrameworkError> {
        self.0.has(key).await
    }
    async fn forget(&self, key: &str) -> Result<bool, suprnova::FrameworkError> {
        self.0.forget(key).await
    }
    async fn flush(&self) -> Result<(), suprnova::FrameworkError> {
        self.0.flush().await
    }
    async fn increment(&self, key: &str, amount: i64) -> Result<i64, suprnova::FrameworkError> {
        self.0.increment(key, amount).await
    }
    async fn decrement(&self, key: &str, amount: i64) -> Result<i64, suprnova::FrameworkError> {
        self.0.decrement(key, amount).await
    }
    async fn tagged_put_raw(
        &self,
        tags: &[&str],
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), suprnova::FrameworkError> {
        self.0.tagged_put_raw(tags, key, value, ttl).await
    }
    async fn flush_tags(&self, tags: &[&str]) -> Result<(), suprnova::FrameworkError> {
        self.0.flush_tags(tags).await
    }
    async fn acquire_lock(
        &self,
        key: &str,
        ttl: Duration,
    ) -> Result<Option<String>, suprnova::FrameworkError> {
        self.0.acquire_lock(key, ttl).await
    }
    async fn release_lock(
        &self,
        _key: &str,
        _token: &str,
    ) -> Result<bool, suprnova::FrameworkError> {
        Err(suprnova::FrameworkError::internal(
            "synthetic release failure",
        ))
    }
    async fn refresh_lock(
        &self,
        key: &str,
        token: &str,
        ttl: Duration,
    ) -> Result<bool, suprnova::FrameworkError> {
        self.0.refresh_lock(key, token, ttl).await
    }
    async fn touch(&self, key: &str, ttl: Duration) -> Result<bool, suprnova::FrameworkError> {
        self.0.touch(key, ttl).await
    }
}

#[tokio::test]
#[serial]
async fn commit_on_success_surfaces_body_error_even_when_release_fails() {
    let store: Arc<dyn CacheStore> =
        Arc::new(FailingReleaseCache(InMemoryCache::with_prefix("idem:")));
    App::bind::<dyn CacheStore>(store);

    // Body fails; the Err-path release also fails. The release failure must be
    // logged (not masked) and the body error must come back without a panic.
    let r = Idempotency::commit_on_success::<_, _, i32>(
        "rel-fail",
        Duration::from_secs(60),
        || async { Err(suprnova::FrameworkError::internal("body boom")) },
    )
    .await;

    assert!(
        r.is_err(),
        "the body error is the only error on this path; a failing release must not swallow or replace it"
    );
}

// ---------------------------------------------------------------------------
// DATA-03b - a lost lease must reach the caller
// ---------------------------------------------------------------------------
//
// `run_under_lease` used to log a warning, stop renewing, park forever, and
// let the body run to completion *unfenced* - returning `Fresh(v)` with no
// signal that exclusivity had been lost. `Ok(false)` from `refresh_lock`
// means specifically that another holder now owns the lock, so at that point
// two callers can be executing the same idempotent body simultaneously and
// both report success. That is the one guarantee this module exists to make.

/// How `refresh_lock` should misbehave.
#[derive(Clone, Copy)]
enum RefreshFault {
    /// Report the token as no longer ours - somebody else holds the lock.
    LostImmediately,
    /// Fail with a backend error `n` times, then behave normally.
    ErrorsThenRecovers(u32),
}

/// Wraps a real store and injects faults into `refresh_lock` only.
///
/// Same decorator shape as the queue tests' `FaultDriver`: a real backend
/// underneath means every other operation behaves exactly as in production,
/// and the fault is deterministic rather than a timing race.
struct RefreshFaultStore {
    inner: Arc<dyn CacheStore>,
    fault: RefreshFault,
    refresh_calls: AtomicU32,
}

impl RefreshFaultStore {
    fn new(fault: RefreshFault) -> Self {
        Self {
            inner: Arc::new(InMemoryCache::with_prefix("idem:")),
            fault,
            refresh_calls: AtomicU32::new(0),
        }
    }
}

#[suprnova::async_trait]
impl CacheStore for RefreshFaultStore {
    async fn get_raw(&self, key: &str) -> Result<Option<String>, suprnova::FrameworkError> {
        self.inner.get_raw(key).await
    }
    async fn put_raw(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), suprnova::FrameworkError> {
        self.inner.put_raw(key, value, ttl).await
    }
    async fn has(&self, key: &str) -> Result<bool, suprnova::FrameworkError> {
        self.inner.has(key).await
    }
    async fn forget(&self, key: &str) -> Result<bool, suprnova::FrameworkError> {
        self.inner.forget(key).await
    }
    async fn flush(&self) -> Result<(), suprnova::FrameworkError> {
        self.inner.flush().await
    }
    async fn increment(&self, key: &str, amount: i64) -> Result<i64, suprnova::FrameworkError> {
        self.inner.increment(key, amount).await
    }
    async fn decrement(&self, key: &str, amount: i64) -> Result<i64, suprnova::FrameworkError> {
        self.inner.decrement(key, amount).await
    }
    async fn tagged_put_raw(
        &self,
        tags: &[&str],
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), suprnova::FrameworkError> {
        self.inner.tagged_put_raw(tags, key, value, ttl).await
    }
    async fn flush_tags(&self, tags: &[&str]) -> Result<(), suprnova::FrameworkError> {
        self.inner.flush_tags(tags).await
    }
    async fn acquire_lock(
        &self,
        key: &str,
        ttl: Duration,
    ) -> Result<Option<String>, suprnova::FrameworkError> {
        self.inner.acquire_lock(key, ttl).await
    }
    async fn release_lock(&self, key: &str, token: &str) -> Result<bool, suprnova::FrameworkError> {
        self.inner.release_lock(key, token).await
    }
    async fn touch(&self, key: &str, ttl: Duration) -> Result<bool, suprnova::FrameworkError> {
        self.inner.touch(key, ttl).await
    }

    async fn refresh_lock(
        &self,
        key: &str,
        token: &str,
        ttl: Duration,
    ) -> Result<bool, suprnova::FrameworkError> {
        let n = self.refresh_calls.fetch_add(1, Ordering::SeqCst);
        match self.fault {
            RefreshFault::LostImmediately => Ok(false),
            RefreshFault::ErrorsThenRecovers(failures) if n < failures => Err(
                suprnova::FrameworkError::internal("injected cache backend failure"),
            ),
            RefreshFault::ErrorsThenRecovers(_) => self.inner.refresh_lock(key, token, ttl).await,
        }
    }
}

fn install_refresh_fault_cache(fault: RefreshFault) -> Arc<RefreshFaultStore> {
    let store = Arc::new(RefreshFaultStore::new(fault));
    App::bind::<dyn CacheStore>(store.clone() as Arc<dyn CacheStore>);
    store
}

/// The headline defect: the body finishes, the caller is told `Fresh`, and
/// nothing anywhere records that another caller may have run the same work
/// at the same time.
#[tokio::test]
#[serial]
async fn a_lost_lease_surfaces_as_unfenced_rather_than_fresh() {
    install_refresh_fault_cache(RefreshFault::LostImmediately);

    // ttl/3 => a 60ms refresh interval; the body outlives it.
    let outcome: Idempotent<i32> =
        Idempotency::once("lease-lost", Duration::from_millis(180), || async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            Ok(9_i32)
        })
        .await
        .expect("the body succeeded, so the call must not error");

    assert!(
        matches!(outcome, Idempotent::FreshUnfenced(9)),
        "a lost lease must be reported as FreshUnfenced - reporting Fresh \
         claims an exclusivity that was demonstrably not held. Got {outcome:?}"
    );
}

/// The body is deliberately *not* cancelled on lease loss: by then it may
/// already have charged a card. It must still run to completion and yield
/// its value.
#[tokio::test]
#[serial]
async fn a_lost_lease_does_not_cancel_the_body() {
    install_refresh_fault_cache(RefreshFault::LostImmediately);
    RAN.store(0, Ordering::SeqCst);

    let outcome: Idempotent<i32> =
        Idempotency::once("lease-lost-body", Duration::from_millis(180), || async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            RAN.fetch_add(1, Ordering::SeqCst);
            Ok(11_i32)
        })
        .await
        .expect("call succeeds");

    assert_eq!(
        RAN.load(Ordering::SeqCst),
        1,
        "the body must run to completion; cancelling it would strand any \
         side effect it had already performed"
    );
    match outcome {
        Idempotent::FreshUnfenced(v) => assert_eq!(v, 11, "the value must survive"),
        other => panic!("expected FreshUnfenced, got {other:?}"),
    }
}

/// A backend error is not evidence somebody took the lock - it is evidence
/// we could not ask. Giving up on the first blip guaranteed the lease would
/// lapse even though the backend recovered milliseconds later.
#[tokio::test]
#[serial]
async fn a_transient_refresh_error_does_not_abandon_the_lease() {
    let store = install_refresh_fault_cache(RefreshFault::ErrorsThenRecovers(1));

    let outcome: Idempotent<i32> =
        Idempotency::once("lease-blip", Duration::from_millis(180), || async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            Ok(13_i32)
        })
        .await
        .expect("call succeeds");

    assert!(
        store.refresh_calls.load(Ordering::SeqCst) >= 2,
        "renewal must have been attempted again after the first failure; \
         only {} attempt(s) were made",
        store.refresh_calls.load(Ordering::SeqCst)
    );
    assert!(
        matches!(outcome, Idempotent::Fresh(13)),
        "one transient error, then recovery, still holds the lease - this \
         must not be downgraded to FreshUnfenced. Got {outcome:?}"
    );
}

/// The control: an unbroken lease is still plain `Fresh`, or every caller
/// would have to handle a warning that never means anything.
#[tokio::test]
#[serial]
async fn an_unbroken_lease_is_still_reported_as_fresh() {
    install_refresh_fault_cache(RefreshFault::ErrorsThenRecovers(0));

    let outcome: Idempotent<i32> =
        Idempotency::once("lease-held", Duration::from_millis(180), || async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            Ok(15_i32)
        })
        .await
        .expect("call succeeds");

    assert!(
        matches!(outcome, Idempotent::Fresh(15)),
        "a healthy lease must stay Fresh. Got {outcome:?}"
    );
}

// ---------------------------------------------------------------------------
// Owner-scoped locking: `commit_on_success_owned` / `release_owned`
//
// The queue worker releases a `unique_until_processing` lock from a different
// task than the one that took it, so the owner token has to survive the trip
// and the release has to be scoped to it.
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn commit_on_success_owned_reports_the_owner_and_hands_it_to_the_body() {
    install_memory_cache();
    let seen: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
    let seen_in_body = Arc::clone(&seen);

    let (outcome, owner) =
        Idempotency::commit_on_success_owned("owned-1", Duration::from_secs(60), move |owner| {
            *seen_in_body.lock().expect("seen mutex") = owner.map(str::to_owned);
            async { Ok::<(), suprnova::FrameworkError>(()) }
        })
        .await
        .expect("first call succeeds");

    assert!(matches!(outcome, Idempotent::Fresh(())));
    let owner = owner.expect("a held lease must report its owner token");
    assert!(!owner.is_empty());
    assert_eq!(
        seen.lock().expect("seen mutex").as_deref(),
        Some(owner.as_str()),
        "the body must see the same token the caller gets back"
    );

    let (dup, dup_owner) = Idempotency::commit_on_success_owned::<_, _, ()>(
        "owned-1",
        Duration::from_secs(60),
        |_owner| async { Ok(()) },
    )
    .await
    .expect("duplicate call succeeds");
    assert!(matches!(dup, Idempotent::Duplicate));
    assert!(
        dup_owner.is_none(),
        "a duplicate never took a lock, so it has no owner token to report"
    );
}

#[tokio::test]
#[serial]
async fn release_owned_is_scoped_to_the_owner_that_took_the_lock() {
    install_memory_cache();
    let (_, owner) = Idempotency::commit_on_success_owned::<_, _, ()>(
        "owned-2",
        Duration::from_secs(60),
        |_owner| async { Ok(()) },
    )
    .await
    .expect("first call succeeds");
    let owner = owner.expect("owner token");

    assert!(
        !Idempotency::release_owned("owned-2", "somebody-elses-token")
            .await
            .expect("release is not an error"),
        "a stale owner must not release a lock it does not hold"
    );
    let (still_held, _) = Idempotency::commit_on_success_owned::<_, _, ()>(
        "owned-2",
        Duration::from_secs(60),
        |_owner| async { Ok(()) },
    )
    .await
    .expect("call succeeds");
    assert!(
        matches!(still_held, Idempotent::Duplicate),
        "the mismatched release must have left the lock in place"
    );

    assert!(
        Idempotency::release_owned("owned-2", &owner)
            .await
            .expect("release succeeds"),
        "the owner that took the lock releases it"
    );
    let (after_release, new_owner) = Idempotency::commit_on_success_owned::<_, _, ()>(
        "owned-2",
        Duration::from_secs(60),
        |_owner| async { Ok(()) },
    )
    .await
    .expect("call succeeds");
    assert!(
        matches!(after_release, Idempotent::Fresh(())),
        "the key is free once its owner released it"
    );
    assert_ne!(
        new_owner.expect("new owner token"),
        owner,
        "a second acquisition mints a fresh token"
    );

    assert!(
        !Idempotency::release_owned("owned-2", &owner)
            .await
            .expect("release is not an error"),
        "replaying the first release must not disturb the newer holder"
    );
}

#[tokio::test]
#[serial]
async fn release_owned_reports_false_for_a_key_that_was_never_locked() {
    install_memory_cache();
    assert!(
        !Idempotency::release_owned("owned-never-taken", "any-token")
            .await
            .expect("release is not an error"),
        "releasing an absent lock is a success case, not an error"
    );
}
