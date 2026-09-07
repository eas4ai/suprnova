//! Task 5: what a proven hit does *not* do.
//!
//! Every other test in this suite reads `counting_route::renders()` and
//! infers "a hit ran no handler" from it staying flat. That is one counter
//! over one mock handler, and it says nothing about the four other costs a
//! cache exists to remove: the ORM query the handler would have issued,
//! the template it would have rendered, the serializer it would have run,
//! and the copy of the body bytes on the way to the wire. This file
//! observes each of them directly, through the `probe_route` counters and
//! the database's own statement callback, and it observes the body by
//! address rather than by value.
//!
//! Ruling R72: gated on the `testing` feature, like `races.rs` and for the
//! same reason - `RenderCache::l0_body_ptr_for_test` and its siblings only
//! exist in the library under that feature, so a feature-matrix build with
//! default features off compiles this file to nothing instead of failing
//! against seams that are not there.
//!
//! Every test is `#[serial_test::serial]` and plain `#[tokio::test]`
//! (current-thread), matching `middleware.rs` and `races.rs`: the installed
//! runtime, the global middleware registry, and the statement counter are
//! all process-global, and `TestContainer::fake()` writes a thread-local a
//! multi-thread runtime could migrate away from between polls.
#![cfg(feature = "testing")]

use std::sync::{Arc, Mutex};

use crate::render_cache_middleware_support;
use render_cache_middleware_support::{
    FrameLog, boot_with_render_cache, dispatch_get, dispatch_get_recording, dispatch_head,
    probe_route, statements,
};
use suprnova::StatusCode;
use suprnova::render_cache::RenderCache;

/// The authority-coherence probe route, and the key its `id=1` render
/// publishes under.
const PROBE: &str = "/probe/{id}";
const PROBE_PATH: &str = "/probe/1";

/// The lease-coherence probe route: the same handler, the same policy
/// shape, `CoherenceMode::Lease` instead of the default `Authority`.
const LEASED_PROBE_PATH: &str = "/probe-leased/1";

/// The statements a request to a policy-covered route costs before it knows
/// whether it is a hit at all: `RenderCacheMiddleware::serve` reads the
/// authority epoch for every GET and HEAD, because the lookup key is
/// derived under it. It is not the hit's own cost, and it is the same on a
/// hit, a miss, and a bypass - so the two tests below subtract it and
/// measure only what each coherence mode adds.
const EPOCH_READ_PER_REQUEST: u64 = 1;

/// A miss runs everything a request would run without a cache at all: the
/// handler once, its ORM query once, the template once, the serializer
/// once, and at least the handler's own statement against the database.
/// This is the baseline the three tests below subtract from - without it,
/// "a hit ran nothing" is satisfied by a probe route whose counters never
/// move in the first place.
#[tokio::test]
#[serial_test::serial]
async fn a_miss_runs_the_handler_the_query_the_template_and_the_serializer_once() {
    let harness = boot_with_render_cache().await;
    statements::reset();

    let response = dispatch_get(&harness, PROBE_PATH, &[]).await;
    assert_eq!(response.status, StatusCode::OK);

    assert_eq!(probe_route::handler_calls(), 1, "the handler ran once");
    assert_eq!(probe_route::queries(), 1, "it issued its one ORM query");
    assert_eq!(probe_route::template_renders(), 1, "it rendered once");
    assert_eq!(probe_route::serializations(), 1, "it serialized once");
    assert!(
        statements::count() >= 1,
        "the handler's own query reached the database; observed {}",
        statements::count()
    );

    let key = RenderCache::key_for_route_for_test(PROBE, &[("id", "1")], None);
    assert!(
        RenderCache::l0_hot_for_test(&key),
        "precondition for every test below: the miss published a hot entry"
    );
}

/// A lease-mode hit runs none of the four, and consults the database only
/// for the epoch every request reads before it knows whether it is a hit at
/// all (see `RenderCacheMiddleware::serve`): the validation lease answers
/// coherence without a ledger read of its own.
#[tokio::test]
#[serial_test::serial]
async fn a_lease_mode_hit_runs_nothing_and_issues_no_statement_of_its_own() {
    let harness = boot_with_render_cache().await;

    dispatch_get(&harness, LEASED_PROBE_PATH, &[]).await;
    assert_eq!(
        probe_route::handler_calls(),
        1,
        "the first request rendered"
    );

    // The lease is granted by the first *hit*, not by publication, so the
    // request measured below has to be the second hit, not the first.
    dispatch_get(&harness, LEASED_PROBE_PATH, &[]).await;

    probe_route::reset();
    statements::reset();
    let hot_before = RenderCache::hot_serves_for_test();

    let hit = dispatch_get(&harness, LEASED_PROBE_PATH, &[]).await;
    assert_eq!(hit.status, StatusCode::OK);
    assert_eq!(
        RenderCache::hot_serves_for_test(),
        hot_before + 1,
        "the response came off the hot path, not out of a render"
    );
    assert_eq!(probe_route::handler_calls(), 0, "no handler ran");
    assert_eq!(probe_route::queries(), 0, "no ORM query ran");
    assert_eq!(probe_route::template_renders(), 0, "no template rendered");
    assert_eq!(probe_route::serializations(), 0, "nothing was serialized");
    assert_eq!(
        statements::count(),
        EPOCH_READ_PER_REQUEST,
        "the hit itself issued nothing; a valid lease answers coherence without consulting \
         the ledger at all"
    );
}

