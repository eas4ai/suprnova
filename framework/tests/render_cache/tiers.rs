//! Tier 1: the database-backed L1 render store and the tier migration.
//!
//! Every test here drives `SqlRenderStore` against a real database - SQLite
//! by default, Postgres and MySQL through the `#[ignore]`d live tests at the
//! bottom - never a mock. What is proven: a publication round-trips its
//! bytes and its fence; `PublicationFence::supersedes` decides every
//! replacement, so a lower or equal fence is `Fenced` and leaves the stored
//! row exactly as it was; bytes over the bound are rejected before any
//! statement runs; a row whose bytes are tampered with in SQL is the
//! codec's miss, not the store's; expiry is measured by the database's own
//! clock, so `get` misses a row past its `expires_at_ms` and `sweep`
//! removes at most `batch` of them per call.

use bytes::Bytes;
use suprnova::DB;
use suprnova::render_cache::L1Provider;
use suprnova::render_cache::ledger::tier_migration_present;
use suprnova::render_cache::providers::SqlRenderStore;
use suprnova_live::render_cache::entry::{EntryLimits, decode};
use suprnova_live::render_cache::store::{PublishOutcome, RenderStore};

use crate::render_cache_tiers_support;
use render_cache_tiers_support::{
    boot, boot_without_the_tier_tables, encoded_entry, fence, key, keys, reset_and_migrate,
    try_connect_live,
};

/// Rows currently in the entries table, counted in SQL rather than through
/// the store, so a test can tell "the store reports a miss" from "the row
/// is gone".
async fn row_count() -> i64 {
    DB::scalar("SELECT COUNT(*) FROM suprnova_render_entries", vec![])
        .await
        .expect("count the entries table")
}

#[tokio::test]
async fn publish_then_get_returns_the_same_bytes_and_fence() {
    let _db = boot().await;
    let store = SqlRenderStore::new(1024 * 1024);
    let key = key("/tier-one");
    let bytes = Bytes::from_static(b"entry-bytes");

    assert_eq!(
        store
            .publish(&key, bytes.clone(), fence(1, 7), 1_000, 60_000)
            .await
            .expect("publish"),
        PublishOutcome::Published
    );

    let hit = store.get(&key).await.expect("get").expect("a hit");
    assert_eq!(hit.bytes, bytes);
    assert_eq!(hit.published_at_ms, 1_000);
    assert_eq!(hit.fence, fence(1, 7));
}

