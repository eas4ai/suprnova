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
    POST_PROCESSED_PATH, SEED_ONLY_PATH, SHELL_READS_PRINCIPAL_PATH, STITCHED_PATH, boot,
    chain_reaches, dispatch, handler_renders,
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
    let reaches_before = chain_reaches(SEED_ONLY_PATH);
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
    assert_eq!(
        chain_reaches(SEED_ONLY_PATH),
        reaches_before + 1,
        "the chain ran and the Live completion middleware served the prepared hit"
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

/// The first render of a stitched document that mounted an identity-bound
/// island publishes a Composite entry: a shell with a hole where the island
/// was, plus the typed declaration a later hit re-mounts it from. The
/// leader still receives its own bytes, so its `ETag` is a strong validator
/// over exactly what it was sent.
#[tokio::test]
#[serial_test::serial]
async fn the_first_render_of_a_stitched_document_publishes_a_composite_entry() {
    let harness = boot().await;
    let first = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-1")],
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    let expected_etag = suprnova_live::render_cache::Validator::strong_for(&first.body).etag();
    assert_eq!(
        first.header("etag"),
        Some(expected_etag.as_str()),
        "the leader's ETag is over its own bytes"
    );
    let stored = RenderCache::inspect_route_for_test(STITCHED_PATH)
        .await
        .expect("stored");
    assert_eq!(stored.kind, EntryKind::Composite);
    assert_eq!(stored.slots, 1);
    assert_eq!(stored.class, RepresentationClass::PublicShellStitched);
    assert!(
        stored.body_bytes < first.body.len(),
        "the shell excludes the island bytes"
    );
    assert!(!String::from_utf8_lossy(&first.body).contains("user-1"));
}

/// A shell that reads the principal itself is not a shared shell: the bytes
/// outside every island already depend on who asked, so nothing is
/// published and every request renders again.
#[tokio::test]
#[serial_test::serial]
async fn a_shell_that_reads_the_principal_is_not_published() {
    let harness = boot().await;
    let first = dispatch(
        &harness,
        Method::GET,
        SHELL_READS_PRINCIPAL_PATH,
        &[("x-test-login", "user-1")],
    )
    .await;
    assert_eq!(first.status, StatusCode::OK);
    assert!(
        RenderCache::inspect_route_for_test(SHELL_READS_PRINCIPAL_PATH)
            .await
            .is_none(),
        "declined: a content read of the principal narrows the shell"
    );
    let before = handler_renders(SHELL_READS_PRINCIPAL_PATH);
    dispatch(
        &harness,
        Method::GET,
        SHELL_READS_PRINCIPAL_PATH,
        &[("x-test-login", "user-2")],
    )
    .await;
    assert_eq!(handler_renders(SHELL_READS_PRINCIPAL_PATH), before + 1);
}

/// Route middleware that rewrites the body after the Live document rendered
/// it runs again on every hit, so its output must never be baked into the
/// stored representation. The publisher catches it without knowing the
/// middleware exists: the document recorded a digest of the body it
/// rendered, and the response carries different bytes.
#[tokio::test]
#[serial_test::serial]
async fn a_post_processed_body_is_not_published() {
    let harness = boot().await;
    let first = dispatch(
        &harness,
        Method::GET,
        POST_PROCESSED_PATH,
        &[("x-test-login", "user-1")],
    )
    .await;
    assert_eq!(first.status, StatusCode::OK);
    assert!(first.text().ends_with("<!-- footer -->"));
    assert!(
        RenderCache::inspect_route_for_test(POST_PROCESSED_PATH)
            .await
            .is_none(),
        "declined: the body digest no longer matches the rendered document"
    );
}
