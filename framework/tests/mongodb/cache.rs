//! PAR-187: `CACHE_DRIVER=mongodb` selects a `CacheStore` over a `cache`
//! collection with a TTL index on the expiry, and locks over a
//! `cache_locks` collection keyed uniquely with an expiry.
//!
//! The unignored tests read the documents the store sends (rendered without
//! a server), the selection by the environment (in a child process) and
//! the errors of a server that does not answer. The `mongodb_` tests run
//! each falsifier against the server at `MONGODB_TEST_URL`.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use suprnova::bson::{Bson, Document, doc};
use suprnova::cache::{CacheConfig, CacheDriver};
use suprnova::testing::TestClock;
use suprnova::{CacheStore, ConditionalIncrement, Mongo, MongoCache};

use crate::store_support::{drop_collections, run_alone_with_drivers, server, unique, unreachable};
use crate::support::UNREACHABLE_URI;

fn is_child() -> bool {
    crate::own_process::is_child()
}

fn moment() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-10T12:00:00.250Z")
        .expect("valid time")
        .with_timezone(&Utc)
}

fn bson_time(at: DateTime<Utc>) -> suprnova::bson::DateTime {
    suprnova::bson::DateTime::from_millis(at.timestamp_millis())
}

fn config(prefix: &str) -> CacheConfig {
    CacheConfig::builder().prefix(prefix).default_ttl(0).build()
}

// --- Selection by the environment ---------------------------------------------

#[test]
fn cache_driver_parses_mongodb() {
    assert_eq!(CacheDriver::parse("mongodb").unwrap(), CacheDriver::MongoDb);
    assert_eq!(
        CacheDriver::parse(" MongoDB ").unwrap(),
        CacheDriver::MongoDb
    );
    let error = CacheDriver::parse("memcached").expect_err("not a driver");
    let message = error.to_string();
    assert!(
        message.contains("memory") && message.contains("redis") && message.contains("mongodb"),
        "the error lists every driver: {message}"
    );
}

#[test]
fn cache_driver_mongodb_builds_the_mongodb_store() {
    run_alone_with_drivers(
        "cache::cache_driver_mongodb_builds_the_mongodb_store_child",
        &[
            ("CACHE_DRIVER", "mongodb"),
            ("CACHE_DEFAULT_TTL", "120"),
            ("CACHE_PREFIX", "shop-cache-"),
            ("MONGODB_URI", UNREACHABLE_URI),
            ("MONGODB_DATABASE", "suprnova_cache"),
        ],
    );
}

#[tokio::test]
async fn cache_driver_mongodb_builds_the_mongodb_store_child() {
    if !is_child() {
        return;
    }
    let config = CacheConfig::from_env().expect("CACHE_DRIVER=mongodb parses");
    assert_eq!(config.driver, CacheDriver::MongoDb);
    Mongo::bootstrap()
        .await
        .expect("the boot registers the connection");
    let store = MongoCache::from_config(&config).expect("the store opens without a server");
    assert_eq!(store.name(), "mongodb");
    assert!(store.locks_are_shared(), "every process shares the locks");
    assert_eq!(store.default_ttl(), Some(Duration::from_secs(120)));
    let error = store.get_raw("key").await.expect_err("no server answers");
    assert!(error.to_string().contains("MongoDB"), "{error}");
}

#[test]
fn cache_driver_mongodb_without_a_connection_is_an_error_naming_mongodb_uri() {
    run_alone_with_drivers(
        "cache::cache_driver_mongodb_without_a_connection_is_an_error_naming_mongodb_uri_child",
        &[("CACHE_DRIVER", "mongodb")],
    );
}

#[tokio::test]
async fn cache_driver_mongodb_without_a_connection_is_an_error_naming_mongodb_uri_child() {
    if !is_child() {
        return;
    }
    let config = CacheConfig::from_env().expect("CACHE_DRIVER=mongodb parses");
    let error = MongoCache::from_config(&config)
        .err()
        .expect("no MongoDB connection");
    assert!(error.to_string().contains("MONGODB_URI"), "{error}");
}

