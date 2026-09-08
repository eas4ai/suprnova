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
    FALLBACK_HTML, NO_DIGEST_PATH, OPTIONAL_FAIL_PATH, OPTIONAL_FALLBACK_PATH, OPTIONAL_OMIT_PATH,
    POST_PROCESSED_PATH, SEED_ONLY_NONCE_PATH, SEED_ONLY_PATH, SHELL_READS_PRINCIPAL_PATH,
    STITCHED_PATH, TWICE_RENDERED_PATH, TestResponse, boot, boot_with_freshness, chain_reaches,
    clock, decoded_snapshot, dispatch, handler_renders, island_tag, set_tenant_refusing,
};

// `rewrite_stored_entry` reaches `render_cache::testing::rewrite_composite_for_test`
// and `RenderCache::shell_for_test` is the framework's own seam; both exist
// only with the `testing` feature, which the minimal profile checked by
// `scripts/check-feature-matrix.sh` leaves off. The four tests that need
// them - and the names only those tests use - carry the same gate.
#[cfg(feature = "testing")]
use crate::render_cache_stitch_support::{
    FALLBACK_PATH, NONCE_PATH, OMIT_PATH, attribute, rewrite_stored_entry,
};
#[cfg(feature = "testing")]
use suprnova_live::render_cache::composite::Segment;

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
    // next, and skip reauthorization for the whole window. `a1` is the
    // leader's own rendered document rather than an assembly, and it holds
    // user-a's island just the same, so it is held to the same value: the
    // rule is about what the bytes contain, not about which code path
    // produced them.
    assert_eq!(a1.header("cache-control"), Some("private, no-store"));
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
    // Pins `lead_render`'s half of the override, which nothing else does:
    // `no-store` must never reach a Composite with no per-principal bytes in
    // it, on the publishing render any more than on the hits below.
    assert!(
        published
            .header("cache-control")
            .expect("cache-control")
            .starts_with("private, max-age="),
        "the render that published a zero-slot Composite keeps the class's private freshness"
    );
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

// ── Task 9: failure policies, nonce regeneration, redeploy shape,
//    concurrency, and what the stored shell does not contain ────────────

