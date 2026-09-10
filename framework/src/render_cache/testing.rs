//! Test seams; hidden from documentation and never used by application code.

use super::registry::RenderCachePolicyTable;

/// The router's registered RenderCache policy table.
#[must_use]
pub fn policy_table(router: &crate::Router) -> RenderCachePolicyTable {
    router.render_cache_policies().clone()
}

/// Rewrites the Composite entry stored for `pattern` in place, so a test can
/// put the store into a state only a redeploy could otherwise produce.
///
/// The stored entry is decoded under the runtime's own key ring, its
/// [`SegmentGraph`](suprnova_live::render_cache::composite::SegmentGraph) is
/// handed to `edit`, every slot's surrounding digest is
/// recomputed from the edited graph and the unchanged shell, and the result
/// is re-encoded and published back under the same key with a fence one
/// token above the one it was found under. The next request for `pattern`
/// therefore sees the rewritten entry exactly as if the running build had
/// published it.
///
/// This is how a test reaches the drift cases that cannot be staged from
/// outside: a stored slot naming a component or contract digest the current
/// registry no longer has is a redeploy, and a redeploy cannot happen inside
/// one process. Nothing else about the entry moves - the header, the class,
/// the observed generation set, and the publication instant are all the ones
/// the real publisher wrote - so freshness and coherence decide exactly what
/// they decided before.
///
/// An `edit` that changes nothing is a legitimate use: the closure sees the
/// stored graph, which is the only way a test outside this crate can observe
/// how many nonce holes or header templates the publisher actually cut.
///
/// # Panics
///
/// Panics when no runtime is installed, `pattern` has no effective policy,
/// the key cannot be derived, no entry is stored for it, the stored entry is
/// not a Composite one, or the edited graph cannot be re-encoded and
/// published. Every one of those is a broken test setup, not a condition a
/// caller could handle.
#[cfg(any(test, feature = "testing"))]
pub async fn rewrite_composite_for_test<F>(pattern: &str, edit: F)
where
    F: FnOnce(&mut suprnova_live::render_cache::composite::SegmentGraph),
{
    use suprnova_live::render_cache::composite::{CompositeEntry, surrounding_digest};
    use suprnova_live::render_cache::entry::{DecodedEntry, decode, encode_composite};
    use suprnova_live::render_cache::key::RenderKey;
    use suprnova_live::render_cache::store::{PublishOutcome, RenderStore as _};

    let runtime = super::RenderCache::runtime().expect("RenderCache installed");
    let policy = runtime
        .table
        .effective_policy(pattern)
        .expect("an effective policy for the rewritten route");
    let input = super::middleware::key_input_for_test(&runtime, pattern, &[], None, &policy);
    let key = RenderKey::derive(&input, &runtime.keys).expect("derive the stored entry's key");
    let stored = runtime
        .l0
        .get(&key)
        .await
        .expect("read the stored entry")
        .expect("an entry is stored for the rewritten route");
    let decoded = decode(&stored.bytes, &runtime.keys, &runtime.limits).expect("decode the entry");
    let DecodedEntry::Composite(entry) = decoded else {
        panic!("the stored entry is not a Composite one");
    };
    let mut graph = entry.graph().clone();
    edit(&mut graph);
    let shell = entry.shell().clone();
    // Recomputed rather than carried over: the assembler recomputes these
    // from the graph and the shell on every hit, so an edit that moved a
    // slot would otherwise leave the entry rejecting itself for a reason
    // the test never asked for.
    for index in 0..graph.slots.len() {
        graph.slots[index].surrounding =
            surrounding_digest(&graph, &shell, index).expect("recompute a surrounding digest");
    }
    let rewritten = CompositeEntry::new(entry.header().clone(), graph, shell)
        .expect("the edited graph is a valid Composite entry");
    let bytes = encode_composite(&rewritten, &runtime.keys).expect("encode the edited entry");
    let mut fence = stored.fence;
    fence.token = fence
        .token
        .checked_add(1)
        .expect("a fresh publication token");
    let outcome = runtime
        .l0
        .publish(&key, bytes, fence, stored.published_at_ms, u64::MAX)
        .await
        .expect("publish the edited entry");
    assert_eq!(
        outcome,
        PublishOutcome::Published,
        "the rewritten entry must replace the one it was read from"
    );
}

