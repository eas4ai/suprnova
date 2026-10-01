//! Provider conformance for [`RenderStore`], expressed against the trait
//! alone.
//!
//! What a provider owes a caller is small and the same everywhere: a key
//! that was never published is a miss; a publication under a fence nothing
//! supersedes is returned byte for byte with the instant and the fence it
//! was published under; an equal or older fence is refused and changes no
//! part of the stored entry; a newer fence replaces it; an evicted key is a
//! miss again; bytes past the configured bound are refused before anything
//! is stored; two keys never see each other's bytes; and occupancy counts
//! what is held. Every one of those is written here once, so the in-process
//! store, the file-backed store, the database-backed store on each dialect,
//! and the Redis-backed store are held to the same words rather than to
//! four sets that merely read alike.
//!
//! Two things are deliberately not here. Corruption is proven only as far
//! as the trait can prove it - the store hands back the bytes it was given
//! and [`decode`] refuses them - because *how* bytes are torn is a
//! provider's own business: a half-written file, a truncated blob column,
//! and a hash field a different build wrote are three different failures
//! with one shared consequence, and only the consequence is portable. And
//! nothing here reaches for a provider's own methods: sweeps, tallies,
//! dialect statements, and hot slots are each proven where they live.
//!
//! The suite is not [`CompleteEntry`]-only: a Composite entry is proven
//! here too, by the same reasoning as the corruption scenario, because a
//! provider stores its bytes exactly as opaquely. A well-formed nested
//! segment graph round-trips through the store and resolves; the same
//! graph, read one ownership level deeper than it was published at, is
//! refused by the closed depth cause rather than assembled anyway; and a
//! stored graph naming a segment kind this build does not recognize - the
//! shape a newer build's entry takes to an older one - is refused whole by
//! [`decode`], never served half-understood. No provider can skip any of
//! this: it is reached from [`run_all`] exactly like every other scenario.
//!
//! Three rules make the suite portable. It never waits, and never asks a
//! provider to: every publication carries its own instant, and
//! [`CONFORMANCE_RETENTION_MS`] is far longer than a run, so no scenario
//! depends on time passing or failing to pass. It cleans up after itself,
//! so [`run_all`] leaves the store as empty as it found it. And it needs a
//! store that is empty and bounded to exactly the `max_bytes` it is told
//! about, since the oversized scenario publishes one byte past that bound
//! and the occupancy scenario counts from nothing.

use bytes::Bytes;
use suprnova_live::crypto::SnapshotKeyRing;
use suprnova_live::render_cache::RenderCacheErrorKind;
use suprnova_live::render_cache::composite::{
    AssemblyInput, CompositeEntry, CompositeHeader, NestedFailureCause, NestedOutcome, Segment,
    SegmentGraph, SlotFailurePolicy, assemble_nested, descend_nested,
};
use suprnova_live::render_cache::entry::{
    CompleteEntry, DecodedEntry, EntryHeader, EntryKind, EntryLimits, SafeHeaders, decode, encode,
    encode_composite, encode_raw_header_for_test_with_kind,
};
use suprnova_live::render_cache::generation::GenerationSet;
use suprnova_live::render_cache::key::RenderKey;
use suprnova_live::render_cache::policy::RepresentationClass;
use suprnova_live::render_cache::store::{
    PublicationFence, PublishOutcome, RenderStore, StoreInspection, StoredEntry,
};
use suprnova_live::render_cache::variance::VarianceDescriptor;

/// The instant every scenario's first publication carries.
pub const CONFORMANCE_PUBLISHED_AT_MS: u64 = 1_000;

/// How much later a scenario's second publication is stamped.
///
/// A refused publication must leave the stored instant alone as well as the
/// stored bytes, which only a *different* instant can show.
pub const CONFORMANCE_REPUBLISH_AFTER_MS: u64 = 1_000;

/// The retention every publication is offered.
///
/// A provider that ages entries off by its own clock measures from that
/// clock, not from the instant above, so this is an hour: long enough that
/// no scenario can reach it, short enough to be an ordinary value rather
/// than the "never age-swept" sentinel that would leave the age path
/// untested.
pub const CONFORMANCE_RETENTION_MS: u64 = 60 * 60 * 1_000;

/// The route-pattern namespace every conformance key is derived under, so a
/// suite run over a shared store cannot collide with a provider's own
/// fixtures.
const NAMESPACE: &str = "/render-store-conformance";

