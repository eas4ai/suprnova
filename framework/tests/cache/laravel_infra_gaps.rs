//! The cache rows of Laravel's infrastructure surface.
//!
//! The Redis store runs on the Redis facade's named connections, the
//! `cache` connection for its commands and the `default` one for its
//! locks, with the cache prefix from `CACHE_PREFIX`, as Laravel's
//! `CacheManager::createRedisDriver` builds it. `Cache::get`, `has` and
//! `missing` dispatch `CacheHit` and `CacheMissed`, as Laravel's
//! `Repository::get` does, and `Cache::remember_with_ttl` takes the
//! lifetime from the value it computed.
//!
//! The `redis_` tests need a Redis server: `CACHE_REDIS_TEST_URL`, or the
//! local one. They write to databases 13 and 14 under a prefix of their
//! own and delete only the keys they wrote.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use suprnova::cache::{Cache, CacheConfig, CacheStore, InMemoryCache, RedisCache};
use suprnova::container::testing::{TestContainer, TestContainerGuard};
use suprnova::events::{dispatched, dispatched_count};
use suprnova::{CacheHit, CacheMissed, EventFacade, FrameworkError, Listener, Redis};

use crate::env_snapshot::{EnvSnapshot, set_env};
use crate::forever_default_ttl::install_recorder;

/// Every variable these tests set, captured and restored around each one.
const VARIABLES: &[&str] = &[
    "CACHE_DRIVER",
    "CACHE_PREFIX",
    "APP_NAME",
    "REDIS_PREFIX",
    "REDIS_URL",
    "REDIS_HOST",
    "REDIS_PORT",
    "REDIS_DB",
    "REDIS_CACHE_DB",
    "REDIS_CACHE_CONNECTION",
    "REDIS_CACHE_LOCK_CONNECTION",
];

fn clear(keys: &[&str]) {
    for key in keys {
        set_env(key, None);
    }
}

fn install_memory_cache() -> TestContainerGuard {
    let guard = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(InMemoryCache::with_prefix("infra-gaps:")));
    guard
}

// ---- PAR-133: the prefix and the connection names -------------------------

#[test]
fn the_cache_prefix_comes_from_cache_prefix() {
    let _env = crate::env_lock::lock_env();
    let _snap = EnvSnapshot::capture(VARIABLES);
    clear(VARIABLES);
    set_env("CACHE_PREFIX", Some("acme_cache:"));
    set_env("APP_NAME", Some("Shop"));

    let config = CacheConfig::from_env().expect("config");
    assert_eq!(config.prefix, "acme_cache:");
}

#[test]
fn the_cache_prefix_defaults_to_the_slug_of_the_app_name() {
    let _env = crate::env_lock::lock_env();
    let _snap = EnvSnapshot::capture(VARIABLES);
    clear(VARIABLES);

    set_env("APP_NAME", Some("Shop"));
    assert_eq!(
        CacheConfig::from_env().expect("config").prefix,
        "shop-cache-"
    );

    set_env("APP_NAME", Some("My Shop"));
    assert_eq!(
        CacheConfig::from_env().expect("config").prefix,
        "my-shop-cache-"
    );

    set_env("APP_NAME", None);
    assert_eq!(
        CacheConfig::from_env().expect("config").prefix,
        "suprnova-cache-"
    );
}

#[test]
fn redis_prefix_no_longer_names_the_cache_prefix() {
    let _env = crate::env_lock::lock_env();
    let _snap = EnvSnapshot::capture(VARIABLES);
    clear(VARIABLES);
    set_env("APP_NAME", Some("Shop"));
    set_env("REDIS_PREFIX", Some("legacy:"));

    assert_eq!(
        CacheConfig::from_env().expect("config").prefix,
        "shop-cache-",
        "REDIS_PREFIX is the connection prefix, not the cache prefix"
    );
}

#[test]
fn an_empty_cache_prefix_stays_empty() {
    let _env = crate::env_lock::lock_env();
    let _snap = EnvSnapshot::capture(VARIABLES);
    clear(VARIABLES);
    set_env("APP_NAME", Some("Shop"));
    set_env("CACHE_PREFIX", Some(""));

    assert_eq!(CacheConfig::from_env().expect("config").prefix, "");
}