// --- The documents the store sends ---------------------------------------------

#[tokio::test]
async fn a_put_keeps_integers_as_numbers_and_stores_its_expiry() {
    let store = MongoCache::new(&unreachable().await, &config("app:"));
    let now = moment();

    let put = store.rendered_put("hits", "42", Some(Duration::from_secs(60)), now);
    assert_eq!(
        put.get_document("filter").unwrap(),
        &doc! { "_id": "app:hits" }
    );
    assert_eq!(
        put.get_document("replacement").unwrap(),
        &doc! {
            "_id": "app:hits",
            "value": 42_i64,
            "expires_at": bson_time(now + chrono::Duration::seconds(60)),
            "tags": [],
        }
    );
    assert!(put.get_bool("upsert").unwrap());

    for (raw, stored) in [
        ("-5", Bson::Int64(-5)),
        ("0", Bson::Int64(0)),
        ("\"text\"", Bson::String("\"text\"".to_owned())),
        ("007", Bson::String("007".to_owned())),
        ("-0", Bson::String("-0".to_owned())),
        ("4.5", Bson::String("4.5".to_owned())),
        (
            "9223372036854775808",
            Bson::String("9223372036854775808".to_owned()),
        ),
    ] {
        let put = store.rendered_put("k", raw, None, now);
        let replacement = put.get_document("replacement").unwrap();
        assert_eq!(replacement.get("value"), Some(&stored), "{raw}");
        assert_eq!(replacement.get("expires_at"), Some(&Bson::Null), "forever");
    }
}

#[tokio::test]
async fn add_increment_tags_and_locks_render_one_atomic_operation_each() {
    let store = MongoCache::new(&unreachable().await, &config("app:"));
    let now = moment();
    let live = doc! {
        "$or": [
            { "expires_at": Bson::Null },
            { "expires_at": { "$gt": bson_time(now) } },
        ]
    };

    let add = store.rendered_add("slot", "1", None, now);
    assert_eq!(
        add.get_document("filter").unwrap(),
        &doc! { "_id": "app:slot", "expires_at": { "$lte": bson_time(now) } },
        "replaces only an expired entry; a live one makes the upsert collide"
    );
    assert!(add.get_bool("upsert").unwrap());

    let increment = store.rendered_increment("hits", 5, now);
    let mut filter = doc! { "_id": "app:hits" };
    filter.extend(live.clone());
    assert_eq!(increment.get_document("filter").unwrap(), &filter);
    assert_eq!(
        increment.get_document("update").unwrap(),
        &doc! {
            "$inc": { "value": 5_i64 },
            "$setOnInsert": { "expires_at": Bson::Null, "tags": [] },
        }
    );
    assert!(increment.get_bool("upsert").unwrap());

    let below = store.rendered_increment_if_below("hits", 1, 10, now);
    let mut filter = doc! { "_id": "app:hits", "value": { "$lt": 10_i64 } };
    filter.extend(live);
    assert_eq!(below.get_document("filter").unwrap(), &filter);

    let tagged = store.rendered_tagged_put(&["users", "posts"], "page", "\"html\"", None, now);
    assert_eq!(
        tagged
            .get_document("replacement")
            .unwrap()
            .get_array("tags")
            .unwrap(),
        &vec![Bson::from("users"), Bson::from("posts")]
    );
    assert_eq!(
        store.rendered_flush_tags(&["users"]),
        doc! { "filter": { "tags": { "$in": ["users"] } } }
    );

    let lock = store.rendered_acquire_lock("job", "token-1", Duration::from_secs(10), now);
    assert_eq!(
        lock.get_document("filter").unwrap(),
        &doc! { "_id": "app:job", "expires_at": { "$lte": bson_time(now) } }
    );
    assert_eq!(
        lock.get_document("update").unwrap(),
        &doc! { "$set": {
            "owner": "token-1",
            "expires_at": bson_time(now + chrono::Duration::seconds(10)),
        } }
    );
    assert!(lock.get_bool("upsert").unwrap());
}