/// An authority-mode hit issues exactly one statement of its own: the
/// batched reread that reads the observed generations and the authority
/// epoch together (`GenerationLedger::current_with_epoch`, one
/// `UNION ALL`). Read separately those would be two round trips, and
/// nothing else in this suite would notice; this assertion is what holds
/// them to one.
///
/// Named "of its own" for the same reason the lease test above is: the
/// per-request epoch read is not part of the hit, and
/// [`EPOCH_READ_PER_REQUEST`] is what separates the two.
#[tokio::test]
#[serial_test::serial]
async fn an_authority_mode_hit_issues_exactly_one_statement_of_its_own() {
    let harness = boot_with_render_cache().await;

    dispatch_get(&harness, PROBE_PATH, &[]).await;
    assert_eq!(
        probe_route::handler_calls(),
        1,
        "the first request rendered"
    );

    // Without this the measurement below is vacuous: a candidate that
    // observed nothing rereads the epoch alone (`current_with_epoch`
    // returns early with no `IN` list to bind), which is also one
    // statement, and the batching this test exists for would go untested.
    let key = RenderCache::key_for_route_for_test(PROBE, &[("id", "1")], None);
    let published = RenderCache::inspect(&key)
        .await
        .expect("inspect")
        .expect("the first request published");
    assert!(
        published.observations >= 1,
        "the probe's ORM read is a dependency the reread has to carry, not an empty list"
    );

    probe_route::reset();
    statements::reset();
    let hot_before = RenderCache::hot_serves_for_test();

    let hit = dispatch_get(&harness, PROBE_PATH, &[]).await;
    assert_eq!(hit.status, StatusCode::OK);
    assert_eq!(
        RenderCache::hot_serves_for_test(),
        hot_before + 1,
        "the response came off the hot path, not out of a render"
    );
    assert_eq!(probe_route::handler_calls(), 0, "no handler ran");
    assert_eq!(probe_route::queries(), 0, "no ORM query ran");
    assert_eq!(probe_route::template_renders(), 0, "no template rendered");
    assert_eq!(probe_route::serializations(), 0, "nothing was serialized");
    assert_eq!(
        statements::count(),
        EPOCH_READ_PER_REQUEST + 1,
        "one batched coherence reread on top of the per-request epoch read, and nothing else"
    );
}

/// The bytes hyper writes to the socket *are* the bytes L0 holds. Proven
/// by address: the frame the server hands hyper carries the same pointer
/// and length as the stored hot body, and that body lies inside the encoded
/// frame L0 stores, so the whole hit path from store to wire copies the
/// body zero times.
///
/// Recorded on the server side deliberately. The client half of `dispatch`
/// reads the response back over a real TCP connection, so a body collected
/// there is a fresh allocation whatever the server wrote from, and
/// comparing its address would prove nothing either way.
#[tokio::test]
#[serial_test::serial]
async fn the_served_body_is_the_stored_bytes() {
    let harness = boot_with_render_cache().await;
    let key = RenderCache::key_for_route_for_test(PROBE, &[("id", "1")], None);

    let first = dispatch_get(&harness, PROBE_PATH, &[]).await;
    assert_eq!(first.status, StatusCode::OK);
    assert!(
        RenderCache::l0_hot_for_test(&key),
        "precondition: the miss published a hot entry"
    );

    let stored = RenderCache::l0_body_ptr_for_test(&key).expect("the hot entry holds a body");
    let frame = RenderCache::l0_frame_ptr_for_test(&key)
        .await
        .expect("L0 holds the encoded frame");
    assert!(
        stored.0 >= frame.0 && stored.0 + stored.1 <= frame.0 + frame.1,
        "ruling R10: the hot body lies inside the stored frame, so L0 holds those bytes once"
    );

    let frames: FrameLog = Arc::new(Mutex::new(Vec::new()));
    let hit = dispatch_get_recording(&harness, PROBE_PATH, &frames).await;
    assert_eq!(hit.status, StatusCode::OK);
    assert_eq!(hit.body.len(), stored.1, "the client read the whole body");

    let recorded = frames
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    assert_eq!(
        recorded.len(),
        1,
        "one data frame, not a chunked stream of copies"
    );
    assert_eq!(
        recorded[0], stored,
        "the frame hyper wrote is the stored buffer itself, at the same address and length"
    );
}

/// A `HEAD` hit and a `304` hit are hits like any other - both come off the
/// hot path, neither runs the handler, the query, the template, or the
/// serializer - and both carry no body at all.
#[tokio::test]
#[serial_test::serial]
async fn head_and_not_modified_hits_carry_no_body_and_change_no_counter() {
    let harness = boot_with_render_cache().await;

    let first = dispatch_get(&harness, PROBE_PATH, &[]).await;
    assert_eq!(first.status, StatusCode::OK);
    assert!(!first.body.is_empty(), "precondition: the miss had a body");
    let validator = first.header("etag").expect("a hit is validated").to_owned();

    probe_route::reset();
    let hot_before = RenderCache::hot_serves_for_test();

    let head = dispatch_head(&harness, PROBE_PATH).await;
    assert_eq!(head.status, StatusCode::OK);
    assert!(head.body.is_empty(), "a HEAD hit carries no body");

    let not_modified = dispatch_get(&harness, PROBE_PATH, &[("if-none-match", &validator)]).await;
    assert_eq!(not_modified.status, StatusCode::NOT_MODIFIED);
    assert!(
        not_modified.body.is_empty(),
        "a 304 hit carries no body either"
    );

    assert_eq!(
        RenderCache::hot_serves_for_test(),
        hot_before + 2,
        "both bodiless hits were served off the hot path"
    );
    assert_eq!(probe_route::handler_calls(), 0, "no handler ran");
    assert_eq!(probe_route::queries(), 0, "no ORM query ran");
    assert_eq!(probe_route::template_renders(), 0, "no template rendered");
    assert_eq!(probe_route::serializations(), 0, "nothing was serialized");
}
