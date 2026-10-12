//! Route and group policy resolution is deterministic: exact route wins,
//! then the longest group prefix; patches narrow; duplicates fail.

use std::collections::BTreeSet;

use suprnova::render_cache::{
    FreshnessPolicy, NegotiatedPolicy, PolicyPatch, RenderCachePolicy, RepresentationClass,
    VarianceDimension,
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

/// The three dimensions that nothing in this host gives a value.
fn dimensions_without_a_producer() -> [VarianceDimension; 3] {
    [
        VarianceDimension::FeatureVersion,
        VarianceDimension::ConfigVersion,
        VarianceDimension::Application("region".to_owned()),
    ]
}

fn varies_on(dimension: VarianceDimension) -> RenderCachePolicy {
    RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .vary(dimension)
        .build()
        .expect("the engine accepts every dimension")
}

fn docs_router() -> Router {
    Router::new().get("/docs/{page}", ok).into()
}

/// What the refusal says. `Router` has no `Debug`, so `expect_err` cannot
/// take the result as it is.
fn refusal(result: Result<Router, suprnova::FrameworkError>) -> String {
    match result {
        Ok(_) => panic!("the policy was registered"),
        Err(refused) => refused.to_string(),
    }
}

/// A route whose policy declares one of these could not build a key, so
/// every request for it went past the cache and nothing said so.
#[test]
fn a_route_policy_that_varies_on_a_dimension_without_a_producer_is_refused() {
    for dimension in dimensions_without_a_producer() {
        let said =
            refusal(docs_router().try_render_cache("/docs/{page}", varies_on(dimension.clone())));
        assert!(said.contains("/docs/{page}"), "names the route: {said}");
        assert!(
            said.contains(&format!("{dimension:?}")),
            "names the dimension: {said}"
        );
        assert!(
            !said.contains("  "),
            "the sentence is one line with one space between its words: {said:?}"
        );
    }
}

#[test]
fn a_group_policy_that_varies_on_a_dimension_without_a_producer_is_refused() {
    for dimension in dimensions_without_a_producer() {
        let said =
            refusal(docs_router().try_render_cache_group("/docs", varies_on(dimension.clone())));
        assert!(said.contains("`/docs`"), "names the group: {said}");
        assert!(
            said.contains(&format!("{dimension:?}")),
            "names the dimension: {said}"
        );
    }
}

/// A patch replaces the whole set of dimensions, so it can bring in a
/// dimension that its group does not have.
#[test]
fn a_patch_that_adds_a_dimension_without_a_producer_is_refused() {
    for dimension in dimensions_without_a_producer() {
        let grouped = docs_router()
            .try_render_cache_group("/docs", public())
            .expect("the group varies on the principal");
        let set = BTreeSet::from([VarianceDimension::Principal, dimension.clone()]);

        let said = refusal(
            grouped.try_render_cache("/docs/{page}", PolicyPatch::default().vary(set.clone())),
        );
        assert!(said.contains("/docs/{page}"), "names the route: {said}");
        assert!(
            said.contains(&format!("{dimension:?}")),
            "names the dimension: {said}"
        );

        let grouped = docs_router()
            .try_render_cache_group("/docs", public())
            .expect("the group varies on the principal");
        let said =
            refusal(grouped.try_render_cache_group("/docs/deep", PolicyPatch::default().vary(set)));
        assert!(
            said.contains("/docs/deep"),
            "names the nested group: {said}"
        );
    }
}

/// The refusal is for the three dimensions alone. Every dimension that
/// has a producer registers, on a route and on a group.
#[test]
fn a_policy_that_varies_on_every_dimension_with_a_producer_is_registered() {
    let policy = || {
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
            .vary(VarianceDimension::Host)
            .vary(VarianceDimension::Locale)
            .vary(VarianceDimension::Tenant)
            .vary(VarianceDimension::Principal)
            .vary_media(
                NegotiatedPolicy::declared(["text/html", "application/json"], "text/html")
                    .expect("media"),
            )
            .vary_encoding(
                NegotiatedPolicy::declared(["identity", "gzip"], "identity").expect("encoding"),
            )
            .build()
            .expect("policy")
    };
    let router = docs_router()
        .try_render_cache_group("/docs", policy())
        .expect("the group is registered")
        .try_render_cache("/docs/{page}", policy())
        .expect("the route is registered");
    let table = suprnova::render_cache::testing::policy_table(&router);
    assert_eq!(
        table
            .effective_policy("/docs/{page}")
            .expect("the route has a policy")
            .vary()
            .len(),
        6
    );
}
