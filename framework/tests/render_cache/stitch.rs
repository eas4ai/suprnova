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
    POST_PROCESSED_PATH, SEED_ONLY_NONCE_PATH, SEED_ONLY_PATH, SHELL_READS_PRINCIPAL_PATH,
    STITCHED_PATH, TestResponse, boot, boot_with_freshness, chain_reaches, clock, decoded_snapshot,
    dispatch, handler_renders, island_tag,
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

/// A stitched document with no identity-bound island at all is still a
/// Composite entry when its bootstrap stamped a nonce: the shell has holes
/// where the nonce was, so every hit mints a fresh one. Publishing it
/// Complete would freeze the first visitor's nonce into the stored body and
/// the stored `Content-Security-Policy` alike and replay both to everybody,
/// which is a nonce that proves nothing.
#[tokio::test]
#[serial_test::serial]
async fn a_zero_slot_document_with_a_nonce_publishes_a_composite_entry() {
    let harness = boot().await;
    let first = dispatch(
        &harness,
        Method::GET,
        SEED_ONLY_NONCE_PATH,
        &[("x-test-login", "user-1")],
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    let policy = first
        .header("content-security-policy")
        .expect("the document declared its own nonce header");
    let stored = RenderCache::inspect_route_for_test(SEED_ONLY_NONCE_PATH)
        .await
        .expect("stored");
    assert_eq!(stored.kind, EntryKind::Composite);
    assert_eq!(stored.slots, 0, "nothing principal-specific to re-mount");
    assert_eq!(stored.class, RepresentationClass::PublicShellStitched);
    assert!(
        stored.body_bytes < first.body.len(),
        "the shell excludes the nonce this render used"
    );
    let nonce = policy
        .strip_prefix("script-src 'nonce-")
        .and_then(|rest| rest.strip_suffix('\''))
        .expect("the harness declares script-src 'nonce-<value>'");
    assert!(
        String::from_utf8_lossy(&first.body).contains(nonce),
        "the rendered body carried the nonce the header declared"
    );
}

/// The hit is assembled, not replayed: the shell comes from the stored
/// entry and the island inside it is mounted again, here and now, for
/// whoever is asking. So the handler never runs, two principals never
/// receive the same island, the same principal on the same session keeps
/// the scope its island was mounted under, and the bytes outside the island
/// are the very bytes the first render produced.
#[tokio::test]
#[serial_test::serial]
async fn a_hit_assembles_each_principals_own_island_without_the_handler() {
    let harness = boot().await;
    let a1 = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-a")],
    )
    .await;
    assert_eq!(a1.status, StatusCode::OK, "{}", a1.text());
    let before = handler_renders(STITCHED_PATH);
    let b1 = dispatch(
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
        "the handler did not run on the hit"
    );
    assert_eq!(b1.header("age"), Some("0"));
    let island_a = island_tag(&a1.text(), "stitch-counter").to_owned();
    let island_b = island_tag(&b1.text(), "stitch-counter").to_owned();
    assert_ne!(island_a, island_b, "each principal gets its own island");
    assert_ne!(
        decoded_snapshot(&island_a)["body"]["scope"],
        decoded_snapshot(&island_b)["body"]["scope"]
    );
    // On `a1`'s own session: the scope a mount is bound to is derived from
    // the session as well as the principal, and every dispatch without the
    // cookie starts a new session, so presenting it is what makes the two
    // requests comparable at all.
    let session_a = a1.session_cookie();
    let a2 = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-a"), ("cookie", &session_a)],
    )
    .await;
    assert_eq!(handler_renders(STITCHED_PATH), before);
    assert_eq!(
        decoded_snapshot(island_tag(&a2.text(), "stitch-counter"))["body"]["scope"],
        decoded_snapshot(&island_a)["body"]["scope"]
    );
    // the shell around the island is byte-identical for a and b
    let shell = |text: &str| text.replace(island_tag(text, "stitch-counter"), "");
    assert_eq!(shell(&a1.text()), shell(&b1.text()));
    // The bytes hold one principal's island, mounted under authority derived
    // for one request, so nothing may store them: a `max-age` here would let
    // a shared browser profile hand user-a's island to whoever sits down
    // next, and skip reauthorization for the whole window.
    assert_eq!(a2.header("cache-control"), Some("private, no-store"));
    assert_eq!(b1.header("cache-control"), Some("private, no-store"));
}

