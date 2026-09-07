//! Tier 1: the database-backed L1 render store, rebuild lease store, Live
//! record store, and the tier migration.
//!
//! Every test here drives an adapter against a real database - SQLite by
//! default, Postgres and MySQL through the `#[ignore]`d live tests at the
//! bottom - never a mock. Two handles over one database stand in for two
//! nodes, which is the only way to prove cross-node behaviour when the
//! runtime that would use it is a process singleton.
//!
//! What is proven of `SqlRenderStore`: a publication round-trips its bytes
//! and its fence; `PublicationFence::supersedes` decides every replacement,
//! so a lower or equal fence is `Fenced` and leaves the stored row exactly
//! as it was; bytes over the bound are rejected before any statement runs;
//! a row whose bytes are tampered with in SQL is the codec's miss, not the
//! store's; expiry is measured by the database's own clock, so `get` misses
//! a row past its `expires_at_ms` and `sweep` removes at most `batch` of
//! them per call.
//!
//! What is proven of `SqlLeaseStore`: exactly one node leads a key, a lease
//! is taken over only once store time has passed its expiry, the former
//! leader mints nothing afterwards, tokens are monotonic per key across
//! tenures, and a release frees the lease without dropping the row that
//! carries the token counter.
//!
//! What is proven of `SqlInstanceRecordStore`: a record is created once,
//! compare-and-store replaces exactly the version it read, a record past
//! store time answers as one that was never written, elapsed records are
//! reclaimed in bounded batches, a record over the codec's bound is refused
//! before any statement, and - the coupling only a database can offer - a
//! claim made inside a host transaction that rolls back leaves no row. The
//! kernel over it answers the engine's own ledger conformance suite.

use std::sync::Arc;

use bytes::Bytes;
use suprnova::render_cache::L1Provider;
use suprnova::render_cache::ledger::tier_migration_present;
use suprnova::render_cache::providers::{SqlInstanceRecordStore, SqlLeaseStore, SqlRenderStore};
use suprnova::{DB, FrameworkError};
use suprnova_live::clock::Clock;
use suprnova_live::identity::UnixMillis;
use suprnova_live::ledger::{
    CasOutcome, DistributedInstanceLedger, InstanceRecordKey, InstanceRecordStore, LedgerError,
    LedgerErrorKind, MAX_RECORD_BYTES, PromotionRecordKey, StoredRecord,
};
use suprnova_live::render_cache::entry::{EntryLimits, decode};
use suprnova_live::render_cache::singleflight::{
    LocalCoordinatorLimits, RebuildAdmission, RebuildCoordinator,
};
use suprnova_live::render_cache::store::{PublishOutcome, RenderStore};
use suprnova_live::render_cache::{FencedLeaseCoordinator, LeaseAttempt, LeaseStore};
use suprnova_live_test_support::{ControlledClock, ledger_conformance};

