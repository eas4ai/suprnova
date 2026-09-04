//! Public-seed Live documents cache as Complete entries capped at the seed
//! deadline; identity-bound islands and no-store intents never store, even
//! when a handler mounts one and never calls `LiveDocument::render`.

mod live_dogfood_support;
mod render_cache_live_support;

use live_dogfood_support::{DOCUMENT_PATH, PRIVATE_DOCUMENT_PATH};
use render_cache_live_support::{
    RAW_PATH, STRIP_PATH, UNREASONED_PATH, boot_with_render_cache_and_live, clock, dispatch_get,
    private_renders, public_renders, public_seed_lifetime_ms, strip_renders, unreasoned_renders,
};
use suprnova::StatusCode;
use suprnova::live::LiveMountKind;
use suprnova::render_cache::collector::{Collector, current_report};
use suprnova::render_cache::live::{
    LiveDocumentFacts, document_declines, record_document_intent, record_mount,
};
use suprnova::view::{DocumentCachePolicy, DocumentResponseIntent};

#[test]
fn declines_identity_bound_islands_no_store_intents_and_deadline_free_seeds() {
    let public = LiveDocumentFacts {
        public_seed_islands: 1,
        identity_bound_islands: 0,
        seed_deadline_ms: Some(10),
        no_store: false,
    };
    assert!(
        !document_declines(Some(&public)),
        "a public seed with a resolved deadline stores"
    );
    let bound = LiveDocumentFacts {
        identity_bound_islands: 1,
        ..public.clone()
    };
    assert!(
        document_declines(Some(&bound)),
        "an identity-bound island never stores; composite stitching is a later plan"
    );
    let no_store = LiveDocumentFacts {
        no_store: true,
        ..public.clone()
    };
    assert!(
        document_declines(Some(&no_store)),
        "a document that declared NoStore never stores"
    );
    assert!(
        !document_declines(None),
        "a plain route with no Live document is left alone"
    );
    let no_deadline = LiveDocumentFacts {
        seed_deadline_ms: None,
        ..public
    };
    assert!(
        document_declines(Some(&no_deadline)),
        "a seed document without a resolvable deadline is not stored"
    );
}

#[tokio::test]
async fn mount_facts_accumulate_across_multiple_mounts_in_one_request() {
    Collector::scope(async {
        record_mount(LiveMountKind::PublicSeed, Some(500));
        record_mount(LiveMountKind::PublicSeed, Some(200));
        record_mount(LiveMountKind::IdentityBound, None);
        let facts = current_report()
            .expect("collector active")
            .live_document
            .expect("facts recorded");
        assert_eq!(facts.public_seed_islands, 2, "counts add across mounts");
        assert_eq!(
            facts.identity_bound_islands, 1,
            "counts add across mount kinds too"
        );
        assert_eq!(
            facts.seed_deadline_ms,
            Some(200),
            "the deadline takes the minimum of every mounted seed's own deadline"
        );
    })
    .await;
}

#[tokio::test]
async fn a_no_store_document_intent_is_sticky_once_recorded() {
    Collector::scope(async {
        record_mount(LiveMountKind::PublicSeed, Some(500));
        let no_store = DocumentResponseIntent::html(StatusCode::OK)
            .expect("intent")
            .with_cache(DocumentCachePolicy::NoStore);
        record_document_intent(&no_store);
        let public = DocumentResponseIntent::html(StatusCode::OK)
            .expect("intent")
            .with_cache(DocumentCachePolicy::Public);
        record_document_intent(&public);
        let facts = current_report()
            .expect("collector active")
            .live_document
            .expect("facts recorded");
        assert!(
            facts.no_store,
            "no_store stays set once any document in the request declared it, \
             even if a later document in the same request did not"
        );
    })
    .await;
}

