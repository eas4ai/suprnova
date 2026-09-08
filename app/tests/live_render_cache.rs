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
    ActionSpec, action_request, advance_clock_ms, decoded_snapshot, empty, get, idempotency,
    invoke, island_tag, render_counter, request, seed_session, seed_session_named, send, setup_app,
    setup_app_on_the_database_profile, setup_app_with_clock,
};
use serde_json::Value;
use suprnova::render_cache::console::{epoch_advance_report_for_test, inspect_report_for_test};
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
/// hit or miss. `Cache-Control` cannot carry it either, and that is
/// deliberate: a slotted stitched route is `private, no-store` on the
/// leader's own rendered document as much as on every assembly after it,
/// because the directive follows what the bytes hold - one principal's
/// islands - and not which code path produced them.
///
/// So the counter is asserted for what it actually is, and the cache claim
/// here rests on what the store holds and what the two documents are made
/// of: a Composite entry with one slot per identity-bound island, read back
/// under the same lookup key the middleware derived, and two principals
/// whose documents differ in their island tags and nowhere else - one
/// shell, islands re-mounted per principal. "The handler did not run on the
/// hit" is asserted directly one layer down, in
/// `a_hit_assembles_each_principals_own_island_without_the_handler`
/// (`framework/tests/render_cache/stitch.rs`), against a render counter
/// that sits inside the route's own chain where this application's does
/// not.
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
        Some("private, no-store"),
        "these bytes hold Alice's three islands, so nothing may store them; \
         which code path produced them is irrelevant to that"
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

    // 3. A second principal is served under the same rule: the class is
    //    `private, no-store` whether the bytes were rendered or assembled,
    //    so this step pins the directive rather than the code path behind
    //    it. The counter still moves, which is the fact this test's own note
    //    is about: a stitched hit is forwarded through the chain the
    //    counting middleware sits outside of. Step 4 is where the document
    //    is shown to be the stored shell with this principal's own islands
    //    in it.
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