/// Runs every conformance scenario against one store.
///
/// `store` must be empty and bounded to `max_bytes`. `keys` derives the
/// suite's lookup keys and signs the entries it publishes, and `limits`
/// bounds the decoding the corruption scenario performs; both may be any
/// valid values, since no scenario depends on which.
///
/// # Panics
///
/// Panics on the first violated contract, naming the contract rather than
/// any stored byte.
pub async fn run_all(
    store: &dyn RenderStore,
    keys: &SnapshotKeyRing,
    limits: &EntryLimits,
    max_bytes: usize,
) {
    miss_on_empty(store, keys).await;
    publish_then_get_returns_the_bytes_and_facts(store, keys, limits).await;
    an_equal_or_older_fence_is_fenced_and_leaves_the_entry(store, keys).await;
    a_newer_fence_replaces(store, keys).await;
    evict_removes(store, keys).await;
    two_keys_never_alias(store, keys).await;
    a_flipped_byte_and_a_truncated_frame_are_misses(store, keys, limits).await;
    a_well_formed_nested_graph_resolves_and_excess_depth_is_refused(store, keys, limits).await;
    a_composite_naming_an_unknown_segment_kind_is_refused_not_ignored(store, keys, limits).await;
    an_oversized_publish_is_rejected_and_stores_nothing(store, keys, max_bytes).await;
    inspect_counts_entries_and_bytes(store, keys).await;
}

/// A key that was never published is a miss, not a failure.
///
/// This scenario runs first, so its opening assertion is also where a store
/// that was not handed over empty is caught, and named as the caller's doing
/// rather than as a scenario the suite failed to clean up after.
async fn miss_on_empty(store: &dyn RenderStore, keys: &SnapshotKeyRing) {
    assert_eq!(
        inspect(store).await,
        StoreInspection {
            entries: 0,
            bytes: 0
        },
        "run_all was handed a store that already holds entries; \
         a store under conformance must be empty before the suite starts"
    );

    let key = key_for(keys, "miss-on-empty");

    assert!(
        get(store, &key).await.is_none(),
        "a key that was never published is a miss"
    );
}

/// A publication comes back byte for byte, with the instant and the fence it
/// was published under.
async fn publish_then_get_returns_the_bytes_and_facts(
    store: &dyn RenderStore,
    keys: &SnapshotKeyRing,
    limits: &EntryLimits,
) {
    let name = "round-trip";
    let key = key_for(keys, name);
    let bytes = encoded_entry(
        keys,
        name,
        b"<!doctype html><html><body>one</body></html>",
        CONFORMANCE_PUBLISHED_AT_MS,
    );

    assert_eq!(
        publish(
            store,
            &key,
            bytes.clone(),
            fence(2, 5),
            CONFORMANCE_PUBLISHED_AT_MS
        )
        .await,
        PublishOutcome::Published,
        "a key nothing holds accepts a publication under any fence"
    );

    let stored = hit(store, &key).await;
    assert!(
        stored.bytes == bytes,
        "a hit returns exactly the bytes that were published"
    );
    assert_eq!(
        stored.published_at_ms, CONFORMANCE_PUBLISHED_AT_MS,
        "a hit returns the instant it was published at"
    );
    assert_eq!(
        stored.fence,
        fence(2, 5),
        "a hit returns the whole fence it was published under"
    );

    let decoded = decode(&stored.bytes, keys, limits).expect("a round-tripped entry still decodes");
    assert_eq!(
        decoded.kind(),
        EntryKind::Complete,
        "a round-tripped entry decodes as the kind it was encoded as"
    );
    assert!(
        decoded.header().key == key,
        "and names the key it was stored under"
    );

    evict(store, &key).await;
}

