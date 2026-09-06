//! A stitched route is never answered by the global RenderCache middleware
//! on its own: on a hit the middleware attaches the decoded entry to the
//! request and calls the rest of the chain, so the route's own guard and
//! tenant middleware run on a hit exactly as they do on a miss, and the Live
//! completion middleware - the last middleware before the handler - is what
//! actually serves it.

use hyper::Method;
use suprnova::StatusCode;
use suprnova::render_cache::{RenderCache, RepresentationClass};
use suprnova_live::render_cache::entry::EntryKind;

use crate::render_cache_stitch_support::{
    SEED_ONLY_PATH, STITCHED_PATH, boot, chain_reaches, dispatch, handler_renders,
};

/// A stitched route whose document holds nothing principal-specific is still
/// a Complete representation: the class says the gate runs again on every
/// hit, not that every document has to be assembled. The hit is served from
/// the stored entry without the handler rendering again, and an anonymous
/// request to the same route is still refused by the route's own guard,
/// which is only possible if the guard ran before the entry was served.
#[tokio::test]
#[serial_test::serial]
async fn a_seed_only_document_under_the_stitched_class_stores_complete_and_still_runs_the_gate() {
    let harness = boot().await;
    let first = dispatch(
        &harness,
        Method::GET,
        SEED_ONLY_PATH,
        &[("x-test-login", "user-1")],
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    let stored = RenderCache::inspect_route_for_test(SEED_ONLY_PATH)
        .await
        .expect("stored");
    assert_eq!(stored.kind, EntryKind::Complete);
    assert_eq!(stored.class, RepresentationClass::PublicShellStitched);
    let before = handler_renders(SEED_ONLY_PATH);
    let hit = dispatch(
        &harness,
        Method::GET,
        SEED_ONLY_PATH,
        &[("x-test-login", "user-2")],
    )
    .await;
    assert_eq!(hit.status, StatusCode::OK);
    assert_eq!(
        handler_renders(SEED_ONLY_PATH),
        before,
        "served from the entry"
    );
    assert_eq!(hit.header("age"), Some("0"));
    assert!(
        hit.header("cache-control")
            .expect("cache-control")
            .starts_with("private")
    );
    let anonymous = dispatch(&harness, Method::GET, SEED_ONLY_PATH, &[]).await;
    assert_eq!(
        anonymous.status,
        StatusCode::UNAUTHORIZED,
        "the guard ran before the entry was served"
    );
}

/// The mechanism behind the guard still running: the cache middleware hands
/// the request on to the rest of the chain on a hit instead of answering it
/// where it stands.
#[tokio::test]
#[serial_test::serial]
async fn a_stitched_route_never_short_circuits_before_its_route_middleware() {
    let harness = boot().await;
    dispatch(
        &harness,
        Method::GET,
        SEED_ONLY_PATH,
        &[("x-test-login", "user-1")],
    )
    .await;
    assert!(
        RenderCache::inspect_route_for_test(SEED_ONLY_PATH)
            .await
            .is_some(),
        "the first request published the shell, so the second one is a hit"
    );
    let before = chain_reaches(SEED_ONLY_PATH);
    dispatch(
        &harness,
        Method::GET,
        SEED_ONLY_PATH,
        &[("x-test-login", "user-1")],
    )
    .await;
    assert_eq!(
        chain_reaches(SEED_ONLY_PATH),
        before + 1,
        "the route chain ran on the hit"
    );
}

/// Task 7 replaces this test with composite publication.
///
/// Until a stitched document with identity-bound islands can be published as
/// a Composite entry, it must not be published at all: `document_declines`
/// no longer declines a stitched route just because an identity-bound island
/// mounted, so without this guard the shell one principal saw - islands,
/// signed snapshots and all - would be stored as a Complete representation
/// and replayed to the next visitor.
#[tokio::test]
#[serial_test::serial]
async fn a_stitched_document_with_identity_bound_islands_is_never_stored_yet() {
    let harness = boot().await;
    let first = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-1")],
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert_eq!(handler_renders(STITCHED_PATH), 1);
    assert!(
        RenderCache::inspect_route_for_test(STITCHED_PATH)
            .await
            .is_none(),
        "a stitched document holding one principal's islands is never stored as a shared shell"
    );
    let second = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-2")],
    )
    .await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.text());
    assert_eq!(
        handler_renders(STITCHED_PATH),
        2,
        "nothing was stored, so every request renders fresh"
    );
    assert!(
        RenderCache::inspect_route_for_test(STITCHED_PATH)
            .await
            .is_none(),
        "still nothing stored after a second principal asked"
    );
}
