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
//!
//! The Tier 2 Redis adapters answer the same contracts further down, through
//! `#[ignore]`d `live_redis_*` tests against a disposable instance, plus two
//! tests that need no Redis at all: an unreachable endpoint is a provider
//! failure whose message carries no URL, and an unusable URL is refused
//! without being echoed.
//!
//! The last section closes the gaps an adapter test cannot reach: that an
//! ordinary request through the real middleware reaches these providers and
//! comes back from them (a publication becomes a shared row or key, an
//! emptied L0 is refilled from it without the handler running, a stitched
//! document's Composite entry round-trips, and the operator's own sweep
//! dispatches to the configured tier), that a Live action on a distributed
//! ledger driver advances a revision a second handle reads, and that two
//! coordinators over one backend publish exactly once - the leader's bytes
//! under the leader's fence, the elapsed leader refused with `LeaseFenced`,
//! and a restarted process finding the records the previous one left.

use std::sync::Arc;

use bytes::Bytes;
use hyper::Method;
use serde_json::Value;
use suprnova::StatusCode;
use suprnova::live::testing::prepare_live_router_for_test;
use suprnova::live::{LedgerDriver, verify_ledger_driver_for_test};
use suprnova::render_cache::RenderCache;
use suprnova::render_cache::ledger::tier_migration_present;
use suprnova::render_cache::providers::{
    RedisInstanceRecordStore, RedisLeaseStore, RedisProviderConfig, RedisRenderStore,
    SqlInstanceRecordStore, SqlLeaseStore, SqlRenderStore,
};
use suprnova::render_cache::{CoordinatorConfig, L1Config, L1Provider, sweep_l1};
use suprnova::{DB, FrameworkError};
use suprnova_live::clock::{Clock, SystemClock};
use suprnova_live::identity::{InstanceId, Revision, ScopeFingerprint, UnixMillis};
use suprnova_live::ledger::{
    CasOutcome, DistributedInstanceLedger, InstanceRecordKey, InstanceRecordStore, LedgerError,
    LedgerErrorKind, LedgerLimits, LiveInstanceLedger, MAX_RECORD_BYTES, PromotionRecordKey,
    StoredRecord,
};
use suprnova_live::render_cache::entry::{EntryKind, EntryLimits, decode};
use suprnova_live::render_cache::singleflight::{
    LocalCoordinatorLimits, RebuildAdmission, RebuildCoordinator,
};
use suprnova_live::render_cache::store::{PublishOutcome, RenderStore};
use suprnova_live::render_cache::{
    FencedLeaseCoordinator, LeaseAttempt, LeaseStore, RenderCacheErrorKind,
};
use suprnova_live_test_support::{ControlledClock, ledger_conformance};