use crate::render_cache_tiers_support;
use render_cache_tiers_support::{
    boot, boot_without_the_tier_tables, encoded_entry, fence, instance_key, key, keys,
    promotion_key, reset_and_migrate, store_deadline, store_now_ms, try_connect_live,
    wide_instance_key, wide_promotion_key,
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

// --- Tier 1: the fenced rebuild lease ---
//
// The rebuild coordinator is a process singleton, so cross-node leadership
// cannot be proven through it. It is proven here instead, at the store:
// two `SqlLeaseStore` handles over one database are two nodes.

/// Rows currently in the leases table. `release` never deletes one, so a
/// test that means "the lease is free" has to be able to say it without
/// meaning "the row is gone".
async fn lease_row_count() -> i64 {
    DB::scalar("SELECT COUNT(*) FROM suprnova_render_leases", vec![])
        .await
        .expect("count the leases table")
}

/// The lease id and store expiry of an attempt that was expected to win.
fn acquired(attempt: LeaseAttempt) -> (u64, u64) {
    match attempt {
        LeaseAttempt::Acquired {
            lease_id,
            expires_at_ms,
        } => (lease_id, expires_at_ms),
        LeaseAttempt::Held => panic!("the key was expected to be acquirable"),
    }
}

#[tokio::test]
async fn only_one_node_takes_a_brand_new_lease() {
    let _db = boot().await;
    let (first, second) = (SqlLeaseStore::new(), SqlLeaseStore::new());
    let key = key("/tier-one");

    let (lease_id, expires_at_ms) = acquired(
        first
            .try_acquire(&key, 1, 30_000)
            .await
            .expect("the first acquire"),
    );
    assert_eq!(lease_id, 1, "the row's own lease counter starts at one");
    assert!(
        expires_at_ms > store_now_ms().await,
        "the expiry is the database's clock plus the lifetime, not this node's"
    );

    assert_eq!(
        second
            .try_acquire(&key, 1, 30_000)
            .await
            .expect("the second acquire"),
        LeaseAttempt::Held,
        "an unexpired lease belongs to whoever took it"
    );
    assert_eq!(lease_row_count().await, 1, "and never a second row");
}

#[tokio::test]
async fn an_expired_lease_is_taken_over_and_the_former_leader_is_fenced() {
    let _db = boot().await;
    let (first, second) = (SqlLeaseStore::new(), SqlLeaseStore::new());
    let key = key("/tier-one");

    let (held, _) = acquired(first.try_acquire(&key, 1, 1_000).await.expect("acquire"));
    assert_eq!(first.mint_token(&key, held).await.expect("mint"), Some(1));

    // Store time, not this node's. Both handles read the same database
    // clock, so moving both offsets is what a real pair of nodes reaches by
    // the lease simply running out.
    first.set_time_offset_for_test(2_000);
    second.set_time_offset_for_test(2_000);

    assert_eq!(
        first.mint_token(&key, held).await.expect("mint"),
        None,
        "expiry is decided by store time"
    );
    let (taken, _) = acquired(second.try_acquire(&key, 1, 1_000).await.expect("takeover"));
    assert_eq!(
        taken,
        held + 1,
        "a takeover advances the row's own lease counter"
    );
    assert_eq!(
        second.mint_token(&key, taken).await.expect("mint"),
        Some(2),
        "a takeover continues the key's tokens rather than restarting them"
    );
    assert_eq!(
        first.mint_token(&key, held).await.expect("mint"),
        None,
        "the former leader is fenced out for good"
    );
}

#[tokio::test]
async fn minting_is_monotonic_within_one_tenure() {
    let _db = boot().await;
    let store = SqlLeaseStore::new();
    let leased = key("/tier-one");
    let never_leased = key("/never-leased");

    let (lease_id, _) = acquired(
        store
            .try_acquire(&leased, 1, 30_000)
            .await
            .expect("acquire"),
    );
    for expected in 1..=3 {
        assert_eq!(
            store.mint_token(&leased, lease_id).await.expect("mint"),
            Some(expected)
        );
    }
    assert_eq!(
        store.mint_token(&leased, lease_id + 9).await.expect("mint"),
        None,
        "a lease id that never held this key mints nothing"
    );
    assert_eq!(
        store.mint_token(&never_leased, 1).await.expect("mint"),
        None,
        "and neither does a key that was never leased"
    );
}

#[tokio::test]
async fn release_frees_the_key_and_keeps_the_row_so_tokens_never_restart() {
    let _db = boot().await;
    let (first, second) = (SqlLeaseStore::new(), SqlLeaseStore::new());
    let key = key("/tier-one");

    let (held, _) = acquired(first.try_acquire(&key, 1, 30_000).await.expect("acquire"));
    assert_eq!(first.mint_token(&key, held).await.expect("mint"), Some(1));
    first.release(&key, held).await.expect("release");
    assert_eq!(
        lease_row_count().await,
        1,
        "release frees the lease; the row carries the token counter and stays"
    );
    assert_eq!(
        first.mint_token(&key, held).await.expect("mint"),
        None,
        "a released lease mints nothing"
    );

    let (taken, _) = acquired(second.try_acquire(&key, 1, 30_000).await.expect("acquire"));
    assert_eq!(taken, held + 1);
    assert_eq!(
        second.mint_token(&key, taken).await.expect("mint"),
        Some(2),
        "the token counter outlives the lease that minted from it"
    );
}

#[tokio::test]
async fn releasing_with_a_lease_id_that_does_not_hold_the_key_changes_nothing() {
    let _db = boot().await;
    let (first, second) = (SqlLeaseStore::new(), SqlLeaseStore::new());
    let leased = key("/tier-one");
    let never_leased = key("/never-leased");

    let (held, _) = acquired(
        first
            .try_acquire(&leased, 1, 30_000)
            .await
            .expect("acquire"),
    );
    first
        .release(&leased, held + 7)
        .await
        .expect("releasing someone else's lease is not an error");
    assert_eq!(
        second
            .try_acquire(&leased, 1, 30_000)
            .await
            .expect("acquire"),
        LeaseAttempt::Held,
        "a stale release frees nothing"
    );
    assert_eq!(
        first.mint_token(&leased, held).await.expect("mint"),
        Some(1)
    );

    second
        .release(&never_leased, 1)
        .await
        .expect("releasing a key that was never leased is not an error");
    assert_eq!(lease_row_count().await, 1);
}

#[tokio::test]
async fn two_coordinators_over_one_database_lead_once_and_bypass_once() {
    let _db = boot().await;
    let limits = LocalCoordinatorLimits {
        lease_ms: 30_000,
        max_waiters: 4,
    };
    let leader = FencedLeaseCoordinator::new(Arc::new(SqlLeaseStore::new()), limits);
    let peer = FencedLeaseCoordinator::new(Arc::new(SqlLeaseStore::new()), limits);
    let key = key("/tier-one");

    let RebuildAdmission::Lead(lease) = leader.admit(&key, 1, 0).await.expect("admit") else {
        panic!("the first coordinator must lead");
    };
    assert!(
        matches!(
            peer.admit(&key, 1, 0).await.expect("admit"),
            RebuildAdmission::Bypass
        ),
        "the node that does not hold the store's lease renders without publishing"
    );

    let fence = leader
        .publish_token(&lease, 0)
        .await
        .expect("publish token");
    assert_eq!(fence.epoch, 1);
    assert_eq!(fence.token, 1);
    leader.release(*lease).await.expect("release");

    let RebuildAdmission::Lead(next) = peer.admit(&key, 1, 0).await.expect("admit") else {
        panic!("a released lease is available to the other node");
    };
    let second = peer.publish_token(&next, 0).await.expect("publish token");
    assert_eq!(
        second.token, 2,
        "tokens are monotonic per key across tenures and across nodes"
    );
    peer.release(*next).await.expect("release");
}

// --- Tier 1: the Live instance and promotion record store ---

/// Rows currently in the instances table, counted in SQL rather than
/// through the store, so a test can tell "the store reports absence" from
/// "the row is gone".
async fn instance_row_count() -> i64 {
    DB::scalar("SELECT COUNT(*) FROM suprnova_live_instances", vec![])
        .await
        .expect("count the instances table")
}

#[tokio::test]
async fn a_record_is_created_once_and_a_second_node_finds_the_key_held() {
    let _db = boot().await;
    let (first, second) = (SqlInstanceRecordStore::new(), SqlInstanceRecordStore::new());
    let key = instance_key(0x21);
    let expires_at = store_deadline(60_000).await;

    assert!(
        first
            .insert_if_absent(&key, b"first", expires_at)
            .await
            .expect("insert"),
        "the first writer creates the record"
    );
    assert!(
        !second
            .insert_if_absent(&key, b"second", expires_at)
            .await
            .expect("insert"),
        "an unexpired record holds its key against every other node"
    );

    let stored = second
        .load(&key)
        .await
        .expect("load")
        .expect("the record is there");
    assert_eq!(
        stored.bytes,
        b"first".to_vec(),
        "and its bytes are the first writer's"
    );
    assert_eq!(stored.version, 1);
    assert_eq!(stored.expires_at, expires_at);
    assert_eq!(instance_row_count().await, 1);
}

#[tokio::test]
async fn compare_and_store_advances_the_version_and_refuses_a_stale_read() {
    let _db = boot().await;
    let (first, second) = (SqlInstanceRecordStore::new(), SqlInstanceRecordStore::new());
    let key = instance_key(0x22);
    let expires_at = store_deadline(60_000).await;
    first
        .insert_if_absent(&key, b"one", expires_at)
        .await
        .expect("insert");

    assert_eq!(
        second
            .compare_and_store(&key, 1, b"two", expires_at)
            .await
            .expect("compare and store"),
        CasOutcome::Stored { version: 2 }
    );
    assert_eq!(
        first
            .compare_and_store(&key, 1, b"three", expires_at)
            .await
            .expect("compare and store"),
        CasOutcome::Conflict,
        "a write derived from a stale read replaces nothing"
    );

    let stored = first.load(&key).await.expect("load").expect("a record");
    assert_eq!(stored.bytes, b"two".to_vec());
    assert_eq!(stored.version, 2);

    assert_eq!(
        first
            .compare_and_store(&instance_key(0x99), 1, b"nothing", expires_at)
            .await
            .expect("compare and store"),
        CasOutcome::Missing,
        "there is nothing to replace at a key no record holds"
    );
    assert_eq!(instance_row_count().await, 1);
}

#[tokio::test]
async fn a_record_past_store_expiry_is_absent_and_frees_its_key() {
    let _db = boot().await;
    let store = SqlInstanceRecordStore::new();
    let key = instance_key(0x23);
    store
        .insert_if_absent(&key, b"short lived", store_deadline(1_000).await)
        .await
        .expect("insert");
    assert!(store.load(&key).await.expect("load").is_some());

    store.set_time_offset_for_test(2_000);

    assert!(
        store.load(&key).await.expect("load").is_none(),
        "a record past store time answers exactly as one that was never written"
    );
    assert_eq!(
        store
            .compare_and_store(&key, 1, b"resurrected", store_deadline(120_000).await)
            .await
            .expect("compare and store"),
        CasOutcome::Missing,
        "and an elapsed record is never replaced back into life"
    );
    assert_eq!(store.count_instances().await.expect("count"), 0);

    let renewed = store_deadline(120_000).await;
    assert!(
        store
            .insert_if_absent(&key, b"second life", renewed)
            .await
            .expect("insert"),
        "an elapsed record is not a holder"
    );
    let stored = store.load(&key).await.expect("load").expect("a record");
    assert_eq!(stored.bytes, b"second life".to_vec());
    assert_eq!(
        stored.version, 1,
        "versions belong to a record, not to a key"
    );
    assert_eq!(instance_row_count().await, 1);
}

#[tokio::test]
async fn counting_instances_excludes_records_past_store_time() {
    let _db = boot().await;
    let store = SqlInstanceRecordStore::new();
    let long = store_deadline(120_000).await;
    let short = store_deadline(1_000).await;
    store
        .insert_if_absent(&instance_key(0x30), b"a", long)
        .await
        .expect("insert");
    store
        .insert_if_absent(&instance_key(0x31), b"b", long)
        .await
        .expect("insert");
    store
        .insert_if_absent(&instance_key(0x32), b"c", short)
        .await
        .expect("insert");
    assert_eq!(store.count_instances().await.expect("count"), 3);

    store.set_time_offset_for_test(2_000);
    assert_eq!(
        store.count_instances().await.expect("count"),
        2,
        "capacity is measured over records that are still there"
    );
}

#[tokio::test]
async fn removing_a_record_leaves_no_row_and_is_not_an_error_when_absent() {
    let _db = boot().await;
    let store = SqlInstanceRecordStore::new();
    let key = instance_key(0x33);
    store
        .insert_if_absent(&key, b"gone soon", store_deadline(60_000).await)
        .await
        .expect("insert");

    store.remove(&key).await.expect("remove");
    assert!(store.load(&key).await.expect("load").is_none());
    assert_eq!(instance_row_count().await, 0);
    store
        .remove(&key)
        .await
        .expect("removing a record that is not there is not a failure");
}

#[tokio::test]
async fn a_claim_inside_a_rolled_back_transaction_leaves_no_row() {
    let _db = boot().await;
    let store = Arc::new(SqlInstanceRecordStore::new());
    let expires_at = store_deadline(60_000).await;

    let result: Result<(), FrameworkError> = DB::transaction({
        let store = Arc::clone(&store);
        move |_tx| {
            let store = Arc::clone(&store);
            Box::pin(async move {
                assert!(
                    store
                        .insert_if_absent(&instance_key(0x34), b"uncommitted", expires_at)
                        .await
                        .expect("insert"),
                    "the record is created inside the host's transaction"
                );
                Err(FrameworkError::bad_request("simulated host failure"))
            })
        }
    })
    .await;

    assert!(result.is_err());
    assert_eq!(
        instance_row_count().await,
        0,
        "the record store joins the ambient transaction, so a rollback takes the claim with it"
    );
    assert!(
        store
            .load(&instance_key(0x34))
            .await
            .expect("load")
            .is_none()
    );
}

#[tokio::test]
async fn promotion_reservations_are_created_once_and_elapse_by_store_time() {
    let _db = boot().await;
    let (first, second) = (SqlInstanceRecordStore::new(), SqlInstanceRecordStore::new());
    let key = promotion_key(0x40);

    assert!(
        first
            .insert_promotion_if_absent(&key, b"reserved", store_deadline(1_000).await)
            .await
            .expect("reserve")
    );
    assert!(
        !second
            .insert_promotion_if_absent(&key, b"raced", store_deadline(1_000).await)
            .await
            .expect("reserve"),
        "a retry identity belongs to the node that reserved it first"
    );
    let stored = second
        .load_promotion(&key)
        .await
        .expect("load")
        .expect("the reservation is there");
    assert_eq!(stored.bytes, b"reserved".to_vec());

    first.set_time_offset_for_test(2_000);
    assert!(
        first.load_promotion(&key).await.expect("load").is_none(),
        "an elapsed reservation frees its retry identity"
    );
    assert!(
        first
            .insert_promotion_if_absent(&key, b"renewed", store_deadline(120_000).await)
            .await
            .expect("reserve")
    );
}

#[tokio::test]
async fn expired_records_are_reclaimed_in_bounded_batches() {
    let _db = boot().await;
    let store = SqlInstanceRecordStore::new();
    let short = store_deadline(1_000).await;
    for tag in 1..=70_u8 {
        store
            .insert_if_absent(&instance_key(tag), b"short lived", short)
            .await
            .expect("insert");
    }
    assert_eq!(instance_row_count().await, 70);

    store.set_time_offset_for_test(2_000);
    assert!(
        store
            .insert_if_absent(&instance_key(0xf0), b"fresh", store_deadline(120_000).await)
            .await
            .expect("insert")
    );
    assert_eq!(
        instance_row_count().await,
        7,
        "one operation reclaims at most 64 elapsed records; the backlog drains over the next"
    );
    assert_eq!(store.count_instances().await.expect("count"), 1);
}

/// A 32-byte identity is the widest the engine accepts, and its hex is
/// exactly twice as long as a 16-byte one's - and, by construction, the
/// narrow fixture's whole hex is the wide one's first half. So a column
/// sized for the narrow end truncates the wide identity onto the narrow
/// one's row: PostgreSQL refuses the insert outright, and a non-strict MySQL
/// silently collides two distinct identities. Proving the two stay their own
/// records is what pins the column width, and a retry identity reaches the
/// wide end straight off the wire, because it is built from the browser's
/// proposed nonce.
async fn assert_full_width_identities_are_their_own_rows() {
    let store = SqlInstanceRecordStore::new();
    let expires_at = store_deadline(60_000).await;

    assert!(
        store
            .insert_if_absent(&instance_key(0x50), b"narrow", expires_at)
            .await
            .expect("insert")
    );
    assert!(
        store
            .insert_if_absent(&wide_instance_key(0x50), b"wide", expires_at)
            .await
            .expect("insert"),
        "a 32-byte instance identity is its own record, not the 16-byte one widened"
    );
    assert_eq!(
        store
            .load(&instance_key(0x50))
            .await
            .expect("load")
            .expect("the narrow record")
            .bytes,
        b"narrow".to_vec()
    );
    assert_eq!(
        store
            .load(&wide_instance_key(0x50))
            .await
            .expect("load")
            .expect("the wide record")
            .bytes,
        b"wide".to_vec(),
        "and a truncating column would have handed back the other one"
    );

    assert!(
        store
            .insert_promotion_if_absent(&promotion_key(0x60), b"narrow", expires_at)
            .await
            .expect("reserve")
    );
    assert!(
        store
            .insert_promotion_if_absent(&wide_promotion_key(0x60), b"wide", expires_at)
            .await
            .expect("reserve"),
        "a 32-byte retry identity - the width a browser nonce actually reaches - is its own reservation"
    );
    assert_eq!(
        store
            .load_promotion(&promotion_key(0x60))
            .await
            .expect("load")
            .expect("the narrow reservation")
            .bytes,
        b"narrow".to_vec()
    );
    assert_eq!(
        store
            .load_promotion(&wide_promotion_key(0x60))
            .await
            .expect("load")
            .expect("the wide reservation")
            .bytes,
        b"wide".to_vec()
    );
}

#[tokio::test]
async fn full_width_identities_are_their_own_rows() {
    let _db = boot().await;
    assert_full_width_identities_are_their_own_rows().await;
}

#[tokio::test]
async fn a_record_over_the_bound_is_refused_before_any_statement() {
    let _db = boot().await;
    let store = SqlInstanceRecordStore::new();
    let key = instance_key(0x35);
    let expires_at = store_deadline(60_000).await;
    let oversized = vec![0_u8; MAX_RECORD_BYTES + 1];

    let refused = store
        .insert_if_absent(&key, &oversized, expires_at)
        .await
        .expect_err("a record over the bound is refused");
    assert_eq!(refused.kind(), LedgerErrorKind::CapacityExceeded);
    assert_eq!(
        instance_row_count().await,
        0,
        "the bound is checked before any SQL"
    );

    store
        .insert_if_absent(&key, b"small", expires_at)
        .await
        .expect("insert");
    let refused = store
        .compare_and_store(&key, 1, &oversized, expires_at)
        .await
        .expect_err("a replacement over the bound is refused");
    assert_eq!(refused.kind(), LedgerErrorKind::CapacityExceeded);
    assert_eq!(
        store
            .load(&key)
            .await
            .expect("load")
            .expect("a record")
            .version,
        1,
        "and leaves the stored record exactly as it was"
    );

    let refused = store
        .insert_promotion_if_absent(&promotion_key(0x41), &oversized, expires_at)
        .await
        .expect_err("a reservation over the bound is refused");
    assert_eq!(refused.kind(), LedgerErrorKind::CapacityExceeded);
}

// --- Tier 1: the instance ledger kernel over the database record store ---

/// Test glue that keeps the SQL store's clock in step with the node clock
/// the conformance suite advances.
///
/// The suite moves one [`ControlledClock`], and a provider that shares that
/// clock with its store - which is what the memory reference does - sees one
/// timeline. A SQL store's clock is the database's, which no test may move,
/// so this wrapper mirrors every advance of the node clock onto the store's
/// own test offset before each operation. What it cannot mirror is the real
/// milliseconds that pass while the suite runs, so store time is always the
/// node's plus that drift; every deadline the suite depends on is either
/// sixty seconds away or already elapsed, so drift decides nothing.
struct MirroredClockStore {
    inner: SqlInstanceRecordStore,
    clock: Arc<ControlledClock>,
    base_ms: u64,
}

impl MirroredClockStore {
    fn new(clock: Arc<ControlledClock>, base_ms: u64) -> Self {
        Self {
            inner: SqlInstanceRecordStore::new(),
            clock,
            base_ms,
        }
    }

    fn mirror(&self) {
        let node_ms = self
            .clock
            .now()
            .expect("the conformance clock is readable")
            .get();
        self.inner
            .set_time_offset_for_test(node_ms.saturating_sub(self.base_ms));
    }
}

#[async_trait::async_trait]
impl InstanceRecordStore for MirroredClockStore {
    async fn load(&self, key: &InstanceRecordKey) -> Result<Option<StoredRecord>, LedgerError> {
        self.mirror();
        self.inner.load(key).await
    }

    async fn insert_if_absent(
        &self,
        key: &InstanceRecordKey,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<bool, LedgerError> {
        self.mirror();
        self.inner.insert_if_absent(key, bytes, expires_at).await
    }

    async fn compare_and_store(
        &self,
        key: &InstanceRecordKey,
        expected_version: u64,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<CasOutcome, LedgerError> {
        self.mirror();
        self.inner
            .compare_and_store(key, expected_version, bytes, expires_at)
            .await
    }

    async fn remove(&self, key: &InstanceRecordKey) -> Result<(), LedgerError> {
        self.mirror();
        self.inner.remove(key).await
    }

    async fn load_promotion(
        &self,
        key: &PromotionRecordKey,
    ) -> Result<Option<StoredRecord>, LedgerError> {
        self.mirror();
        self.inner.load_promotion(key).await
    }

    async fn insert_promotion_if_absent(
        &self,
        key: &PromotionRecordKey,
        bytes: &[u8],
        expires_at: UnixMillis,
    ) -> Result<bool, LedgerError> {
        self.mirror();
        self.inner
            .insert_promotion_if_absent(key, bytes, expires_at)
            .await
    }

    async fn count_instances(&self) -> Result<usize, LedgerError> {
        self.mirror();
        self.inner.count_instances().await
    }
}

/// One ledger handle over the database record store, on a node clock the
/// conformance suite owns.
fn conformance_ledger(
    clock: &Arc<ControlledClock>,
    base_ms: u64,
) -> Arc<dyn suprnova_live::ledger::LiveInstanceLedger> {
    Arc::new(DistributedInstanceLedger::new(
        Arc::new(MirroredClockStore::new(Arc::clone(clock), base_ms)),
        Arc::clone(clock) as Arc<dyn Clock>,
        ledger_conformance::conformance_limits(),
    ))
}

#[tokio::test]
async fn the_ledger_kernel_over_the_database_store_answers_the_conformance_suite() {
    let _db = boot().await;
    let base_ms = store_now_ms().await;
    let clock = Arc::new(ControlledClock::new(UnixMillis::new(base_ms)));

    ledger_conformance::run_all(conformance_ledger(&clock, base_ms), Arc::clone(&clock))
        .await
        .expect("the database record store answers the ledger conformance suite");
}

#[tokio::test]
async fn two_ledger_handles_over_one_database_answer_the_two_node_suite() {
    let _db = boot().await;
    let base_ms = store_now_ms().await;
    let clock = Arc::new(ControlledClock::new(UnixMillis::new(base_ms)));

    ledger_conformance::run_two_node(
        conformance_ledger(&clock, base_ms),
        conformance_ledger(&clock, base_ms),
        Arc::clone(&clock),
    )
    .await
    .expect("two database-backed ledgers answer the two-node conformance suite");
}

// --- Live-DB tests (gated by #[ignore]) ---
//
// The tests above run against SQLite unconditionally; the ones below run
// the same publication, fencing, sweep, lease takeover, and
// compare-and-store contracts against real Postgres and MySQL, where the
// upsert dialect, the placeholder syntax, the row-locking read, the blob
// column type, and - the reason store time exists at all - the
// "milliseconds since the Unix epoch" expression all differ from SQLite's.
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
                fence(1, 2),
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
                fence(1, 3),
                3_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Published
    );

    let hit = store.get(&key).await.expect("get").expect("a hit");
    assert_eq!(hit.bytes.as_ref(), b"higher");
    assert_eq!(hit.fence, fence(1, 3));
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

/// Takes a lease over, mints against it, and refuses the former leader -
/// the whole cross-node lease contract a dialect can break, because the
/// takeover's guard, its row-locking read, and its store clock are all
/// per-dialect statements.
async fn assert_lease_takeover_and_fencing() {
    let (first, second) = (SqlLeaseStore::new(), SqlLeaseStore::new());
    let key = key("/tier-one");

    let (held, _) = acquired(first.try_acquire(&key, 1, 1_000).await.expect("acquire"));
    assert_eq!(first.mint_token(&key, held).await.expect("mint"), Some(1));
    assert_eq!(
        second.try_acquire(&key, 1, 1_000).await.expect("acquire"),
        LeaseAttempt::Held
    );

    first.set_time_offset_for_test(2_000);
    second.set_time_offset_for_test(2_000);

    let (taken, _) = acquired(second.try_acquire(&key, 1, 1_000).await.expect("takeover"));
    assert_eq!(taken, held + 1);
    assert_eq!(second.mint_token(&key, taken).await.expect("mint"), Some(2));
    assert_eq!(first.mint_token(&key, held).await.expect("mint"), None);

    second.release(&key, taken).await.expect("release");
    assert_eq!(
        lease_row_count().await,
        1,
        "release frees the lease and keeps the token counter"
    );
}

/// Creates a record, replaces it at the version it was read at, and refuses
/// a write derived from a stale read - the compare-and-store contract, whose
/// guarded update and confirming re-read are per-dialect statements too.
async fn assert_record_creation_and_cas_conflict() {
    let (first, second) = (SqlInstanceRecordStore::new(), SqlInstanceRecordStore::new());
    let key = instance_key(0x21);
    let expires_at = store_deadline(60_000).await;

    assert!(
        first
            .insert_if_absent(&key, b"first", expires_at)
            .await
            .expect("insert")
    );
    assert!(
        !second
            .insert_if_absent(&key, b"second", expires_at)
            .await
            .expect("insert")
    );
    assert_eq!(
        second
            .compare_and_store(&key, 1, b"two", expires_at)
            .await
            .expect("compare and store"),
        CasOutcome::Stored { version: 2 }
    );
    assert_eq!(
        first
            .compare_and_store(&key, 1, b"three", expires_at)
            .await
            .expect("compare and store"),
        CasOutcome::Conflict
    );
    let stored = first.load(&key).await.expect("load").expect("a record");
    assert_eq!(stored.bytes, b"two".to_vec());
    assert_eq!(stored.version, 2);
    assert_eq!(first.count_instances().await.expect("count"), 1);
}

#[tokio::test]
#[ignore = "requires live Postgres; run with --ignored live_postgres"]
async fn live_postgres_lease_takeover_and_fencing() {
    let url = std::env::var("PG_TEST_URL")
        .expect("set PG_TEST_URL to a disposable Postgres - this test drops and recreates tables");
    let conn = try_connect_live(&url)
        .await
        .expect("Postgres test DB not reachable - check PG_TEST_URL");
    let _guard = reset_and_migrate(conn).await;

    assert_lease_takeover_and_fencing().await;
}

#[tokio::test]
#[ignore = "requires live MySQL; run with --ignored live_mysql"]
async fn live_mysql_lease_takeover_and_fencing() {
    let url = std::env::var("MYSQL_TEST_URL")
        .expect("set MYSQL_TEST_URL to a disposable MySQL - this test drops and recreates tables");
    let conn = try_connect_live(&url)
        .await
        .expect("MySQL test DB not reachable - check MYSQL_TEST_URL");
    let _guard = reset_and_migrate(conn).await;

    assert_lease_takeover_and_fencing().await;
}

#[tokio::test]
#[ignore = "requires live Postgres; run with --ignored live_postgres"]
async fn live_postgres_record_creation_and_cas_conflict() {
    let url = std::env::var("PG_TEST_URL")
        .expect("set PG_TEST_URL to a disposable Postgres - this test drops and recreates tables");
    let conn = try_connect_live(&url)
        .await
        .expect("Postgres test DB not reachable - check PG_TEST_URL");
    let _guard = reset_and_migrate(conn).await;

    assert_record_creation_and_cas_conflict().await;
    assert_full_width_identities_are_their_own_rows().await;
}

#[tokio::test]
#[ignore = "requires live MySQL; run with --ignored live_mysql"]
async fn live_mysql_record_creation_and_cas_conflict() {
    let url = std::env::var("MYSQL_TEST_URL")
        .expect("set MYSQL_TEST_URL to a disposable MySQL - this test drops and recreates tables");
    let conn = try_connect_live(&url)
        .await
        .expect("MySQL test DB not reachable - check MYSQL_TEST_URL");
    let _guard = reset_and_migrate(conn).await;

    assert_record_creation_and_cas_conflict().await;
    assert_full_width_identities_are_their_own_rows().await;
}