/// The todo listing's dependency is the `todos` table, not the clock: an
/// ordinary model write through this application's own `POST /todos/random`
/// route advances that table's generation, and the stored listing stops
/// being current although its five fresh minutes have barely started.
///
/// What "stops being current" looks like on this route is worth stating,
/// because it is not a foreground rebuild. The policy declares a minute of
/// stale service, and a moved entry is evaluated at an effective age of at
/// least its fresh interval, so it lands in that stale band: the visitor
/// who arrives first after the write is handed the copy on hand once, under
/// the `Warning` header that says so, and the refresh runs behind the
/// request. Neither the header nor the scheduled rebuild appears on a hit
/// against an entry nothing invalidated, which is what makes them the proof.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_orm_write_invalidates_the_todos_document_through_generations() {
    let app = setup_app(10).await;

    // 1. The first request renders and publishes.
    let before = render_counter::renders();
    let first = get(&app, "/live/todos", None).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert_eq!(render_counter::renders(), before + 1);
    let first_text = first.text();
    assert!(first_text.contains("<h1>Todos</h1>"), "{first_text}");
    let counted = todo_count(&first_text);

    // 2. The second is a hit: the counting middleware sits closer to the
    //    handler than `RenderCacheMiddleware`, so it is never reached. It is
    //    a *fresh* hit, carrying no stale marking and scheduling nothing.
    let before = render_counter::renders();
    let rebuilds_before = RenderCache::background_rebuilds_for_test();
    let second = get(&app, "/live/todos", None).await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(
        render_counter::renders(),
        before,
        "the second GET is served from the stored entry"
    );
    assert_eq!(second.body, first.body);
    assert!(second.header("warning").is_none());
    assert_eq!(
        RenderCache::background_rebuilds_for_test(),
        rebuilds_before,
        "nothing has invalidated the entry, so nothing is scheduled to replace it"
    );

    // 3. A write through the application's own route, with the session and
    //    the token that route's CSRF check requires - the same proof a
    //    browser would send. Nothing here relaxes that check.
    let writer = seed_session(&app).await;
    let created = send(
        app.addr,
        request(&app, Method::POST, "/todos/random", Some(&writer), true)
            .header("x-csrf-token", &writer.csrf)
            .body(empty())
            .expect("build the todo write"),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());

    // 4. The entry is no longer current. Its five minutes are barely
    //    started, so the advanced generation of the `todos` table is the
    //    only thing that can explain either of these: the response is now
    //    marked stale, and exactly one rebuild is scheduled to replace it.
    //    The rebuild decision is taken on the request's own path, so this
    //    reading is final by the time the dispatch returns.
    let renders_before = render_counter::renders();
    let moved = get(&app, "/live/todos", None).await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.text());
    assert_eq!(
        moved.header("warning"),
        Some("110 - \"Response is Stale\""),
        "the write moved the entry, so what is served is served as stale"
    );
    assert_eq!(
        RenderCache::background_rebuilds_for_test(),
        rebuilds_before + 1,
        "and exactly one rebuild is scheduled to replace it"
    );

    // 5. That rebuild really runs: a render reaches the handler that no
    //    request of this test is waiting on.
    render_counter::wait_until_renders_at_least(renders_before + 1).await;

    // 6. And the listing the application renders now has the written row in
    //    it. The stored copy is dropped first, deliberately: the rebuild's
    //    publication lands at a moment no barrier reachable from this
    //    application makes observable, so asserting on whichever request
    //    happened to catch it would be a race. Dropping L0 makes the next
    //    request an ordinary render, which is all this step claims; that the
    //    write is what invalidated the entry is step 4's claim, and step 4
    //    needs no rebuild at all.
    RenderCache::clear_l0_for_test();
    let rebuilt = get(&app, "/live/todos", None).await;
    assert_eq!(rebuilt.status, StatusCode::OK, "{}", rebuilt.text());
    let rebuilt_text = rebuilt.text();
    assert_eq!(
        todo_count(&rebuilt_text),
        counted + 1,
        "the render lists the todo the write added: {rebuilt_text}"
    );

    // 7. And the cache is current again: what that render published serves
    //    the next request without reaching the handler, and without the
    //    stale marking, so the route came all the way back to a plain hit.
    let before = render_counter::renders();
    let settled = get(&app, "/live/todos", None).await;
    assert_eq!(settled.status, StatusCode::OK);
    assert_eq!(
        render_counter::renders(),
        before,
        "the republished entry serves the next request in turn"
    );
    assert_eq!(settled.body, rebuilt.body);
    assert!(settled.header("warning").is_none());
}

/// The number `live/todos.html` printed, so a test asserts on what the
/// document says rather than on how many rows it happens to hold.
fn todo_count(html: &str) -> usize {
    let marker = "<p>Todo count: ";
    let start = html
        .find(marker)
        .map(|index| index + marker.len())
        .unwrap_or_else(|| panic!("no todo count in {html}"));
    let end = html[start..].find('<').expect("count end") + start;
    html[start..end].parse().expect("a decimal todo count")
}