use crate::live_dogfood_support;
use crate::render_cache_stitch_support;
use crate::render_cache_tiers_support;
use live_dogfood_support::{
    ActionRequest, DOCUMENT_PATH, PRIVATE_DOCUMENT_PATH, build_public_router,
    dispatch as dispatch_live, get as live_get, private_action_request, production_middleware,
    session_cookie,
};
use render_cache_stitch_support::{
    SEED_ONLY_PATH, STITCHED_PATH, boot_on_the_database_profile_for_test,
    boot_on_the_redis_profile_for_test, dispatch as dispatch_route, handler_renders, island_tag,
};
use render_cache_tiers_support::{
    boot, boot_redis, boot_without_the_tier_tables, clear_redis_prefix, encoded_entry, fence,
    install, instance_key, key, keys, promotion_key, redis_config_on_a_closed_port, redis_deadline,
    redis_keys, redis_now_ms, redis_url, reset_and_migrate, store_deadline, store_now_ms,
    tier_config, try_connect_live, wide_instance_key, wide_promotion_key,
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

/// A record store whose view of store time a test can move forward.
///
/// Both distributed adapters carry the same doc-hidden seam, and
/// [`MirroredClockStore`] is written against this rather than against either
/// of them so the conformance suite runs over the database store and the
/// Redis store through one piece of glue.
trait OffsetRecordStore: InstanceRecordStore {
    fn set_time_offset_for_test(&self, offset_ms: u64);
}

impl OffsetRecordStore for SqlInstanceRecordStore {
    fn set_time_offset_for_test(&self, offset_ms: u64) {
        // The adapter's own inherent method, not this trait method: an
        // inherent method wins over a trait method of the same name, so this
        // is a delegation rather than the infinite recursion it reads as.
        Self::set_time_offset_for_test(self, offset_ms);
    }
}

impl OffsetRecordStore for RedisInstanceRecordStore {
    fn set_time_offset_for_test(&self, offset_ms: u64) {
        // The adapter's own inherent method; see the note on the database
        // store's copy of this delegation.
        Self::set_time_offset_for_test(self, offset_ms);
    }
}

/// Test glue that keeps a distributed store's clock in step with the node
/// clock the conformance suite advances.
///
/// The suite moves one [`ControlledClock`], and a provider that shares that
/// clock with its store - which is what the memory reference does - sees one
/// timeline. A distributed store's clock is the backend's, which no test may
/// move, so this wrapper mirrors every advance of the node clock onto the
/// store's own test offset before each operation. What it cannot mirror is
/// the real milliseconds that pass while the suite runs, so store time is
/// always the node's plus that drift; every deadline the suite depends on is
/// either sixty seconds away or already elapsed, so drift decides nothing.
struct MirroredClockStore<S: OffsetRecordStore> {
    inner: S,
    clock: Arc<ControlledClock>,
    base_ms: u64,
}

impl<S: OffsetRecordStore> MirroredClockStore<S> {
    fn new(inner: S, clock: Arc<ControlledClock>, base_ms: u64) -> Self {
        Self {
            inner,
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
impl<S: OffsetRecordStore> InstanceRecordStore for MirroredClockStore<S> {
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
fn conformance_ledger<S: OffsetRecordStore + 'static>(
    store: S,
    clock: &Arc<ControlledClock>,
    base_ms: u64,
) -> Arc<dyn suprnova_live::ledger::LiveInstanceLedger> {
    Arc::new(DistributedInstanceLedger::new(
        Arc::new(MirroredClockStore::new(store, Arc::clone(clock), base_ms)),
        Arc::clone(clock) as Arc<dyn Clock>,
        ledger_conformance::conformance_limits(),
    ))
}

#[tokio::test]
async fn the_ledger_kernel_over_the_database_store_answers_the_conformance_suite() {
    let _db = boot().await;
    let base_ms = store_now_ms().await;
    let clock = Arc::new(ControlledClock::new(UnixMillis::new(base_ms)));

    ledger_conformance::run_all(
        conformance_ledger(SqlInstanceRecordStore::new(), &clock, base_ms),
        Arc::clone(&clock),
    )
    .await
    .expect("the database record store answers the ledger conformance suite");
}

#[tokio::test]
async fn two_ledger_handles_over_one_database_answer_the_two_node_suite() {
    let _db = boot().await;
    let base_ms = store_now_ms().await;
    let clock = Arc::new(ControlledClock::new(UnixMillis::new(base_ms)));

    ledger_conformance::run_two_node(
        conformance_ledger(SqlInstanceRecordStore::new(), &clock, base_ms),
        conformance_ledger(SqlInstanceRecordStore::new(), &clock, base_ms),
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

// --- Tier 2: the Redis L1 store, lease store, and instance record store ---
//
// Everything below drives a Redis adapter against a real Redis. The tests
// that need one are `#[ignore]`d and scope every key under a UUID-unique
// prefix, so a shared instance is safe and a run leaves nothing behind. To
// run them, point the URL env var at a DISPOSABLE Redis and pass `--ignored`:
//
//   REDIS_TEST_URL=redis://127.0.0.1:6379/ \
//     cargo test -p suprnova --test render_cache -- --ignored tiers::live_redis
//
// The two tests that prove how an unreachable Redis is reported need no
// Redis at all and run in the default suite.

/// The token counter behind one key, read straight from Redis so a test can
/// tell "the store answered `None`" from "the counter restarted".
async fn redis_token(
    conn: &mut redis::aio::ConnectionManager,
    prefix: &str,
    key: &suprnova_live::render_cache::key::RenderKey,
) -> Option<i64> {
    redis::cmd("GET")
        .arg(format!("{prefix}token:{}", key.to_base64url()))
        .query_async(conn)
        .await
        .expect("read the token counter")
}

/// Members currently in the instance expiry index, elapsed ones included.
///
/// `count_instances` deliberately excludes elapsed members, so proving that
/// reclamation is bounded needs the raw index rather than the port.
async fn redis_index_size(conn: &mut redis::aio::ConnectionManager, prefix: &str) -> i64 {
    redis::cmd("ZCARD")
        .arg(format!("{prefix}instances"))
        .query_async(conn)
        .await
        .expect("read the instance expiry index")
}

#[tokio::test]
async fn a_redis_store_on_a_closed_port_reports_a_provider_failure_without_the_url() {
    let config = redis_config_on_a_closed_port();
    let entries = RedisRenderStore::connect(&config, 1024 * 1024)
        .await
        .expect("a store is a handle, so building one needs no live Redis");
    let leases = RedisLeaseStore::connect(&config)
        .await
        .expect("a store is a handle, so building one needs no live Redis");
    let records = RedisInstanceRecordStore::connect(&config)
        .await
        .expect("a store is a handle, so building one needs no live Redis");
    let key = key("/tier-two");

    let failed = entries.get(&key).await.expect_err("nothing answers");
    assert_eq!(failed.kind(), RenderCacheErrorKind::ProviderUnavailable);
    let rendered = format!("{failed} {failed:?}");
    assert!(!rendered.contains("127.0.0.1"), "{rendered}");
    assert!(!rendered.contains("redis://"), "{rendered}");

    let failed = leases
        .try_acquire(&key, 1, 30_000)
        .await
        .expect_err("nothing answers");
    assert_eq!(failed.kind(), RenderCacheErrorKind::ProviderUnavailable);

    let failed = records
        .load(&instance_key(0x51))
        .await
        .expect_err("nothing answers");
    assert_eq!(failed.kind(), LedgerErrorKind::ProviderUnavailable);
    // The ledger's own failure is held to the render cache arm's standard:
    // neither the address nor the scheme reaches an operator's log.
    let rendered = format!("{failed} {failed:?}");
    assert!(!rendered.contains("127.0.0.1"), "{rendered}");
    assert!(!rendered.contains("redis://"), "{rendered}");

    // And nothing any of the three prints carries the endpoint either.
    let printed = format!("{entries:?} {leases:?} {records:?} {config:?}");
    assert!(!printed.contains("127.0.0.1"), "{printed}");
}

#[tokio::test]
async fn a_redis_provider_url_that_is_not_a_url_is_refused_without_being_echoed() {
    let config = RedisProviderConfig {
        url: "not-a-redis-url://user:hunter2@host".to_owned(),
        prefix: "suprnova_tiers_invalid:".to_owned(),
    };
    let refused = RedisRenderStore::connect(&config, 1024)
        .await
        .expect_err("an unusable URL is refused");
    let rendered = refused.to_string();
    assert!(!rendered.contains("hunter2"), "{rendered}");
    assert!(!rendered.contains("not-a-redis-url"), "{rendered}");
}

#[tokio::test]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_publish_fences_and_eviction_is_a_miss() {
    let (config, mut conn, _cleanup) = boot_redis().await;
    let store = RedisRenderStore::connect(&config, 1024 * 1024)
        .await
        .expect("connect the Redis L1 store");
    let key = key("/tier-two");

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
        PublishOutcome::Published,
        "an absent key is published under any fence"
    );
    for (label, bytes, sent) in [
        ("a lower token", &b"lower"[..], fence(1, 1)),
        ("an equal fence", &b"equal"[..], fence(1, 2)),
        ("a lower epoch", &b"older"[..], fence(0, 9)),
    ] {
        assert_eq!(
            store
                .publish(&key, Bytes::copy_from_slice(bytes), sent, 2_000, 60_000)
                .await
                .expect("publish"),
            PublishOutcome::Fenced,
            "{label} never replaces the stored publication"
        );
    }
    let held = store.get(&key).await.expect("get").expect("a hit");
    assert_eq!(
        held.bytes.as_ref(),
        b"first",
        "a fenced publication changes nothing"
    );
    assert_eq!(held.fence, fence(1, 2));
    assert_eq!(held.published_at_ms, 1_000);

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
    let inspection = store.inspect().await.expect("inspect");
    assert_eq!(inspection.entries, 1);
    assert_eq!(inspection.bytes, b"higher".len());

    let oversize = RedisRenderStore::connect(&config, 4)
        .await
        .expect("connect a tiny store");
    assert_eq!(
        oversize
            .publish(
                &key,
                Bytes::from_static(b"far too large"),
                fence(9, 9),
                4_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Rejected,
        "an entry over the bound is refused before any command"
    );
    assert_eq!(
        store.get(&key).await.expect("get").expect("a hit").fence,
        fence(1, 3),
        "and a rejected publication leaves the stored entry alone"
    );

    // Retention is Redis's own: a finite one becomes the hash's lifetime,
    // and the contract's "never age-swept" becomes no lifetime at all.
    let entry_hash = format!("{}entry:{}", config.prefix, key.to_base64url());
    let lifetime: i64 = redis::cmd("PTTL")
        .arg(&entry_hash)
        .query_async(&mut conn)
        .await
        .expect("read the entry lifetime");
    assert!(lifetime > 0 && lifetime <= 60_000, "{lifetime}");
    store
        .publish(
            &key,
            Bytes::from_static(b"kept"),
            fence(1, 4),
            4_000,
            u64::MAX,
        )
        .await
        .expect("publish");
    let lifetime: i64 = redis::cmd("PTTL")
        .arg(&entry_hash)
        .query_async(&mut conn)
        .await
        .expect("read the entry lifetime");
    assert_eq!(lifetime, -1, "a never-age-swept entry carries no lifetime");

    // A hash missing a field this build writes with the others is a torn
    // write or another writer's key, and there is nothing to serve.
    let _: i64 = redis::cmd("HDEL")
        .arg(&entry_hash)
        .arg("digest")
        .query_async(&mut conn)
        .await
        .expect("remove the digest field");
    assert!(store.get(&key).await.expect("get").is_none());

    // The key evicted out from under the store - by an operator, by Redis
    // itself, or by a restart - is a miss and nothing worse.
    let _: i64 = redis::cmd("DEL")
        .arg(&entry_hash)
        .query_async(&mut conn)
        .await
        .expect("delete the entry hash");
    assert!(store.get(&key).await.expect("get").is_none());

    store
        .publish(
            &key,
            Bytes::from_static(b"again"),
            fence(2, 1),
            5_000,
            60_000,
        )
        .await
        .expect("publish");
    store.evict(&key).await.expect("evict");
    assert!(store.get(&key).await.expect("get").is_none());
    assert_eq!(store.inspect().await.expect("inspect").entries, 0);
}

#[tokio::test]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_bytes_tampered_with_in_the_hash_decode_as_a_miss() {
    let (config, mut conn, _cleanup) = boot_redis().await;
    let store = RedisRenderStore::connect(&config, 1024 * 1024)
        .await
        .expect("connect the Redis L1 store");
    let key = key("/tier-two");

    store
        .publish(&key, encoded_entry("/tier-two"), fence(1, 1), 1_000, 60_000)
        .await
        .expect("publish");
    let sound = store.get(&key).await.expect("get").expect("a hit");
    decode(&sound.bytes, &keys(), &EntryLimits::default()).expect("the untampered entry decodes");

    let mut torn = sound.bytes.to_vec();
    let last = torn.len() - 1;
    torn[last] ^= 0x01;
    let _: i64 = redis::cmd("HSET")
        .arg(format!("{}entry:{}", config.prefix, key.to_base64url()))
        .arg("bytes")
        .arg(torn)
        .query_async(&mut conn)
        .await
        .expect("tamper with the stored bytes");

    let served = store
        .get(&key)
        .await
        .expect("get")
        .expect("the row is there");
    decode(&served.bytes, &keys(), &EntryLimits::default())
        .expect_err("a tampered entry is the codec's miss, never the store's hit");
}

#[tokio::test]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_inspection_stops_at_the_scan_cap() {
    let (config, mut conn, _cleanup) = boot_redis().await;
    let store = RedisRenderStore::connect(&config, 1024 * 1024)
        .await
        .expect("connect the Redis L1 store");

    let seeded = 11_000;
    let script = redis::Script::new(
        r"
        local prefix = ARGV[1]
        local count = tonumber(ARGV[2])
        for index = 1, count do
            redis.call('HSET', prefix .. 'entry:seed' .. index,
                'bytes', 'x', 'epoch', 1, 'token', 1,
                'digest', ARGV[3], 'published_at_ms', 1)
        end
        return count
        ",
    );
    let written: i64 = script
        .arg(config.prefix.as_str())
        .arg(seeded)
        .arg("0".repeat(64))
        .invoke_async(&mut conn)
        .await
        .expect("seed more entries than the inspection cap");
    assert_eq!(written, seeded);

    let inspection = store.inspect().await.expect("inspect");
    assert!(
        inspection.entries >= 10_000,
        "the inspection reads up to its cap: {inspection:?}"
    );
    assert!(
        i64::try_from(inspection.entries).expect("a count") < seeded,
        "and never the whole keyspace: {inspection:?}"
    );
}

#[tokio::test]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_a_lease_is_taken_over_by_store_time_and_the_former_leader_is_fenced() {
    let (config, mut conn, _cleanup) = boot_redis().await;
    let first = RedisLeaseStore::connect(&config).await.expect("connect");
    let second = RedisLeaseStore::connect(&config).await.expect("connect");
    let key = key("/tier-two");

    let (held, expires_at_ms) = acquired(
        first
            .try_acquire(&key, 1, 1_000)
            .await
            .expect("the first acquire"),
    );
    assert_eq!(held, 1, "the key's own tenure counter starts at one");
    assert!(
        expires_at_ms > redis_now_ms(&mut conn).await,
        "the expiry is Redis's clock plus the lifetime, not this node's"
    );
    assert_eq!(first.mint_token(&key, held).await.expect("mint"), Some(1));
    assert_eq!(first.mint_token(&key, held).await.expect("mint"), Some(2));
    assert_eq!(
        second.try_acquire(&key, 1, 1_000).await.expect("acquire"),
        LeaseAttempt::Held,
        "an unexpired lease belongs to whoever took it"
    );
    assert_eq!(
        second.mint_token(&key, held + 1).await.expect("mint"),
        None,
        "and a lease id that holds nothing mints nothing"
    );

    // Nobody waits: store time is what ends a tenure, and the test moves it.
    first.set_time_offset_for_test(2_000);
    second.set_time_offset_for_test(2_000);

    let (taken, _) = acquired(second.try_acquire(&key, 2, 1_000).await.expect("takeover"));
    assert_eq!(taken, held + 1, "a takeover advances the tenure counter");
    assert_eq!(
        second.mint_token(&key, taken).await.expect("mint"),
        Some(3),
        "and the token counter carries on rather than restarting"
    );
    assert_eq!(
        first.mint_token(&key, held).await.expect("mint"),
        None,
        "the former leader is fenced out"
    );

    // A release by a lease id that no longer holds the key changes nothing.
    first.release(&key, held).await.expect("stale release");
    assert_eq!(
        second.try_acquire(&key, 2, 1_000).await.expect("acquire"),
        LeaseAttempt::Held
    );

    second.release(&key, taken).await.expect("release");
    let (after, _) = acquired(first.try_acquire(&key, 3, 1_000).await.expect("acquire"));
    assert_eq!(after, taken + 1);
    assert_eq!(
        first.mint_token(&key, after).await.expect("mint"),
        Some(4),
        "the token counter outlives every tenure, so it never restarts"
    );
    assert_eq!(redis_token(&mut conn, &config.prefix, &key).await, Some(4));

    // Neither key carries a lifetime. Were Redis to reclaim the lease hash at
    // the tenure's expiry, the next acquisition would start the tenure
    // counter over at one and a former leader holding tenure one would mint
    // against a tenure that is not its own; were it to reclaim the token
    // counter, that leader's already-minted token could outrank the
    // publication that replaced it.
    for name in [
        format!("{}lease:{}", config.prefix, key.to_base64url()),
        format!("{}token:{}", config.prefix, key.to_base64url()),
    ] {
        let lifetime: i64 = redis::cmd("PTTL")
            .arg(&name)
            .query_async(&mut conn)
            .await
            .expect("read the key lifetime");
        assert_eq!(lifetime, -1, "{name} must outlive every tenure");
    }
}

#[tokio::test]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_two_coordinators_lead_once_and_bypass_once() {
    let (config, _conn, _cleanup) = boot_redis().await;
    let limits = LocalCoordinatorLimits {
        lease_ms: 30_000,
        max_waiters: 4,
    };
    let leader = FencedLeaseCoordinator::new(
        Arc::new(RedisLeaseStore::connect(&config).await.expect("connect")),
        limits,
    );
    let peer = FencedLeaseCoordinator::new(
        Arc::new(RedisLeaseStore::connect(&config).await.expect("connect")),
        limits,
    );
    let key = key("/tier-two");

    let RebuildAdmission::Lead(lease) = leader.admit(&key, 1, 1_000).await.expect("admit") else {
        panic!("the first coordinator over an unheld key leads");
    };
    assert!(
        matches!(
            peer.admit(&key, 1, 1_000).await.expect("admit"),
            RebuildAdmission::Bypass
        ),
        "a node that does not hold the store's lease renders without publishing"
    );

    let published = leader.publish_token(&lease, 1_000).await.expect("token");
    assert_eq!(published.token, 1);
    leader.release(*lease).await.expect("release");

    let RebuildAdmission::Lead(next) = peer.admit(&key, 1, 2_000).await.expect("admit") else {
        panic!("the released key is the peer's to lead");
    };
    assert_eq!(
        peer.publish_token(&next, 2_000).await.expect("token").token,
        2,
        "exactly one token per fence, and never a reissued one"
    );
    peer.release(*next).await.expect("release");
}

#[tokio::test]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_records_are_created_once_and_compare_and_store_fences_a_stale_read() {
    let (config, mut conn, _cleanup) = boot_redis().await;
    let first = RedisInstanceRecordStore::connect(&config)
        .await
        .expect("connect");
    let second = RedisInstanceRecordStore::connect(&config)
        .await
        .expect("connect");
    let key = instance_key(0x61);
    let wide = wide_instance_key(0x62);
    let expires_at = redis_deadline(&mut conn, 60_000).await;

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
            .expect("insert"),
        "an unexpired record holds its key against every peer"
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
        CasOutcome::Conflict,
        "a write derived from a stale read is refused"
    );
    assert_eq!(
        first
            .compare_and_store(&instance_key(0x63), 1, b"nothing", expires_at)
            .await
            .expect("compare and store"),
        CasOutcome::Missing,
        "and a key nothing holds has nothing to replace"
    );
    let stored = first.load(&key).await.expect("load").expect("a record");
    assert_eq!(stored.bytes, b"two".to_vec());
    assert_eq!(stored.version, 2);
    assert_eq!(stored.expires_at, expires_at);

    // The hash carries its own deadline as a lifetime too, so Redis reclaims
    // a record's bytes whether or not a creating operation reaches it.
    let lifetime: i64 = redis::cmd("PTTL")
        .arg(format!(
            "{}instance:{}:{}",
            config.prefix,
            hex::encode(key.scope.as_bytes()),
            hex::encode(key.instance_id.as_bytes())
        ))
        .query_async(&mut conn)
        .await
        .expect("read the record lifetime");
    assert!(lifetime > 0 && lifetime <= 60_000, "{lifetime}");

    // A full-width identity is its own key, never a truncation of a shorter
    // one that shares its opening bytes.
    assert!(
        first
            .insert_if_absent(&wide, b"wide", expires_at)
            .await
            .expect("insert")
    );
    assert_eq!(first.count_instances().await.expect("count"), 2);

    first.remove(&key).await.expect("remove");
    assert!(first.load(&key).await.expect("load").is_none());
    assert_eq!(first.count_instances().await.expect("count"), 1);

    let oversized = vec![0_u8; MAX_RECORD_BYTES + 1];
    assert_eq!(
        first
            .insert_if_absent(&instance_key(0x64), &oversized, expires_at)
            .await
            .expect_err("a record over the bound is refused")
            .kind(),
        LedgerErrorKind::CapacityExceeded
    );
    assert_eq!(
        first
            .compare_and_store(&wide, 1, &oversized, expires_at)
            .await
            .expect_err("a replacement over the bound is refused")
            .kind(),
        LedgerErrorKind::CapacityExceeded
    );
    assert_eq!(
        first.count_instances().await.expect("count"),
        1,
        "and a refused record reaches no command"
    );
}

