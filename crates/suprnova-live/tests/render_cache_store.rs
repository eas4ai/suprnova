//! L0 returns shared immutable bytes, publishes atomically under a fence,
//! and evicts within its bounds.

use bytes::Bytes;
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{KeyId, UnixMillis};
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::store::{
    MemoryRenderStore, MemoryStoreLimits, PublicationFence, PublishOutcome, RenderStore,
};

fn keys_from(root: u8) -> SnapshotKeyRing {
    let active = KeyRecord::new(
        KeyId::parse("render-cache-test").expect("key id"),
        RootKey::new(vec![root; 32]).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(u64::MAX / 2),
        UnixMillis::new(u64::MAX),
    )
    .expect("key record");
    SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
}

fn keys() -> SnapshotKeyRing {
    keys_from(5)
}

fn fence(token: u64) -> PublicationFence {
    PublicationFence {
        epoch: 1,
        generation_digest: [0_u8; 32],
        token,
    }
}

#[tokio::test]
async fn a_hit_shares_the_stored_bytes_without_copying() {
    let store = MemoryRenderStore::new(MemoryStoreLimits {
        max_entries: 4,
        max_bytes: 1024,
    });
    let key = RenderKey::for_test(&keys(), "/a");
    let bytes = Bytes::from(vec![7_u8; 100]);
    assert_eq!(
        store
            .publish(&key, bytes.clone(), fence(1), 1_000, u64::MAX)
            .await
            .expect("publish"),
        PublishOutcome::Published
    );
    let hit = store.get(&key).await.expect("get").expect("hit");
    assert_eq!(
        hit.bytes.as_ptr(),
        bytes.as_ptr(),
        "the same allocation is shared"
    );
    assert_eq!(hit.published_at_ms, 1_000);
}

#[tokio::test]
async fn publication_is_fenced_and_a_stale_fence_is_rejected() {
    let store = MemoryRenderStore::new(MemoryStoreLimits {
        max_entries: 4,
        max_bytes: 1024,
    });
    let key = RenderKey::for_test(&keys(), "/a");
    assert_eq!(
        store
            .publish(&key, Bytes::from_static(b"v2"), fence(2), 2_000, u64::MAX)
            .await
            .expect("p"),
        PublishOutcome::Published
    );
    assert_eq!(
        store
            .publish(&key, Bytes::from_static(b"v1"), fence(1), 3_000, u64::MAX)
            .await
            .expect("p"),
        PublishOutcome::Fenced
    );
    assert_eq!(
        store
            .get(&key)
            .await
            .expect("get")
            .expect("hit")
            .bytes
            .as_ref(),
        b"v2"
    );
    let mut other_epoch = fence(3);
    other_epoch.epoch = 2;
    assert_eq!(
        store
            .publish(
                &key,
                Bytes::from_static(b"e2"),
                other_epoch,
                4_000,
                u64::MAX
            )
            .await
            .expect("p"),
        PublishOutcome::Published,
        "a newer epoch always wins"
    );
}

#[tokio::test]
async fn bounds_evict_the_least_recently_used_entry_and_reject_oversized_entries() {
    let store = MemoryRenderStore::new(MemoryStoreLimits {
        max_entries: 2,
        max_bytes: 300,
    });
    let keys = keys();
    let a = RenderKey::for_test(&keys, "/a");
    let b = RenderKey::for_test(&keys, "/b");
    let c = RenderKey::for_test(&keys, "/c");
    store
        .publish(&a, Bytes::from(vec![1_u8; 100]), fence(1), 1, u64::MAX)
        .await
        .expect("a");
    store
        .publish(&b, Bytes::from(vec![2_u8; 100]), fence(1), 2, u64::MAX)
        .await
        .expect("b");
    store.get(&a).await.expect("touch a");
    store
        .publish(&c, Bytes::from(vec![3_u8; 100]), fence(1), 3, u64::MAX)
        .await
        .expect("c");
    assert!(
        store.get(&b).await.expect("get").is_none(),
        "least recently used entry evicted"
    );
    assert!(store.get(&a).await.expect("get").is_some());
    assert_eq!(
        store
            .publish(&a, Bytes::from(vec![9_u8; 301]), fence(2), 4, u64::MAX)
            .await
            .expect("oversized"),
        PublishOutcome::Rejected
    );
    assert!(
        store.get(&a).await.expect("get").is_some(),
        "a rejected publication never poisons the valid entry"
    );
    let inspection = store.inspect().await.expect("inspect");
    assert_eq!(inspection.entries, 2);
    assert!(inspection.bytes <= 300);
}

