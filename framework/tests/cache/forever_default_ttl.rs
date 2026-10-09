//! Regression: HIGH audit finding `cache` #252 - `Cache::forever` is
//! not forever when `CACHE_DEFAULT_TTL` is set.
//!
//! Both stores must honour `put_raw(_, _, None)` as "no expiration",
//! independent of any facade-level default. `Cache::forever` reaches
//! the store with `None` directly; `Cache::put(None)` reaches the
//! store with the facade-resolved default TTL applied.
//!
//! These tests exercise the store-level contract (where the audit
//! divergence lived) rather than the facade - that lets them run
//! without an `App` boot, and they catch the exact bug the audit
//! flagged: a store substituting its own default on `None`.

use std::sync::Arc;
use std::time::Duration;
use suprnova::cache::config::CacheConfig;
use suprnova::cache::store::CacheStore;
use suprnova::cache::{CacheDriver, InMemoryCache};

#[tokio::test]
async fn store_put_raw_none_ttl_means_no_expiration_in_memory() {
    // Construct a memory store with a 1-second configured default - the
    // store MUST NOT substitute this when `put_raw` is called with `None`.
    // (Pre-fix Redis did exactly that substitution; in-memory always
    // honoured None, so this test pins the contract uniformly.)
    let config = CacheConfig {
        driver: CacheDriver::Memory,
        url: "unused".into(),
        prefix: "test-forever:".into(),
        default_ttl: 1,
        sweep_interval: 0,
    };
    let store: Arc<dyn CacheStore> = Arc::new(InMemoryCache::with_config(&config));

    store.put_raw("k", "\"v\"", None).await.expect("put_raw");

    // After 1.2s - past the configured default - the key MUST still be
    // present. If the store substituted the default on None, this would
    // already have expired.
    tokio::time::sleep(Duration::from_millis(1_200)).await;

    let v = store.get_raw("k").await.expect("get_raw");
    assert_eq!(
        v.as_deref(),
        Some("\"v\""),
        "InMemoryCache::put_raw with None ttl must mean 'no expiration', \
         not 'apply the configured default'"
    );
}

#[tokio::test]
async fn store_exposes_default_ttl_for_facade_consumption() {
    // The facade reads `default_ttl()` to resolve `Cache::put(None)`.
    // Verify that the in-memory store returns what the config set,
    // converted to Duration. (Pre-fix the in-memory store didn't track
    // a default at all - the facade had no way to apply it uniformly.)
    let config = CacheConfig {
        driver: CacheDriver::Memory,
        url: "unused".into(),
        prefix: "test-default:".into(),
        default_ttl: 42,
        sweep_interval: 0,
    };
    let store: Arc<dyn CacheStore> = Arc::new(InMemoryCache::with_config(&config));
    assert_eq!(store.default_ttl(), Some(Duration::from_secs(42)));

    let zero_config = CacheConfig {
        default_ttl: 0,
        ..config
    };
    let zero_store: Arc<dyn CacheStore> = Arc::new(InMemoryCache::with_config(&zero_config));
    assert_eq!(
        zero_store.default_ttl(),
        None,
        "default_ttl=0 in config must mean None (no facade default)"
    );
}

#[tokio::test]
async fn store_put_raw_some_ttl_still_expires() {
    // Companion test: the bug-fix didn't break ordinary TTL semantics.
    let store: Arc<dyn CacheStore> = Arc::new(InMemoryCache::with_prefix("test-some:"));

    store
        .put_raw("k", "\"v\"", Some(Duration::from_millis(100)))
        .await
        .expect("put_raw");

    tokio::time::sleep(Duration::from_millis(200)).await;

    let v = store.get_raw("k").await.expect("get_raw");
    assert!(
        v.is_none(),
        "put_raw with Some(100ms) ttl must still expire after 200ms"
    );
}

/// A store that records the TTL every write reached it with, over a real
/// [`InMemoryCache`] that reports a configured default.
///
/// The facade is where `None` turns into either "the configured default" or
/// "forever", so the facade's own tests have to see what the store was
/// handed. Recording it is exact and needs no sleeping past a TTL.
struct TtlRecorder {
    inner: InMemoryCache,
    writes: std::sync::Mutex<Vec<(String, Option<Duration>)>>,
}