#[tokio::test]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_an_elapsed_record_answers_as_one_that_was_never_written() {
    let (config, mut conn, _cleanup) = boot_redis().await;
    let store = RedisInstanceRecordStore::connect(&config)
        .await
        .expect("connect");
    let key = instance_key(0x65);
    let promotion = promotion_key(0x66);
    let expires_at = redis_deadline(&mut conn, 1_000).await;

    assert!(
        store
            .insert_if_absent(&key, b"record", expires_at)
            .await
            .expect("insert")
    );
    assert!(
        store
            .insert_promotion_if_absent(&promotion, b"reservation", expires_at)
            .await
            .expect("insert")
    );
    assert!(
        !store
            .insert_promotion_if_absent(&promotion, b"again", expires_at)
            .await
            .expect("insert"),
        "an unexpired reservation holds its retry identity"
    );
    assert_eq!(store.count_instances().await.expect("count"), 1);

    store.set_time_offset_for_test(5_000);

    assert!(store.load(&key).await.expect("load").is_none());
    assert!(
        store
            .load_promotion(&promotion)
            .await
            .expect("load")
            .is_none()
    );
    assert_eq!(
        store.count_instances().await.expect("count"),
        0,
        "an elapsed member is not counted against capacity"
    );
    assert_eq!(
        store
            .compare_and_store(&key, 1, b"revived", redis_deadline(&mut conn, 60_000).await)
            .await
            .expect("compare and store"),
        CasOutcome::Missing,
        "and an elapsed record is never replaced back into life"
    );

    // The key is free again: creation over an elapsed record starts a new
    // record, at the version a creation gives.
    let renewed = redis_deadline(&mut conn, 60_000).await;
    assert!(
        store
            .insert_if_absent(&key, b"fresh", renewed)
            .await
            .expect("insert")
    );
    assert!(
        store
            .insert_promotion_if_absent(&promotion, b"fresh", renewed)
            .await
            .expect("insert")
    );
    let stored = store.load(&key).await.expect("load").expect("a record");
    assert_eq!(stored.version, 1);
    assert_eq!(stored.bytes, b"fresh".to_vec());
}

