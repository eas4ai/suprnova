//! The dogfood public document is served from the RenderCache and its
//! cached seed still promotes, and the dogfood dashboard is served as a
//! stitched document: one shared shell whose islands are re-mounted for
//! whoever is asking.
//!
//! Ruling R78: asserting only that the route responds would pass whether the
//! representation was stored or silently declined, so every claim here is
//! made against something that can only be true of a stored entry actually
//! being served. The render count comes from
//! `live_support::render_counter`, a middleware registered after
//! `RenderCache::install` and therefore reached only on requests the cache
//! forwards; the entry itself is read back through
//! `RenderCache::store_inspection` and
//! `RenderCache::inspect_route_for_test`, the latter deriving the same
//! lookup key the middleware derived.
//!
//! Ruling R84: the proof runs as an anonymous visitor, which is this route's
//! intended audience. The application installs `FeatureMiddleware` globally,
//! and a flag read during a render records the context's user id as an
//! observed principal, so a signed-in visitor of a route declaring no
//! `Principal` variance would be declined. This document reads no flag and
//! no translation - `templates/live/public.html` and
//! `templates/live/counter.html` contain neither - so nothing is observed
//! either way, and the anonymous run is the honest one to measure.

mod live_support;

use hyper::{Method, StatusCode};
use live_support::{
    ActionSpec, action_request, decoded_snapshot, empty, get, idempotency, invoke, island_tag,
    render_counter, request, seed_session, send, setup_app,
};
use serde_json::Value;
use suprnova::render_cache::{EntryKind, RenderCache, RepresentationClass};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_public_document_is_a_hit_whose_seed_still_promotes() {
    let app = setup_app(4).await;

    // 1. The first anonymous request renders.
    let before = render_counter::renders();
    let first = get(&app, "/live/public", None).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert_eq!(
        render_counter::renders(),
        before + 1,
        "the first GET reaches the handler"
    );
    assert!(
        first.text().contains("<h1>Public counter</h1>"),
        "the document really rendered: {}",
        first.text()
    );

    // 2. The second request is a hit. This is the assertion that separates a
    //    cache from the appearance of one: the counting middleware sits
    //    closer to the handler than `RenderCacheMiddleware`, so a request
    //    answered from a stored entry never reaches it.
    let before = render_counter::renders();
    let second = get(&app, "/live/public", None).await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.text());
    assert_eq!(
        render_counter::renders(),
        before,
        "the second GET never reaches the handler: it is served from the cache"
    );

    // 3. There is a real stored entry behind that hit, under the declared
    //    class rather than demoted by an observed identity.
    let store = RenderCache::store_inspection()
        .await
        .expect("store inspection");
    assert_eq!(
        store.entries, 1,
        "the render was stored, not declined: {store:?}"
    );
    assert!(store.bytes > 0);
    let stored = RenderCache::inspect_route_for_test("/live/public")
        .await
        .expect("the entry is reachable under the route's own lookup key");
    assert_eq!(
        stored.class,
        RepresentationClass::PublicShared,
        "stored under the declared class, undemoted by any observed identity"
    );
    assert_eq!(stored.status, 200);
    assert!(stored.body_bytes > 0);

    // 4. The served representation is the stored one, with the metadata a
    //    cached response carries. `PublicShared` with the builder's default
    //    `SharedCachePolicy::Private` means no `s-maxage` for a shared
    //    proxy, and a `max-age` that is the policy's five minutes bounded
    //    underneath by the public seed's own promotion deadline (24 hours),
    //    so five minutes is what survives.
    assert_eq!(second.body, first.body, "byte for byte the stored entry");
    let etag = first.header("etag").expect("etag").to_owned();
    assert_eq!(second.header("etag"), Some(etag.as_str()));
    assert!(
        second.header("age").is_some(),
        "a served entry carries its age"
    );
    let cache_control = first.header("cache-control").expect("cache-control");
    assert!(
        cache_control.starts_with("private, max-age="),
        "{cache_control}"
    );
    let max_age: u64 = cache_control
        .split("max-age=")
        .nth(1)
        .expect("max-age")
        .parse()
        .expect("seconds");
    assert_eq!(max_age, 300, "the policy's fresh interval: {cache_control}");

    // 5. The seed inside the cached document still promotes, exactly as one
    //    freshly rendered does.
    let html = second.text();
    let snapshot = decoded_snapshot(island_tag(&html, "public-counter"));
    let key = idempotency(1);
    let before = render_counter::renders();
    let promoted = send(
        app.addr,
        action_request(
            &app,
            ActionSpec {
                component: "app.counter",
                document_key: "public-counter",
                snapshot,
                seed: true,
                base_revision: "0",
                operations: invoke("increment"),
                model_proposals: Value::Object(Default::default()),
                idempotency_key: &key,
            },
            None,
            true,
        ),
    )
    .await;
    assert_eq!(promoted.status, StatusCode::OK, "{}", promoted.text());
    let accepted = promoted.json();
    assert_eq!(
        accepted["outcome"], "accepted",
        "a cached public-seed document promotes like a fresh one: {accepted}"
    );
    assert!(
        accepted["render"]["html"]
            .as_str()
            .is_some_and(|html| html.contains("Count: 1")),
        "{accepted}"
    );
    assert_eq!(
        render_counter::renders(),
        before + 1,
        "the action is a POST: it bypasses the cache and runs the handler"
    );

    // 6. A conditional request on the served validator is answered from the
    //    same entry with no body and no render.
    let before = render_counter::renders();
    let conditional = send(
        app.addr,
        request(&app, Method::GET, "/live/public", None, false)
            .header("if-none-match", &etag)
            .body(empty())
            .expect("build conditional request"),
    )
    .await;
    assert_eq!(
        conditional.status,
        StatusCode::NOT_MODIFIED,
        "{}",
        conditional.text()
    );
    assert!(conditional.body.is_empty(), "a 304 carries no body");
    assert_eq!(
        render_counter::renders(),
        before,
        "the conditional GET is answered from the entry, not a render"
    );
}