/// The three declared failure policies, all driven by the same real
/// reauthorization refusal on a hit - and one that is not the route guard.
///
/// The `OPTIONAL_*` routes are guarded by `AuthMiddleware::optional()`, so a
/// signed-out visitor is admitted, the whole chain runs, and the identity-bound
/// island is the only thing that refuses them: `validate_request_context`
/// has no principal to bind the mount to. That is the per-slot refusal a
/// stitched hit has to survive, and Task 8's anonymous test cannot reach it,
/// because there the guard answers 401 before any slot is considered.
///
/// `Omit` leaves the island out, `Fallback` puts the declared fragment
/// there, and `FailDocument` abandons assembly and hands the request to the
/// route's own handler - which, facing the same refusal, fails its own mount,
/// so the visitor sees the handler's 500 rather than a document quietly
/// missing an island nobody declared could go missing.
///
/// In every case the shell the signed-in leader published is left exactly as
/// it was, none of the leader's island reaches the signed-out visitor, and a
/// signed-in visitor afterwards is still served an assembled document with no
/// handler render at all.
#[tokio::test]
#[serial_test::serial]
async fn omit_and_fallback_policies_take_effect_and_fail_document_runs_the_handler() {
    let harness = boot().await;
    let mut leader_bodies = Vec::new();
    for path in [
        OPTIONAL_OMIT_PATH,
        OPTIONAL_FALLBACK_PATH,
        OPTIONAL_FAIL_PATH,
    ] {
        let published = dispatch(&harness, Method::GET, path, &[("x-test-login", "user-a")]).await;
        assert_eq!(published.status, StatusCode::OK, "{}", published.text());
        assert_eq!(
            RenderCache::inspect_route_for_test(path)
                .await
                .expect("stored")
                .kind,
            EntryKind::Composite,
            "{path} published a Composite entry to assemble from"
        );
        leader_bodies.push(published.text());
    }

    let omit_before = handler_renders(OPTIONAL_OMIT_PATH);
    let omitted = dispatch(&harness, Method::GET, OPTIONAL_OMIT_PATH, &[]).await;
    assert_eq!(omitted.status, StatusCode::OK, "{}", omitted.text());
    assert!(
        !omitted.text().contains("data-suprnova-live-root"),
        "the island is left out"
    );
    assert_eq!(
        handler_renders(OPTIONAL_OMIT_PATH),
        omit_before,
        "an omitted slot still assembles, so the handler never ran"
    );

    let fallback_before = handler_renders(OPTIONAL_FALLBACK_PATH);
    let fallback = dispatch(&harness, Method::GET, OPTIONAL_FALLBACK_PATH, &[]).await;
    assert_eq!(fallback.status, StatusCode::OK, "{}", fallback.text());
    assert!(fallback.text().contains(FALLBACK_HTML));
    assert!(!fallback.text().contains("data-suprnova-live-root"));
    assert_eq!(
        handler_renders(OPTIONAL_FALLBACK_PATH),
        fallback_before,
        "a fallback slot still assembles, so the handler never ran"
    );

    let before = handler_renders(OPTIONAL_FAIL_PATH);
    let reaches_before = chain_reaches(OPTIONAL_FAIL_PATH);
    assert!(
        RenderCache::inspect_route_for_test(OPTIONAL_FAIL_PATH)
            .await
            .is_some(),
        "an entry is stored, so this request is a hit and not another miss"
    );
    let failed = dispatch(&harness, Method::GET, OPTIONAL_FAIL_PATH, &[]).await;
    assert_eq!(
        chain_reaches(OPTIONAL_FAIL_PATH),
        reaches_before + 1,
        "the cache middleware handed the request on: the hit path was taken"
    );
    assert_eq!(
        handler_renders(OPTIONAL_FAIL_PATH),
        before + 1,
        "fail-document hands the request to the handler"
    );
    assert_eq!(
        failed.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "and the handler's own mount failure is what the visitor sees"
    );

    // Nothing the signed-in leader was sent reached the signed-out visitor
    // in any of the three, and every entry is still exactly what it was.
    for (path, leader) in [
        OPTIONAL_OMIT_PATH,
        OPTIONAL_FALLBACK_PATH,
        OPTIONAL_FAIL_PATH,
    ]
    .into_iter()
    .zip(&leader_bodies)
    {
        let island = island_tag(leader, document_key_of(path));
        for refused in [&omitted, &fallback, &failed] {
            assert!(
                !refused.text().contains(island),
                "no refused visitor received the leader's island from {path}"
            );
        }
        let entry = RenderCache::inspect_route_for_test(path)
            .await
            .expect("the entry survived the refused hit");
        assert_eq!(entry.kind, EntryKind::Composite);
        assert_eq!(entry.slots, 1);
    }

    // And a signed-in visitor is still served an assembled document.
    let before = handler_renders(OPTIONAL_FAIL_PATH);
    let ok = dispatch(
        &harness,
        Method::GET,
        OPTIONAL_FAIL_PATH,
        &[("x-test-login", "user-b")],
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text());
    assert_eq!(handler_renders(OPTIONAL_FAIL_PATH), before);
    assert!(ok.text().contains("data-suprnova-live-root"));
}

/// The document key each `OPTIONAL_*` route mounts its island under.
fn document_key_of(path: &str) -> &'static str {
    match path {
        OPTIONAL_OMIT_PATH => "stitch-optional-omit",
        OPTIONAL_FALLBACK_PATH => "stitch-optional-fallback",
        OPTIONAL_FAIL_PATH => "stitch-optional-fail",
        other => panic!("no document key recorded for {other}"),
    }
}