#[tokio::test]
async fn the_indexes_expire_entries_and_locks_on_their_expiry() {
    let store = MongoCache::new(&unreachable().await, &config("app:"));
    let indexes = store.rendered_indexes();
    let ttl = doc! { "key": { "expires_at": 1 }, "expireAfterSeconds": 0_i64 };
    assert!(
        indexes
            .get_array("cache")
            .unwrap()
            .contains(&Bson::Document(ttl.clone())),
        "{indexes}"
    );
    assert!(
        indexes
            .get_array("cache")
            .unwrap()
            .contains(&Bson::Document(doc! { "key": { "tags": 1 } })),
        "{indexes}"
    );
    assert!(
        indexes
            .get_array("cache_locks")
            .unwrap()
            .contains(&Bson::Document(ttl)),
        "{indexes}"
    );
}

#[tokio::test]
async fn a_server_that_does_not_answer_fails_the_store_as_mongodb() {
    let store = MongoCache::new(&unreachable().await, &config("app:"));
    for error in [
        store.get_raw("k").await.expect_err("no server"),
        store.put_raw("k", "1", None).await.expect_err("no server"),
        store.increment("k", 1).await.expect_err("no server"),
        store
            .acquire_lock("k", Duration::from_secs(1))
            .await
            .expect_err("no server"),
    ] {
        assert!(error.to_string().contains("MongoDB"), "{error}");
    }
}

#[tokio::test]
async fn a_collection_name_mongodb_refuses_is_an_error() {
    let connection = unreachable().await;
    assert!(MongoCache::with_collections(&connection, &config("a:"), "", "locks").is_err());
    assert!(MongoCache::with_collections(&connection, &config("a:"), "cache", "lo$cks").is_err());
    assert!(
        MongoCache::with_collections(&connection, &config("a:"), "app_cache", "app_locks").is_ok()
    );
}

// --- Against a server --------------------------------------------------------------

async fn store_on_server(
    prefix: &str,
) -> (Arc<MongoCache>, suprnova::MongoConnection, [String; 2]) {
    let connection = server().await;
    let names = [unique("par187_cache"), unique("par187_locks")];
    let store = MongoCache::with_collections(&connection, &config(prefix), &names[0], &names[1])
        .expect("valid names");
    (Arc::new(store), connection, names)
}