/// `/live/me` is stored once per principal and never once for everybody:
/// each signed-in visitor's second request is answered without a render,
/// out of an entry whose key carries their own opaque principal material,
/// and a third visitor - who shares nothing with either - renders.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_private_document_is_cached_per_principal_and_never_crosses() {
    let app = setup_app(10).await;
    let ada = seed_session_named(&app, "Ada Lovelace").await;
    let grace = seed_session_named(&app, "Grace Hopper").await;

    // 1. One render each.
    let before = render_counter::renders();
    let ada_first = get(&app, "/live/me", Some(&ada)).await;
    assert_eq!(ada_first.status, StatusCode::OK, "{}", ada_first.text());
    let grace_first = get(&app, "/live/me", Some(&grace)).await;
    assert_eq!(grace_first.status, StatusCode::OK, "{}", grace_first.text());
    assert_eq!(render_counter::renders(), before + 2);

    // 2. Each second request is a hit.
    let before = render_counter::renders();
    let ada_second = get(&app, "/live/me", Some(&ada)).await;
    let grace_second = get(&app, "/live/me", Some(&grace)).await;
    assert_eq!(
        render_counter::renders(),
        before,
        "both principals are served from their own stored entry"
    );
    assert_eq!(ada_second.body, ada_first.body);
    assert_eq!(grace_second.body, grace_first.body);

    // 3. And the two stored representations are different documents, each
    //    naming its own principal. This is the claim the render count alone
    //    cannot make: two hits prove storage, these bytes prove partition.
    assert_ne!(ada_first.body, grace_first.body);
    assert!(
        ada_second.text().contains("Ada Lovelace") && !ada_second.text().contains("Grace Hopper"),
        "{}",
        ada_second.text()
    );
    assert!(
        grace_second.text().contains("Grace Hopper")
            && !grace_second.text().contains("Ada Lovelace"),
        "{}",
        grace_second.text()
    );
    assert_eq!(
        ada_second.header("cache-control"),
        Some("private, max-age=60"),
        "a private entry is never offered to a shared cache"
    );

    // 4. A third principal shares neither entry.
    let curie = seed_session_named(&app, "Marie Curie").await;
    let before = render_counter::renders();
    let curie_first = get(&app, "/live/me", Some(&curie)).await;
    assert_eq!(curie_first.status, StatusCode::OK, "{}", curie_first.text());
    assert_eq!(
        render_counter::renders(),
        before + 1,
        "a principal with no entry of their own renders"
    );
    assert!(curie_first.text().contains("Marie Curie"));

    // 5. An anonymous visitor is redirected before the handler runs, so no
    //    signed-in document is ever a candidate to serve them - and nothing
    //    is stored under the anonymous key either. The second anonymous
    //    request proves that the way this suite proves everything else: the
    //    counting middleware sits between the cache middleware and the
    //    route's own chain, so a request the cache answered would never
    //    reach it, and both of these do. A `302` is refused by eligibility
    //    before any of the private machinery is consulted, which is why the
    //    refusal costs a render each time rather than becoming an entry.
    let before = render_counter::renders();
    let anonymous = get(&app, "/live/me", None).await;
    assert_eq!(anonymous.status, StatusCode::FOUND);
    assert_eq!(anonymous.header("location"), Some("/login"));
    assert!(anonymous.body.is_empty());
    let again = get(&app, "/live/me", None).await;
    assert_eq!(again.status, StatusCode::FOUND);
    assert_eq!(again.header("location"), Some("/login"));
    assert_eq!(
        render_counter::renders(),
        before + 2,
        "the redirect is never stored: each anonymous request is forwarded again"
    );

    // 6. And the anonymous traffic poisoned nothing: Ada's page is hers
    //    again, and serving it needs no second render.
    //
    //    Ada's first entry is deliberately not the one asserted on here.
    //    Seeding Marie Curie in step 4 inserted a `users` row, and the
    //    handler resolves its principal through the provider that reads that
    //    table, so every stored `/live/me` entry observed the write and
    //    went out of date - a `PrivateCached` entry has no stale band, so the
    //    next request for each rebuilds in the foreground. That is the
    //    dependency contract doing its job, not the anonymous requests doing
    //    damage, and the pair below separates the two: one render to
    //    republish, then a hit whose body still names nobody but Ada.
    let before = render_counter::renders();
    let republished = get(&app, "/live/me", Some(&ada)).await;
    assert_eq!(republished.status, StatusCode::OK, "{}", republished.text());
    assert_eq!(render_counter::renders(), before + 1);
    let before = render_counter::renders();
    let ada_after = get(&app, "/live/me", Some(&ada)).await;
    assert_eq!(
        render_counter::renders(),
        before,
        "Ada is served her own stored document again, with no render"
    );
    assert_eq!(ada_after.body, republished.body);
    assert!(
        ada_after.text().contains("Ada Lovelace")
            && !ada_after.text().contains("Grace Hopper")
            && !ada_after.text().contains("Marie Curie"),
        "{}",
        ada_after.text()
    );
}

