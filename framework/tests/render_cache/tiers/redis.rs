//! Tier 2: the Redis L1 store, rebuild lease store, and Live instance
//! record store.
//!
//! Everything here drives a Redis adapter against a real Redis. The tests
//! that need one are `#[ignore]`d and scope every key under a UUID-unique
//! prefix, so a shared instance is safe and a run leaves nothing behind. To
//! run them, point the URL env var at a DISPOSABLE Redis and pass
//! `--ignored`:
//!
//!   REDIS_TEST_URL=redis://127.0.0.1:6379/ \
//!     cargo test -p suprnova --test render_cache -- --ignored tiers::redis::live_redis
//!
//! The two tests that prove how an unreachable Redis is reported need no
//! Redis at all and run in the default suite.
//!
//! The adapters answer the same contracts `sql` proves of the database
//! tier - one accepted publication per fence, an evicted entry answering as
//! a miss, a tampered hash decoding as a miss, a lease taken over only by
//! store time, compare-and-store fencing a stale read - and the last
//! section answers the parent module's three two-node proofs with this
//! tier's handles, so both tiers are held to one contract rather than two
//! that read alike.

use std::sync::Arc;

use bytes::Bytes;
use hyper::Method;
use suprnova::StatusCode;
use suprnova::render_cache::RenderCache;
use suprnova::render_cache::providers::{
    RedisInstanceRecordStore, RedisLeaseStore, RedisProviderConfig, RedisRenderStore,
};
use suprnova_live::identity::{InstanceId, ScopeFingerprint, UnixMillis};
use suprnova_live::ledger::{CasOutcome, InstanceRecordStore, LedgerErrorKind, MAX_RECORD_BYTES};
use suprnova_live::render_cache::entry::{EntryKind, EntryLimits, decode};
use suprnova_live::render_cache::singleflight::{
    LocalCoordinatorLimits, RebuildAdmission, RebuildCoordinator,
};
use suprnova_live::render_cache::store::{PublishOutcome, RenderStore};
use suprnova_live::render_cache::{
    FencedLeaseCoordinator, LeaseAttempt, LeaseStore, RenderCacheErrorKind,
};
use suprnova_live_test_support::{ControlledClock, ledger_conformance};

use crate::render_cache_stitch_support;
use crate::render_cache_tiers_support;
use render_cache_stitch_support::{
    SEED_ONLY_PATH, STITCHED_PATH, boot_on_the_redis_profile_for_test, dispatch as dispatch_route,
    handler_renders, island_tag,
};
use render_cache_tiers_support::{
    boot, boot_redis, clear_redis_prefix, encoded_entry, fence, instance_key, key, keys,
    promotion_key, redis_config_on_a_closed_port, redis_deadline, redis_keys, redis_now_ms,
    redis_url, wide_instance_key, wide_promotion_key,
};

use super::{
    LedgerDriverEnv, a_second_nodes_ledger, acquired,
    assert_a_restarted_node_reads_the_records_the_previous_one_left,
    assert_an_elapsed_leader_is_fenced_and_the_takeover_publishes_instead,
    assert_two_nodes_publish_one_entry_and_the_bypassing_node_publishes_nothing,
    conformance_ledger, mount_and_act_on_the_identity_bound_island,
};

// --- The Redis L1 store, lease store, and instance record store ---

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

// --- Tier 2 equivalents of the two-node proofs ---

/// The Redis twin of `sql`'s `coordinator_over_the_database`.
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

    assert_two_nodes_publish_one_entry_and_the_bypassing_node_publishes_nothing(
        &leader,
        &peer,
        &leader_entries,
        &peer_entries,
        &key,
    )
    .await;
}

#[tokio::test]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_a_leader_whose_lease_elapsed_is_fenced_and_the_takeover_publishes_instead() {
    let (config, _conn, _cleanup) = boot_redis().await;
    let dying_store = Arc::new(RedisLeaseStore::connect(&config).await.expect("connect"));
    let taking_store = Arc::new(RedisLeaseStore::connect(&config).await.expect("connect"));
    let entries = RedisRenderStore::connect(&config, 1024 * 1024)
        .await
        .expect("connect");
    let key = key("/tier-two");

    assert_an_elapsed_leader_is_fenced_and_the_takeover_publishes_instead(
        &dying_store,
        &taking_store,
        &entries,
        &key,
    )
    .await;
}

#[tokio::test]
#[ignore = "requires live Redis; run with --ignored live_redis"]
async fn live_redis_a_restarted_node_reads_the_instance_records_the_previous_one_left() {
    let (config, mut conn, _cleanup) = boot_redis().await;
    let key = instance_key(0x71);
    let expires_at = redis_deadline(&mut conn, 120_000).await;
    let config = &config;

    assert_a_restarted_node_reads_the_records_the_previous_one_left(
        &key,
        expires_at,
        move || async move {
            RedisInstanceRecordStore::connect(config)
                .await
                .expect("connect")
        },
    )
    .await;
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
