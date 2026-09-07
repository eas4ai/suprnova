//! Shared setup for the Tier 1 database provider tests in
//! `framework/tests/render_cache/tiers.rs`.
//!
//! Deliberately small and self-contained, the same way
//! `render_cache_operations_support` is: these tests drive
//! `SqlRenderStore` directly against a real database rather than through
//! the middleware, so nothing here needs the request harness.
//!
//! [`boot`] applies both render cache migrations to a fresh in-memory
//! SQLite database and mounts it on the thread-local test container, so
//! every `DB::*` resolution inside the store lands on it.
//! [`boot_without_the_tier_tables`] applies only the original migration,
//! which is what `tier_migration_present` has to report `false` for.
//! [`reset_and_migrate`] is the live-database counterpart, following
//! `render_cache/ledger.rs`'s own pattern: drop the tier tables if a prior
//! run left them behind, apply the tier migration, and mount the
//! connection.
#![allow(dead_code)]

use bytes::Bytes;
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::database::transaction::ExecutorChoice;
use suprnova::render_cache::providers::sql_now_ms;
use suprnova::testing::{TestContainer, TestContainerGuard, TestDatabase};
use suprnova::{DB, PRIMARY_CONNECTION_NAME};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{IdempotencyKey, InstanceId, KeyId, ScopeFingerprint, UnixMillis};
use suprnova_live::ledger::{InstanceRecordKey, PromotionRecordKey};
use suprnova_live::render_cache::RepresentationClass;
use suprnova_live::render_cache::entry::{CompleteEntry, EntryHeader, SafeHeaders, encode};
use suprnova_live::render_cache::generation::GenerationSet;
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::store::PublicationFence;
use suprnova_live::render_cache::variance::VarianceDescriptor;

/// Both render cache migrations: the original tables and the tier tables
/// the Tier 1 providers need.
pub struct TierMigrator;

#[async_trait::async_trait]
impl MigratorTrait for TierMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(suprnova::render_cache::migration::Migration),
            Box::new(suprnova::render_cache::migration::TierMigration),
        ]
    }
}

/// The original render cache migration alone - a database that carries the
/// generation ledger but none of the tier tables.
pub struct BaseOnlyMigrator;

#[async_trait::async_trait]
impl MigratorTrait for BaseOnlyMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(suprnova::render_cache::migration::Migration)]
    }
}

/// A fresh SQLite database with both render cache migrations applied.
pub async fn boot() -> TestDatabase {
    TestDatabase::fresh::<TierMigrator>()
        .await
        .expect("both render cache migrations apply cleanly to a fresh SQLite database")
}

/// A fresh SQLite database carrying only the original render cache
/// migration, so the tier tables are genuinely absent.
pub async fn boot_without_the_tier_tables() -> TestDatabase {
    TestDatabase::fresh::<BaseOnlyMigrator>()
        .await
        .expect("the original render cache migration applies cleanly to a fresh SQLite database")
}

/// The key ring the fixture keys and entries are derived under.
pub fn keys() -> SnapshotKeyRing {
    let active = KeyRecord::new(
        KeyId::parse("render-cache-test").expect("key id"),
        RootKey::new(vec![9; 32]).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(u64::MAX / 2),
        UnixMillis::new(u64::MAX),
    )
    .expect("key record");
    SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
}

/// A fixture lookup key for a route pattern.
pub fn key(pattern: &str) -> RenderKey {
    RenderKey::for_test(&keys(), pattern)
}

/// A publication fence at `epoch` and `token`, with a digest that varies
/// with the token so a replaced row's digest column is visibly the new
/// one.
pub fn fence(epoch: u64, token: u64) -> PublicationFence {
    let mut generation_digest = [0_u8; 32];
    generation_digest[0] = u8::try_from(token % 251).expect("a byte");
    PublicationFence {
        epoch,
        generation_digest,
        token,
    }
}

