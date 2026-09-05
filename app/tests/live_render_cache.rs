//! The dogfood public document is served from the RenderCache and its
//! cached seed still promotes.
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
    render_counter, request, send, setup_app,
};
use serde_json::Value;
use suprnova::render_cache::{RenderCache, RepresentationClass};

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