#[tokio::test]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_reclamation_is_bounded_and_drains_over_the_operations_that_follow() {
    let (config, mut conn, _cleanup) = boot_redis().await;
    let store = RedisInstanceRecordStore::connect(&config)
        .await
        .expect("connect");
    let elapsing = redis_deadline(&mut conn, 1_000).await;

    let seeded = 70;
    for tag in 0..seeded {
        assert!(
            store
                .insert_if_absent(&instance_key(0x80 + tag), b"record", elapsing)
                .await
                .expect("insert")
        );
    }
    assert_eq!(
        redis_index_size(&mut conn, &config.prefix).await,
        i64::from(seeded)
    );

    store.set_time_offset_for_test(5_000);
    let far = redis_deadline(&mut conn, 60_000).await;

    // One creating operation reclaims at most sixty-four elapsed members,
    // the same bound the in-memory reference store applies, so a burst of
    // expiries that arrive together is paid for over the operations that
    // follow rather than by whichever one is unlucky.
    store
        .insert_if_absent(&instance_key(0x01), b"live", far)
        .await
        .expect("insert");
    assert_eq!(
        redis_index_size(&mut conn, &config.prefix).await,
        i64::from(seeded) - 64 + 1
    );
    store
        .insert_if_absent(&instance_key(0x02), b"live", far)
        .await
        .expect("insert");
    assert_eq!(
        redis_index_size(&mut conn, &config.prefix).await,
        2,
        "and the backlog drains over the operations that follow"
    );
    assert_eq!(store.count_instances().await.expect("count"), 2);
}

#[tokio::test]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_the_ledger_kernel_over_the_redis_store_answers_the_conformance_suite() {
    let (config, mut conn, _cleanup) = boot_redis().await;
    let base_ms = redis_now_ms(&mut conn).await;
    let clock = Arc::new(ControlledClock::new(UnixMillis::new(base_ms)));
    let store = RedisInstanceRecordStore::connect(&config)
        .await
        .expect("connect");

    ledger_conformance::run_all(
        conformance_ledger(store, &clock, base_ms),
        Arc::clone(&clock),
    )
    .await
    .expect("the Redis record store answers the ledger conformance suite");
}

#[tokio::test]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_two_ledger_handles_over_one_redis_answer_the_two_node_suite() {
    let (config, mut conn, _cleanup) = boot_redis().await;
    let base_ms = redis_now_ms(&mut conn).await;
    let clock = Arc::new(ControlledClock::new(UnixMillis::new(base_ms)));

    ledger_conformance::run_two_node(
        conformance_ledger(
            RedisInstanceRecordStore::connect(&config)
                .await
                .expect("connect"),
            &clock,
            base_ms,
        ),
        conformance_ledger(
            RedisInstanceRecordStore::connect(&config)
                .await
                .expect("connect"),
            &clock,
            base_ms,
        ),
        Arc::clone(&clock),
    )
    .await
    .expect("two Redis-backed ledgers answer the two-node conformance suite");
}

#[tokio::test]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_a_run_leaves_nothing_behind_under_its_own_prefix() {
    let (config, mut conn, _cleanup) = boot_redis().await;
    let store = RedisRenderStore::connect(&config, 1024 * 1024)
        .await
        .expect("connect");
    let leases = RedisLeaseStore::connect(&config).await.expect("connect");
    let records = RedisInstanceRecordStore::connect(&config)
        .await
        .expect("connect");
    let key = key("/tier-two");

    store
        .publish(
            &key,
            Bytes::from_static(b"bytes"),
            fence(1, 1),
            1_000,
            60_000,
        )
        .await
        .expect("publish");
    let (lease_id, _) = acquired(leases.try_acquire(&key, 1, 30_000).await.expect("acquire"));
    leases.mint_token(&key, lease_id).await.expect("mint");
    records
        .insert_if_absent(
            &instance_key(0x67),
            b"record",
            redis_deadline(&mut conn, 60_000).await,
        )
        .await
        .expect("insert");
    records
        .insert_promotion_if_absent(
            &wide_promotion_key(0x68),
            b"reservation",
            redis_deadline(&mut conn, 60_000).await,
        )
        .await
        .expect("insert");

    let written = redis_keys(&mut conn, &config.prefix).await;
    for kind in [
        "entry:",
        "lease:",
        "token:",
        "instance:",
        "promotion:",
        "instances",
    ] {
        assert!(
            written
                .iter()
                .any(|name| name.starts_with(&format!("{}{kind}", config.prefix))),
            "every adapter writes under the configured namespace: {kind} missing from {written:?}"
        );
    }

    clear_redis_prefix(&mut conn, &config.prefix).await;
    assert!(redis_keys(&mut conn, &config.prefix).await.is_empty());
}

// --- Profiles: what `install` and the Live ledger boot accept and refuse ---