/// Every response an assembled hit sends carries a strong validator over
/// exactly the bytes it sent, and no assembled response is ever a 304.
///
/// A stitched document is assembled per request: a nonce and island
/// identities minted for this request alone. So no two assemblies are the
/// same representation, and a 304 - even for `If-None-Match: *` - would tell
/// the client to pair the body it already holds with the nonce headers just
/// minted for a document it has never seen. The conditional is therefore not
/// evaluated at all: every assembled `GET` answers 200 with its body and its
/// own validator, and `HEAD` answers with the same headers and no body.
#[tokio::test]
#[serial_test::serial]
async fn conditional_and_head_requests_use_the_assembled_validator() {
    let harness = boot().await;
    dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-a")],
    )
    .await;
    let b = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-b")],
    )
    .await;
    let etag_b = b.header("etag").expect("etag").to_owned();
    assert_eq!(
        etag_b,
        suprnova_live::render_cache::Validator::strong_for(&b.body).etag()
    );
    let wildcard = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-b"), ("if-none-match", "*")],
    )
    .await;
    assert_eq!(
        wildcard.status,
        StatusCode::OK,
        "an assembled document is never answered 304, not even for `*`"
    );
    assert!(!wildcard.body.is_empty());
    let wildcard_etag = suprnova_live::render_cache::Validator::strong_for(&wildcard.body).etag();
    assert_eq!(wildcard.header("etag"), Some(wildcard_etag.as_str()));
    let echoed = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-b"), ("if-none-match", &etag_b)],
    )
    .await;
    assert_eq!(
        echoed.status,
        StatusCode::OK,
        "nor for the validator of a document this same principal received"
    );
    assert!(!echoed.body.is_empty());
    let echoed_etag = suprnova_live::render_cache::Validator::strong_for(&echoed.body).etag();
    assert_eq!(
        echoed.header("etag"),
        Some(echoed_etag.as_str()),
        "and the response carries a validator over its own bytes"
    );
    assert_ne!(
        echoed_etag, etag_b,
        "which is a different document from the one the client had"
    );
    let head = dispatch(
        &harness,
        Method::HEAD,
        STITCHED_PATH,
        &[("x-test-login", "user-b")],
    )
    .await;
    assert_eq!(head.status, StatusCode::OK);
    assert!(head.body.is_empty());
    let head_etag = head.header("etag").expect("etag").to_owned();
    assert_ne!(
        head_etag,
        suprnova_live::render_cache::Validator::strong_for(&[]).etag(),
        "the validator is over the document assembled for this request, \
         not over the empty body a HEAD sends"
    );
    assert_eq!(
        head.header("cache-control"),
        b.header("cache-control"),
        "and the rest of the metadata is the metadata a GET would carry"
    );
}

/// A zero-slot Composite is assembled too. Nothing is re-mounted, but the
/// nonce is minted for this request and reaches the body and the header the
/// document declared it in together, so two hits are handler-free and
/// differ from one another in exactly the nonce - and in the validator that
/// describes them.
#[tokio::test]
#[serial_test::serial]
async fn a_zero_slot_composite_is_assembled_with_a_fresh_nonce_on_every_hit() {
    let harness = boot().await;
    let published = dispatch(
        &harness,
        Method::GET,
        SEED_ONLY_NONCE_PATH,
        &[("x-test-login", "user-1")],
    )
    .await;
    assert_eq!(published.status, StatusCode::OK, "{}", published.text());
    let before = handler_renders(SEED_ONLY_NONCE_PATH);
    let first = dispatch(
        &harness,
        Method::GET,
        SEED_ONLY_NONCE_PATH,
        &[("x-test-login", "user-2")],
    )
    .await;
    let second = dispatch(
        &harness,
        Method::GET,
        SEED_ONLY_NONCE_PATH,
        &[("x-test-login", "user-3")],
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert_eq!(second.status, StatusCode::OK, "{}", second.text());
    assert_eq!(
        handler_renders(SEED_ONLY_NONCE_PATH),
        before,
        "both hits were assembled without the handler"
    );
    let first_nonce = declared_nonce(&first);
    let second_nonce = declared_nonce(&second);
    assert_ne!(first_nonce, second_nonce, "every hit mints its own nonce");
    assert!(
        first.text().contains(&first_nonce) && second.text().contains(&second_nonce),
        "each body carries the nonce its own header declared"
    );
    assert_eq!(
        first.text().replace(&first_nonce, "<nonce>"),
        second.text().replace(&second_nonce, "<nonce>"),
        "the two assembled bodies differ only in the nonce"
    );
    assert_ne!(
        first.header("etag"),
        second.header("etag"),
        "and each validator describes its own bytes"
    );
    let first_etag = suprnova_live::render_cache::Validator::strong_for(&first.body).etag();
    assert_eq!(first.header("etag"), Some(first_etag.as_str()));
    // No slot means no per-principal bytes in the document, only a
    // per-request nonce, so this one keeps the class's private `max-age`
    // rather than the `no-store` a slotted entry is sent with.
    assert!(
        first
            .header("cache-control")
            .expect("cache-control")
            .starts_with("private, max-age="),
        "a zero-slot Composite keeps the class's private freshness"
    );
}

/// The nonce a response declared in its own `Content-Security-Policy`.
fn declared_nonce(response: &TestResponse) -> String {
    response
        .header("content-security-policy")
        .and_then(|policy| policy.strip_prefix("script-src 'nonce-"))
        .and_then(|rest| rest.strip_suffix('\''))
        .expect("the harness declares script-src 'nonce-<value>'")
        .to_owned()
}

#[tokio::test]
#[serial_test::serial]
async fn an_anonymous_or_logged_out_request_never_receives_an_assembled_document() {
    let harness = boot().await;
    dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-a")],
    )
    .await;
    let anonymous = dispatch(&harness, Method::GET, STITCHED_PATH, &[]).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert!(
        RenderCache::inspect_route_for_test(STITCHED_PATH)
            .await
            .is_some(),
        "the entry is untouched"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn a_stale_servable_composite_assembles_with_a_warning_and_no_background_render() {
    let harness = boot_with_freshness(1_000, 60_000, 0).await;
    dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-a")],
    )
    .await;
    clock(&harness).advance_ms(5_000);
    let before = handler_renders(STITCHED_PATH);
    let stale = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-b")],
    )
    .await;
    assert_eq!(stale.status, StatusCode::OK);
    assert!(stale.header("warning").is_some());
    assert_eq!(
        handler_renders(STITCHED_PATH),
        before,
        "no foreground and no background render"
    );
    let again = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-b")],
    )
    .await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(
        handler_renders(STITCHED_PATH),
        before,
        "still none: stitched routes never rebuild in the background"
    );
}