/// An equal or older fence is refused, and leaves every part of the stored
/// entry alone.
async fn an_equal_or_older_fence_is_fenced_and_leaves_the_entry(
    store: &dyn RenderStore,
    keys: &SnapshotKeyRing,
) {
    let name = "fenced";
    let key = key_for(keys, name);
    let held = encoded_entry(
        keys,
        name,
        b"<!doctype html><html><body>held</body></html>",
        CONFORMANCE_PUBLISHED_AT_MS,
    );
    let refused = encoded_entry(
        keys,
        name,
        b"<!doctype html><html><body>refused</body></html>",
        CONFORMANCE_PUBLISHED_AT_MS + CONFORMANCE_REPUBLISH_AFTER_MS,
    );

    assert_eq!(
        publish(
            store,
            &key,
            held.clone(),
            fence(2, 5),
            CONFORMANCE_PUBLISHED_AT_MS
        )
        .await,
        PublishOutcome::Published,
        "the entry the fence rule is proven against is stored first"
    );

    for (contract, sent) in [
        (
            "an equal fence never replaces the entry it equals",
            fence(2, 5),
        ),
        ("a lower token in the same epoch is fenced", fence(2, 4)),
        ("a lower epoch is fenced whatever its token", fence(1, 9)),
    ] {
        assert_eq!(
            publish(
                store,
                &key,
                refused.clone(),
                sent,
                CONFORMANCE_PUBLISHED_AT_MS + CONFORMANCE_REPUBLISH_AFTER_MS,
            )
            .await,
            PublishOutcome::Fenced,
            "{contract}"
        );

        let stored = hit(store, &key).await;
        assert!(
            stored.bytes == held,
            "{contract}, and leaves the stored bytes untouched"
        );
        assert_eq!(
            stored.published_at_ms, CONFORMANCE_PUBLISHED_AT_MS,
            "{contract}, and leaves the stored instant untouched"
        );
        assert_eq!(
            stored.fence,
            fence(2, 5),
            "{contract}, and leaves the stored fence untouched"
        );
    }

    evict(store, &key).await;
}

/// A newer fence replaces the entry, by token within an epoch and by epoch
/// regardless of token.
async fn a_newer_fence_replaces(store: &dyn RenderStore, keys: &SnapshotKeyRing) {
    let name = "replaced";
    let key = key_for(keys, name);
    let later = CONFORMANCE_PUBLISHED_AT_MS + CONFORMANCE_REPUBLISH_AFTER_MS;
    let latest = later + CONFORMANCE_REPUBLISH_AFTER_MS;
    let first = encoded_entry(
        keys,
        name,
        b"<!doctype html><html><body>first</body></html>",
        CONFORMANCE_PUBLISHED_AT_MS,
    );
    let second = encoded_entry(
        keys,
        name,
        b"<!doctype html><html><body>second</body></html>",
        later,
    );
    let third = encoded_entry(
        keys,
        name,
        b"<!doctype html><html><body>third</body></html>",
        latest,
    );

    publish(store, &key, first, fence(2, 5), CONFORMANCE_PUBLISHED_AT_MS).await;

    assert_eq!(
        publish(store, &key, second.clone(), fence(2, 6), later).await,
        PublishOutcome::Published,
        "a higher token in the same epoch replaces the entry"
    );
    let stored = hit(store, &key).await;
    assert!(
        stored.bytes == second,
        "and the stored bytes are the new ones"
    );
    assert_eq!(
        stored.published_at_ms, later,
        "and the stored instant is the new one"
    );
    assert_eq!(
        stored.fence,
        fence(2, 6),
        "and the stored fence is the new one"
    );

    assert_eq!(
        publish(store, &key, third.clone(), fence(3, 0), latest).await,
        PublishOutcome::Published,
        "a higher epoch replaces the entry however low its token"
    );
    let stored = hit(store, &key).await;
    assert!(
        stored.bytes == third,
        "and the stored bytes are the newest ones"
    );
    assert_eq!(
        stored.fence,
        fence(3, 0),
        "and the stored fence is the newest"
    );

    evict(store, &key).await;
}

/// An evicted key is a miss again, and evicting it a second time is not a
/// failure.
async fn evict_removes(store: &dyn RenderStore, keys: &SnapshotKeyRing) {
    let name = "evicted";
    let key = key_for(keys, name);
    let bytes = encoded_entry(
        keys,
        name,
        b"<!doctype html><html><body>gone</body></html>",
        CONFORMANCE_PUBLISHED_AT_MS,
    );

    publish(store, &key, bytes, fence(1, 1), CONFORMANCE_PUBLISHED_AT_MS).await;
    assert!(
        get(store, &key).await.is_some(),
        "the entry eviction is proven against is stored first"
    );

    evict(store, &key).await;
    assert!(
        get(store, &key).await.is_none(),
        "an evicted key is a miss again"
    );

    evict(store, &key).await;
    assert!(
        get(store, &key).await.is_none(),
        "and evicting a key that is already gone is not a failure"
    );
}