#[tokio::test]
async fn the_database_profile_refuses_to_install_without_the_tier_migration() {
    let _db = boot_without_the_tier_tables().await;

    let refused = install(tier_config(
        L1Config::Database {
            max_bytes: 1024 * 1024,
        },
        CoordinatorConfig::Local {
            lease_ms: 30_000,
            max_waiters: 128,
        },
    ))
    .await
    .expect_err("a database L1 tier without its table must not install");
    let message = refused.to_string();
    assert!(
        message.contains("m20260906_000000_create_render_cache_tier_tables"),
        "the refusal names the migration to add: {message}"
    );

    // The coordinator reaches a different table of the same migration, so it
    // has to be refused on its own too - a database coordinator in front of
    // a file L1 is a legitimate shape, and it still needs the tables.
    let refused = install(tier_config(
        L1Config::Disabled,
        CoordinatorConfig::Database {
            lease_ms: 30_000,
            max_waiters: 128,
        },
    ))
    .await
    .expect_err("a database coordinator without its table must not install");
    assert!(
        refused
            .to_string()
            .contains("m20260906_000000_create_render_cache_tier_tables"),
        "{refused}"
    );
}

/// The `L1Provider::Database` arm of the sweep every operator reaches, over
/// a provider this test built.
///
/// [`sweep_l1`] is the body of `RenderCache::sweep` with the runtime lookup
/// lifted out, and it is what this calls: installing a runtime to reach one
/// `match` arm would bind a *process* singleton that every other test in this
/// binary can see, and `RenderCache::sweep` adds nothing over `sweep_l1` but
/// that lookup and the file tier's epoch read.
///
/// The epoch argument is deliberately a value no ledger would return: the
/// database arm must not consult it, and passing one proves the arm ignores
/// what it is given rather than merely that no ledger was queried.
#[tokio::test]
async fn the_database_l1_provider_sweeps_its_own_rows_through_the_facade_body() {
    let _db = boot().await;
    let provider = L1Provider::Database(SqlRenderStore::new(1024 * 1024));

    // A retention of zero makes each row due by the database's own clock the
    // moment it is written, so nothing here waits on a timer: the sweep's
    // `now` is read after the publication's.
    for pattern in ["/facade-a", "/facade-b"] {
        provider
            .publish(
                &key(pattern),
                Bytes::from_static(b"due immediately"),
                fence(1, 1),
                1_000,
                0,
            )
            .await
            .expect("publish");
    }
    assert_eq!(row_count().await, 2);

    let swept = sweep_l1(&provider, 1_000, u64::MAX)
        .await
        .expect("the database tier sweeps");
    assert_eq!(swept.removed, 2);
    assert!(!swept.more_remain);
    assert_eq!(row_count().await, 0);
}

#[tokio::test]
async fn a_redis_profile_whose_endpoint_answers_nothing_refuses_to_install() {
    let _db = boot().await;
    let closed = redis_config_on_a_closed_port();

    let refused = install(tier_config(
        L1Config::Redis {
            url: closed.url.clone(),
            prefix: closed.prefix.clone(),
            max_bytes: 1024 * 1024,
        },
        CoordinatorConfig::Local {
            lease_ms: 30_000,
            max_waiters: 128,
        },
    ))
    .await
    .expect_err("a Redis tier nothing answers must not install");
    let message = refused.to_string();
    assert!(
        message.contains("RENDER_CACHE_REDIS_URL"),
        "the refusal names the setting to fix: {message}"
    );
    assert!(!message.contains("127.0.0.1"), "{message}");
    assert!(!message.contains(&closed.url), "{message}");

    // And the coordinator alone is refused on the same terms.
    let refused = install(tier_config(
        L1Config::Disabled,
        CoordinatorConfig::Redis {
            url: closed.url.clone(),
            prefix: closed.prefix.clone(),
            lease_ms: 30_000,
            max_waiters: 128,
        },
    ))
    .await
    .expect_err("a Redis coordinator nothing answers must not install");
    assert!(
        refused.to_string().contains("RENDER_CACHE_REDIS_URL"),
        "{refused}"
    );
}

#[tokio::test]
async fn the_live_database_ledger_driver_needs_the_same_migration() {
    let _db = boot_without_the_tier_tables().await;
    let refused = verify_ledger_driver_for_test(&LedgerDriver::Database)
        .await
        .expect_err("a database ledger without its tables must not boot");
    let message = refused.to_string();
    assert!(message.contains("LIVE_LEDGER_DRIVER"), "{message}");
    assert!(
        message.contains("m20260906_000000_create_render_cache_tier_tables"),
        "{message}"
    );
}

#[tokio::test]
async fn the_live_database_ledger_driver_boots_once_the_migration_is_applied() {
    let _db = boot().await;
    verify_ledger_driver_for_test(&LedgerDriver::Database)
        .await
        .expect("the database ledger driver boots against the tier tables");
}

// --- End to end: the profiles through the middleware, the Live ledger, and
// --- two nodes over one backend ---
//
// Everything above proves one adapter against one backend. What follows
// proves the two things an adapter alone cannot: that an ordinary request
// through the real middleware reaches these providers and comes back from
// them, and that two nodes sharing one backend agree about who may publish
// and what the current revision is.
//
// Two nodes are two adapter handles and two coordinators over one backend,
// for the same reason the sections above give: the RenderCache runtime and
// the Live runtime are process singletons, so a second runtime is not
// something a test process can have. What a single-node middleware test does
// prove is the wiring - that `RenderCache::install` on a profile actually
// puts these providers on the request path - and that is what it is here for.

/// The `admit`, `publish_token`, and `release` calls the middleware makes,
/// paired here with the publication they fence, so a test that says "one node
/// publishes" is exercising the coordinator the middleware uses rather than
/// the lease store underneath it.
fn coordinator_over_the_database(lease_ms: u64) -> FencedLeaseCoordinator<SqlLeaseStore> {
    FencedLeaseCoordinator::new(
        Arc::new(SqlLeaseStore::new()),
        LocalCoordinatorLimits {
            lease_ms,
            max_waiters: 4,
        },
    )
}

