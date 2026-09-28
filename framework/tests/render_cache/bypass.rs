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

/// A lease-mode hit runs none of the four and consults the database not at
/// all. Both halves of "not at all" are load-bearing: the validation lease
/// answers coherence without a ledger read, and the epoch the lookup key is
/// derived under is leased alongside it rather than read per request
/// (`00-overview.md:330` budgets a Complete L0 hit "with no
/// database/provider round trip"; spec 18, lines 112-131, says leases exist
/// "to avoid querying authority on every hot hit").
#[tokio::test]
#[serial_test::serial]
async fn a_lease_mode_hit_runs_nothing_and_issues_no_statement() {
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
        0,
        "a lease-mode hot hit reaches the database zero times: the lease answers coherence, \
         and the epoch the key is derived under is leased with it"
    );
}

/// An authority-mode hit issues exactly one statement: the batched reread
/// that reads the observed generations and the authority epoch together
/// (`GenerationLedger::current_with_epoch`, one `UNION ALL`). Read
/// separately those would be two round trips, and nothing else in this
/// suite would notice; this assertion is what holds them to one.
///
/// One, not two: the epoch this request's lookup key was derived under came
/// from the runtime's leased epoch, which that same reread renews - no
/// request reads the epoch on its own.
#[tokio::test]
#[serial_test::serial]
async fn an_authority_mode_hit_issues_exactly_one_statement() {
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
        1,
        "one batched coherence reread, and nothing else"
    );
}

/// The epoch costs one authority read per runtime, not one per request.
///
/// Measured as a difference rather than as an absolute, so it does not
/// depend on how many statements a probe render happens to issue: two
/// misses of the same shape on a fresh boot differ by exactly the one read
/// that fills the epoch lease, and the second miss - and every request
/// after it - pays nothing for the epoch at all.
///
/// `/probe/2` rather than a second request to `/probe/1`: the second
/// request has to be a miss too, and after the first publication only a
/// different key is one.
#[tokio::test]
#[serial_test::serial]
async fn the_epoch_is_read_once_at_first_use() {
    let harness = boot_with_render_cache().await;

    statements::reset();
    let first = dispatch_get(&harness, PROBE_PATH, &[]).await;
    assert_eq!(first.status, StatusCode::OK);
    let first_miss = statements::count();

    statements::reset();
    let second = dispatch_get(&harness, "/probe/2", &[]).await;
    assert_eq!(second.status, StatusCode::OK);
    let second_miss = statements::count();

    assert_eq!(
        probe_route::handler_calls(),
        2,
        "both dispatches were misses and rendered, so they did the same work"
    );
    assert!(
        second_miss >= 1,
        "control: the second miss still reached the database for its own render; observed {second_miss}"
    );
    assert_eq!(
        first_miss,
        second_miss + 1,
        "the first request of a runtime pays one statement more than an otherwise identical \
         second one: the single authority read that fills the epoch lease. first {first_miss}, \
         second {second_miss}"
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
