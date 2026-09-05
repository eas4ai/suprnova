//! Route and group policy resolution is deterministic: exact route wins,
//! then the longest group prefix; patches narrow; duplicates fail.

use suprnova::render_cache::{
    FreshnessPolicy, PolicyPatch, RenderCachePolicy, RepresentationClass, VarianceDimension,
};
use suprnova::{HttpResponse, Request, Router};

async fn ok(_request: Request) -> suprnova::Response {
    Ok(HttpResponse::text("ok"))
}

fn public() -> RenderCachePolicy {
    RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        // Fix round 5: this file's exact_route_policy_wins... test narrows
        // this policy to PrivateCached with a group patch; PrivateCached
        // with no declared variance is now rejected at build time (see
        // `RenderCachePolicy::validate`'s own doc), so a dimension must be
        // declared here even though this file's assertions are about
        // freshness and class resolution, not variance.
        .vary(VarianceDimension::Principal)
        .build()
        .expect("policy")
}

#[test]
fn exact_route_policy_wins_over_the_longest_group_prefix() {
    let router: Router = Router::new()
        .get("/docs", ok)
        .get("/docs/{page}", ok)
        .get("/docs/private/{page}", ok)
        .into();
    let router = router
        .try_render_cache_group("/docs", public())
        .expect("group")
        .try_render_cache_group(
            "/docs/private",
            PolicyPatch::default().class(RepresentationClass::PrivateCached),
        )
        .expect("nested group")
        .try_render_cache(
            "/docs/{page}",
            PolicyPatch::default().freshness(FreshnessPolicy::new(5_000, 0, 0).expect("f")),
        )
        .expect("route");
    let table = suprnova::render_cache::testing::policy_table(&router);
    assert_eq!(
        table
            .effective_policy("/docs")
            .expect("group")
            .freshness()
            .fresh_ms(),
        60_000
    );
    assert_eq!(
        table
            .effective_policy("/docs/{page}")
            .expect("route")
            .freshness()
            .fresh_ms(),
        5_000
    );
    assert_eq!(
        table
            .effective_policy("/docs/private/{page}")
            .expect("nested")
            .class(),
        RepresentationClass::PrivateCached
    );
    assert!(table.effective_policy("/other").is_none());
}

fn private_group_router() -> Router {
    let router: Router = Router::new().get("/a", ok).into();
    router
        .try_render_cache_group(
            "/",
            RenderCachePolicy::builder(RepresentationClass::PrivateCached)
                .vary(VarianceDimension::Principal)
                .build()
                .expect("p"),
        )
        .expect("group")
}

#[test]
fn duplicates_and_widening_patches_fail_at_construction() {
    assert!(
        private_group_router()
            .try_render_cache_group("/", public())
            .is_err(),
        "duplicate group prefix"
    );
    assert!(
        private_group_router()
            .try_render_cache(
                "/a",
                PolicyPatch::default().class(RepresentationClass::PublicShared)
            )
            .is_err(),
        "a route may not widen its group"
    );
    assert!(
        private_group_router()
            .try_render_cache("/missing", public())
            .is_err(),
        "an unregistered route cannot be opted in"
    );
}

#[test]
fn a_full_route_policy_overrides_a_stricter_enclosing_group() {
    let router = private_group_router()
        .try_render_cache("/a", public())
        .expect("a full policy is a complete override, not a patch");
    let table = suprnova::render_cache::testing::policy_table(&router);
    assert_eq!(
        table.effective_policy("/a").expect("route").class(),
        RepresentationClass::PublicShared
    );
}
