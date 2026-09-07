//! The in-process L0 store answers the portable `RenderStore` conformance
//! suite, and its hot slot obeys the same rules the suite proves for bytes.
//!
//! `render_store_conformance::run_all` is the portable half: the very
//! scenarios the framework runs over the file-backed, SQL-backed, and
//! Redis-backed providers, so what passes here is proven of every provider
//! in exactly the same words rather than in a second set that merely reads
//! alike.
//!
//! What follows the suite is memory-only by construction.
//! `MemoryRenderStore::publish_hot` and `MemoryRenderStore::hot_get` are
//! inherent methods rather than trait methods, because a hot slot holds a
//! decoded body and its formed header values - state no out-of-process
//! provider can hand back. The trait therefore cannot reach them, and a
//! suite written against the trait alone cannot either. This section runs
//! them over the same store the suite just left empty, so the hot slot is
//! shown to be evicted with its bytes and fenced with its bytes, which is
//! the whole of what the suite proves for the bytes themselves.

use std::sync::Arc;

use bytes::Bytes;
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{KeyId, UnixMillis};
use suprnova_live::render_cache::entry::{
    CompleteEntry, EntryHeader, EntryLimits, SafeHeaders, encode,
};
use suprnova_live::render_cache::generation::GenerationSet;
use suprnova_live::render_cache::hot::HotEntry;
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::policy::{
    FreshnessPolicy, RepresentationClass, SharedCachePolicy,
};
use suprnova_live::render_cache::store::{
    MemoryRenderStore, MemoryStoreLimits, PublicationFence, PublishOutcome, RenderStore,
};
use suprnova_live::render_cache::variance::VarianceDescriptor;
use suprnova_live_test_support::render_store_conformance;

/// The byte bound the store under conformance is built with, and the bound
/// the suite's oversized scenario publishes one byte past.
const MAX_BYTES: usize = 1 << 20;

/// The publication instant the hot section publishes at.
const PUBLISHED_AT_MS: u64 = 1_000;

/// The route pattern the hot section's key is derived from. Outside the
/// suite's own namespace, so neither can disturb the other.
const HOT_PATTERN: &str = "/memory-hot-slot";

// The same key ring `tests/render_cache_entry.rs` derives from root seed 3;
// each `tests/*.rs` file is its own crate, so the two cannot share a helper.
fn keys() -> SnapshotKeyRing {
    let active = KeyRecord::new(
        KeyId::parse("render-cache-test").expect("key id"),
        RootKey::new(vec![3; 32]).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(u64::MAX / 2),
        UnixMillis::new(u64::MAX),
    )
    .expect("key record");
    SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
}

fn fence(epoch: u64, token: u64) -> PublicationFence {
    PublicationFence {
        epoch,
        generation_digest: [0_u8; 32],
        token,
    }
}

fn freshness() -> FreshnessPolicy {
    FreshnessPolicy::new(60_000, 30_000, 30_000).expect("freshness policy")
}

fn shared() -> SharedCachePolicy {
    SharedCachePolicy::SMaxAge { seconds: 30 }
}

/// A Complete entry for [`HOT_PATTERN`]'s key carrying `body`.
fn complete_entry(keys: &SnapshotKeyRing, body: &'static [u8]) -> CompleteEntry {
    CompleteEntry::new(
        EntryHeader {
            key: RenderKey::for_test(keys, HOT_PATTERN),
            class: RepresentationClass::PublicShared,
            variance: VarianceDescriptor::new(),
            published_at_ms: PUBLISHED_AT_MS,
            fresh_ms: 60_000,
            stale_servable_ms: 30_000,
            stale_on_error_ms: 30_000,
            observed: GenerationSet::default(),
            epoch: 1,
            seed_deadline_ms: None,
            status: 200,
            headers: SafeHeaders::from_pairs([("content-type", "text/html; charset=utf-8")])
                .expect("safe headers"),
            content_encoding: None,
        },
        Bytes::from_static(body),
    )
}