#[test]
fn the_connection_names_default_to_cache_and_default() {
    let _env = crate::env_lock::lock_env();
    let _snap = EnvSnapshot::capture(VARIABLES);
    clear(VARIABLES);

    let config = CacheConfig::from_env().expect("config");
    assert_eq!(config.connection, "cache");
    assert_eq!(config.lock_connection, "default");

    set_env("REDIS_CACHE_CONNECTION", Some("default"));
    set_env("REDIS_CACHE_LOCK_CONNECTION", Some("locks"));
    let config = CacheConfig::from_env().expect("config");
    assert_eq!(config.connection, "default");
    assert_eq!(config.lock_connection, "locks");
}

#[test]
fn the_builder_names_the_connections() {
    let config = CacheConfig::builder()
        .connection("sessions")
        .lock_connection("locks")
        .prefix("app:")
        .build();
    assert_eq!(config.connection, "sessions");
    assert_eq!(config.lock_connection, "locks");
    assert_eq!(config.prefix, "app:");

    let defaults = CacheConfig::builder().build();
    assert_eq!(defaults.connection, "cache");
    assert_eq!(defaults.lock_connection, "default");
}

#[tokio::test]
async fn set_connection_on_the_memory_store_is_refused() {
    let _container = install_memory_cache();

    let error = Cache::set_connection("other").expect_err("the memory store has no connection");
    assert!(
        error.to_string().contains("Redis"),
        "the refusal says why: {error}"
    );
}

#[tokio::test]
async fn a_store_on_an_unknown_connection_is_refused() {
    let name = format!("infra-gaps-undefined-{}", uuid::Uuid::new_v4().simple());
    let config = CacheConfig::builder().connection(name.clone()).build();

    let Err(error) = RedisCache::connect_named(&config).await else {
        panic!("no connection has the name {name}");
    };
    assert!(error.to_string().contains(&name), "{error}");
}

/// The store sends one command when it is built, so a boot whose cache
/// connection cannot answer fails, as `RedisCache::connect` failed it; the
/// error names where it tried and never the password.
#[tokio::test]
async fn a_store_whose_connection_cannot_answer_fails_to_build() {
    let name = format!("infra-gaps-down-{}", uuid::Uuid::new_v4().simple());
    // Port 1 refuses the connection at once on a local host.
    Redis::define(&name, "redis://cache-user:s3cret-pw@127.0.0.1:1/13").expect("define");
    let config = CacheConfig::builder().connection(name.clone()).build();

    let Err(error) = RedisCache::connect_named(&config).await else {
        panic!("nothing listens on port 1, so the store must not build");
    };
    let rendered = format!("{error} {error:?}");
    assert!(
        !rendered.contains("s3cret-pw") && !rendered.contains("cache-user"),
        "the error leaks the connection's credentials: {rendered}"
    );
    Redis::purge(&name);
}

// ---- PAR-133: against Redis ------------------------------------------------

/// The server the `redis_` tests use, without a database number.
fn redis_base_url() -> url::Url {
    let raw = std::env::var("CACHE_REDIS_TEST_URL")
        .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let mut url = url::Url::parse(&raw).expect("a Redis URL");
    url.set_path("");
    url
}

fn redis_url_on(database: u8) -> String {
    let mut url = redis_base_url();
    url.set_path(&format!("/{database}"));
    url.to_string()
}

async fn raw_on(database: u8) -> redis::aio::MultiplexedConnection {
    redis::Client::open(redis_url_on(database))
        .expect("open a raw Redis client")
        .get_multiplexed_async_connection()
        .await
        .expect("connect a raw Redis client")
}

async fn raw_get(database: u8, key: &str) -> Option<String> {
    let mut connection = raw_on(database).await;
    redis::cmd("GET")
        .arg(key)
        .query_async(&mut connection)
        .await
        .expect("GET")
}

async fn raw_exists(database: u8, key: &str) -> bool {
    let mut connection = raw_on(database).await;
    redis::cmd("EXISTS")
        .arg(key)
        .query_async::<i64>(&mut connection)
        .await
        .expect("EXISTS")
        == 1
}

