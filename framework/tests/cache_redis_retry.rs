//! Live-Redis test for the transient-command retry.
//!
//! This lives in its own test binary rather than alongside
//! `cache_redis_integration.rs` because it is destructive: `CLIENT KILL TYPE
//! normal` closes every other client on the instance, and the writes in a
//! sibling test correctly do not retry, so a shared binary would fail them.
//! Cargo runs test binaries one at a time, so on its own it can only reach
//! its own store.
//!
//! Point `CACHE_REDIS_TEST_URL` at a throwaway Redis and run:
//!
//! ```sh
//! cargo test -p suprnova --test cache_redis_retry -- --ignored
//! ```

use std::sync::Arc;
use suprnova::cache::store::CacheStore;
use suprnova::cache::{CacheConfig, RedisCache};

fn redis_url() -> String {
    std::env::var("CACHE_REDIS_TEST_URL")
        .or_else(|_| std::env::var("REDIS_URL"))
        .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string())
}

async fn fresh_store(prefix: &str) -> Arc<dyn CacheStore> {
    let cfg = CacheConfig {
        driver: suprnova::cache::CacheDriver::Redis,
        url: redis_url(),
        prefix: format!("{}{}:", prefix, uuid::Uuid::new_v4()),
        default_ttl: 0,
    };
    let cache = RedisCache::connect(&cfg)
        .await
        .expect("connect to test Redis (set CACHE_REDIS_TEST_URL if not on localhost)");
    Arc::new(cache)
}

/// A connection killed out from under the store must not fail the caller's read.
///
/// `CLIENT KILL TYPE normal` closes every *other* normal client (Redis skips
/// the issuing connection unless `SKIPME no` is passed), which includes the
/// `ConnectionManager` the store holds. Point `CACHE_REDIS_TEST_URL` at a
/// throwaway Redis before running this - it disconnects every other client on
/// the instance.
#[tokio::test]
#[ignore = "requires a THROWAWAY Redis at CACHE_REDIS_TEST_URL; kills every other client"]
async fn redis_get_survives_a_killed_connection() {
    let s = fresh_store("kill-retry").await;
    s.put_raw("k", "{\"v\":1}", None).await.unwrap();
    assert_eq!(
        s.get_raw("k").await.unwrap().as_deref(),
        Some("{\"v\":1}"),
        "control: the value reads back before the kill"
    );

    let client = redis::Client::open(redis_url()).expect("second client");
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