#[tokio::test]
async fn a_store_bounded_to_zero_entries_admits_nothing() {
    let store = MemoryRenderStore::new(MemoryStoreLimits {
        max_entries: 0,
        max_bytes: 1024,
    });
    let key = RenderKey::for_test(&keys(), "/a");
    assert_eq!(
        store
            .publish(&key, Bytes::from(vec![7_u8; 32]), fence(1), 1_000, u64::MAX)
            .await
            .expect("publish"),
        PublishOutcome::Rejected,
        "a store bounded to zero entries stores nothing"
    );
    assert!(store.get(&key).await.expect("get").is_none());
    let inspection = store.inspect().await.expect("inspect");
    assert_eq!(inspection.entries, 0);
    assert_eq!(inspection.bytes, 0);
}

#[tokio::test]
async fn a_store_bounded_to_zero_bytes_admits_nothing() {
    let store = MemoryRenderStore::new(MemoryStoreLimits {
        max_entries: 4,
        max_bytes: 0,
    });
    let key = RenderKey::for_test(&keys(), "/a");
    assert_eq!(
        store
            .publish(&key, Bytes::from(vec![7_u8; 32]), fence(1), 1_000, u64::MAX)
            .await
            .expect("publish"),
        PublishOutcome::Rejected,
        "a store bounded to zero bytes stores nothing"
    );
    assert!(store.get(&key).await.expect("get").is_none());
    let inspection = store.inspect().await.expect("inspect");
    assert_eq!(inspection.entries, 0);
    assert_eq!(inspection.bytes, 0);
}

#[tokio::test]
async fn an_entry_exactly_at_the_byte_bound_is_stored() {
    let store = MemoryRenderStore::new(MemoryStoreLimits {
        max_entries: 4,
        max_bytes: 64,
    });
    let key = RenderKey::for_test(&keys(), "/a");
    assert_eq!(
        store
            .publish(&key, Bytes::from(vec![9_u8; 64]), fence(1), 1_000, u64::MAX)
            .await
            .expect("publish"),
        PublishOutcome::Published,
        "an entry exactly at the byte bound is admitted"
    );
    let hit = store.get(&key).await.expect("get").expect("hit");
    assert_eq!(hit.bytes.len(), 64);
    let inspection = store.inspect().await.expect("inspect");
    assert_eq!(inspection.entries, 1);
    assert_eq!(inspection.bytes, 64);
}

/// Fix round 1 (R94/F11): `clear` empties the store outright, for
/// `RenderCache::advance_epoch` to call - every L0 key embeds the epoch it
/// was derived under, so a full clear is correct the instant the epoch
/// moves, with no filesystem to reconcile against.
#[tokio::test]
async fn clear_empties_the_store_and_a_publish_after_clear_starts_fresh() {
    let store = MemoryRenderStore::new(MemoryStoreLimits {
        max_entries: 4,
        max_bytes: 1024,
    });
    let a = RenderKey::for_test(&keys(), "/a");
    let b = RenderKey::for_test(&keys(), "/b");
    store
        .publish(&a, Bytes::from(vec![1_u8; 50]), fence(1), 1, u64::MAX)
        .await
        .expect("a");
    store
        .publish(&b, Bytes::from(vec![2_u8; 50]), fence(1), 2, u64::MAX)
        .await
        .expect("b");
    assert_eq!(store.inspect().await.expect("inspect").entries, 2);

    store.clear();

    let inspection = store.inspect().await.expect("inspect");
    assert_eq!(inspection.entries, 0, "clear empties the entry map");
    assert_eq!(inspection.bytes, 0, "clear resets the byte tally");
    assert!(store.get(&a).await.expect("get").is_none());
    assert!(store.get(&b).await.expect("get").is_none());

    // A publish after clear is not blocked by any residual eviction-order
    // state.
    assert_eq!(
        store
            .publish(&a, Bytes::from(vec![3_u8; 10]), fence(1), 3, u64::MAX)
            .await
            .expect("publish after clear"),
        PublishOutcome::Published
    );
    assert_eq!(store.inspect().await.expect("inspect").entries, 1);
}