async fn raw_set(database: u8, key: &str, value: &str) {
    let mut connection = raw_on(database).await;
    redis::cmd("SET")
        .arg(key)
        .arg(value)
        .query_async::<()>(&mut connection)
        .await
        .expect("SET");
}

/// Deletes the keys a test wrote when it ends, passed or not, so a failed
/// run leaves nothing behind in the shared databases.
struct WrittenKeys(Vec<(u8, String)>);

impl WrittenKeys {
    fn new() -> Self {
        Self(Vec::new())
    }

    fn add(&mut self, database: u8, key: impl Into<String>) -> String {
        let key = key.into();
        self.0.push((database, key.clone()));
        key
    }
}

impl Drop for WrittenKeys {
    fn drop(&mut self) {
        for (database, key) in &self.0 {
            let Ok(client) = redis::Client::open(redis_url_on(*database)) else {
                continue;
            };
            if let Ok(mut connection) = client.get_connection() {
                let _: redis::RedisResult<i64> = redis::cmd("DEL").arg(key).query(&mut connection);
            }
        }
    }
}

/// A connection prefix no other run uses.
fn unique_redis_prefix() -> String {
    format!("infra-{}-", uuid::Uuid::new_v4().simple())
}

/// The environment of a deployment with `CACHE_DRIVER=redis`, the `cache`
/// connection on database 13, the `default` connection on
/// `default_database`, and `APP_NAME=Shop`.
fn redis_environment(redis_prefix: &str, default_database: u8) {
    clear(VARIABLES);
    set_env("CACHE_DRIVER", Some("redis"));
    set_env("REDIS_URL", Some(&redis_url_on(default_database)));
    set_env("REDIS_CACHE_DB", Some("13"));
    set_env("REDIS_PREFIX", Some(redis_prefix));
    set_env("APP_NAME", Some("Shop"));
}

/// Bind the store the bootstrap builds for `CACHE_DRIVER=redis`, on the
/// facade connections as the environment now names them.
async fn bind_store_from_environment() -> TestContainerGuard {
    Redis::purge("cache");
    Redis::purge("default");
    let config = CacheConfig::from_env().expect("config");
    let store = RedisCache::connect_named(&config)
        .await
        .expect("connect to the test Redis (set CACHE_REDIS_TEST_URL if not on localhost)");
    let guard = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(store));
    guard
}

#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost; writes to databases 13 and 14"]
async fn redis_put_runs_on_the_cache_connection_database() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snap = EnvSnapshot::capture(VARIABLES);
    let prefix = unique_redis_prefix();
    redis_environment(&prefix, 0);
    let mut written = WrittenKeys::new();
    let key = written.add(13, format!("{prefix}shop-cache-k"));
    let _container = bind_store_from_environment().await;

    Cache::put("k", &1, None).await.expect("put");

    assert_eq!(
        raw_get(13, &key).await.as_deref(),
        Some("1"),
        "the value is in database 13, the cache connection's"
    );
    assert!(
        !raw_exists(0, &key).await,
        "and not in the default connection's database"
    );
}

#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost; writes to databases 13 and 14"]
async fn redis_stored_key_is_the_connection_prefix_then_the_cache_prefix_then_the_key() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snap = EnvSnapshot::capture(VARIABLES);
    redis_environment("shop-", 0);
    let mut written = WrittenKeys::new();
    let key = written.add(13, "shop-shop-cache-k");
    let _container = bind_store_from_environment().await;

    Cache::put("k", &"v", None).await.expect("put");

    assert_eq!(raw_get(13, &key).await.as_deref(), Some("\"v\""));
    let read: Option<String> = Cache::get("k").await.expect("get");
    assert_eq!(
        read.as_deref(),
        Some("v"),
        "the store reads its own key back"
    );
}