/// The encoded bytes of a Complete entry for [`HOT_PATTERN`], and the hot
/// entry prepared from exactly those bytes - the pairing
/// `MemoryRenderStore::publish_hot` puts on its caller.
fn hot_fixture(
    keys: &SnapshotKeyRing,
    body: &'static [u8],
    fence: PublicationFence,
) -> (Bytes, Arc<HotEntry>) {
    let entry = complete_entry(keys, body);
    let bytes = encode(&entry, keys).expect("the hot fixture encodes");
    let hot = HotEntry::prepare(entry, shared(), &freshness(), PUBLISHED_AT_MS, fence)
        .expect("the hot fixture prepares");
    (bytes, Arc::new(hot))
}

#[tokio::test]
async fn the_in_process_store_conforms_and_fences_and_evicts_its_hot_slot_with_the_bytes() {
    let keys = keys();
    let store = MemoryRenderStore::new(MemoryStoreLimits {
        max_entries: 8,
        max_bytes: MAX_BYTES,
    });

    render_store_conformance::run_all(&store, &keys, &EntryLimits::default(), MAX_BYTES).await;

    // --- The hot slot, which the trait cannot reach ---
    //
    // The store the suite just ran over is empty again (its last scenario
    // asserts exactly that), so the publications below start from the same
    // place every scenario above did.
    let key = RenderKey::for_test(&keys, HOT_PATTERN);
    let (bytes, hot) = hot_fixture(
        &keys,
        b"<!doctype html><html><body>hot</body></html>",
        fence(1, 1),
    );

    assert_eq!(
        store.publish_hot(
            &key,
            bytes.clone(),
            Arc::clone(&hot),
            fence(1, 1),
            PUBLISHED_AT_MS
        ),
        PublishOutcome::Published,
        "a hot publication lands under the rules a plain one lands under"
    );
    assert!(
        store.hot_get(&key).is_some(),
        "a hot publication leaves a hot slot to serve the next hit from"
    );

    store.evict(&key).await.expect("evict");
    assert!(
        store.hot_get(&key).is_none(),
        "evict drops the hot slot with the bytes it was published beside"
    );
    assert!(
        store.get(&key).await.expect("get").is_none(),
        "and leaves the key the miss the suite proves it to be"
    );

    // Republished, so the fence rule is exercised over a slot that is
    // actually there.
    assert_eq!(
        store.publish_hot(
            &key,
            bytes.clone(),
            Arc::clone(&hot),
            fence(1, 1),
            PUBLISHED_AT_MS
        ),
        PublishOutcome::Published,
        "an evicted key accepts the fence it held before, having no fence left to compare against"
    );

    // Bytes alone: this publication is a plain one, so it has no hot entry
    // to pair with and none is prepared.
    let replacement = encode(
        &complete_entry(
            &keys,
            b"<!doctype html><html><body>replacement</body></html>",
        ),
        &keys,
    )
    .expect("the replacement entry encodes");
    assert_eq!(
        store
            .publish(
                &key,
                replacement.clone(),
                fence(1, 1),
                PUBLISHED_AT_MS + 1_000,
                u64::MAX,
            )
            .await
            .expect("publish"),
        PublishOutcome::Fenced,
        "an equal fence is fenced for a hot publication exactly as it is for a plain one"
    );
    assert!(
        store.hot_get(&key).is_some(),
        "and a fenced publication leaves the hot slot exactly where it was"
    );

    assert_eq!(
        store
            .publish(
                &key,
                replacement.clone(),
                fence(1, 2),
                PUBLISHED_AT_MS + 1_000,
                u64::MAX,
            )
            .await
            .expect("publish"),
        PublishOutcome::Published,
        "a newer fence replaces the stored bytes"
    );
    assert!(
        store.hot_get(&key).is_none(),
        "and takes the hot slot prepared from the bytes it replaced with it, \
         so no hit is ever served a body the store no longer holds"
    );
    let hit = store.get(&key).await.expect("get").expect("a hit");
    assert!(
        hit.bytes == replacement,
        "while the stored bytes are the replacement's"
    );
}