/// A real, signed Complete entry for `pattern`, the bytes a publication
/// actually carries. The corruption test needs bytes that decode before
/// they are tampered with; nothing else about the entry matters here.
pub fn encoded_entry(pattern: &str) -> Bytes {
    let ring = keys();
    let entry = CompleteEntry::new(
        EntryHeader {
            key: RenderKey::for_test(&ring, pattern),
            class: RepresentationClass::PublicShared,
            variance: VarianceDescriptor::new(),
            published_at_ms: 1_000,
            fresh_ms: 60_000,
            stale_servable_ms: 0,
            stale_on_error_ms: 0,
            observed: GenerationSet::default(),
            epoch: 1,
            seed_deadline_ms: None,
            status: 200,
            headers: SafeHeaders::from_pairs([("content-type", "text/html; charset=utf-8")])
                .expect("safe headers"),
            content_encoding: None,
        },
        Bytes::from_static(b"<!doctype html><html><body>tier one</body></html>"),
    );
    encode(&entry, &ring).expect("encode the fixture entry")
}

/// The database's own clock, as milliseconds since the Unix epoch.
///
/// Every expiry a tier adapter writes or compares is measured on this
/// clock, so a test that wants a record to outlive a scenario - or to have
/// elapsed before one - has to start from the same number the adapter will.
/// Reading it through [`sql_now_ms`] rather than a hard-coded SQLite
/// expression keeps the live Postgres and MySQL copies of these tests
/// honest.
///
/// # Panics
///
/// Panics when no test database is mounted, when the backend is one no
/// dialect expression is proven against, or when the clock read fails -
/// each of which is a broken fixture rather than a provider failure.
pub async fn store_now_ms() -> u64 {
    let exec = ExecutorChoice::resolve_read(None, Some(PRIMARY_CONNECTION_NAME), None)
        .await
        .expect("a read executor over the test database");
    let sql = format!(
        "SELECT {}",
        sql_now_ms(exec.backend()).expect("a supported dialect")
    );
    let now: i64 = DB::scalar(&sql, vec![]).await.expect("the store clock");
    u64::try_from(now).expect("the store clock is after the Unix epoch")
}

/// An instant `after_ms` milliseconds ahead of the database's own clock.
pub async fn store_deadline(after_ms: u64) -> UnixMillis {
    UnixMillis::new(store_now_ms().await.saturating_add(after_ms))
}

/// The one trusted scope every record fixture in these tests belongs to.
///
/// # Panics
///
/// Panics when the fixture bytes stop being a valid scope fingerprint.
pub fn scope() -> ScopeFingerprint {
    ScopeFingerprint::from_bytes(&varied::<32>(0x10)).expect("the fixture scope is valid")
}

/// One instance record address at the *shortest* identity the engine
/// accepts, distinct per `tag`.
///
/// # Panics
///
/// Panics when the fixture bytes stop being a valid instance identity.
pub fn instance_key(tag: u8) -> InstanceRecordKey {
    InstanceRecordKey {
        scope: scope(),
        instance_id: InstanceId::from_bytes(&varied::<16>(tag))
            .expect("the fixture instance identity is valid"),
    }
}

/// One instance record address at the *longest* identity the engine accepts.
///
/// `InstanceId` is 16 to 32 bytes, so its hex runs from 32 to 64 characters
/// and the column has to hold the wide end. A fixture that only ever used
/// the narrow end would pass against a column half the size it needs, while
/// PostgreSQL refused every real full-width identity and a non-strict MySQL
/// truncated two distinct ones onto one row.
///
/// # Panics
///
/// Panics when the fixture bytes stop being a valid instance identity.
pub fn wide_instance_key(tag: u8) -> InstanceRecordKey {
    InstanceRecordKey {
        scope: scope(),
        instance_id: InstanceId::from_bytes(&varied::<32>(tag))
            .expect("the widest fixture instance identity is valid"),
    }
}

/// One promotion reservation address at the shortest retry identity the
/// engine accepts, distinct per `tag`.
///
/// # Panics
///
/// Panics when the fixture bytes stop being a valid retry identity.
pub fn promotion_key(tag: u8) -> PromotionRecordKey {
    PromotionRecordKey {
        scope: scope(),
        idempotency_key: IdempotencyKey::from_bytes(&varied::<16>(tag))
            .expect("the fixture retry identity is valid"),
    }
}

