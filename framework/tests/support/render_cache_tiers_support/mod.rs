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
use suprnova::DB;
use suprnova::database::transaction::ExecutorChoice;
use suprnova::render_cache::providers::sql_now_ms;
use suprnova::testing::{TestContainer, TestContainerGuard, TestDatabase};
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
    let exec = ExecutorChoice::resolve_read(None, None, None)
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

/// One instance record address, distinct per `tag`.
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

/// One promotion reservation address, distinct per `tag`.
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