#[tokio::test]
async fn a_lower_token_is_fenced_and_leaves_the_stored_row() {
    let _db = boot().await;
    let store = SqlRenderStore::new(1024 * 1024);
    let key = key("/tier-one");

    store
        .publish(
            &key,
            Bytes::from_static(b"second"),
            fence(1, 2),
            2_000,
            60_000,
        )
        .await
        .expect("publish");
    assert_eq!(
        store
            .publish(
                &key,
                Bytes::from_static(b"first"),
                fence(1, 1),
                1_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Fenced
    );

    let hit = store.get(&key).await.expect("get").expect("a hit");
    assert_eq!(
        hit.bytes.as_ref(),
        b"second",
        "a fenced publication leaves the stored row untouched"
    );
    assert_eq!(hit.fence, fence(1, 2));
    assert_eq!(hit.published_at_ms, 2_000);
    assert_eq!(row_count().await, 1, "and stores no second row");
}

#[tokio::test]
async fn an_equal_fence_is_fenced_and_leaves_the_stored_row() {
    let _db = boot().await;
    let store = SqlRenderStore::new(1024 * 1024);
    let key = key("/tier-one");

    store
        .publish(
            &key,
            Bytes::from_static(b"held"),
            fence(3, 4),
            1_000,
            60_000,
        )
        .await
        .expect("publish");
    assert_eq!(
        store
            .publish(
                &key,
                Bytes::from_static(b"equal"),
                fence(3, 4),
                5_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Fenced,
        "supersedes is strict: an equal fence never replaces"
    );

    let hit = store.get(&key).await.expect("get").expect("a hit");
    assert_eq!(hit.bytes.as_ref(), b"held");
    assert_eq!(hit.published_at_ms, 1_000);
}

#[tokio::test]
async fn a_higher_epoch_replaces_the_stored_row() {
    let _db = boot().await;
    let store = SqlRenderStore::new(1024 * 1024);
    let key = key("/tier-one");

    store
        .publish(&key, Bytes::from_static(b"old"), fence(1, 9), 1_000, 60_000)
        .await
        .expect("publish");
    assert_eq!(
        store
            .publish(&key, Bytes::from_static(b"new"), fence(2, 1), 2_000, 60_000)
            .await
            .expect("publish"),
        PublishOutcome::Published,
        "a newer epoch supersedes a higher token at the older epoch"
    );

    let hit = store.get(&key).await.expect("get").expect("a hit");
    assert_eq!(hit.bytes.as_ref(), b"new");
    assert_eq!(hit.fence, fence(2, 1));
    assert_eq!(row_count().await, 1, "replacement, never a second row");
}

#[tokio::test]
async fn bytes_over_the_bound_are_rejected_and_store_no_row() {
    let _db = boot().await;
    let store = SqlRenderStore::new(8);
    let key = key("/tier-one");

    assert_eq!(
        store
            .publish(
                &key,
                Bytes::from_static(b"nine-byte"),
                fence(1, 1),
                1_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Rejected
    );
    assert_eq!(row_count().await, 0, "the bound is checked before any SQL");
    assert!(store.get(&key).await.expect("get").is_none());

    assert_eq!(
        store
            .publish(
                &key,
                Bytes::from_static(b"8-bytes!"),
                fence(1, 1),
                1_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Published,
        "the bound is inclusive at the exact edge"
    );
}

#[tokio::test]
async fn bytes_corrupted_in_the_row_decode_as_a_miss() {
    let _db = boot().await;
    let store = SqlRenderStore::new(1024 * 1024);
    let key = key("/tier-one");
    let entry = encoded_entry("/tier-one");

    store
        .publish(&key, entry.clone(), fence(1, 1), 1_000, 60_000)
        .await
        .expect("publish");
    let stored = store.get(&key).await.expect("get").expect("a hit");
    decode(&stored.bytes, &keys(), &EntryLimits::default())
        .expect("the entry decodes before it is tampered with");

    // Torn or foreign bytes are not the store's business: it hands back
    // what the row holds, and the codec's integrity check is what turns
    // them into a miss.
    let mut torn = entry.to_vec();
    let last = torn.len() - 1;
    torn[last] ^= 0xff;
    DB::affecting_statement(
        "UPDATE suprnova_render_entries SET bytes = ? WHERE render_key = ?",
        vec![torn.into(), key.to_base64url().into()],
    )
    .await
    .expect("corrupt the stored bytes directly");

    let tampered = store
        .get(&key)
        .await
        .expect("get")
        .expect("the row is still there");
    assert!(
        decode(&tampered.bytes, &keys(), &EntryLimits::default()).is_err(),
        "the codec's integrity check makes tampered bytes a miss"
    );
}

#[tokio::test]
async fn evict_removes_the_row() {
    let _db = boot().await;
    let store = SqlRenderStore::new(1024 * 1024);
    let key = key("/tier-one");

    store
        .publish(
            &key,
            Bytes::from_static(b"gone soon"),
            fence(1, 1),
            1_000,
            60_000,
        )
        .await
        .expect("publish");
    store.evict(&key).await.expect("evict");

    assert!(store.get(&key).await.expect("get").is_none());
    assert_eq!(row_count().await, 0);
    store
        .evict(&key)
        .await
        .expect("evicting an absent key is not an error");
}

#[tokio::test]
async fn inspect_counts_rows_and_bytes() {
    let _db = boot().await;
    let store = SqlRenderStore::new(1024 * 1024);

    let empty = store.inspect().await.expect("inspect an empty table");
    assert_eq!(empty.entries, 0);
    assert_eq!(empty.bytes, 0);

    store
        .publish(
            &key("/a"),
            Bytes::from_static(b"aaaa"),
            fence(1, 1),
            1_000,
            60_000,
        )
        .await
        .expect("publish");
    store
        .publish(
            &key("/b"),
            Bytes::from_static(b"bbbbbb"),
            fence(1, 1),
            1_000,
            60_000,
        )
        .await
        .expect("publish");

    let filled = store.inspect().await.expect("inspect");
    assert_eq!(filled.entries, 2);
    assert_eq!(filled.bytes, 10);
}

#[tokio::test]
async fn a_row_past_its_expiry_is_a_miss_by_store_time() {
    let _db = boot().await;
    let store = SqlRenderStore::new(1024 * 1024);
    let key = key("/tier-one");

    store
        .publish(
            &key,
            Bytes::from_static(b"retained"),
            fence(1, 1),
            1_000,
            60_000,
        )
        .await
        .expect("publish");
    assert!(
        store.get(&key).await.expect("get").is_some(),
        "inside its retention the row is a hit"
    );

    // The store's own clock, not this node's: the offset moves what the
    // database reports as now, which is the only clock the expiry
    // comparison consults.
    store.set_time_offset_for_test(120_000);
    assert!(
        store.get(&key).await.expect("get").is_none(),
        "past its retention the row is a miss"
    );
    assert_eq!(
        row_count().await,
        1,
        "get reports a miss; reclaiming the row is sweep's job"
    );
}

#[tokio::test]
async fn a_never_age_swept_retention_outlives_any_offset() {
    let _db = boot().await;
    let store = SqlRenderStore::new(1024 * 1024);
    let key = key("/tier-one");

    // `u64::MAX` is the store contract's "never age-swept" retention, and
    // it is what every caller with no policy in scope passes. Saturating it
    // into the expiry column rather than wrapping it is the whole point:
    // wrapped, the row would be expired the instant it was published.
    store
        .publish(
            &key,
            Bytes::from_static(b"forever"),
            fence(1, 1),
            1_000,
            u64::MAX,
        )
        .await
        .expect("publish");

    store.set_time_offset_for_test(315_360_000_000);
    assert!(
        store.get(&key).await.expect("get").is_some(),
        "ten years on, a never-age-swept row is still a hit"
    );
    assert_eq!(store.sweep(10).await.expect("sweep").removed, 0);
    assert_eq!(row_count().await, 1);
}

#[tokio::test]
async fn sweep_removes_expired_rows_in_bounded_batches() {
    let _db = boot().await;
    let store = SqlRenderStore::new(1024 * 1024);
    for pattern in ["/a", "/b", "/c"] {
        store
            .publish(
                &key(pattern),
                Bytes::from_static(b"retained"),
                fence(1, 1),
                1_000,
                60_000,
            )
            .await
            .expect("publish");
    }

    let nothing_due = store.sweep(10).await.expect("sweep");
    assert_eq!(nothing_due.removed, 0, "nothing is due inside retention");
    assert!(!nothing_due.more_remain);
    assert_eq!(row_count().await, 3);

    store.set_time_offset_for_test(120_000);
    let first = store.sweep(2).await.expect("sweep");
    assert_eq!(first.removed, 2, "at most `batch` rows per call");
    assert!(
        first.more_remain,
        "a backlog larger than the batch is reported as remaining work"
    );
    assert_eq!(row_count().await, 1);

    let second = store.sweep(2).await.expect("sweep");
    assert_eq!(second.removed, 1);
    assert!(!second.more_remain);
    assert_eq!(row_count().await, 0);
    assert_eq!(store.inspect().await.expect("inspect").entries, 0);
}

#[tokio::test]
async fn the_l1_provider_delegates_to_the_database_store() {
    let _db = boot().await;
    let provider = L1Provider::Database(SqlRenderStore::new(1024 * 1024));
    let key = key("/tier-one");

    assert_eq!(
        provider
            .publish(
                &key,
                Bytes::from_static(b"through the enum"),
                fence(1, 1),
                1_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Published
    );
    let hit = provider.get(&key).await.expect("get").expect("a hit");
    assert_eq!(hit.bytes.as_ref(), b"through the enum");
    assert_eq!(provider.inspect().await.expect("inspect").entries, 1);
    provider.evict(&key).await.expect("evict");
    assert!(provider.get(&key).await.expect("get").is_none());
}

#[tokio::test]
async fn the_tier_migration_probe_reports_what_the_database_carries() {
    {
        let _db = boot_without_the_tier_tables().await;
        assert!(
            !tier_migration_present().await.expect("probe"),
            "the original migration alone carries none of the tier tables"
        );
    }
    let _db = boot().await;
    assert!(
        tier_migration_present().await.expect("probe"),
        "the tier migration creates the table the probe looks for"
    );
}

// --- Live-DB tests (gated by #[ignore]) ---
//
// The tests above run against SQLite unconditionally; the two below run the
// same publication, fencing, and sweep contract against real Postgres and
// MySQL, where the upsert dialect, the placeholder syntax, the blob column
// type, and - the reason store time exists at all - the "milliseconds since
// the Unix epoch" expression all differ from SQLite's.
//
// Skipped by default. To run them, point the URL env var at a DISPOSABLE
// database and pass `--ignored`:
//
//   PG_TEST_URL=postgres://postgres:pw@127.0.0.1:55998/suprnova_test \
//     cargo test -p suprnova --test render_cache -- --ignored tiers::live_postgres
//
//   MYSQL_TEST_URL=mysql://root:pw@127.0.0.1:55997/suprnova_test \
//     cargo test -p suprnova --test render_cache -- --ignored tiers::live_mysql

/// Publishes, proves the fence in both directions, and reclaims by store
/// time - the whole Tier 1 L1 contract that a dialect can break.
async fn assert_publish_fencing_and_sweep() {
    let store = SqlRenderStore::new(1024 * 1024);
    let key = key("/tier-one");

    assert_eq!(
        store
            .publish(
                &key,
                Bytes::from_static(b"first"),
                fence(1, 1),
                1_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Published
    );
    assert_eq!(
        store
            .publish(
                &key,
                Bytes::from_static(b"lower"),
                fence(1, 1),
                2_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Fenced
    );
    let held = store.get(&key).await.expect("get").expect("a hit");
    assert_eq!(
        held.bytes.as_ref(),
        b"first",
        "a fenced publication leaves every column of the stored row alone"
    );
    assert_eq!(held.published_at_ms, 1_000);

    // A superseding fence must still land: this is the direction that
    // proves the dialect's guarded conflict branch (`WHERE` on Postgres,
    // `IF(...)` per column on MySQL) is satisfiable rather than merely
    // strict. The same guard is exercised with no read-compare in front of
    // it by `live_postgres_the_guarded_upsert_refuses_a_lower_fence_and_takes_a_higher_one`
    // and its MySQL twin, the unit tests inside `sql_store.rs`.
    assert_eq!(
        store
            .publish(
                &key,
                Bytes::from_static(b"higher"),
                fence(1, 2),
                3_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Published
    );

    let hit = store.get(&key).await.expect("get").expect("a hit");
    assert_eq!(hit.bytes.as_ref(), b"higher");
    assert_eq!(hit.fence, fence(1, 2));
    assert_eq!(hit.published_at_ms, 3_000);
    assert_eq!(store.inspect().await.expect("inspect").entries, 1);

    assert_eq!(store.sweep(10).await.expect("sweep").removed, 0);
    store.set_time_offset_for_test(120_000);
    assert!(store.get(&key).await.expect("get").is_none());
    let swept = store.sweep(10).await.expect("sweep");
    assert_eq!(swept.removed, 1);
    assert!(!swept.more_remain);
    assert_eq!(store.inspect().await.expect("inspect").entries, 0);
}

#[tokio::test]
#[ignore = "requires live Postgres; run with --ignored live_postgres"]
async fn live_postgres_publish_fencing_and_sweep() {
    let url = std::env::var("PG_TEST_URL")
        .expect("set PG_TEST_URL to a disposable Postgres - this test drops and recreates tables");
    let conn = try_connect_live(&url)
        .await
        .expect("Postgres test DB not reachable - check PG_TEST_URL");
    let _guard = reset_and_migrate(conn).await;

    assert!(tier_migration_present().await.expect("probe"));
    assert_publish_fencing_and_sweep().await;
}

#[tokio::test]
#[ignore = "requires live MySQL; run with --ignored live_mysql"]
async fn live_mysql_publish_fencing_and_sweep() {
    let url = std::env::var("MYSQL_TEST_URL")
        .expect("set MYSQL_TEST_URL to a disposable MySQL - this test drops and recreates tables");
    let conn = try_connect_live(&url)
        .await
        .expect("MySQL test DB not reachable - check MYSQL_TEST_URL");
    let _guard = reset_and_migrate(conn).await;

    assert!(tier_migration_present().await.expect("probe"));
    assert_publish_fencing_and_sweep().await;
}