/// Reauthorization on a hit is done again for this request, never replayed
/// from the entry: with the tenant middleware resolving a different tenant
/// on every request, one visitor on one session receives two islands bound
/// to two different scopes, even though the shell around them is the shell
/// one earlier render published.
///
/// A stitched entry stores an island's *declaration* and never its snapshot,
/// so there is nothing to replay; this is what that looks like from outside.
#[tokio::test]
#[serial_test::serial]
async fn a_hit_binds_its_island_to_the_authority_of_the_request_in_front_of_it() {
    let harness = boot().await;
    let leader = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-a")],
    )
    .await;
    assert_eq!(leader.status, StatusCode::OK, "{}", leader.text());
    let session = leader.session_cookie();
    let leader_scope =
        decoded_snapshot(island_tag(&leader.text(), "stitch-counter"))["body"]["scope"].clone();

    set_tenant_refusing(true);
    let before = handler_renders(STITCHED_PATH);
    let first = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-a"), ("cookie", &session)],
    )
    .await;
    let second = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-a"), ("cookie", &session)],
    )
    .await;
    set_tenant_refusing(false);
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert_eq!(second.status, StatusCode::OK, "{}", second.text());
    assert_eq!(
        handler_renders(STITCHED_PATH),
        before,
        "both were assembled from the stored shell"
    );
    let first_scope =
        decoded_snapshot(island_tag(&first.text(), "stitch-counter"))["body"]["scope"].clone();
    let second_scope =
        decoded_snapshot(island_tag(&second.text(), "stitch-counter"))["body"]["scope"].clone();
    assert_ne!(
        first_scope, second_scope,
        "the same visitor on the same session, under two tenants, gets two scopes"
    );
    assert_ne!(
        first_scope, leader_scope,
        "and neither is the scope the shell was published under"
    );
}

/// The nonce is minted per request and reaches the body and the header
/// together, on a route that also has an island to re-mount - so this is the
/// whole shape at once: one private slot, at least one nonce hole in the
/// shell, and the `Content-Security-Policy` the document declared rebuilt
/// from a stored template.
///
/// A replayed nonce proves nothing, so the miss render's nonce must never
/// appear again, and two hits by the same visitor must not share one either.
// Needs `rewrite_stored_entry`; see this file's gated import.
#[cfg(feature = "testing")]
#[tokio::test]
#[serial_test::serial]
async fn nonces_are_regenerated_in_the_body_and_the_csp_header_on_every_hit() {
    let harness = boot().await;
    let first = dispatch(
        &harness,
        Method::GET,
        NONCE_PATH,
        &[("x-test-login", "user-a")],
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());

    let stored = RenderCache::inspect_route_for_test(NONCE_PATH)
        .await
        .expect("stored");
    assert_eq!(stored.kind, EntryKind::Composite);
    assert_eq!(stored.slots, 1, "one identity-bound island to re-mount");
    // The stored graph is not otherwise observable from outside the crate,
    // so it is read through the rewrite seam with an edit that changes
    // nothing.
    let mut holes = 0usize;
    let mut templates = 0usize;
    rewrite_stored_entry(NONCE_PATH, |graph| {
        holes = graph
            .segments
            .iter()
            .filter(|segment| matches!(segment, Segment::Nonce))
            .count();
        templates = graph.nonce_headers.len();
    })
    .await;
    assert!(holes >= 1, "the shell has a hole where the nonce was");
    assert_eq!(
        templates, 1,
        "and the declared content-security-policy is a template, not a stored value"
    );

    let n1 = declared_nonce(&first);
    let before = handler_renders(NONCE_PATH);
    let hit = dispatch(
        &harness,
        Method::GET,
        NONCE_PATH,
        &[("x-test-login", "user-b")],
    )
    .await;
    assert_eq!(hit.status, StatusCode::OK, "{}", hit.text());
    assert_eq!(
        handler_renders(NONCE_PATH),
        before,
        "the hit was assembled, not rendered"
    );
    let n2 = declared_nonce(&hit);
    assert_ne!(n1, n2);
    assert!(
        hit.text().contains(&format!("nonce=\"{n2}\"")),
        "the bootstrap script tags carry the fresh nonce"
    );
    assert!(
        !hit.text().contains(&n1),
        "the miss render's nonce never reappears"
    );
    // The island in the assembled document was mounted for this request, not
    // replayed from the render the shell was cut out of: a replayed island
    // would carry the miss render's own scope.
    assert_ne!(
        decoded_snapshot(island_tag(&hit.text(), "stitch-nonce"))["body"]["scope"],
        decoded_snapshot(island_tag(&first.text(), "stitch-nonce"))["body"]["scope"],
        "the hit's island is not the one the shell was cut from"
    );

    let again = dispatch(
        &harness,
        Method::GET,
        NONCE_PATH,
        &[("x-test-login", "user-b")],
    )
    .await;
    assert_ne!(
        declared_nonce(&again),
        n2,
        "two hits by one visitor do not share a nonce either"
    );
}