/// Two keys are two entries: neither read, replacement, nor eviction of one
/// reaches the other.
async fn two_keys_never_alias(store: &dyn RenderStore, keys: &SnapshotKeyRing) {
    let (left_name, right_name) = ("alias-left", "alias-right");
    let (left, right) = (key_for(keys, left_name), key_for(keys, right_name));
    let left_bytes = encoded_entry(
        keys,
        left_name,
        b"<!doctype html><html><body>left</body></html>",
        CONFORMANCE_PUBLISHED_AT_MS,
    );
    let right_bytes = encoded_entry(
        keys,
        right_name,
        b"<!doctype html><html><body>right</body></html>",
        CONFORMANCE_PUBLISHED_AT_MS,
    );

    publish(
        store,
        &left,
        left_bytes.clone(),
        fence(1, 1),
        CONFORMANCE_PUBLISHED_AT_MS,
    )
    .await;
    publish(
        store,
        &right,
        right_bytes.clone(),
        fence(4, 9),
        CONFORMANCE_PUBLISHED_AT_MS,
    )
    .await;

    let left_stored = hit(store, &left).await;
    let right_stored = hit(store, &right).await;
    assert!(
        left_stored.bytes == left_bytes && right_stored.bytes == right_bytes,
        "each key returns its own bytes"
    );
    assert_eq!(
        (left_stored.fence, right_stored.fence),
        (fence(1, 1), fence(4, 9)),
        "and its own fence, so one key's fence never gates another's publication"
    );

    evict(store, &left).await;
    assert!(
        get(store, &left).await.is_none(),
        "evicting one key removes it"
    );
    let survivor = hit(store, &right).await;
    assert!(
        survivor.bytes == right_bytes,
        "and leaves the other key exactly as it was"
    );

    evict(store, &right).await;
}

/// A store returns the bytes it was given; a caller decides whether they are
/// an entry. Bytes that were tampered with therefore reach the caller and
/// fail to decode, rather than being served as a representation.
///
/// Three tampered frames, because one flipped byte is not one check. A byte
/// flipped inside the frame is refused by whichever of the codec's layers
/// reaches it first - the integrity tag, the canonical header bounds, the
/// header's own typed rebuild, or the body digest - and which one that is
/// depends on where the byte fell. A byte flipped in the *trailing integrity
/// tag* leaves every one of those layers satisfied, so only the tag
/// comparison can refuse it, and a frame cut in half leaves the tag missing
/// altogether. Between them the three pin the outcome without depending on
/// which layer answers.
async fn a_flipped_byte_and_a_truncated_frame_are_misses(
    store: &dyn RenderStore,
    keys: &SnapshotKeyRing,
    limits: &EntryLimits,
) {
    let name = "corrupted";
    let key = key_for(keys, name);
    let valid = encoded_entry(
        keys,
        name,
        b"<!doctype html><html><body>intact</body></html>",
        CONFORMANCE_PUBLISHED_AT_MS,
    );

    publish(
        store,
        &key,
        valid.clone(),
        fence(1, 1),
        CONFORMANCE_PUBLISHED_AT_MS,
    )
    .await;
    let intact = hit(store, &key).await;
    decode(&intact.bytes, keys, limits).expect(
        "the entry the corruption scenario tampers with decodes before it is tampered with",
    );

    let mut flipped = valid.to_vec();
    let middle = flipped.len() / 2;
    flipped[middle] ^= 0b0000_0001;
    let mut flipped_tag = valid.to_vec();
    let last = flipped_tag.len() - 1;
    flipped_tag[last] ^= 0b0000_0001;
    let truncated = valid.slice(..valid.len() / 2);

    for (contract, corrupted, sent) in [
        (
            "an entry with one flipped byte inside the frame is refused",
            Bytes::from(flipped),
            fence(1, 2),
        ),
        (
            "an entry whose integrity tag alone was altered fails that check",
            Bytes::from(flipped_tag),
            fence(1, 3),
        ),
        (
            "a truncated frame fails its structural check",
            truncated,
            fence(1, 4),
        ),
    ] {
        assert_eq!(
            publish(
                store,
                &key,
                corrupted,
                sent,
                CONFORMANCE_PUBLISHED_AT_MS + CONFORMANCE_REPUBLISH_AFTER_MS,
            )
            .await,
            PublishOutcome::Published,
            "{contract}, so a store stores what it is given rather than validating it"
        );

        let stored = hit(store, &key).await;
        // The kind alone, never the error or the decoded entry: a failed
        // assertion here must name the contract, not print the frame the
        // scenario just tampered with.
        let refused = decode(&stored.bytes, keys, limits)
            .err()
            .map(|error| error.kind());
        assert_eq!(
            refused,
            Some(RenderCacheErrorKind::EntryInvalid),
            "{contract}, and is refused as an invalid entry rather than by any other outcome"
        );
    }

    evict(store, &key).await;
}