/// One promotion reservation address at the longest retry identity the
/// engine accepts.
///
/// This is the width that actually arrives: a retry identity is built from
/// the browser-proposed nonce, which may be a full 32 bytes, so 64-character
/// hex reaches the column straight off the wire. See [`wide_instance_key`]
/// for what a column sized for the narrow end would do with it.
///
/// # Panics
///
/// Panics when the fixture bytes stop being a valid retry identity.
pub fn wide_promotion_key(tag: u8) -> PromotionRecordKey {
    PromotionRecordKey {
        scope: scope(),
        idempotency_key: IdempotencyKey::from_bytes(&varied::<32>(tag))
            .expect("the widest fixture retry identity is valid"),
    }
}

/// Identity bytes that vary within one fixture as well as between two, so
/// no fixture is a run of one repeated byte.
fn varied<const LENGTH: usize>(start: u8) -> [u8; LENGTH] {
    std::array::from_fn(|offset| start.wrapping_add(u8::try_from(offset % 256).unwrap_or(0)))
}

/// Connects to a live database, returning `None` when it is not reachable.
pub async fn try_connect_live(url: &str) -> Option<sea_orm::DatabaseConnection> {
    use sea_orm::ConnectOptions;
    use std::time::Duration;
    let mut opts = ConnectOptions::new(url.to_string());
    opts.connect_timeout(Duration::from_secs(2))
        .acquire_timeout(Duration::from_secs(2));
    sea_orm::Database::connect(opts).await.ok()
}

/// Drops the four tier tables if a prior failed run left them behind,
/// applies the tier migration fresh, and mounts the connection on the
/// thread-local test container so `DB::*` resolves to it - the same shape
/// as `render_cache/ledger.rs`'s own `reset_and_migrate`.
pub async fn reset_and_migrate(conn: sea_orm::DatabaseConnection) -> TestContainerGuard {
    use sea_orm::ConnectionTrait;
    for table in [
        "suprnova_render_entries",
        "suprnova_render_leases",
        "suprnova_live_instances",
        "suprnova_live_promotions",
    ] {
        let _ = conn
            .execute_raw(sea_orm::Statement::from_string(
                conn.get_database_backend(),
                format!("DROP TABLE IF EXISTS {table}"),
            ))
            .await;
    }
    let manager = sea_orm_migration::SchemaManager::new(&conn);
    suprnova::render_cache::migration::TierMigration
        .up(&manager)
        .await
        .expect("the tier migration applies to the live database");

    suprnova::render_cache::mark_installed();
    let guard = TestContainer::fake();
    TestContainer::singleton(suprnova::DbConnection::from_raw(conn));
    guard
}

// --- Tier 2: Redis ---

/// Where the live Redis tests connect.
///
/// `REDIS_TEST_URL` first so one throwaway instance can be pointed at
/// without disturbing whatever `REDIS_URL` names, then `REDIS_URL`, then the
/// default port - the same resolution order the cache and queue suites use.
pub fn redis_url() -> String {
    std::env::var("REDIS_TEST_URL")
        .or_else(|_| std::env::var("REDIS_URL"))
        .unwrap_or_else(|_| "redis://127.0.0.1:6379/".to_owned())
}

/// A provider configuration whose key namespace no other test shares.
///
/// Every live Redis test scopes itself under a fresh UUID, so a concurrent
/// run, a prior failed run, and whatever else happens to live in the
/// instance are all invisible to it.
pub fn redis_config() -> suprnova::render_cache::providers::RedisProviderConfig {
    suprnova::render_cache::providers::RedisProviderConfig {
        url: redis_url(),
        prefix: format!("suprnova_tiers_test:{}:", uuid::Uuid::new_v4()),
    }
}