#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost; writes to databases 13 and 14"]
async fn redis_set_connection_moves_later_commands() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snap = EnvSnapshot::capture(VARIABLES);
    let prefix = unique_redis_prefix();
    redis_environment(&prefix, 0);
    let mut written = WrittenKeys::new();
    let first = written.add(13, format!("{prefix}shop-cache-first"));
    let second_in_13 = written.add(13, format!("{prefix}shop-cache-second"));
    let second_in_14 = written.add(14, format!("{prefix}shop-cache-second"));
    Redis::define("other", &redis_url_on(14)).expect("define other");
    let _container = bind_store_from_environment().await;

    Cache::put("first", &1, None).await.expect("put first");
    Cache::set_connection("other").expect("the Redis store changes connection");
    Cache::put("second", &2, None).await.expect("put second");

    assert_eq!(raw_get(13, &first).await.as_deref(), Some("1"));
    assert_eq!(
        raw_get(14, &second_in_14).await.as_deref(),
        Some("2"),
        "after set_connection the write lands on database 14"
    );
    assert!(!raw_exists(13, &second_in_13).await);
    let read: Option<i64> = Cache::get("second").await.expect("get");
    assert_eq!(read, Some(2), "reads follow the new connection too");
    let missing: Option<i64> = Cache::get("first").await.expect("get");
    assert_eq!(missing, None, "the old connection's keys are not read");

    let error = Cache::set_connection("infra-gaps-never-defined")
        .expect_err("an undefined connection is refused");
    assert!(error.to_string().contains("infra-gaps-never-defined"));
    Redis::purge("other");
}

#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost; writes to databases 13 and 14"]
async fn redis_locks_run_on_the_lock_connection() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snap = EnvSnapshot::capture(VARIABLES);
    let prefix = unique_redis_prefix();
    // The default connection, which takes the locks, on database 14.
    redis_environment(&prefix, 14);
    let mut written = WrittenKeys::new();
    let lock_key = written.add(14, format!("{prefix}shop-cache-\0lock:job"));
    written.add(13, format!("{prefix}shop-cache-\0lock:job"));
    let _container = bind_store_from_environment().await;

    let guard = Cache::lock("job", Duration::from_secs(30))
        .await
        .expect("lock")
        .expect("the lock is free");

    assert!(
        raw_exists(14, &lock_key).await,
        "the lock is on the lock connection's database"
    );
    assert!(!raw_exists(13, &lock_key).await);
    assert!(guard.release().await.expect("release"));
    assert!(!raw_exists(14, &lock_key).await);
}

#[tokio::test]
#[ignore = "requires Redis at CACHE_REDIS_TEST_URL or default localhost; writes to databases 13 and 14"]
async fn redis_flush_deletes_only_keys_under_both_prefixes() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snap = EnvSnapshot::capture(VARIABLES);
    let prefix = unique_redis_prefix();
    redis_environment(&prefix, 0);
    let mut written = WrittenKeys::new();
    let cached = written.add(13, format!("{prefix}shop-cache-k"));
    let tagged = written.add(13, format!("{prefix}shop-cache-tagged"));
    let other_cache_prefix = written.add(13, format!("{prefix}other-cache-k"));
    let no_connection_prefix = written.add(13, "shop-cache-k-infra-gaps");
    let _container = bind_store_from_environment().await;

    Cache::put("k", &1, None).await.expect("put");
    Cache::tags_put(&["users"], "tagged", &2, None)
        .await
        .expect("tags_put");
    raw_set(13, &other_cache_prefix, "kept").await;
    raw_set(13, &no_connection_prefix, "kept").await;

    Cache::flush().await.expect("flush");

    assert!(!raw_exists(13, &cached).await, "the cache's key is gone");
    assert!(!raw_exists(13, &tagged).await, "and its tagged key");
    assert_eq!(
        raw_get(13, &other_cache_prefix).await.as_deref(),
        Some("kept"),
        "a key under the connection prefix only survives"
    );
    assert_eq!(
        raw_get(13, &no_connection_prefix).await.as_deref(),
        Some("kept"),
        "a key without the connection prefix survives"
    );
}

// ---- PAR-134: CacheHit and CacheMissed ------------------------------------