/// Both requests a client makes when it already has a copy are answered out
/// of the stored entry: a conditional GET on the served validator, and a
/// HEAD for the metadata alone. Neither renders.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conditional_and_head_requests_are_answered_from_the_stored_entry() {
    let app = setup_app(6).await;

    let first = get(&app, "/live/todos", None).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    let etag = first.header("etag").expect("etag").to_owned();
    let content_type = first
        .header("content-type")
        .expect("content-type")
        .to_owned();

    let before = render_counter::renders();
    let conditional = send(
        app.addr,
        request(&app, Method::GET, "/live/todos", None, false)
            .header("if-none-match", &etag)
            .body(empty())
            .expect("build the conditional request"),
    )
    .await;
    assert_eq!(conditional.status, StatusCode::NOT_MODIFIED);
    assert!(conditional.body.is_empty(), "a 304 carries no body");
    assert_eq!(conditional.header("etag"), Some(etag.as_str()));

    let head = send(
        app.addr,
        request(&app, Method::HEAD, "/live/todos", None, false)
            .body(empty())
            .expect("build the HEAD request"),
    )
    .await;
    assert_eq!(head.status, StatusCode::OK, "{}", head.text());
    assert!(head.body.is_empty(), "a HEAD response carries no body");
    assert_eq!(head.header("etag"), Some(etag.as_str()));
    assert_eq!(head.header("content-type"), Some(content_type.as_str()));

    assert_eq!(
        render_counter::renders(),
        before,
        "neither the conditional GET nor the HEAD reached the handler"
    );
}

/// Past its fresh minute and inside its stale-servable one, the listing is
/// served immediately from the copy on hand - marked stale, so nothing
/// downstream mistakes it for current - and refreshed behind the request.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_service_is_marked_and_rebuilt_in_the_background() {
    let app = setup_app_with_clock(8).await;

    // 1. Publish, at the clock's starting reading.
    let before = render_counter::renders();
    let fresh = get(&app, "/live/todos", None).await;
    assert_eq!(fresh.status, StatusCode::OK, "{}", fresh.text());
    assert_eq!(render_counter::renders(), before + 1);
    assert!(
        fresh.header("warning").is_none(),
        "a fresh response is not marked stale"
    );

    // 2. Past the policy's 300_000 fresh milliseconds, still inside its
    //    60_000 stale-servable ones.
    advance_clock_ms(&app, 300_001);

    let rebuilds_before = RenderCache::background_rebuilds_for_test();
    let renders_before = render_counter::renders();
    let stale = get(&app, "/live/todos", None).await;
    assert_eq!(stale.status, StatusCode::OK, "{}", stale.text());
    assert_eq!(
        stale.body, fresh.body,
        "the visitor is served the copy on hand, not a fresh render"
    );
    assert_eq!(
        stale.header("warning"),
        Some("110 - \"Response is Stale\""),
        "stale service is marked as such"
    );
    assert_eq!(
        stale.header("age"),
        Some("300"),
        "the age of the copy that was served, in whole seconds"
    );

    // 3. The rebuild decision is taken on the request's own path, so this
    //    reading is already final now that the dispatch has returned.
    assert_eq!(
        RenderCache::background_rebuilds_for_test(),
        rebuilds_before + 1,
        "serving stale scheduled exactly one rebuild"
    );

    // 4. And that rebuild really runs: a render reaches the handler that no
    //    request of this test is waiting on. `wait_until_renders_at_least`
    //    is a state barrier on the counter, not a timed wait.
    render_counter::wait_until_renders_at_least(renders_before + 1).await;
    assert_eq!(
        RenderCache::background_rebuilds_for_test(),
        rebuilds_before + 1,
        "and only that one: the stale service itself never rendered"
    );
}

/// Booted on the Database profile's providers, the same document is served
/// from a row in `suprnova_render_entries` rather than from memory alone.
///
/// The proof is the L1 store read directly, under the very key the
/// middleware derived: an in-memory hit would answer the same way to a
/// client, and the L1 tier holding the entry is what only a database-backed
/// boot can produce.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_database_profile_serves_a_hit_through_the_sql_stores() {
    let app = setup_app_on_the_database_profile(6).await;

    let before = render_counter::renders();
    let first = get(&app, "/live/todos", None).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert_eq!(render_counter::renders(), before + 1);

    let stored = RenderCache::inspect_l1_for_test("/live/todos", &[], None)
        .await
        .expect("the SQL store holds the published entry");
    assert_eq!(stored.class, RepresentationClass::PublicShared);
    assert_eq!(stored.kind, EntryKind::Complete);
    assert_eq!(stored.status, 200);
    assert!(stored.body_bytes > 0);

    // The in-process tier is emptied and nothing else moves, so the next
    // request can only be answered out of the SQL store.
    RenderCache::clear_l0_for_test();
    let before = render_counter::renders();
    let second = get(&app, "/live/todos", None).await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.text());
    assert_eq!(
        render_counter::renders(),
        before,
        "with L0 emptied, the hit came out of the database tier"
    );
    assert_eq!(second.body, first.body);
}