/// A redeploy in which a slot's component contract changed: the stored
/// declaration names a contract digest the running registry does not have,
/// which is drift between the entry and the build, and drift is a slot
/// failure - never a substitution of whatever component now answers to that
/// route and slot.
///
/// Each of the three policies is exercised against the same mismatch, so the
/// policy - not the kind of failure - is what decides the outcome.
// Needs `rewrite_stored_entry`; see this file's gated import.
#[cfg(feature = "testing")]
#[tokio::test]
#[serial_test::serial]
async fn a_slot_whose_declaration_no_longer_matches_the_catalog_is_a_slot_failure() {
    let harness = boot().await;
    for path in [STITCHED_PATH, OMIT_PATH, FALLBACK_PATH] {
        let published = dispatch(&harness, Method::GET, path, &[("x-test-login", "user-a")]).await;
        assert_eq!(published.status, StatusCode::OK, "{}", published.text());
        // Re-encode the stored entry with the slot's contract digest
        // changed, under the runtime's own ring.
        rewrite_stored_entry(path, |graph| {
            graph.slots[0].contract_digest =
                suprnova_live::identity::ContentDigest::from_bytes(&[0xAB; 32])
                    .expect("digest")
                    .to_base64url();
        })
        .await;
    }

    let before = handler_renders(STITCHED_PATH);
    let response = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-b")],
    )
    .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        handler_renders(STITCHED_PATH),
        before + 1,
        "fail-document: the handler rendered"
    );

    let before = handler_renders(OMIT_PATH);
    let omitted = dispatch(
        &harness,
        Method::GET,
        OMIT_PATH,
        &[("x-test-login", "user-b")],
    )
    .await;
    assert_eq!(omitted.status, StatusCode::OK);
    assert_eq!(
        handler_renders(OMIT_PATH),
        before,
        "omit: assembled without the handler"
    );
    assert!(!omitted.text().contains("data-suprnova-live-root"));

    let before = handler_renders(FALLBACK_PATH);
    let fallback = dispatch(
        &harness,
        Method::GET,
        FALLBACK_PATH,
        &[("x-test-login", "user-b")],
    )
    .await;
    assert_eq!(fallback.status, StatusCode::OK);
    assert_eq!(
        handler_renders(FALLBACK_PATH),
        before,
        "fallback: assembled without the handler"
    );
    assert!(fallback.text().contains(FALLBACK_HTML));
    assert!(!fallback.text().contains("data-suprnova-live-root"));
}

/// A slot whose component name no longer matches is the same drift from the
/// other direction: the route and slot still resolve, but what they resolve
/// to is not what the entry recorded, and the declaration - not the running
/// catalog - is what a stitched hit is checked against.
// Needs `rewrite_stored_entry`; see this file's gated import.
#[cfg(feature = "testing")]
#[tokio::test]
#[serial_test::serial]
async fn a_slot_naming_a_component_the_registry_does_not_have_is_a_slot_failure() {
    let harness = boot().await;
    dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-a")],
    )
    .await;
    rewrite_stored_entry(STITCHED_PATH, |graph| {
        "tests.no-such-component".clone_into(&mut graph.slots[0].component);
    })
    .await;
    let before = handler_renders(STITCHED_PATH);
    let response = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-b")],
    )
    .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        handler_renders(STITCHED_PATH),
        before + 1,
        "fail-document: the handler rendered"
    );
}