/// A well-formed nested segment graph survives the store and resolves; the
/// identical decoded entry, resolved as though it sat two ownership levels
/// deeper than it was published at, is refused by the closed depth cause
/// rather than assembled anyway.
///
/// The store's own contract is indifferent to Complete versus Composite: it
/// hands back whatever bytes it was given, exactly as
/// [`a_flipped_byte_and_a_truncated_frame_are_misses`] proves for a
/// corrupted frame. This proves that indifference holds for a nested graph
/// specifically, and resolves the accept and the depth-refusal from the
/// same two stored, fetched-back entries, so the refusal below is shown
/// refusing a graph that otherwise resolves, not merely failing to parse
/// anything.
async fn a_well_formed_nested_graph_resolves_and_excess_depth_is_refused(
    store: &dyn RenderStore,
    keys: &SnapshotKeyRing,
    limits: &EntryLimits,
) {
    let leaf_name = "nested-leaf";
    let leaf_key = key_for(keys, leaf_name);
    let leaf_body = Bytes::from_static(b"<p>leaf</p>");
    let leaf = composite_entry_for(
        keys,
        leaf_name,
        SegmentGraph {
            segments: vec![Segment::Literal {
                len: leaf_body.len() as u32,
            }],
            slots: Vec::new(),
            shell_islands: Vec::new(),
            nonce_headers: Vec::new(),
        },
        leaf_body.clone(),
    );
    publish(
        store,
        &leaf_key,
        encode_composite(&leaf, keys).expect("the leaf composite entry encodes"),
        fence(1, 1),
        CONFORMANCE_PUBLISHED_AT_MS,
    )
    .await;
    let leaf_stored = hit(store, &leaf_key).await;
    let leaf_decoded = match decode(&leaf_stored.bytes, keys, limits).expect("the leaf decodes") {
        DecodedEntry::Composite(entry) => entry,
        DecodedEntry::Complete(_) => panic!("a Composite entry decoded as Complete"),
    };
    assert!(
        leaf_decoded.shell() == &leaf_body,
        "the leaf decodes back to exactly the shell it was published with"
    );

    let outer_name = "nested-outer";
    let outer_key = key_for(keys, outer_name);
    let outer_version = 1;
    let outer = composite_entry_for(
        keys,
        outer_name,
        SegmentGraph {
            segments: vec![Segment::Nested {
                key: leaf_key.clone(),
                version: outer_version,
                assembled_len: leaf_body.len() as u32,
                on_failure: SlotFailurePolicy::Omit,
            }],
            slots: Vec::new(),
            shell_islands: Vec::new(),
            nonce_headers: Vec::new(),
        },
        Bytes::new(),
    );
    publish(
        store,
        &outer_key,
        encode_composite(&outer, keys).expect("the outer composite entry encodes"),
        fence(1, 1),
        CONFORMANCE_PUBLISHED_AT_MS,
    )
    .await;
    let outer_stored = hit(store, &outer_key).await;
    let outer_decoded = match decode(&outer_stored.bytes, keys, limits).expect("the outer decodes")
    {
        DecodedEntry::Composite(entry) => entry,
        DecodedEntry::Complete(_) => panic!("a Composite entry decoded as Complete"),
    };

    // Accepted: resolved at the depth it was actually published at (the
    // outer entry names no ancestors of its own), proving a well-formed
    // depth-2 graph assembles rather than every graph merely failing to
    // parse.
    let assembled = assemble_nested(
        &outer_decoded,
        AssemblyInput {
            outcomes: Vec::new(),
            nonce: None,
        },
        vec![NestedOutcome::Resolved {
            version: outer_version,
            body: leaf_body.clone(),
        }],
        &[],
        1 << 20,
    )
    .expect("a well-formed depth-2 graph, fetched back from the store, assembles");
    assert!(
        assembled.body() == &leaf_body,
        "the assembled body is exactly the resolved inner entry's bytes"
    );

    // Refused: the identical decoded entry, resolved as though two more
    // ownership levels already sat above it, so its own single
    // `Segment::Nested` would make a fourth level -- one past
    // `MAX_NESTING_DEPTH`.
    let ancestor_a = key_for(keys, "nested-ancestor-a");
    let ancestor_b = key_for(keys, "nested-ancestor-b");
    let refused = assemble_nested(
        &outer_decoded,
        AssemblyInput {
            outcomes: Vec::new(),
            nonce: None,
        },
        vec![NestedOutcome::Resolved {
            version: outer_version,
            body: leaf_body.clone(),
        }],
        &[ancestor_a.clone(), ancestor_b.clone()],
        1 << 20,
    )
    .err()
    .map(|error| error.kind());
    assert_eq!(
        refused,
        Some(RenderCacheErrorKind::AssemblyFailed),
        "a fourth ownership level is refused rather than assembled"
    );
    assert_eq!(
        descend_nested(&[ancestor_a, ancestor_b, outer_key.clone()], &leaf_key),
        Err(NestedFailureCause::DepthExceeded),
        "the precise closed cause is the depth bound, never a cycle or a mismatch"
    );

    evict(store, &outer_key).await;
    evict(store, &leaf_key).await;
}