/// A configuration pointed at a loopback port nothing listens on.
///
/// The port is taken and released, so a connection to it is refused rather
/// than accepted and left hanging - which is what makes the failure a
/// provider failure a test can assert on quickly.
///
/// # Panics
///
/// Panics when no loopback port can be bound, which is a broken environment
/// rather than a provider failure.
pub fn redis_config_on_a_closed_port() -> suprnova::render_cache::providers::RedisProviderConfig {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("a loopback port for the closed-port fixture");
    let port = listener
        .local_addr()
        .expect("the bound loopback address")
        .port();
    drop(listener);
    suprnova::render_cache::providers::RedisProviderConfig {
        url: format!("redis://127.0.0.1:{port}/"),
        prefix: "suprnova_tiers_closed:".to_owned(),
    }
}

/// A live connection for the assertions and the cleanup that have to look at
/// Redis itself rather than through an adapter.
///
/// # Panics
///
/// Panics when the URL is unusable or the instance does not answer `PING`,
/// which for an `--ignored` live test is a misconfigured run rather than a
/// failure of the code under test.
pub async fn boot_redis() -> (
    suprnova::render_cache::providers::RedisProviderConfig,
    redis::aio::ConnectionManager,
) {
    let config = redis_config();
    let client =
        redis::Client::open(config.url.clone()).expect("REDIS_TEST_URL is a valid Redis URL");
    let mut conn = redis::aio::ConnectionManager::new(client)
        .await
        .expect("live Redis is reachable - set REDIS_TEST_URL to a disposable instance");
    let pong: String = redis::cmd("PING")
        .query_async(&mut conn)
        .await
        .expect("the live Redis answers PING");
    assert_eq!(pong, "PONG");
    (config, conn)
}

/// Redis's own clock, as milliseconds since the Unix epoch.
///
/// Every expiry the Redis adapters write or compare is measured on this
/// clock, exactly as the SQL adapters measure theirs on the database's, so a
/// test that wants a record to outlive a scenario starts from the same
/// number the adapter will.
///
/// # Panics
///
/// Panics when `TIME` cannot be read, which is a broken fixture.
pub async fn redis_now_ms(conn: &mut redis::aio::ConnectionManager) -> u64 {
    let clock: (u64, u64) = redis::cmd("TIME")
        .query_async(conn)
        .await
        .expect("the Redis store clock");
    clock
        .0
        .saturating_mul(1_000)
        .saturating_add(clock.1 / 1_000)
}

/// An instant `after_ms` milliseconds ahead of Redis's own clock.
pub async fn redis_deadline(conn: &mut redis::aio::ConnectionManager, after_ms: u64) -> UnixMillis {
    UnixMillis::new(redis_now_ms(conn).await.saturating_add(after_ms))
}

/// Every key currently under `prefix`, in no particular order.
///
/// # Panics
///
/// Panics when the scan fails, which is a broken fixture.
pub async fn redis_keys(conn: &mut redis::aio::ConnectionManager, prefix: &str) -> Vec<String> {
    let mut cursor = "0".to_owned();
    let mut found = Vec::new();
    loop {
        let (next, batch): (String, Vec<String>) = redis::cmd("SCAN")
            .arg(&cursor)
            .arg("MATCH")
            .arg(format!("{prefix}*"))
            .arg("COUNT")
            .arg(512)
            .query_async(conn)
            .await
            .expect("scan the test prefix");
        found.extend(batch);
        cursor = next;
        if cursor == "0" {
            return found;
        }
    }
}

/// Deletes every key this test wrote, so a shared instance is left as it was
/// found.
///
/// # Panics
///
/// Panics when the deletion fails, which is a broken fixture.
pub async fn clear_redis_prefix(conn: &mut redis::aio::ConnectionManager, prefix: &str) {
    let keys = redis_keys(conn, prefix).await;
    for batch in keys.chunks(256) {
        let mut command = redis::cmd("DEL");
        for key in batch {
            command.arg(key);
        }
        let _: i64 = command
            .query_async(conn)
            .await
            .expect("delete the test prefix");
    }
}