/// Several principals hitting one stitched route at once each receive their
/// own island - the island belonging to *that* principal, not merely a
/// different one from its neighbour's - and none of them makes the handler
/// run.
///
/// The barrier is the join itself, never a sleep: the requests are started
/// before any of them is awaited.
///
/// The first pair races the rebuild. Whichever wins the lease publishes the
/// shell while the other is already in flight behind it, which is the shape
/// in which a waiter could most easily be handed the leader's own bytes; had
/// that happened, the two scopes would be equal.
///
/// The second burst is where attribution is proved, and it needs the session
/// pinned. A mount's scope is a digest over (session, principal, tenant), and
/// every request sent without a cookie mints a fresh session, so four
/// distinct scopes would follow from session freshness alone even if the four
/// responses had been shuffled between the principals who asked for them. So
/// each principal is given one serial request first, its session cookie and
/// resulting scope recorded, and the concurrent burst then presents that same
/// cookie: the tenant is absent, so each principal's scope is fixed, and the
/// concurrent response is checked against the scope belonging to the
/// principal who sent it.
#[tokio::test(flavor = "current_thread")]
#[serial_test::serial]
async fn concurrent_principals_during_one_rebuild_each_receive_their_own_island() {
    let harness = boot().await;
    let a = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-a")],
    );
    let b = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-b")],
    );
    let (a, b) = tokio::join!(a, b);
    assert_eq!(a.status, StatusCode::OK, "{}", a.text());
    assert_eq!(b.status, StatusCode::OK, "{}", b.text());
    assert_ne!(
        decoded_snapshot(island_tag(&a.text(), "stitch-counter"))["body"]["scope"],
        decoded_snapshot(island_tag(&b.text(), "stitch-counter"))["body"]["scope"],
        "the waiter behind the rebuild was not handed the leader's own island: \
         that would have given both requests one scope"
    );

    let published = RenderCache::inspect_route_for_test(STITCHED_PATH)
        .await
        .expect("stored");
    assert_eq!(published.kind, EntryKind::Composite);
    let before = handler_renders(STITCHED_PATH);

    // One serial request per principal, to learn the session each of them is
    // on and the scope that session and principal produce together.
    let logins = ["user-c", "user-d", "user-e", "user-f"];
    let mut sessions = Vec::new();
    let mut expected = Vec::new();
    for login in logins {
        let primed = dispatch(
            &harness,
            Method::GET,
            STITCHED_PATH,
            &[("x-test-login", login)],
        )
        .await;
        assert_eq!(primed.status, StatusCode::OK, "{}", primed.text());
        sessions.push(primed.session_cookie());
        expected.push(scope_of(&primed.text()));
    }

    // The same four, at once, each on the session it was just given.
    let headers = [
        [
            ("x-test-login", logins[0]),
            ("cookie", sessions[0].as_str()),
        ],
        [
            ("x-test-login", logins[1]),
            ("cookie", sessions[1].as_str()),
        ],
        [
            ("x-test-login", logins[2]),
            ("cookie", sessions[2].as_str()),
        ],
        [
            ("x-test-login", logins[3]),
            ("cookie", sessions[3].as_str()),
        ],
    ];
    let (c, d, e, f) = tokio::join!(
        dispatch(&harness, Method::GET, STITCHED_PATH, &headers[0]),
        dispatch(&harness, Method::GET, STITCHED_PATH, &headers[1]),
        dispatch(&harness, Method::GET, STITCHED_PATH, &headers[2]),
        dispatch(&harness, Method::GET, STITCHED_PATH, &headers[3]),
    );
    assert_eq!(
        handler_renders(STITCHED_PATH),
        before,
        "every one of those requests was assembled, none rendered"
    );

    let mut scopes = Vec::new();
    for ((response, login), expected) in [&c, &d, &e, &f].into_iter().zip(logins).zip(&expected) {
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        let scope = scope_of(&response.text());
        assert_eq!(
            &scope, expected,
            "{login}'s concurrent response carries {login}'s own island, not \
             another principal's"
        );
        scopes.push(scope);
    }
    let distinct: std::collections::BTreeSet<&String> = scopes.iter().collect();
    assert_eq!(
        distinct.len(),
        scopes.len(),
        "and no two of the four shared one island"
    );
}

/// The scope the `stitch-counter` island in `html` was mounted under.
fn scope_of(html: &str) -> String {
    decoded_snapshot(island_tag(html, "stitch-counter"))["body"]["scope"]
        .as_str()
        .expect("a scope in the emitted snapshot")
        .to_owned()
}