#[tokio::test]
async fn get_and_missing_of_an_absent_key_dispatch_two_cache_missed() {
    let _container = install_memory_cache();
    let _events = EventFacade::fake();

    let read: Option<String> = Cache::get("absent").await.expect("get");
    assert_eq!(read, None);
    assert!(Cache::missing("absent").await.expect("missing"));

    assert_eq!(dispatched_count::<CacheMissed>(|e| e.key == "absent"), 2);
    assert_eq!(dispatched_count::<CacheHit>(|_| true), 0);
    let missed = dispatched::<CacheMissed>(|_| true);
    assert!(missed.iter().all(|e| e.store == "memory"), "{missed:?}");
}

#[tokio::test]
async fn a_stored_null_is_present_and_dispatches_cache_hit() {
    let _container = install_memory_cache();
    let _events = EventFacade::fake();

    Cache::put("n", &Value::Null, None).await.expect("put");
    assert!(!Cache::missing("n").await.expect("missing"));
    let read: Option<Value> = Cache::get("n").await.expect("get");
    assert_eq!(read, Some(Value::Null));

    assert_eq!(dispatched_count::<CacheMissed>(|_| true), 0);
    let hits = dispatched::<CacheHit>(|e| e.key == "n");
    assert_eq!(hits.len(), 2, "missing and get each hit");
    assert_eq!(hits[0].value, None, "missing did not fetch the value");
    assert_eq!(hits[1].value, Some(Value::Null), "get fetched the null");
}

#[tokio::test]
async fn a_hit_carries_the_value_the_read_fetched() {
    let _container = install_memory_cache();
    let _events = EventFacade::fake();

    Cache::put("user", &json!({"name": "alice"}), None)
        .await
        .expect("put");
    let _: Option<Value> = Cache::get("user").await.expect("get");
    assert!(Cache::has("user").await.expect("has"));

    let hits = dispatched::<CacheHit>(|e| e.key == "user");
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].value, Some(json!({"name": "alice"})));
    assert_eq!(hits[0].store, "memory");
    assert_eq!(hits[1].value, None, "has checks presence without the value");
}

#[tokio::test]
async fn reads_built_on_get_dispatch_one_event_each() {
    let _container = install_memory_cache();
    let _events = EventFacade::fake();

    let _: String = Cache::remember("remembered", None, || async { Ok("v".to_string()) })
        .await
        .expect("remember miss");
    let _: String = Cache::remember("remembered", None, || async { Ok("w".to_string()) })
        .await
        .expect("remember hit");
    let _: String = Cache::remember_forever("forever", || async { Ok("v".to_string()) })
        .await
        .expect("remember_forever");
    let _: String = Cache::sear("forever", || async { Ok("w".to_string()) })
        .await
        .expect("sear");
    let pulled: Option<String> = Cache::pull("forever").await.expect("pull");
    assert_eq!(pulled.as_deref(), Some("v"));
    let _: Option<String> = Cache::pull("forever").await.expect("pull again");

    assert_eq!(
        dispatched_count::<CacheMissed>(|e| e.key == "remembered"),
        1
    );
    assert_eq!(dispatched_count::<CacheHit>(|e| e.key == "remembered"), 1);
    // remember_forever misses, sear hits, pull hits, the second pull misses.
    assert_eq!(dispatched_count::<CacheMissed>(|e| e.key == "forever"), 2);
    assert_eq!(dispatched_count::<CacheHit>(|e| e.key == "forever"), 2);
}

#[tokio::test]
async fn the_debug_text_of_a_cache_hit_does_not_print_the_value() {
    let _container = install_memory_cache();
    let _events = EventFacade::fake();

    Cache::put("secret", &"s3cret-value", None)
        .await
        .expect("put");
    let _: Option<String> = Cache::get("secret").await.expect("get");

    let hits = dispatched::<CacheHit>(|e| e.key == "secret");
    assert_eq!(hits.len(), 1);
    let text = format!("{:?}", hits[0]);
    assert!(
        !text.contains("s3cret-value"),
        "Debug prints the value: {text}"
    );
    assert!(text.contains("secret"), "Debug still names the key: {text}");
}

/// A listener that always fails, for both events.
struct FailingListener;

#[suprnova::async_trait]
impl Listener<CacheHit> for FailingListener {
    async fn handle(&self, _event: &CacheHit) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("the hit listener failed"))
    }
}

