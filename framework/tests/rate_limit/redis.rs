//! `RedisRateLimiter` against a live Redis.
//!
//! Every test is `#[ignore]`d so a plain `cargo test` passes without a
//! server. Run them with
//! `cargo test -p suprnova --test rate_limit -- --ignored --test-threads=1 redis::`
//! and `REDIS_TEST_URL` naming a database the tests may write to. Each test
//! scopes its keys under a fresh prefix and deletes them when it ends.

use std::time::Duration;

use suprnova::rate_limit::redis::RedisRateLimiter;
use suprnova::rate_limit::{RateLimiterDriver, SlidingWindowConfig};
use suprnova::testing::TestClock;

fn url() -> String {
    std::env::var("REDIS_TEST_URL")
        .or_else(|_| std::env::var("REDIS_URL"))
        .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned())
}

/// Delete every key under `prefix`, so a run leaves nothing behind.
async fn delete_prefix(prefix: &str) {
    let client = suprnova::redis::Client::open(url()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let keys: Vec<String> = suprnova::redis::cmd("KEYS")
        .arg(format!("{prefix}*"))
        .query_async(&mut conn)
        .await
        .unwrap();
    if !keys.is_empty() {
        let _: i64 = suprnova::redis::cmd("DEL")
            .arg(&keys)
            .query_async(&mut conn)
            .await
            .unwrap();
    }
}

/// Milliseconds before the hit set behind `key` expires.
async fn pttl(prefix: &str, key: &str) -> i64 {
    let client = suprnova::redis::Client::open(url()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    suprnova::redis::cmd("PTTL")
        .arg(format!("{prefix}rl:{key}"))
        .query_async(&mut conn)
        .await
        .unwrap()
}

/// One key, two quotas. A shorter-window quota must not delete the hits a
/// longer-window quota on the same key still counts, and must not shorten
/// the key's lifetime below the longer window. The in-memory driver keeps
/// the longest window it has seen for a key; the Redis driver used to prune
/// and expire with whichever window the current caller passed.
#[tokio::test]
#[ignore = "needs Redis: set REDIS_TEST_URL"]
async fn redis_a_shorter_window_keeps_the_history_of_a_longer_one() {
    let clock = TestClock::freeze();
    let prefix = format!("rl-mixed-{}:", uuid::Uuid::new_v4());
    let limiter = RedisRateLimiter::connect(&url(), &prefix)
        .await
        .expect("connect to the test Redis");
    let key = "account:42";
    let long = SlidingWindowConfig {
        max_requests: 2,
        window: Duration::from_secs(60),
    };
    let short = SlidingWindowConfig {
        max_requests: 5,
        window: Duration::from_secs(10),
    };

    assert!(limiter.try_acquire(key, &long).await.unwrap());
    assert!(limiter.try_acquire(key, &long).await.unwrap());
    assert!(
        !limiter.try_acquire(key, &long).await.unwrap(),
        "the long quota is spent"
    );

    // The two long hits are now 20s old: outside the short window, still
    // inside the long one.
    clock.advance(chrono::Duration::seconds(20));
    assert_eq!(
        limiter.retry_after(key, &short).await.unwrap(),
        None,
        "the short quota has room"
    );
    assert!(
        limiter.try_acquire(key, &short).await.unwrap(),
        "the short quota has room"
    );
    assert!(
        pttl(&prefix, key).await > 10_000,
        "the short quota must not shorten the key's lifetime below the long window"
    );

    assert!(
        !limiter.try_acquire(key, &long).await.unwrap(),
        "the long quota's hits are 20s old, inside its 60s window, so it is still spent"
    );
    assert_eq!(
        limiter.retry_after(key, &long).await.unwrap(),
        Some(Duration::from_secs(40)),
        "the long quota reopens when its second hit leaves the 60s window"
    );

    delete_prefix(&prefix).await;
}