impl TtlRecorder {
    fn with_default(seconds: u64) -> Arc<Self> {
        let config = CacheConfig {
            driver: CacheDriver::Memory,
            url: "unused".into(),
            prefix: "ttl-recorder:".into(),
            default_ttl: seconds,
            sweep_interval: 0,
        };
        Arc::new(Self {
            inner: InMemoryCache::with_config(&config),
            writes: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn ttl_for(&self, key: &str) -> Option<Option<Duration>> {
        self.writes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, ttl)| *ttl)
    }

    fn record(&self, key: &str, ttl: Option<Duration>) {
        self.writes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((key.to_string(), ttl));
    }
}

#[suprnova::async_trait]
impl CacheStore for TtlRecorder {
    async fn get_raw(&self, key: &str) -> Result<Option<String>, suprnova::FrameworkError> {
        self.inner.get_raw(key).await
    }
    async fn put_raw(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), suprnova::FrameworkError> {
        self.record(key, ttl);
        self.inner.put_raw(key, value, ttl).await
    }
    fn default_ttl(&self) -> Option<Duration> {
        self.inner.default_ttl()
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
        self.record(key, ttl);
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
    async fn refresh_lock(
        &self,
        key: &str,
        token: &str,
        ttl: Duration,
    ) -> Result<bool, suprnova::FrameworkError> {
        self.inner.refresh_lock(key, token, ttl).await
    }
    async fn touch(&self, key: &str, ttl: Duration) -> Result<bool, suprnova::FrameworkError> {
        self.inner.touch(key, ttl).await
    }
}

fn install_recorder(
    default_seconds: u64,
) -> (
    suprnova::container::testing::TestContainerGuard,
    Arc<TtlRecorder>,
) {
    let guard = suprnova::container::testing::TestContainer::fake();
    let recorder = TtlRecorder::with_default(default_seconds);
    suprnova::container::testing::TestContainer::bind::<dyn CacheStore>(recorder.clone());
    (guard, recorder)
}

/// DRIVERS-001: `remember_forever` and its `sear` alias store without a TTL
/// even when `CACHE_DEFAULT_TTL` is set. They used to go through
/// `Cache::put(.., None)`, which applies the default, so a "forever" value
/// expired after an hour.
#[tokio::test]
async fn remember_forever_and_sear_ignore_the_configured_default_ttl() {
    let (_guard, recorder) = install_recorder(3600);

    let stored: String = suprnova::Cache::remember_forever("forever:remember", || async {
        Ok("computed".to_string())
    })
    .await
    .expect("remember_forever");
    assert_eq!(stored, "computed");
    assert_eq!(
        recorder.ttl_for("forever:remember"),
        Some(None),
        "remember_forever must reach the store with no TTL, not the 3600s default"
    );

    let seared: String =
        suprnova::Cache::sear("forever:sear", || async { Ok("seared".to_string()) })
            .await
            .expect("sear");
    assert_eq!(seared, "seared");
    assert_eq!(
        recorder.ttl_for("forever:sear"),
        Some(None),
        "sear is remember_forever's alias and must not expire either"
    );

    // `remember` with `None` still means "the configured default".
    let _: String =
        suprnova::Cache::remember("default:remember", None, || async { Ok("v".to_string()) })
            .await
            .expect("remember");
    assert_eq!(
        recorder.ttl_for("default:remember"),
        Some(Some(Duration::from_secs(3600))),
        "remember(None) keeps applying the configured default"
    );
}

/// DRIVERS-002: `tags_put(.., None)` applies `CACHE_DEFAULT_TTL` the way
/// `put(.., None)` does. It used to hand `None` straight to the store, which
/// stores it forever, so a tagged value outlived the configured default.
#[tokio::test]
async fn tags_put_without_a_ttl_applies_the_configured_default() {
    let (_guard, recorder) = install_recorder(3600);

    suprnova::Cache::tags_put(&["users"], "tagged:default", &"v", None)
        .await
        .expect("tags_put");
    assert_eq!(
        recorder.ttl_for("tagged:default"),
        Some(Some(Duration::from_secs(3600))),
        "tags_put(None) must apply the configured 3600s default, like put(None)"
    );

    suprnova::Cache::tags_put(
        &["users"],
        "tagged:explicit",
        &"v",
        Some(Duration::from_secs(5)),
    )
    .await
    .expect("tags_put explicit");
    assert_eq!(
        recorder.ttl_for("tagged:explicit"),
        Some(Some(Duration::from_secs(5))),
        "an explicit TTL is passed through unchanged"
    );

    // The tagged counterpart of `forever` is the way to opt out of the default.
    suprnova::Cache::tags_forever(&["users"], "tagged:forever", &"v")
        .await
        .expect("tags_forever");
    assert_eq!(
        recorder.ttl_for("tagged:forever"),
        Some(None),
        "tags_forever must reach the store with no TTL"
    );
    suprnova::Cache::flush_tags(&["users"])
        .await
        .expect("flush_tags");
    assert!(
        !suprnova::Cache::has("tagged:forever").await.expect("has"),
        "a value stored by tags_forever is still flushed by its tag"
    );
}