#[suprnova::async_trait]
impl Listener<CacheMissed> for FailingListener {
    async fn handle(&self, _event: &CacheMissed) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("the miss listener failed"))
    }
}

#[tokio::test]
async fn a_failing_listener_does_not_fail_the_read() {
    let _container = install_memory_cache();
    let listener = Arc::new(FailingListener);
    EventFacade::listen::<CacheHit, _>(listener.clone()).await;
    EventFacade::listen::<CacheMissed, _>(listener).await;

    let read: Option<String> = Cache::get("absent").await.expect("a miss still answers");
    assert_eq!(read, None);
    Cache::put("present", &"v", None).await.expect("put");
    let read: Option<String> = Cache::get("present").await.expect("a hit still answers");
    assert_eq!(read.as_deref(), Some("v"));
    assert!(Cache::has("present").await.expect("has still answers"));
    assert!(
        Cache::missing("absent")
            .await
            .expect("missing still answers")
    );
}

// ---- PAR-134: Cache::remember_with_ttl -------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Token {
    value: String,
    expires_in: u64,
}

#[tokio::test]
async fn remember_with_ttl_takes_the_lifetime_from_the_value() {
    let (_container, recorder) = install_recorder(3600);

    let token: Token = Cache::remember_with_ttl(
        "token",
        |t: &Token| Some(Duration::from_secs(t.expires_in)),
        || async {
            Ok(Token {
                value: "abc".to_string(),
                expires_in: 120,
            })
        },
    )
    .await
    .expect("remember_with_ttl");

    assert_eq!(token.expires_in, 120);
    assert_eq!(
        recorder.ttl_for("token"),
        Some(Some(Duration::from_secs(120))),
        "the value is stored for the 120 seconds it reports"
    );
}

#[tokio::test]
async fn remember_with_ttl_returns_a_cached_value_without_computing() {
    let (_container, recorder) = install_recorder(3600);
    let cached = Token {
        value: "cached".to_string(),
        expires_in: 5,
    };
    Cache::put("token", &cached, None).await.expect("put");

    let token: Token = Cache::remember_with_ttl(
        "token",
        |_: &Token| -> Option<Duration> { panic!("a hit computes no lifetime") },
        || async { Err::<Token, _>(FrameworkError::internal("a hit computes no value")) },
    )
    .await
    .expect("remember_with_ttl");

    assert_eq!(token, cached);
    assert_eq!(
        recorder.ttl_for("token"),
        Some(Some(Duration::from_secs(3600))),
        "only the put wrote"
    );
}

#[tokio::test]
async fn remember_with_ttl_none_uses_the_configured_default() {
    let (_container, recorder) = install_recorder(3600);

    let value: String = Cache::remember_with_ttl(
        "defaulted",
        |_: &String| None,
        || async { Ok("v".to_string()) },
    )
    .await
    .expect("remember_with_ttl");

    assert_eq!(value, "v");
    assert_eq!(
        recorder.ttl_for("defaulted"),
        Some(Some(Duration::from_secs(3600))),
        "None means the configured default, as remember's None does"
    );
}

#[tokio::test]
async fn remember_with_ttl_zero_returns_the_value_unstored() {
    let (_container, recorder) = install_recorder(3600);

    let value: String = Cache::remember_with_ttl(
        "fleeting",
        |_: &String| Some(Duration::ZERO),
        || async { Ok("v".to_string()) },
    )
    .await
    .expect("remember_with_ttl");

    assert_eq!(value, "v");
    assert_eq!(recorder.ttl_for("fleeting"), None, "nothing was written");
    assert!(Cache::missing("fleeting").await.expect("missing"));
}

#[tokio::test]
async fn remember_with_ttl_passes_on_the_computation_error() {
    let (_container, recorder) = install_recorder(3600);

    let result: Result<String, FrameworkError> = Cache::remember_with_ttl(
        "failed",
        |_: &String| Some(Duration::from_secs(60)),
        || async { Err(FrameworkError::internal("the source is down")) },
    )
    .await;

    assert!(result.is_err());
    assert_eq!(recorder.ttl_for("failed"), None, "nothing was written");
}