/// The dashboard is this application's stitched route: one shared shell is
/// published once, and every hit re-mounts its three identity-bound islands
/// for whoever is asking, behind the route's own login gate.
///
/// Ruling R78 in the form this class needs. `render_counter` cannot carry
/// the "the handler did not run" claim here, and saying so is the point of
/// this note: it is a global middleware registered after
/// `RenderCache::install`, so it sits outside the route's own chain, and a
/// stitched hit is deliberately forwarded through that whole chain before
/// anything is served. It therefore counts every request to the dashboard,
/// hit or miss. What only an assembled hit can produce is
/// `Cache-Control: private, no-store`: the composite responder is the only
/// writer of that value, it runs only after a whole document has been
/// assembled, and that path never calls the handler. So the counter is
/// asserted for what it actually is, and the stored Composite entry, the
/// per-principal islands, and the no-store response carry the cache claim.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_dashboard_is_stitched_per_principal_from_one_shared_shell() {
    let app = setup_app(6).await;
    let alice = seed_session(&app).await;
    let bob = seed_session(&app).await;

    // 1. The first signed-in request renders and publishes a shell.
    let before = render_counter::renders();
    let first = get(&app, "/live", Some(&alice)).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert_eq!(
        render_counter::renders(),
        before + 1,
        "the first GET reaches the handler"
    );
    assert_eq!(
        first.header("cache-control"),
        Some("private, max-age=300"),
        "the render that published the shell is not itself an assembly"
    );

    // 2. What was stored is a segment graph, not a finished answer: one
    //    slot per identity-bound island, under the declared class.
    let stored = RenderCache::inspect_route_for_test("/live")
        .await
        .expect("the entry is reachable under the route's own lookup key");
    assert_eq!(stored.kind, EntryKind::Composite);
    assert_eq!(
        stored.class,
        RepresentationClass::PublicShellStitched,
        "stored under the declared class, undemoted by any observed identity"
    );
    assert_eq!(stored.status, 200);
    assert_eq!(
        stored.slots, 3,
        "one slot for the counter, the uploader, and the feed"
    );

    // 3. A second principal is answered from that shell without the
    //    handler running. `private, no-store` is written by nothing but the
    //    composite responder, and the composite responder is reached only
    //    after a document has been fully assembled from re-mounted islands.
    //    The counter still moves, which is the fact this test's own note is
    //    about: a stitched hit is forwarded through the chain the counting
    //    middleware sits outside of.
    let before = render_counter::renders();
    let second = get(&app, "/live", Some(&bob)).await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.text());
    assert_eq!(
        render_counter::renders(),
        before + 1,
        "a stitched hit is forwarded through the route chain, so it is counted too"
    );
    assert_eq!(
        second.header("cache-control"),
        Some("private, no-store"),
        "an assembled document holds one principal's islands: nothing may store it"
    );
    assert_eq!(second.header("age"), Some("0"));

    // 4. The islands in it belong to the principal who asked, and nothing
    //    else in the document does: two fresh principals differ only in the
    //    identity-bearing island tags, which is exactly what the shell has
    //    holes for.
    let alice_counter = decoded_snapshot(island_tag(&first.text(), "dashboard-counter"));
    let bob_counter = decoded_snapshot(island_tag(&second.text(), "dashboard-counter"));
    assert_ne!(
        alice_counter["body"]["scope"], bob_counter["body"]["scope"],
        "each principal is mounted under its own scope"
    );
    let shell = |html: &str| {
        let mut rest = html.to_owned();
        for key in ["dashboard-counter", "dashboard-uploader", "dashboard-feed"] {
            let island = island_tag(&rest, key).to_owned();
            rest = rest.replace(&island, "");
        }
        rest
    };
    assert_eq!(
        shell(&first.text()),
        shell(&second.text()),
        "the two documents differ only in their island tags"
    );

    // 5. The same login on the same session keeps the scope its island was
    //    mounted under: a mount's scope is derived from the session as well
    //    as the principal, so carrying the cookie is what makes the two
    //    requests comparable at all.
    let repeat = get(&app, "/live", Some(&alice)).await;
    assert_eq!(repeat.status, StatusCode::OK, "{}", repeat.text());
    assert_eq!(repeat.header("cache-control"), Some("private, no-store"));
    assert_eq!(
        decoded_snapshot(island_tag(&repeat.text(), "dashboard-counter"))["body"]["scope"],
        alice_counter["body"]["scope"]
    );

    // 6. The gate runs on every hit: an anonymous visitor gets the route's
    //    own redirect, never an assembled document. The prepared hit is
    //    dropped unread when the chain refuses.
    let anonymous = get(&app, "/live", None).await;
    assert_eq!(anonymous.status, StatusCode::FOUND);
    assert_eq!(anonymous.header("location"), Some("/login"));
    assert!(anonymous.body.is_empty(), "a redirect carries no document");

    // 7. A Composite response never answers 304. Each assembly is a
    //    distinct representation - fresh instance identities, and a fresh
    //    nonce where a document has one - so the validator is strong over
    //    the bytes it was sent with and never matches a later request.
    let etag = second.header("etag").expect("etag").to_owned();
    let conditional = send(
        app.addr,
        request(&app, Method::GET, "/live", Some(&bob), false)
            .header("if-none-match", &etag)
            .body(empty())
            .expect("build conditional request"),
    )
    .await;
    assert_eq!(
        conditional.status,
        StatusCode::OK,
        "Composite responses never answer 304: each assembly is a distinct representation"
    );
    assert!(!conditional.body.is_empty());
    assert_ne!(
        conditional.header("etag"),
        Some(etag.as_str()),
        "the validator describes this assembly, not the last one"
    );
}