/// A stored Composite entry naming a segment kind this build does not
/// recognize is refused whole by [`decode`], never served half-understood.
///
/// This is the forward-compatibility case: a future build may add a
/// segment kind this one has never heard of, and the bytes such a build
/// writes are exactly as valid, and exactly as opaque to a provider, as any
/// other Composite entry. A build meeting that entry has no way to tell
/// which part of an unknown segment is safe to skip and which changes the
/// meaning of what surrounds it, so the only closed answer is to refuse the
/// whole entry rather than assemble around the part it understands -- a
/// half-understood cache entry would serve a wrong document, never no
/// document. Mirrors
/// [`a_flipped_byte_and_a_truncated_frame_are_misses`]'s rhythm (an intact
/// positive control, then the same key republished mutated) for a defect
/// that is well-formed and correctly signed rather than corrupted.
async fn a_composite_naming_an_unknown_segment_kind_is_refused_not_ignored(
    store: &dyn RenderStore,
    keys: &SnapshotKeyRing,
    limits: &EntryLimits,
) {
    let name = "nested-unknown-kind";
    let key = key_for(keys, name);
    let inner_key = key_for(keys, "nested-unknown-kind-inner");
    let entry = composite_entry_for(
        keys,
        name,
        SegmentGraph {
            segments: vec![Segment::Nested {
                key: inner_key,
                version: 1,
                assembled_len: 0,
                on_failure: SlotFailurePolicy::Omit,
            }],
            slots: Vec::new(),
            shell_islands: Vec::new(),
            nonce_headers: Vec::new(),
        },
        Bytes::new(),
    );
    let header_json = serde_json::to_value(CompositeHeader {
        entry: entry.header().clone(),
        graph: entry.graph().clone(),
    })
    .expect("the composite header serializes");

    let intact = encode_raw_header_for_test_with_kind(
        &header_json,
        entry.shell(),
        keys,
        EntryKind::Composite,
    );
    assert_eq!(
        publish(
            store,
            &key,
            intact,
            fence(1, 1),
            CONFORMANCE_PUBLISHED_AT_MS
        )
        .await,
        PublishOutcome::Published,
        "the intact entry publishes"
    );
    let stored = hit(store, &key).await;
    decode(&stored.bytes, keys, limits).expect(
        "the entry the forward-compatibility scenario mutates decodes before it is mutated",
    );

    let mut mutated = header_json;
    mutated["graph"]["segments"][0]["kind"] = serde_json::json!("future_segment");
    let forward_incompatible =
        encode_raw_header_for_test_with_kind(&mutated, entry.shell(), keys, EntryKind::Composite);

    assert_eq!(
        publish(
            store,
            &key,
            forward_incompatible,
            fence(1, 2),
            CONFORMANCE_PUBLISHED_AT_MS + CONFORMANCE_REPUBLISH_AFTER_MS,
        )
        .await,
        PublishOutcome::Published,
        "a store stores what it is given rather than validating it, exactly as for corrupted bytes"
    );
    let stored = hit(store, &key).await;
    let refused = decode(&stored.bytes, keys, limits)
        .err()
        .map(|error| error.kind());
    assert_eq!(
        refused,
        Some(RenderCacheErrorKind::EntryInvalid),
        "an unrecognized segment kind is refused as an invalid entry, never partially trusted"
    );

    evict(store, &key).await;
}