/// The two operator commands the framework registers, reached the way this
/// application's console binary reaches them: `render-cache:inspect` reports
/// one entry's shape and never a byte of its body, and
/// `render-cache:epoch-advance` puts every stored entry out of reach.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_operator_commands_inspect_without_a_body_and_advance_the_epoch() {
    let app = setup_app(8).await;

    let before = render_counter::renders();
    let first = get(&app, "/live/todos", None).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text());
    assert_eq!(render_counter::renders(), before + 1);
    let body = first.text();

    // 1. Inspection by the entry's own key text, which is what an operator
    //    reads out of application logging.
    let key = RenderCache::key_for_route_for_test("/live/todos", &[], None);
    assert!(key.starts_with("rk1."), "{key}");
    console(&["render-cache:inspect", &key])
        .await
        .expect("render-cache:inspect is reachable through the console entry point");
    // The text that command printed. `inspect_report_for_test` is the
    // framework's own seam for exactly this: it builds the string
    // `run_inspect_command` hands to `println!`, so the assertions below are
    // about what the operator was shown, without capturing process stdout -
    // which this test cannot do without reaching for `unsafe`.
    let report = inspect_report_for_test(&key)
        .await
        .expect("render-cache:inspect");
    assert!(
        report.contains("PublicShared") && report.contains("body_bytes"),
        "the report names the class and the byte count: {report}"
    );
    assert!(
        report.contains("current epoch: "),
        "and the authority epoch the entry has to be read against: {report}"
    );

    // 2. And none of the document. Checked against the served body itself,
    //    line by line, rather than against a guessed substring: every
    //    non-trivial line of what the visitor was sent must be absent from
    //    what the operator was shown.
    for line in body.lines().map(str::trim).filter(|line| line.len() > 3) {
        assert!(
            !report.contains(line),
            "the inspect report must carry no body text, but it contains {line:?}: {report}"
        );
    }

    // 3. The emergency invalidation. Keys are namespaced by the authority
    //    epoch, so advancing it puts every stored entry out of reach at
    //    once and the next request renders.
    let advanced = epoch_advance_report_for_test()
        .await
        .expect("render-cache:epoch-advance");
    assert!(
        advanced.starts_with("epoch advanced to "),
        "the report names the epoch it advanced to: {advanced}"
    );
    console(&["render-cache:epoch-advance"])
        .await
        .expect("render-cache:epoch-advance is reachable through the console entry point");
    let before = render_counter::renders();
    let after = get(&app, "/live/todos", None).await;
    assert_eq!(after.status, StatusCode::OK, "{}", after.text());
    assert_eq!(
        render_counter::renders(),
        before + 1,
        "every pre-advance entry is unreachable, so the route renders again"
    );
}

/// Runs one console command exactly as `app/src/bin/console.rs` does: the
/// framework's public `dispatch_argv_with_init` over an argv whose first
/// element is the binary name, resolving through the same registry the
/// operator's `console <name>` reaches.
///
/// The initializer is empty rather than `app::bootstrap::register`, which
/// the binary passes: the test harness has already installed this process's
/// database, container bindings, and RenderCache runtime, and running the
/// real bootstrap on top of them would connect a second database and
/// replace what the requests above were served from.
async fn console(argv: &[&str]) -> Result<(), suprnova::FrameworkError> {
    let argv: Vec<String> = std::iter::once("console".to_owned())
        .chain(argv.iter().map(|part| (*part).to_owned()))
        .collect();
    suprnova::console::dispatch_argv_with_init(argv, || async {}).await
}