/// Applies `edit` to a *copy* of the graph and header already stored for
/// `pattern`, without publishing anything, and runs the same publish-time
/// nested-composition check `build_composite_entry` runs before a real
/// composite is ever stored (`stitch::refuse_unsafe_nesting`). `None` means
/// the check would have allowed it; `Some(reason)` is the closed decline
/// reason label ([`super::decline::LookupDeclineReason::as_str`]) the real
/// publish path would have declined with.
///
/// Nothing in the current capture path can produce a [`Segment::Nested`]
/// from a real render - there is no typed authoring surface for one yet -
/// so this is how a test drives the check with a hand-built graph instead:
/// the same shape [`rewrite_composite_for_test`] uses to reach a hit-time
/// case a redeploy alone could otherwise produce, applied here to a
/// publish-time one nothing can produce yet at all.
///
/// # Panics
///
/// Panics on the same broken-test-setup conditions
/// [`rewrite_composite_for_test`] does.
#[cfg(any(test, feature = "testing"))]
pub async fn nested_publish_check_for_test<F>(pattern: &str, edit: F) -> Option<&'static str>
where
    F: FnOnce(&mut suprnova_live::render_cache::composite::SegmentGraph),
{
    use suprnova_live::render_cache::entry::{DecodedEntry, decode};
    use suprnova_live::render_cache::key::RenderKey;
    use suprnova_live::render_cache::store::RenderStore as _;

    let runtime = super::RenderCache::runtime().expect("RenderCache installed");
    let policy = runtime
        .table
        .effective_policy(pattern)
        .expect("an effective policy for the checked route");
    let input = super::middleware::key_input_for_test(&runtime, pattern, &[], None, &policy);
    let key = RenderKey::derive(&input, &runtime.keys).expect("derive the stored entry's key");
    let stored = runtime
        .l0
        .get(&key)
        .await
        .expect("read the stored entry")
        .expect("an entry is stored for the checked route");
    let decoded = decode(&stored.bytes, &runtime.keys, &runtime.limits).expect("decode the entry");
    let DecodedEntry::Composite(entry) = decoded else {
        panic!("the stored entry is not a Composite one");
    };
    let header = entry.header().clone();
    let mut graph = entry.graph().clone();
    edit(&mut graph);
    match super::stitch::refuse_unsafe_nesting(&runtime, &header, &graph).await {
        Ok(()) => None,
        Err(error) => Some(super::stitch::composite_build_error_reason(error).as_str()),
    }
}

/// Publishes a bare Complete entry directly to L0, under a key derived from
/// `fixture_pattern` exactly as [`suprnova_live::render_cache::key::RenderKey::for_test`]
/// derives one - no route registration, no dispatch, and no policy needed -
/// with the given class and freshness window, and returns the key it was
/// published under.
///
/// This is the only way a test can name an inner entry whose class or
/// freshness genuinely differs from an entry a real dispatch through this
/// harness would ever produce (every route this harness registers is
/// `PublicShellStitched`): [`nested_publish_check_for_test`] and
/// [`refuse_unsafe_nesting`](super::stitch::refuse_unsafe_nesting) need a
/// *stored*, decodable entry to compare against, and forging one that
/// [`suprnova_live::render_cache::entry::decode`] will still accept needs
/// this runtime's own key ring, which an external test cannot reach any
/// other way.
///
/// # Panics
///
/// Panics on the same broken-test-setup conditions
/// [`rewrite_composite_for_test`] does, and if `class` and `fresh_ms`
/// cannot be encoded into a valid entry.
#[cfg(any(test, feature = "testing"))]
pub async fn publish_bare_entry_for_test(
    fixture_pattern: &str,
    class: suprnova_live::render_cache::RepresentationClass,
    fresh_ms: u64,
) -> suprnova_live::render_cache::key::RenderKey {
    use suprnova_live::render_cache::entry::{CompleteEntry, EntryHeader, SafeHeaders, encode};
    use suprnova_live::render_cache::generation::GenerationSet;
    use suprnova_live::render_cache::key::RenderKey;
    use suprnova_live::render_cache::store::{PublicationFence, PublishOutcome, RenderStore as _};
    use suprnova_live::render_cache::variance::VarianceDescriptor;

    let runtime = super::RenderCache::runtime().expect("RenderCache installed");
    let key = RenderKey::for_test(&runtime.keys, fixture_pattern);
    let header = EntryHeader {
        key: key.clone(),
        class,
        variance: VarianceDescriptor::new(),
        published_at_ms: 0,
        fresh_ms,
        stale_servable_ms: 0,
        stale_on_error_ms: 0,
        observed: GenerationSet::default(),
        epoch: 1,
        seed_deadline_ms: None,
        status: 200,
        headers: SafeHeaders::from_pairs(Vec::<(String, String)>::new())
            .expect("empty headers are always safe"),
        content_encoding: None,
    };
    let entry = CompleteEntry::new(header, bytes::Bytes::from_static(b"nested fixture"));
    let bytes = encode(&entry, &runtime.keys).expect("encode the fixture entry");
    let fence = PublicationFence {
        epoch: 1,
        generation_digest: [0; 32],
        token: 0,
    };
    let outcome = runtime
        .l0
        .publish(&key, bytes, fence, 0, u64::MAX)
        .await
        .expect("publish the fixture entry");
    assert_eq!(
        outcome,
        PublishOutcome::Published,
        "a fresh fixture key must always publish"
    );
    key
}