async fn cleanup(connection: &suprnova::MongoConnection, names: &[String; 2]) {
    drop_collections(connection, &[&names[0], &names[1]]).await;
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_an_entry_put_for_one_second_is_not_read_after_two() {
    let (store, connection, names) = store_on_server("ttl:").await;
    let clock = TestClock::freeze();

    store
        .put_raw("short", "\"lived\"", Some(Duration::from_secs(1)))
        .await
        .expect("put");
    store.put_raw("forever", "1", None).await.expect("put");
    assert_eq!(
        store.get_raw("short").await.unwrap().as_deref(),
        Some("\"lived\"")
    );
    clock.advance(chrono::Duration::seconds(2));
    assert_eq!(
        store.get_raw("short").await.unwrap(),
        None,
        "the expiry is checked"
    );
    assert!(!store.has("short").await.unwrap());
    clock.advance(chrono::Duration::days(3650));
    assert_eq!(
        store.get_raw("forever").await.unwrap().as_deref(),
        Some("1")
    );

    // The server removes what expired: a TTL index on the expiry.
    let indexes: Vec<_> = connection
        .collection::<Document>(&names[0])
        .list_indexes()
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(
        indexes.iter().any(|index| {
            index.keys == doc! { "expires_at": 1 }
                && index
                    .options
                    .as_ref()
                    .and_then(|options| options.expire_after)
                    == Some(Duration::ZERO)
        }),
        "a TTL index on expires_at: {indexes:?}"
    );
    cleanup(&connection, &names).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_add_raw_on_a_present_key_answers_false() {
    let (store, connection, names) = store_on_server("add:").await;

    assert!(store.add_raw("k", "1", None).await.unwrap());
    assert!(
        !store.add_raw("k", "2", None).await.unwrap(),
        "the key is present"
    );
    assert_eq!(store.get_raw("k").await.unwrap().as_deref(), Some("1"));

    let racers: Vec<_> = (0..8)
        .map(|n| {
            let store = Arc::clone(&store);
            tokio::spawn(async move { store.add_raw("raced", &n.to_string(), None).await })
        })
        .collect();
    let mut added = 0;
    for racer in racers {
        if racer.await.unwrap().expect("add") {
            added += 1;
        }
    }
    assert_eq!(added, 1, "one insert wins");
    cleanup(&connection, &names).await;
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_add_raw_replaces_an_expired_entry() {
    let (store, connection, names) = store_on_server("expired:").await;
    let clock = TestClock::freeze();
    assert!(
        store
            .add_raw("k", "1", Some(Duration::from_secs(1)))
            .await
            .unwrap()
    );
    clock.advance(chrono::Duration::seconds(2));
    assert!(
        store.add_raw("k", "2", None).await.unwrap(),
        "the old one expired"
    );
    assert_eq!(store.get_raw("k").await.unwrap().as_deref(), Some("2"));
    cleanup(&connection, &names).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_concurrent_increments_lose_no_step() {
    let (store, connection, names) = store_on_server("count:").await;

    let tasks: Vec<_> = (0..4)
        .map(|_| {
            let store = Arc::clone(&store);
            tokio::spawn(async move {
                for _ in 0..25 {
                    store.increment("hits", 1).await.expect("increment");
                }
            })
        })
        .collect();
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(store.get_raw("hits").await.unwrap().as_deref(), Some("100"));
    assert_eq!(store.decrement("hits", 40).await.unwrap(), 60);

    let ceilinged: Vec<_> = (0..12)
        .map(|_| {
            let store = Arc::clone(&store);
            tokio::spawn(async move { store.increment_if_below("seats", 1, 5).await })
        })
        .collect();
    let mut taken = 0;
    for task in ceilinged {
        if let ConditionalIncrement::Incremented(_) = task.await.unwrap().expect("counted") {
            taken += 1;
        }
    }
    assert_eq!(taken, 5, "never past the ceiling");
    assert_eq!(
        store.increment_if_below("seats", 1, 5).await.unwrap(),
        ConditionalIncrement::Unchanged(5)
    );

    store.put_raw("word", "\"text\"", None).await.unwrap();
    assert!(store.increment("word", 1).await.is_err(), "not a number");
    assert!(store.increment_if_below("word", 1, 9).await.is_err());
    assert_eq!(
        store.get_raw("word").await.unwrap().as_deref(),
        Some("\"text\"")
    );
    store
        .put_raw("max", &i64::MAX.to_string(), None)
        .await
        .unwrap();
    assert!(
        store.increment("max", 1).await.is_err(),
        "an overflow is an error"
    );
    assert_eq!(
        store.get_raw("max").await.unwrap(),
        Some(i64::MAX.to_string())
    );
    cleanup(&connection, &names).await;
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_a_counter_keeps_its_expiry_and_restarts_after_it() {
    let (store, connection, names) = store_on_server("expiry:").await;
    let clock = TestClock::freeze();
    store
        .put_raw("window", "7", Some(Duration::from_secs(60)))
        .await
        .unwrap();
    assert_eq!(store.increment("window", 1).await.unwrap(), 8);
    clock.advance(chrono::Duration::seconds(61));
    assert_eq!(
        store.get_raw("window").await.unwrap(),
        None,
        "the increment kept the expiry"
    );
    assert_eq!(
        store.increment("window", 1).await.unwrap(),
        1,
        "an expired counter reads as 0"
    );
    cleanup(&connection, &names).await;
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_flush_tags_removes_every_entry_with_the_tag_and_no_other() {
    let (store, connection, names) = store_on_server("tags:").await;
    store
        .tagged_put_raw(&["a"], "only_a", "1", None)
        .await
        .unwrap();
    store
        .tagged_put_raw(&["b"], "only_b", "2", None)
        .await
        .unwrap();
    store
        .tagged_put_raw(&["a", "b"], "both", "3", None)
        .await
        .unwrap();
    store
        .tagged_put_raw(&["a"], "retagged", "4", None)
        .await
        .unwrap();
    store.put_raw("retagged", "5", None).await.unwrap();

    store.flush_tags(&["a"]).await.expect("flush");
    assert_eq!(store.get_raw("only_a").await.unwrap(), None);
    assert_eq!(store.get_raw("both").await.unwrap(), None);
    assert_eq!(store.get_raw("only_b").await.unwrap().as_deref(), Some("2"));
    assert_eq!(
        store.get_raw("retagged").await.unwrap().as_deref(),
        Some("5"),
        "an untagged write cleared the tag"
    );
    cleanup(&connection, &names).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_two_concurrent_acquire_lock_never_both_succeed() {
    let (store, connection, names) = store_on_server("locks:").await;

    let contenders: Vec<_> = (0..8)
        .map(|_| {
            let store = Arc::clone(&store);
            tokio::spawn(async move { store.acquire_lock("job", Duration::from_secs(30)).await })
        })
        .collect();
    let mut tokens = Vec::new();
    for contender in contenders {
        if let Some(token) = contender.await.unwrap().expect("acquire") {
            tokens.push(token);
        }
    }
    assert_eq!(tokens.len(), 1, "one holder");
    let token = tokens.remove(0);

    assert!(!store.release_lock("job", "someone-else").await.unwrap());
    assert!(
        store
            .acquire_lock("job", Duration::from_secs(30))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !store
            .refresh_lock("job", "someone-else", Duration::from_secs(60))
            .await
            .unwrap()
    );
    assert!(
        store
            .refresh_lock("job", &token, Duration::from_secs(60))
            .await
            .unwrap()
    );
    assert!(store.release_lock("job", &token).await.unwrap());
    assert!(
        !store.release_lock("job", &token).await.unwrap(),
        "released once"
    );
    assert!(
        store
            .acquire_lock("job", Duration::from_secs(30))
            .await
            .unwrap()
            .is_some()
    );
    cleanup(&connection, &names).await;
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_an_expired_lock_is_taken_over_and_its_old_holder_releases_nothing() {
    let (store, connection, names) = store_on_server("takeover:").await;
    let clock = TestClock::freeze();
    let old = store
        .acquire_lock("job", Duration::from_secs(1))
        .await
        .unwrap()
        .expect("free");
    clock.advance(chrono::Duration::seconds(2));
    let new = store
        .acquire_lock("job", Duration::from_secs(30))
        .await
        .unwrap()
        .expect("the old lock expired");
    assert_ne!(old, new);
    assert!(!store.release_lock("job", &old).await.unwrap());
    assert!(store.release_lock("job", &new).await.unwrap());
    cleanup(&connection, &names).await;
}

#[tokio::test]
#[ignore = "needs MONGODB_TEST_URL"]
async fn mongodb_touch_forget_and_flush() {
    let (store, connection, names) = store_on_server("one:").await;
    let other = MongoCache::with_collections(&connection, &config("two:"), &names[0], &names[1])
        .expect("valid names");
    let clock = TestClock::freeze();

    store
        .put_raw("k", "1", Some(Duration::from_secs(5)))
        .await
        .unwrap();
    assert!(store.touch("k", Duration::from_secs(60)).await.unwrap());
    clock.advance(chrono::Duration::seconds(10));
    assert!(store.has("k").await.unwrap(), "touch extended the expiry");
    assert!(
        !store
            .touch("absent", Duration::from_secs(60))
            .await
            .unwrap()
    );

    assert!(store.forget("k").await.unwrap());
    assert!(!store.forget("k").await.unwrap());

    store.put_raw("a", "1", None).await.unwrap();
    other.put_raw("a", "2", None).await.unwrap();
    store.flush().await.expect("flush");
    assert_eq!(store.get_raw("a").await.unwrap(), None);
    assert_eq!(
        other.get_raw("a").await.unwrap().as_deref(),
        Some("2"),
        "a flush keeps to its prefix"
    );
    cleanup(&connection, &names).await;
}