/// Bytes past the configured bound are refused, and refused before anything
/// is stored.
async fn an_oversized_publish_is_rejected_and_stores_nothing(
    store: &dyn RenderStore,
    keys: &SnapshotKeyRing,
    max_bytes: usize,
) {
    let key = key_for(keys, "oversized");
    let before = inspect(store).await;
    let oversized = Bytes::from(vec![0x5a_u8; max_bytes + 1]);

    assert_eq!(
        publish(
            store,
            &key,
            oversized,
            fence(1, 1),
            CONFORMANCE_PUBLISHED_AT_MS
        )
        .await,
        PublishOutcome::Rejected,
        "one byte past the configured bound is rejected"
    );
    assert!(
        get(store, &key).await.is_none(),
        "a rejected publication leaves its key a miss"
    );
    assert_eq!(
        inspect(store).await,
        before,
        "and changes no part of the store's occupancy"
    );
}

/// Occupancy counts the entries held and the bytes they occupy, and follows
/// both publication and eviction.
///
/// This scenario runs last, so its opening assertion is also the proof that
/// every scenario before it removed what it published.
async fn inspect_counts_entries_and_bytes(store: &dyn RenderStore, keys: &SnapshotKeyRing) {
    assert_eq!(
        inspect(store).await,
        StoreInspection {
            entries: 0,
            bytes: 0
        },
        "a store every published entry has been evicted from holds nothing"
    );

    let (first_name, second_name) = ("occupancy-first", "occupancy-second");
    let (first, second) = (key_for(keys, first_name), key_for(keys, second_name));
    let first_bytes = encoded_entry(
        keys,
        first_name,
        b"<!doctype html><html><body>counted once</body></html>",
        CONFORMANCE_PUBLISHED_AT_MS,
    );
    let second_bytes = encoded_entry(
        keys,
        second_name,
        b"<!doctype html><html><body>counted twice, and longer than the first</body></html>",
        CONFORMANCE_PUBLISHED_AT_MS,
    );

    publish(
        store,
        &first,
        first_bytes.clone(),
        fence(1, 1),
        CONFORMANCE_PUBLISHED_AT_MS,
    )
    .await;
    publish(
        store,
        &second,
        second_bytes.clone(),
        fence(1, 1),
        CONFORMANCE_PUBLISHED_AT_MS,
    )
    .await;
    assert_eq!(
        inspect(store).await,
        StoreInspection {
            entries: 2,
            bytes: first_bytes.len() + second_bytes.len()
        },
        "occupancy counts every entry held and the exact bytes they occupy"
    );

    evict(store, &first).await;
    assert_eq!(
        inspect(store).await,
        StoreInspection {
            entries: 1,
            bytes: second_bytes.len()
        },
        "and an eviction removes exactly the entry it evicted from the count"
    );

    evict(store, &second).await;
    assert_eq!(
        inspect(store).await,
        StoreInspection {
            entries: 0,
            bytes: 0
        },
        "and the store is empty once the last entry is evicted"
    );
}

/// The lookup key one scenario's fixture is stored under.
fn key_for(keys: &SnapshotKeyRing, name: &str) -> RenderKey {
    RenderKey::for_test(keys, &format!("{NAMESPACE}/{name}"))
}

/// A publication fence, with a digest that varies with the token so a
/// partially replaced entry is visibly partial rather than plausible.
fn fence(epoch: u64, token: u64) -> PublicationFence {
    let mut generation_digest = [0_u8; 32];
    generation_digest[0] = u8::try_from(token % 251).unwrap_or(0);
    PublicationFence {
        epoch,
        generation_digest,
        token,
    }
}

