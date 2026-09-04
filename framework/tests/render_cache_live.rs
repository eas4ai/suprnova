//! Public-seed Live documents cache as Complete entries capped at the seed
//! deadline; identity-bound islands and no-store intents never store.

mod live_dogfood_support;
mod render_cache_live_support;

use live_dogfood_support::{DOCUMENT_PATH, PRIVATE_DOCUMENT_PATH};
use render_cache_live_support::{
    boot_with_render_cache_and_live, clock, dispatch_get, public_seed_lifetime_ms,
};
use suprnova::StatusCode;
use suprnova::live::LiveMountKind;
use suprnova::render_cache::RepresentationClass;
use suprnova::render_cache::live::{LiveDocumentFacts, document_class};
use suprnova_live::view::DocumentCachePolicy;

#[test]
fn classification_declines_identity_bound_islands_and_no_store_intents() {
    let public = LiveDocumentFacts {
        cache: DocumentCachePolicy::Public,
        public_seed_islands: 1,
        identity_bound_islands: 0,
        seed_deadline_ms: Some(10),
    };
    assert_eq!(
        document_class(Some(&public), RepresentationClass::PublicShared),
        RepresentationClass::PublicShared
    );
    let bound = LiveDocumentFacts {
        identity_bound_islands: 1,
        ..public.clone()
    };
    assert_eq!(
        document_class(Some(&bound), RepresentationClass::PublicShared),
        RepresentationClass::Uncacheable,
        "Composite stitching is a later plan"
    );
    let no_store = LiveDocumentFacts {
        cache: DocumentCachePolicy::NoStore,
        ..public.clone()
    };
    assert_eq!(
        document_class(Some(&no_store), RepresentationClass::PublicShared),
        RepresentationClass::Uncacheable
    );
    let private = LiveDocumentFacts {
        cache: DocumentCachePolicy::Private,
        ..public.clone()
    };
    assert_eq!(
        document_class(Some(&private), RepresentationClass::PublicShared),
        RepresentationClass::PrivateCached
    );
    assert_eq!(
        document_class(None, RepresentationClass::PublicShared),
        RepresentationClass::PublicShared,
        "a plain route keeps its class"
    );
    let no_deadline = LiveDocumentFacts {
        seed_deadline_ms: None,
        ..public
    };
    assert_eq!(
        document_class(Some(&no_deadline), RepresentationClass::PublicShared),
        RepresentationClass::Uncacheable,
        "a seed document without a deadline is not stored"
    );
    let _ = LiveMountKind::PublicSeed;
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
    let max_age: u64 = first
        .header("cache-control")
        .expect("cache-control")
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
    assert_eq!(
        second.body, first.body,
        "the cached document carries the same signed seed"
    );
    assert!(second.header("age").is_some());

    clock(&harness).advance_ms(public_seed_lifetime_ms(&harness) + 1);
    let expired = dispatch_get(&harness, DOCUMENT_PATH, &[]).await;
    assert_eq!(expired.status, StatusCode::OK);
    // A stale-but-otherwise-fresh entry (`fresh_ms` is deliberately far
    // longer than the seed lifetime; see the harness's own doc) would be
    // served unchanged here without the seed-deadline check: same body,
    // and an `Age` header counting up from the original publish rather than
    // `0`. Past the seed deadline the entry is dead instead, so this is a
    // brand-new render: a different signed seed (so a different body) that
    // was itself just published (`Age: 0`), not the frozen dead one.
    assert_ne!(
        expired.body, first.body,
        "past the seed deadline the entry is dead and a fresh document renders"
    );
    assert_eq!(
        expired.header("age"),
        Some("0"),
        "the fresh render was just published, not served from the dead entry"
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
    assert!(
        again.header("age").is_none(),
        "an identity-bound document is never served from a stored entry"
    );
    assert!(
        again.header("etag").is_none(),
        "an identity-bound document was never stored, so it has no validator"
    );
}