#[tokio::test]
#[serial_test::serial]
async fn a_public_seed_document_is_a_hit_until_its_seed_deadline() {
    let harness = boot_with_render_cache_and_live().await;
    let first = dispatch_get(&harness, DOCUMENT_PATH, &[]).await;
    assert_eq!(
        first.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    assert_eq!(public_renders(), 1, "the first GET renders");
    // Also proves R86: a demoted `private, ...` header (the pre-fix
    // behavior on this exact route, since `DocumentResponseIntent::html()`
    // defaults to `Private`) would never contain `s-maxage` at all.
    let cache_control = first.header("cache-control").expect("cache-control");
    assert!(
        cache_control.starts_with("public,"),
        "a public-seed document is not demoted to a private class: {cache_control}"
    );
    let max_age: u64 = cache_control
        .split("max-age=")
        .nth(1)
        .expect("max-age")
        .split(',')
        .next()
        .expect("max-age value")
        .parse()
        .expect("seconds");
    assert!(
        max_age * 1_000 <= public_seed_lifetime_ms(&harness),
        "max-age never outlives the seed: {max_age}"
    );

    let second = dispatch_get(&harness, DOCUMENT_PATH, &[]).await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(
        public_renders(),
        1,
        "the second GET is served from the cache, not a render"
    );

    clock(&harness).advance_ms(public_seed_lifetime_ms(&harness) + 1);
    let expired = dispatch_get(&harness, DOCUMENT_PATH, &[]).await;
    assert_eq!(expired.status, StatusCode::OK);
    assert_eq!(
        public_renders(),
        2,
        "past the seed deadline the entry is dead and a fresh document renders"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn an_identity_bound_dashboard_is_never_stored() {
    let harness = boot_with_render_cache_and_live().await;
    // Sign in on one request, as a login handler would, so the
    // identity-bound render on the next request binds the session that
    // survives the framework's fixation rotation (the same sequence
    // `live_public_seed_actions.rs`'s own identity-bound test uses).
    let login = dispatch_get(&harness, DOCUMENT_PATH, &[("x-test-login", "user-7")]).await;
    assert_eq!(
        login.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&login.body)
    );
    let cookie = login.session_cookie();

    let first = dispatch_get(
        &harness,
        PRIVATE_DOCUMENT_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(
        first.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    assert_eq!(private_renders(), 1, "the first GET renders");

    let again = dispatch_get(
        &harness,
        PRIVATE_DOCUMENT_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(
        again.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&again.body)
    );
    // The route declares `Principal` variance, so `classify`'s own
    // key/value guard is satisfied for two requests from the same user and
    // cannot be what declines the second one (see finding 4): whatever
    // declines it here is `document_declines`'s identity-bound branch and
    // nothing else.
    assert_eq!(
        private_renders(),
        2,
        "an identity-bound document is re-rendered every time, never a hit"
    );
    assert!(
        again.header("age").is_none(),
        "an identity-bound document is never served from a stored entry"
    );
    assert!(
        again.header("etag").is_none(),
        "an identity-bound document was never stored, so it has no validator"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn an_identity_bound_mount_declines_even_when_the_handler_never_calls_render() {
    let harness = boot_with_render_cache_and_live().await;
    let login = dispatch_get(&harness, DOCUMENT_PATH, &[("x-test-login", "user-7")]).await;
    let cookie = login.session_cookie();

    let first = dispatch_get(
        &harness,
        RAW_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(
        first.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    assert_eq!(private_renders(), 1, "the first GET renders");

    let again = dispatch_get(
        &harness,
        RAW_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(again.status, StatusCode::OK);
    // R87: the fact is recorded at `mount`, not at `render` - this route's
    // handler never calls `render` at all, so if the fact were recorded
    // there instead (the brief's original placement), `report.live_document`
    // would be `None` here and this identity-bound mount would be stored
    // and served like any other `PrivateCached` entry with satisfied
    // variance.
    assert_eq!(
        private_renders(),
        2,
        "an identity-bound mount declines even when render is never called"
    );
    assert!(again.header("etag").is_none());
}

#[tokio::test]
#[serial_test::serial]
async fn a_declared_private_cached_route_with_no_identity_read_is_still_cached() {
    // R89: `UNREASONED_PATH` declares `PrivateCached` with `Principal`
    // variance and reads no identity in its handler. `classify` starts
    // from the declared class and only narrows further, so this always
    // produces `(PrivateCached, [])` on every request - a shape the R86
    // invariant must not decline, since the declared class already forced
    // `Principal` variance (Task 14 round 6) and the key is already
    // partitioned by the resolved principal before the render begins.
    let harness = boot_with_render_cache_and_live().await;
    let login = dispatch_get(&harness, DOCUMENT_PATH, &[("x-test-login", "user-7")]).await;
    let cookie = login.session_cookie();

    let first = dispatch_get(
        &harness,
        UNREASONED_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(
        first.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    assert_eq!(unreasoned_renders(), 1, "the first GET renders");

    let again = dispatch_get(
        &harness,
        UNREASONED_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(
        unreasoned_renders(),
        1,
        "a declared-PrivateCached route whose handler reads no identity is a hit for the \
         same signed-in principal, not permanently uncacheable"
    );
    assert!(
        again.header("etag").is_some(),
        "a stored entry carries a validator"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn a_class_narrowed_with_no_attached_reason_is_declined() {
    // Finding 8: `STRIP_PATH` declares `PublicShared` with no variance and
    // reads an identity, which `classify` would normally narrow to
    // `PrivateCached` with `ClassificationReason::PrincipalObserved`
    // attached. The handler immediately strips that reason via the
    // test-only seam, producing exactly the shape
    // `is_unreasoned_private_class`'s call site exists to catch: a class
    // genuinely narrowed away from the declared one, with no reason left
    // for the value guard to check the key against. Since nothing
    // partitions the key here, storing this would let every caller share
    // one entry keyed to nobody in particular.
    let harness = boot_with_render_cache_and_live().await;
    // A session cookie is carried on both requests: without one, every
    // response carries a fresh `Set-Cookie` and is ineligible for storage
    // regardless of classification, which would make this test pass
    // vacuously.
    let login = dispatch_get(&harness, DOCUMENT_PATH, &[("x-test-login", "user-7")]).await;
    let cookie = login.session_cookie();
    let first = dispatch_get(
        &harness,
        STRIP_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(
        first.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    assert_eq!(strip_renders(), 1, "the first GET renders");

    let again = dispatch_get(
        &harness,
        STRIP_PATH,
        &[("x-test-login", "user-7"), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(
        strip_renders(),
        2,
        "a class narrowed with no attached reason must be declined, never stored"
    );
    assert!(again.header("etag").is_none());
}