/// What the shared shell is allowed to contain, stated over its actual
/// bytes: not the leader's island markup, and not the signed snapshot that
/// island carried.
///
/// `inspect_route_for_test` reports only a length, which shows the shell is
/// smaller than the document but not what came out of it. This reads the
/// stored bytes.
// Needs `RenderCache::shell_for_test`; see this file's gated import.
#[cfg(feature = "testing")]
#[tokio::test]
#[serial_test::serial]
async fn the_stored_shell_holds_no_island_markup_and_no_signed_snapshot() {
    let harness = boot().await;
    let first = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-a")],
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    let body = first.text();
    let island = island_tag(&body, "stitch-counter").to_owned();
    let snapshot = attribute(&island, "data-suprnova-live-snapshot").to_owned();

    let shell = RenderCache::shell_for_test(STITCHED_PATH)
        .await
        .expect("a Composite entry is stored");
    let shell = String::from_utf8(shell.to_vec()).expect("the shell is UTF-8");
    assert!(
        !shell.contains(&island),
        "the leader's island markup is not in the shared shell"
    );
    assert!(
        !shell.contains(&snapshot),
        "nor is the signed snapshot it carried"
    );
    assert!(
        !shell.contains("data-suprnova-live-snapshot"),
        "nor any signed snapshot at all"
    );
    assert!(
        !shell.contains("data-suprnova-live-root"),
        "nor any island root the request mounted"
    );
    assert!(
        shell.contains("<html") && shell.contains("</html>"),
        "what is left is the document around the island: {} bytes starting {:?}",
        shell.len(),
        // By character, not by byte: a byte index that landed inside a
        // multi-byte character would panic while formatting the failure.
        shell.chars().take(80).collect::<String>()
    );
}

/// A handler that mounts an identity-bound island and then writes its own
/// response never records a document digest, so nothing proves the bytes
/// being cut are the bytes a Live document rendered. The publisher declines
/// rather than cutting a shell out of a body it cannot vouch for, and the
/// route renders on every request.
///
/// `STITCHED_PATH` is the positive control: the same island, the same
/// chain, one `LiveDocument::render`, and a Composite entry.
#[tokio::test]
#[serial_test::serial]
async fn a_document_that_was_never_rendered_is_not_published() {
    let harness = boot().await;
    let control = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-1")],
    )
    .await;
    assert_eq!(control.status, StatusCode::OK, "{}", control.text());
    assert!(
        RenderCache::inspect_route_for_test(STITCHED_PATH)
            .await
            .is_some(),
        "the control route, which renders its document, publishes"
    );

    let first = dispatch(
        &harness,
        Method::GET,
        NO_DIGEST_PATH,
        &[("x-test-login", "user-1")],
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert!(
        first.text().contains("data-suprnova-live-root"),
        "the handler really did mount and emit the island"
    );
    assert!(
        RenderCache::inspect_route_for_test(NO_DIGEST_PATH)
            .await
            .is_none(),
        "declined: no document digest was ever recorded for these bytes"
    );
    let before = handler_renders(NO_DIGEST_PATH);
    dispatch(
        &harness,
        Method::GET,
        NO_DIGEST_PATH,
        &[("x-test-login", "user-2")],
    )
    .await;
    assert_eq!(handler_renders(NO_DIGEST_PATH), before + 1);
}

/// Two rendered documents in one request are two bodies, and the mounts
/// recorded belong to both of them. No single shell can be cut from that,
/// so the capture is marked invalid and the route publishes nothing.
///
/// `STITCHED_PATH` is the positive control: the same island, the same
/// chain, one `LiveDocument::render`, and a Composite entry. Without it a
/// boot that published nothing at all would satisfy every assertion here.
#[tokio::test]
#[serial_test::serial]
async fn a_request_that_rendered_two_documents_is_not_published() {
    let harness = boot().await;
    let control = dispatch(
        &harness,
        Method::GET,
        STITCHED_PATH,
        &[("x-test-login", "user-1")],
    )
    .await;
    assert_eq!(control.status, StatusCode::OK, "{}", control.text());
    assert!(
        RenderCache::inspect_route_for_test(STITCHED_PATH)
            .await
            .is_some(),
        "the control route, which renders one document, publishes"
    );

    let first = dispatch(
        &harness,
        Method::GET,
        TWICE_RENDERED_PATH,
        &[("x-test-login", "user-1")],
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert!(
        RenderCache::inspect_route_for_test(TWICE_RENDERED_PATH)
            .await
            .is_none(),
        "declined: the capture is not a faithful account of one body"
    );
    let before = handler_renders(TWICE_RENDERED_PATH);
    dispatch(
        &harness,
        Method::GET,
        TWICE_RENDERED_PATH,
        &[("x-test-login", "user-2")],
    )
    .await;
    assert_eq!(handler_renders(TWICE_RENDERED_PATH), before + 1);
}
