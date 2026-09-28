//! Live-Redis test for the transient-command retry.
//!
//! **Requires `CACHE_REDIS_TEST_URL`, and it must point at a throwaway Redis.**
//! This suite issues `CLIENT KILL TYPE normal`, which disconnects every other
//! client on the instance - every other test process, every application, every
//! open `redis-cli`. It therefore does not fall back to `REDIS_URL` and does
//! not default to localhost the way `cache_redis_integration` does: an
//! instance you share is exactly the one it must not be pointed at. With the
//! variable unset, each test prints a skip line and returns.
//!
//! It also lives in its own test binary rather than alongside
//! `cache_redis_integration.rs` because the kill is indiscriminate: the writes
//! in a sibling test correctly do not retry, so a shared binary would fail
//! them. Cargo runs test binaries one at a time, so on its own it can only
//! reach its own store.
//!
//! ```sh
//! docker run --rm -d -p 6399:6379 --name suprnova-retry-redis redis:7-alpine
//! CACHE_REDIS_TEST_URL=redis://127.0.0.1:6399 \
//!   cargo test -p suprnova --test cache_redis_retry -- --ignored
//! ```

use std::sync::Arc;
use suprnova::cache::store::CacheStore;
use suprnova::cache::{CacheConfig, RedisCache};

/// The throwaway-Redis URL, or `None` after printing why the test is skipping.
///
/// Deliberately narrower than `cache_redis_integration`'s `redis_url()`, which
/// falls back to `REDIS_URL` and then to localhost. Either fallback would aim
/// `CLIENT KILL TYPE normal` at whatever Redis happened to be configured, so
/// this suite requires the operator to name the instance it is allowed to
/// disrupt.
fn throwaway_redis_url_or_skip(test_name: &str) -> Option<String> {
    match std::env::var("CACHE_REDIS_TEST_URL") {
        Ok(url) => Some(url),
        Err(_) => {
            eprintln!(
                "[{test_name}] skipping: CACHE_REDIS_TEST_URL not set. \
                 This test kills every client on the instance; point it at a \
                 throwaway Redis, never a shared one."
            );
            None
        }
    }
}

async fn fresh_store(url: &str, prefix: &str) -> Arc<dyn CacheStore> {
    let cfg = CacheConfig {
        driver: suprnova::cache::CacheDriver::Redis,
        url: url.to_string(),
        prefix: format!("{}{}:", prefix, uuid::Uuid::new_v4()),
        default_ttl: 0,
    };
    let cache = RedisCache::connect(&cfg)
        .await
        .expect("connect to the throwaway Redis named by CACHE_REDIS_TEST_URL");
    Arc::new(cache)
}

/// A connection killed out from under the store must not fail the caller's read.
///
/// `CLIENT KILL TYPE normal` closes every *other* normal client (Redis skips
/// the issuing connection unless `SKIPME no` is passed), which includes the
/// `ConnectionManager` the store holds.
#[tokio::test]
#[ignore = "requires a THROWAWAY Redis at CACHE_REDIS_TEST_URL; kills every other client"]
async fn redis_get_survives_a_killed_connection() {
    let Some(url) = throwaway_redis_url_or_skip("redis_get_survives_a_killed_connection") else {
        return;
    };

    let s = fresh_store(&url, "kill-retry").await;
    s.put_raw("k", "{\"v\":1}", None).await.unwrap();
    assert_eq!(
        s.get_raw("k").await.unwrap().as_deref(),
        Some("{\"v\":1}"),
        "control: the value reads back before the kill"
    );

    let client = redis::Client::open(url).expect("second client");
    let mut killer = client
        .get_multiplexed_async_connection()
        .await
        .expect("second connection");
    let _: redis::Value = redis::cmd("CLIENT")
        .arg("KILL")
        .arg("TYPE")
        .arg("normal")
        .query_async(&mut killer)
        .await
        .expect("CLIENT KILL");

    assert_eq!(
        s.get_raw("k").await.unwrap().as_deref(),
        Some("{\"v\":1}"),
        "the retry must absorb the dropped connection instead of failing the read"
    );
}
