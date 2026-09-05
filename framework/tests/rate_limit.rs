use std::sync::Arc;
use std::time::Duration;
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::rate_limit::{RateLimiterDriver, SlidingWindowConfig};

fn cfg(max: u32, window_secs: u64) -> SlidingWindowConfig {
    SlidingWindowConfig {
        max_requests: max,
        window: Duration::from_secs(window_secs),
    }
}

#[tokio::test(start_paused = true)]
async fn allows_up_to_max_within_window_then_rejects() {
    let limiter: Arc<dyn RateLimiterDriver> = Arc::new(InMemoryRateLimiter::new());
    let key = "user:1";
    let c = cfg(3, 10);

    assert!(limiter.try_acquire(key, &c).await.unwrap());
    assert!(limiter.try_acquire(key, &c).await.unwrap());
    assert!(limiter.try_acquire(key, &c).await.unwrap());
    assert!(
        !limiter.try_acquire(key, &c).await.unwrap(),
        "4th must be rejected"
    );
}

#[tokio::test(start_paused = true)]
async fn window_slides_so_old_hits_expire() {
    let limiter: Arc<dyn RateLimiterDriver> = Arc::new(InMemoryRateLimiter::new());
    let key = "user:2";
    let c = cfg(2, 10);

    assert!(limiter.try_acquire(key, &c).await.unwrap());
    assert!(limiter.try_acquire(key, &c).await.unwrap());
    assert!(!limiter.try_acquire(key, &c).await.unwrap());

    tokio::time::advance(Duration::from_secs(11)).await;

    assert!(limiter.try_acquire(key, &c).await.unwrap());
    assert!(limiter.try_acquire(key, &c).await.unwrap());
    assert!(!limiter.try_acquire(key, &c).await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn distinct_keys_have_independent_buckets() {
    let limiter: Arc<dyn RateLimiterDriver> = Arc::new(InMemoryRateLimiter::new());
    let c = cfg(1, 60);
    assert!(limiter.try_acquire("a", &c).await.unwrap());
    assert!(limiter.try_acquire("b", &c).await.unwrap());
    assert!(!limiter.try_acquire("a", &c).await.unwrap());
    assert!(!limiter.try_acquire("b", &c).await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn retry_after_reflects_oldest_entry_in_window() {
    let limiter: Arc<dyn RateLimiterDriver> = Arc::new(InMemoryRateLimiter::new());
    let c = cfg(1, 30);
    assert!(limiter.try_acquire("k", &c).await.unwrap());

    tokio::time::advance(Duration::from_secs(10)).await;
    let retry = limiter.retry_after("k", &c).await.unwrap();
    // window=30, oldest entry is 10s old → retry-after = 20s.
    assert_eq!(retry, Some(Duration::from_secs(20)));
}

#[tokio::test(start_paused = true)]
async fn periodic_sweep_preserves_active_long_window() {
    let limiter =
        InMemoryRateLimiter::with_periodic_sweep(Duration::from_secs(60), Duration::from_secs(900));
    let quota = cfg(1, 3600);
    assert!(limiter.try_acquire("hourly", &quota).await.unwrap());
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(960)).await;
    tokio::task::yield_now().await;
    assert!(!limiter.try_acquire("hourly", &quota).await.unwrap());
    assert_eq!(limiter.bucket_count(), 1);
    assert_eq!(
        limiter.purge_inactive(Duration::from_secs(900), tokio::time::Instant::now()),
        0
    );
    tokio::time::advance(Duration::from_secs(2640)).await;
    assert!(limiter.try_acquire("hourly", &quota).await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn shorter_window_does_not_discard_active_long_window_history() {
    let limiter = InMemoryRateLimiter::new();
    assert!(limiter.try_acquire("shared", &cfg(2, 3600)).await.unwrap());
    tokio::time::advance(Duration::from_secs(120)).await;
    assert_eq!(
        limiter.retry_after("shared", &cfg(1, 60)).await.unwrap(),
        None
    );
    assert!(limiter.try_acquire("shared", &cfg(1, 60)).await.unwrap());
    assert!(!limiter.try_acquire("shared", &cfg(2, 3600)).await.unwrap());
    tokio::time::advance(Duration::from_secs(60)).await;
    assert_eq!(
        limiter.retry_after("shared", &cfg(1, 60)).await.unwrap(),
        None
    );
    assert_eq!(
        limiter.purge_inactive(Duration::from_secs(60), tokio::time::Instant::now()),
        0
    );
    assert!(!limiter.try_acquire("shared", &cfg(2, 3600)).await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn retry_after_waits_until_enough_hits_expire_for_lower_quota() {
    let limiter = InMemoryRateLimiter::new();
    assert!(limiter.try_acquire("shared", &cfg(3, 60)).await.unwrap());
    tokio::time::advance(Duration::from_secs(10)).await;
    assert!(limiter.try_acquire("shared", &cfg(3, 60)).await.unwrap());
    tokio::time::advance(Duration::from_secs(10)).await;
    assert!(limiter.try_acquire("shared", &cfg(3, 60)).await.unwrap());
    assert_eq!(
        limiter.retry_after("shared", &cfg(1, 60)).await.unwrap(),
        Some(Duration::from_secs(60))
    );
    tokio::time::advance(Duration::from_secs(60)).await;
    assert_eq!(
        limiter.retry_after("shared", &cfg(1, 60)).await.unwrap(),
        None
    );
    assert!(limiter.try_acquire("shared", &cfg(1, 60)).await.unwrap());
}