#[tokio::test]
async fn two_nodes_publish_one_entry_and_the_node_that_bypassed_publishes_nothing() {
    let _db = boot().await;
    let leader = coordinator_over_the_database(30_000);
    let peer = coordinator_over_the_database(30_000);
    let leader_entries = SqlRenderStore::new(1024 * 1024);
    let peer_entries = SqlRenderStore::new(1024 * 1024);
    let key = key("/tier-one");

    let RebuildAdmission::Lead(lease) = leader.admit(&key, 1, 0).await.expect("admit") else {
        panic!("the node the store admits leads");
    };
    assert!(
        matches!(
            peer.admit(&key, 1, 0).await.expect("admit"),
            RebuildAdmission::Bypass
        ),
        "the other node renders without publishing"
    );

    let fence = leader
        .publish_token(&lease, 0)
        .await
        .expect("the leader mints exactly one fence");
    assert_eq!(
        leader_entries
            .publish(
                &key,
                Bytes::from_static(b"the leader's bytes"),
                fence,
                1_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Published,
        "one publication is accepted per fence"
    );
    leader.release(*lease).await.expect("release");

    // A `Bypass` carries no lease, so the bypassing node has nothing to mint
    // a fence from and never reaches `publish` at all - which is exactly what
    // the middleware does with it. Were it to publish anyway, under the only
    // fence it could have observed, the store refuses it: an equal fence does
    // not supersede.
    assert_eq!(
        peer_entries
            .publish(
                &key,
                Bytes::from_static(b"the bypassing node's bytes"),
                fence,
                2_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Fenced
    );

    assert_eq!(row_count().await, 1, "one fence, one row, one publication");
    let stored = peer_entries
        .get(&key)
        .await
        .expect("get")
        .expect("both nodes read the same row");
    assert_eq!(
        stored.bytes.as_ref(),
        b"the leader's bytes",
        "the bytes every node serves are the leader's"
    );
    assert_eq!(stored.fence, fence);
    assert_eq!(stored.published_at_ms, 1_000);
}

#[tokio::test]
async fn a_leader_whose_lease_elapsed_is_fenced_and_the_takeover_publishes_instead() {
    let _db = boot().await;
    let dying_store = Arc::new(SqlLeaseStore::new());
    let taking_store = Arc::new(SqlLeaseStore::new());
    let limits = LocalCoordinatorLimits {
        lease_ms: 1_000,
        max_waiters: 4,
    };
    let dying = FencedLeaseCoordinator::new(Arc::clone(&dying_store), limits);
    let taking = FencedLeaseCoordinator::new(Arc::clone(&taking_store), limits);
    let entries = SqlRenderStore::new(1024 * 1024);
    let key = key("/tier-one");

    let RebuildAdmission::Lead(dead) = dying.admit(&key, 1, 0).await.expect("admit") else {
        panic!("the first node leads");
    };

    // Store time, never a node's: both handles read the database's own clock,
    // and moving their offsets is what a real pair of nodes reaches by the
    // leader simply not coming back before its lease ran out. Nothing waits.
    dying_store.set_time_offset_for_test(2_000);
    taking_store.set_time_offset_for_test(2_000);

    let RebuildAdmission::Lead(taken) = taking.admit(&key, 1, 5_000).await.expect("admit") else {
        panic!("an elapsed lease is taken over");
    };
    let fence = taking.publish_token(&taken, 5_000).await.expect("token");
    assert_eq!(
        fence.token, 1,
        "the leader died before it minted anything, so this is the key's first \
         token - tokens count publications, not tenures"
    );
    assert_eq!(
        entries
            .publish(
                &key,
                Bytes::from_static(b"the takeover's bytes"),
                fence,
                5_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Published
    );
    taking.release(*taken).await.expect("release");

    // The former leader finishes its render and comes back to publish. It is
    // refused before it reaches the store at all, and its result is discarded.
    let refused = dying
        .publish_token(&dead, 5_000)
        .await
        .expect_err("an elapsed lease mints nothing");
    assert_eq!(refused.kind(), RenderCacheErrorKind::LeaseFenced);
    // And its own clock buys it nothing: `publish_token` is answered by store
    // time, so asking as though no time had passed is refused identically.
    let refused = dying
        .publish_token(&dead, 0)
        .await
        .expect_err("a node clock never extends a distributed lease");
    assert_eq!(refused.kind(), RenderCacheErrorKind::LeaseFenced);
    dying.release(*dead).await.expect("release");

    assert_eq!(row_count().await, 1);
    assert_eq!(
        entries
            .get(&key)
            .await
            .expect("get")
            .expect("a hit")
            .bytes
            .as_ref(),
        b"the takeover's bytes",
        "the row is the node that held the lease when it published"
    );
}

#[tokio::test]
async fn a_restarted_node_reads_the_instance_records_the_previous_one_left() {
    let _db = boot().await;
    let key = instance_key(0x70);
    let expires_at = store_deadline(120_000).await;

    // Everything the previous process did, inside its own scope: a restart
    // takes every handle and every byte of in-process state with it, and the
    // scope ending is what stands in for that here.
    {
        let before = SqlInstanceRecordStore::new();
        assert!(
            before
                .insert_if_absent(&key, b"written before the restart", expires_at)
                .await
                .expect("insert")
        );
        assert_eq!(
            before
                .compare_and_store(&key, 1, b"advanced before the restart", expires_at)
                .await
                .expect("compare and store"),
            CasOutcome::Stored { version: 2 }
        );
    }

    // The record is in the database, so the process that comes up next finds
    // the key held at exactly the version the last one left it at.
    let after = SqlInstanceRecordStore::new();
    let stored = after
        .load(&key)
        .await
        .expect("load")
        .expect("the record outlived the handle that wrote it");
    assert_eq!(stored.bytes, b"advanced before the restart".to_vec());
    assert_eq!(stored.version, 2);
    assert_eq!(after.count_instances().await.expect("count"), 1);
    assert!(
        !after
            .insert_if_absent(&key, b"a restart is not a free key", expires_at)
            .await
            .expect("insert"),
        "a restart does not release the instances the previous process held"
    );
    assert_eq!(
        after
            .compare_and_store(&key, 2, b"advanced after the restart", expires_at)
            .await
            .expect("compare and store"),
        CasOutcome::Stored { version: 3 },
        "and the new process continues the record's versions"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn the_database_profile_publishes_to_sql_l1_serves_from_it_and_sweeps_through_the_facade() {
    let harness = boot_on_the_database_profile_for_test().await;

    // The publication reaches the shared table, not only this process's L0.
    let first = dispatch_route(
        &harness,
        Method::GET,
        SEED_ONLY_PATH,
        &[("x-test-login", "user-1")],
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert_eq!(
        row_count().await,
        1,
        "the entry is a row every node can read"
    );

    // With L0 emptied and the epoch untouched, the next request derives the
    // same key and can only be answered from L1.
    RenderCache::clear_l0_for_test();
    assert!(
        RenderCache::inspect_route_for_test(SEED_ONLY_PATH)
            .await
            .is_none(),
        "precondition: nothing is left in memory"
    );
    let before = handler_renders(SEED_ONLY_PATH);
    let hit = dispatch_route(
        &harness,
        Method::GET,
        SEED_ONLY_PATH,
        &[("x-test-login", "user-2")],
    )
    .await;
    assert_eq!(hit.status, StatusCode::OK, "{}", hit.text());
    assert_eq!(
        handler_renders(SEED_ONLY_PATH),
        before,
        "an L1 hit runs no handler"
    );
    assert!(
        RenderCache::inspect_route_for_test(SEED_ONLY_PATH)
            .await
            .is_some(),
        "and the entry it served is promoted back into L0"
    );

    // A stitched document's Composite entry round-trips through the same
    // table: the shell comes back from SQL and the island inside it is
    // mounted here and now, for whoever is asking.
    let a1 = dispatch_route(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-a")],
    )
    .await;
    assert_eq!(a1.status, StatusCode::OK, "{}", a1.text());
    let stored = RenderCache::inspect_route_for_test(STITCHED_PATH)
        .await
        .expect("stored");
    assert_eq!(stored.kind, EntryKind::Composite);
    assert_eq!(stored.slots, 1);
    assert_eq!(row_count().await, 2);

    RenderCache::clear_l0_for_test();
    let before = handler_renders(STITCHED_PATH);
    let b1 = dispatch_route(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-b")],
    )
    .await;
    assert_eq!(b1.status, StatusCode::OK, "{}", b1.text());
    assert_eq!(
        handler_renders(STITCHED_PATH),
        before,
        "the composite entry came back from SQL and the handler never ran"
    );
    let island_a = island_tag(&a1.text(), "stitch-counter").to_owned();
    let island_b = island_tag(&b1.text(), "stitch-counter").to_owned();
    assert_ne!(
        island_a, island_b,
        "each principal's island is mounted for that principal"
    );
    let shell = |text: &str| text.replace(island_tag(text, "stitch-counter"), "");
    assert_eq!(
        shell(&a1.text()),
        shell(&b1.text()),
        "and the shell around it is the bytes the leader stored"
    );

    // `RenderCache::sweep` is the database arm here. Nothing published above
    // is due yet, and a row another node published with a retention of zero
    // is due by the database's own clock the instant it was written - so this
    // waits on nothing and still proves the facade reached the arm that reads
    // store time rather than a directory or a no-op.
    let swept = RenderCache::sweep().await.expect("sweep");
    assert_eq!(swept.removed, 0);
    assert!(!swept.more_remain);
    assert_eq!(row_count().await, 2);

    SqlRenderStore::new(1024 * 1024)
        .publish(
            &key("/another-node"),
            Bytes::from_static(b"due immediately"),
            fence(1, 1),
            1_000,
            0,
        )
        .await
        .expect("publish");
    assert_eq!(row_count().await, 3);
    let swept = RenderCache::sweep().await.expect("sweep");
    assert_eq!(
        swept.removed, 1,
        "the facade dispatched to the database arm"
    );
    assert!(!swept.more_remain);
    assert_eq!(
        row_count().await,
        2,
        "and the rows that are still live are left alone"
    );
}

/// Sets `LIVE_LEDGER_DRIVER` (and, for the Redis driver, its endpoint and key
/// namespace) for the body of one test and unsets them however that test
/// ends, so a failed assertion never leaves the variable set for whatever
/// runs next in this process. Every caller holds the environment lock.
struct LedgerDriverEnv {
    names: Vec<&'static str>,
}

impl LedgerDriverEnv {
    fn set(pairs: &[(&'static str, String)]) -> Self {
        for (name, value) in pairs {
            // SAFETY: the environment lock each caller holds is what
            // serialises every environment mutation in this test binary.
            unsafe { std::env::set_var(name, value) };
        }
        Self {
            names: pairs.iter().map(|(name, _)| *name).collect(),
        }
    }
}

impl Drop for LedgerDriverEnv {
    fn drop(&mut self) {
        for name in &self.names {
            // SAFETY: as above.
            unsafe { std::env::remove_var(name) };
        }
    }
}

/// The revision carried by a Live snapshot body or an accepted action, which
/// the wire spells as a decimal string.
fn revision_of(value: &Value) -> Revision {
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        other => panic!("no revision here: {other}"),
    };
    Revision::parse(&text).expect("a canonical decimal revision")
}

/// The scope and instance identities of the single row in
/// `suprnova_live_instances`, decoded from the hex the columns store.
///
/// Read out of the table rather than out of the snapshot: the columns are
/// what a second node addresses a record by, so taking the identities from
/// there is what makes the read that follows a genuine second reader of this
/// row rather than a second decoding of the first reader's own state.
async fn the_only_instance_identity() -> (ScopeFingerprint, InstanceId) {
    assert_eq!(instance_row_count().await, 1, "exactly one mounted island");
    let scope: String = DB::scalar("SELECT scope FROM suprnova_live_instances", vec![])
        .await
        .expect("the stored scope");
    let instance: String = DB::scalar("SELECT instance FROM suprnova_live_instances", vec![])
        .await
        .expect("the stored instance identity");
    (
        ScopeFingerprint::from_bytes(&hex::decode(scope).expect("hex")).expect("a scope"),
        InstanceId::from_bytes(&hex::decode(instance).expect("hex")).expect("an instance identity"),
    )
}

/// A ledger handle with nothing in common with the running runtime's but the
/// backend: its own record store, its own clock, and the limits the runtime
/// itself builds. This is the "second process" of these tests.
fn a_second_nodes_ledger<S: InstanceRecordStore + 'static>(
    store: S,
) -> Arc<dyn LiveInstanceLedger> {
    Arc::new(DistributedInstanceLedger::new(
        Arc::new(store),
        Arc::new(SystemClock),
        LedgerLimits::new(30_000, 604_800_000, 64, 100_000).expect("the runtime's ledger limits"),
    ))
}

/// Mounts the identity-bound dogfood island, acts on it once, and answers
/// with the revision the mount carried and the revision the action committed.
///
/// Shared by the database and Redis ledger tests: what differs between them
/// is which driver the runtime bound, never what the browser does.
async fn mount_and_act_on_the_identity_bound_island() -> (Revision, Revision) {
    live_dogfood_support::fixture();
    let router = Arc::new(build_public_router());
    prepare_live_router_for_test(&router).expect("prepare the Live runtime");
    let middleware = production_middleware();

    // Sign in on one request, as a login handler would, so the identity-bound
    // render on the next request binds the session that survives the
    // framework's fixation rotation.
    let mut login = live_get(DOCUMENT_PATH);
    login
        .headers_mut()
        .insert("x-test-login", "user-7".parse().expect("header"));
    let (status, headers, body) =
        dispatch_live(Arc::clone(&router), Arc::clone(&middleware), login).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let signed_in = session_cookie(&headers);

    let mut private = live_get(PRIVATE_DOCUMENT_PATH);
    private
        .headers_mut()
        .insert("x-test-login", "user-7".parse().expect("header"));
    private
        .headers_mut()
        .insert("cookie", signed_in.parse().expect("cookie"));
    let (status, headers, body) =
        dispatch_live(Arc::clone(&router), Arc::clone(&middleware), private).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let cookie = session_cookie(&headers);
    let snapshot = live_dogfood_support::decoded_snapshot(&body);
    let mounted = revision_of(&snapshot["body"]["revision"]);

    let (status, _, body) = dispatch_live(
        Arc::clone(&router),
        Arc::clone(&middleware),
        private_action_request(ActionRequest {
            snapshot,
            cookie: &cookie,
            fetch_site: Some("same-origin"),
            login: Some("user-7"),
            idempotency_key: "QEFCQ0RFRkdISUpLTE1OTw",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let accepted: Value = serde_json::from_slice(&body).expect("an accepted action");
    assert_eq!(
        accepted["outcome"],
        "accepted",
        "{}",
        String::from_utf8_lossy(&body)
    );
    // The action's own response carries the successor envelope, and the
    // revision inside it is the one the ledger committed.
    let committed = revision_of(&accepted["snapshot"]["body"]["revision"]);
    assert_eq!(
        committed.get(),
        mounted.get() + 1,
        "the action committed the mounted revision's successor"
    );
    (mounted, committed)
}

#[tokio::test]
#[serial_test::serial]
async fn live_actions_on_the_database_ledger_driver_advance_a_revision_a_second_node_reads() {
    let _env = crate::env_lock::lock_env_async().await;
    let _driver = LedgerDriverEnv::set(&[("LIVE_LEDGER_DRIVER", "database".to_owned())]);
    let _db = boot().await;

    let (_mounted, committed) = mount_and_act_on_the_identity_bound_island().await;

    let (scope, instance) = the_only_instance_identity().await;
    assert_eq!(
        a_second_nodes_ledger(SqlInstanceRecordStore::new())
            .current_accepted_revision(&scope, &instance)
            .await
            .expect("the second node's read"),
        Some(committed),
        "a second process reads the revision this one committed, out of the database"
    );
}

// --- Tier 2 equivalents of everything above ---

/// [`coordinator_over_the_database`]'s Redis twin.
async fn coordinator_over_redis(
    config: &RedisProviderConfig,
    lease_ms: u64,
) -> FencedLeaseCoordinator<RedisLeaseStore> {
    FencedLeaseCoordinator::new(
        Arc::new(RedisLeaseStore::connect(config).await.expect("connect")),
        LocalCoordinatorLimits {
            lease_ms,
            max_waiters: 4,
        },
    )
}

#[tokio::test]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_two_nodes_publish_one_entry_and_the_node_that_bypassed_publishes_nothing() {
    let (config, _conn, _cleanup) = boot_redis().await;
    let leader = coordinator_over_redis(&config, 30_000).await;
    let peer = coordinator_over_redis(&config, 30_000).await;
    let leader_entries = RedisRenderStore::connect(&config, 1024 * 1024)
        .await
        .expect("connect");
    let peer_entries = RedisRenderStore::connect(&config, 1024 * 1024)
        .await
        .expect("connect");
    let key = key("/tier-two");

    let RebuildAdmission::Lead(lease) = leader.admit(&key, 1, 0).await.expect("admit") else {
        panic!("the node the store admits leads");
    };
    assert!(
        matches!(
            peer.admit(&key, 1, 0).await.expect("admit"),
            RebuildAdmission::Bypass
        ),
        "the other node renders without publishing"
    );

    let fence = leader
        .publish_token(&lease, 0)
        .await
        .expect("the leader mints exactly one fence");
    assert_eq!(
        leader_entries
            .publish(
                &key,
                Bytes::from_static(b"the leader's bytes"),
                fence,
                1_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Published
    );
    leader.release(*lease).await.expect("release");

    assert_eq!(
        peer_entries
            .publish(
                &key,
                Bytes::from_static(b"the bypassing node's bytes"),
                fence,
                2_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Fenced,
        "one publication is accepted per fence, here as in SQL"
    );
    let stored = peer_entries
        .get(&key)
        .await
        .expect("get")
        .expect("both nodes read the same key");
    assert_eq!(stored.bytes.as_ref(), b"the leader's bytes");
    assert_eq!(stored.fence, fence);
    assert_eq!(peer_entries.inspect().await.expect("inspect").entries, 1);
}

#[tokio::test]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_a_leader_whose_lease_elapsed_is_fenced_and_the_takeover_publishes_instead() {
    let (config, _conn, _cleanup) = boot_redis().await;
    let dying_store = Arc::new(RedisLeaseStore::connect(&config).await.expect("connect"));
    let taking_store = Arc::new(RedisLeaseStore::connect(&config).await.expect("connect"));
    let limits = LocalCoordinatorLimits {
        lease_ms: 1_000,
        max_waiters: 4,
    };
    let dying = FencedLeaseCoordinator::new(Arc::clone(&dying_store), limits);
    let taking = FencedLeaseCoordinator::new(Arc::clone(&taking_store), limits);
    let entries = RedisRenderStore::connect(&config, 1024 * 1024)
        .await
        .expect("connect");
    let key = key("/tier-two");

    let RebuildAdmission::Lead(dead) = dying.admit(&key, 1, 0).await.expect("admit") else {
        panic!("the first node leads");
    };

    // Redis's own clock, moved by the store's test offset, exactly as the
    // database tier moves the database's. Nothing waits.
    dying_store.set_time_offset_for_test(2_000);
    taking_store.set_time_offset_for_test(2_000);

    let RebuildAdmission::Lead(taken) = taking.admit(&key, 1, 5_000).await.expect("admit") else {
        panic!("an elapsed lease is taken over");
    };
    let fence = taking.publish_token(&taken, 5_000).await.expect("token");
    assert_eq!(
        fence.token, 1,
        "the leader died before it minted anything, so this is the key's first token"
    );
    assert_eq!(
        entries
            .publish(
                &key,
                Bytes::from_static(b"the takeover's bytes"),
                fence,
                5_000,
                60_000
            )
            .await
            .expect("publish"),
        PublishOutcome::Published
    );
    taking.release(*taken).await.expect("release");

    let refused = dying
        .publish_token(&dead, 5_000)
        .await
        .expect_err("an elapsed lease mints nothing");
    assert_eq!(refused.kind(), RenderCacheErrorKind::LeaseFenced);
    let refused = dying
        .publish_token(&dead, 0)
        .await
        .expect_err("a node clock never extends a distributed lease");
    assert_eq!(refused.kind(), RenderCacheErrorKind::LeaseFenced);
    dying.release(*dead).await.expect("release");

    assert_eq!(
        entries
            .get(&key)
            .await
            .expect("get")
            .expect("a hit")
            .bytes
            .as_ref(),
        b"the takeover's bytes"
    );
}

#[tokio::test]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_a_restarted_node_reads_the_instance_records_the_previous_one_left() {
    let (config, mut conn, _cleanup) = boot_redis().await;
    let key = instance_key(0x71);
    let expires_at = redis_deadline(&mut conn, 120_000).await;

    // The previous process, in its own scope; see the database twin's note.
    {
        let before = RedisInstanceRecordStore::connect(&config)
            .await
            .expect("connect");
        assert!(
            before
                .insert_if_absent(&key, b"written before the restart", expires_at)
                .await
                .expect("insert")
        );
        assert_eq!(
            before
                .compare_and_store(&key, 1, b"advanced before the restart", expires_at)
                .await
                .expect("compare and store"),
            CasOutcome::Stored { version: 2 }
        );
    }

    let after = RedisInstanceRecordStore::connect(&config)
        .await
        .expect("connect");
    let stored = after
        .load(&key)
        .await
        .expect("load")
        .expect("the record outlived the handle that wrote it");
    assert_eq!(stored.bytes, b"advanced before the restart".to_vec());
    assert_eq!(stored.version, 2);
    assert_eq!(after.count_instances().await.expect("count"), 1);
    assert!(
        !after
            .insert_if_absent(&key, b"a restart is not a free key", expires_at)
            .await
            .expect("insert")
    );
    assert_eq!(
        after
            .compare_and_store(&key, 2, b"advanced after the restart", expires_at)
            .await
            .expect("compare and store"),
        CasOutcome::Stored { version: 3 }
    );
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_the_redis_profile_publishes_to_the_redis_l1_and_serves_from_it() {
    let (config, mut conn, _cleanup) = boot_redis().await;
    let harness = boot_on_the_redis_profile_for_test(config.clone()).await;

    let first = dispatch_route(
        &harness,
        Method::GET,
        SEED_ONLY_PATH,
        &[("x-test-login", "user-1")],
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    let entries = redis_keys(&mut conn, &format!("{}entry:", config.prefix)).await;
    assert_eq!(
        entries.len(),
        1,
        "the publication reached the accelerator every node reads: {entries:?}"
    );

    RenderCache::clear_l0_for_test();
    let before = handler_renders(SEED_ONLY_PATH);
    let hit = dispatch_route(
        &harness,
        Method::GET,
        SEED_ONLY_PATH,
        &[("x-test-login", "user-2")],
    )
    .await;
    assert_eq!(hit.status, StatusCode::OK, "{}", hit.text());
    assert_eq!(
        handler_renders(SEED_ONLY_PATH),
        before,
        "an L1 hit runs no handler"
    );
    assert!(
        RenderCache::inspect_route_for_test(SEED_ONLY_PATH)
            .await
            .is_some(),
        "and the entry it served is promoted back into L0"
    );

    let a1 = dispatch_route(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-a")],
    )
    .await;
    assert_eq!(a1.status, StatusCode::OK, "{}", a1.text());
    assert_eq!(
        RenderCache::inspect_route_for_test(STITCHED_PATH)
            .await
            .expect("stored")
            .kind,
        EntryKind::Composite
    );
    RenderCache::clear_l0_for_test();
    let before = handler_renders(STITCHED_PATH);
    let b1 = dispatch_route(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-b")],
    )
    .await;
    assert_eq!(b1.status, StatusCode::OK, "{}", b1.text());
    assert_eq!(
        handler_renders(STITCHED_PATH),
        before,
        "the composite entry came back from Redis and the handler never ran"
    );
    let shell = |text: &str| text.replace(island_tag(text, "stitch-counter"), "");
    assert_eq!(shell(&a1.text()), shell(&b1.text()));
    assert_ne!(
        island_tag(&a1.text(), "stitch-counter"),
        island_tag(&b1.text(), "stitch-counter")
    );

    // Redis expires its own keys, so the facade's sweep has nothing to do on
    // this tier - it is a no-op, not a misconfiguration.
    let swept = RenderCache::sweep().await.expect("sweep");
    assert_eq!(swept.removed, 0);
    assert!(!swept.more_remain);
    assert_eq!(
        redis_keys(&mut conn, &format!("{}entry:", config.prefix))
            .await
            .len(),
        2,
        "and the entries the sweep left alone are still there"
    );
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_live_actions_on_the_redis_ledger_driver_advance_a_revision_a_second_node_reads()
{
    let _env = crate::env_lock::lock_env_async().await;
    let (config, mut conn, _cleanup) = boot_redis().await;
    // Under the run's own prefix, so the guard that clears it clears this too.
    let live_prefix = format!("{}live:", config.prefix);
    let _driver = LedgerDriverEnv::set(&[
        ("LIVE_LEDGER_DRIVER", "redis".to_owned()),
        ("LIVE_REDIS_URL", redis_url()),
        ("LIVE_REDIS_PREFIX", live_prefix.clone()),
    ]);
    let _db = boot().await;

    let (_mounted, committed) = mount_and_act_on_the_identity_bound_island().await;

    let instances = redis_keys(&mut conn, &format!("{live_prefix}instance:")).await;
    assert_eq!(
        instances.len(),
        1,
        "exactly one mounted island: {instances:?}"
    );
    let mut parts = instances[0]
        .strip_prefix(&format!("{live_prefix}instance:"))
        .expect("the record key is under the configured namespace")
        .split(':');
    let scope = ScopeFingerprint::from_bytes(
        &hex::decode(parts.next().expect("a scope segment")).expect("hex"),
    )
    .expect("a scope");
    let instance = InstanceId::from_bytes(
        &hex::decode(parts.next().expect("an instance segment")).expect("hex"),
    )
    .expect("an instance identity");

    assert_eq!(
        a_second_nodes_ledger(
            RedisInstanceRecordStore::connect(&config_at(&live_prefix))
                .await
                .expect("connect")
        )
        .current_accepted_revision(&scope, &instance)
        .await
        .expect("the second node's read"),
        Some(committed),
        "a second process reads the revision this one committed, out of Redis"
    );
}

/// The Live ledger's own namespace as a provider configuration, so a second
/// node's record store addresses exactly the keys the runtime wrote.
fn config_at(prefix: &str) -> RedisProviderConfig {
    RedisProviderConfig {
        url: redis_url(),
        prefix: prefix.to_owned(),
    }
}