/// A real, signed Complete entry for one scenario's key: the bytes a
/// publication actually carries, so a round trip is proven over an entry
/// rather than over a string a codec would never produce.
///
/// `published_at_ms` is the instant the caller goes on to publish these
/// bytes under, so an entry's own header never disagrees with the instant
/// the store records for it.
fn encoded_entry(
    keys: &SnapshotKeyRing,
    name: &str,
    body: &'static [u8],
    published_at_ms: u64,
) -> Bytes {
    let entry = CompleteEntry::new(
        EntryHeader {
            key: key_for(keys, name),
            class: RepresentationClass::PublicShared,
            variance: VarianceDescriptor::new(),
            published_at_ms,
            fresh_ms: 60_000,
            stale_servable_ms: 0,
            stale_on_error_ms: 0,
            observed: GenerationSet::default(),
            epoch: 1,
            seed_deadline_ms: None,
            status: 200,
            headers: SafeHeaders::from_pairs([("content-type", "text/html; charset=utf-8")])
                .expect("the conformance headers are replayable"),
            content_encoding: None,
        },
        Bytes::from_static(body),
    );
    encode(&entry, keys).expect("the conformance entry encodes")
}

/// A signed Composite entry for one scenario's key, built straight from
/// `graph` and `shell` rather than through the typed authoring surface: the
/// nested-segment scenarios prove the store and the codec, not composition.
fn composite_entry_for(
    keys: &SnapshotKeyRing,
    name: &str,
    graph: SegmentGraph,
    shell: Bytes,
) -> CompositeEntry {
    CompositeEntry::new(
        EntryHeader {
            key: key_for(keys, name),
            class: RepresentationClass::PublicShellStitched,
            variance: VarianceDescriptor::new(),
            published_at_ms: CONFORMANCE_PUBLISHED_AT_MS,
            fresh_ms: 60_000,
            stale_servable_ms: 0,
            stale_on_error_ms: 0,
            observed: GenerationSet::default(),
            epoch: 1,
            seed_deadline_ms: None,
            status: 200,
            headers: SafeHeaders::from_pairs([("content-type", "text/html; charset=utf-8")])
                .expect("the conformance headers are replayable"),
            content_encoding: None,
        },
        graph,
        shell,
    )
    .expect("the conformance composite entry constructs")
}

/// Reads a key, naming the operation rather than the key when a provider
/// fails.
async fn get(store: &dyn RenderStore, key: &RenderKey) -> Option<StoredEntry> {
    store.get(key).await.expect("a store answers get")
}

/// Reads a key that must be a hit.
async fn hit(store: &dyn RenderStore, key: &RenderKey) -> StoredEntry {
    get(store, key)
        .await
        .expect("a key published in this scenario is a hit")
}

/// Publishes with the suite's retention, naming the operation rather than
/// the entry when a provider fails.
async fn publish(
    store: &dyn RenderStore,
    key: &RenderKey,
    bytes: Bytes,
    fence: PublicationFence,
    now_ms: u64,
) -> PublishOutcome {
    store
        .publish(key, bytes, fence, now_ms, CONFORMANCE_RETENTION_MS)
        .await
        .expect("a store answers publish")
}

/// Removes a key.
async fn evict(store: &dyn RenderStore, key: &RenderKey) {
    store.evict(key).await.expect("a store answers evict");
}

/// Reads occupancy.
async fn inspect(store: &dyn RenderStore) -> StoreInspection {
    store.inspect().await.expect("a store answers inspect")
}

#[cfg(test)]
mod tests {
    use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
    use suprnova_live::identity::{KeyId, UnixMillis};
    use suprnova_live::render_cache::entry::EntryLimits;
    use suprnova_live::render_cache::store::{MemoryRenderStore, MemoryStoreLimits};

    use super::run_all;

    /// The maximum any scenario in this suite, including the nested and
    /// forward-compatibility scenarios, publishes under.
    const MAX_BYTES: usize = 1 << 20;

    fn keys() -> SnapshotKeyRing {
        let active = KeyRecord::new(
            KeyId::parse("render-store-embedded").expect("key id"),
            RootKey::new(vec![11; 32]).expect("root key"),
            UnixMillis::new(0),
            UnixMillis::new(u64::MAX / 2),
            UnixMillis::new(u64::MAX),
        )
        .expect("key record");
        SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
    }

    #[tokio::test]
    async fn the_suite_passes_over_the_embedded_provider() {
        let store = MemoryRenderStore::new(MemoryStoreLimits {
            max_entries: 32,
            max_bytes: MAX_BYTES,
        });

        run_all(&store, &keys(), &EntryLimits::default(), MAX_BYTES).await;
    }
}
