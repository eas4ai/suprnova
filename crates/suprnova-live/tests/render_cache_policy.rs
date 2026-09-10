//! Route policy is deterministic, patches only narrow, and a concrete
//! response's eligibility fails closed.

use std::collections::BTreeSet;

use suprnova_live::render_cache::{
    CoherenceMode, DeclineReason, Eligibility, FailurePolicy, FreshnessPolicy, NegotiatedPolicy,
    PolicyPatch, QueryPolicy, RenderCacheErrorKind, RenderCachePolicy, RepresentationClass,
    ResponseSignals, SharedCachePolicy, StorageLayers, VarianceDimension,
};

fn public_policy() -> RenderCachePolicy {
    RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 30_000, 120_000).expect("bounded freshness"))
        .layers(StorageLayers::l0_and_l1())
        .shared(SharedCachePolicy::Private)
        .failure(FailurePolicy::Open)
        .query(QueryPolicy::declared(["page", "sort"]))
        .vary(VarianceDimension::Locale)
        .build()
        .expect("valid policy")
}

fn get_html() -> ResponseSignals {
    ResponseSignals {
        method: "GET".to_owned(),
        status: 200,
        streaming: false,
        sets_cookie: false,
        content_type: Some("text/html; charset=utf-8".to_owned()),
        header_names: vec!["cache-control".to_owned(), "content-type".to_owned()],
        private_observed: false,
    }
}

#[test]
fn a_public_get_document_is_eligible_for_its_declared_class() {
    let policy = public_policy();
    assert_eq!(
        policy.eligibility(&get_html()),
        Eligibility::Store(RepresentationClass::PublicShared)
    );
    let mut head = get_html();
    head.method = "HEAD".to_owned();
    assert_eq!(
        policy.eligibility(&head),
        Eligibility::Store(RepresentationClass::PublicShared)
    );
}

#[test]
fn state_changing_streaming_cookie_setting_and_error_responses_decline() {
    let policy = public_policy();
    let mut post = get_html();
    post.method = "POST".to_owned();
    assert_eq!(
        policy.eligibility(&post),
        Eligibility::Decline(DeclineReason::Method)
    );
    let mut streaming = get_html();
    streaming.streaming = true;
    assert_eq!(
        policy.eligibility(&streaming),
        Eligibility::Decline(DeclineReason::Streaming)
    );
    let mut cookie = get_html();
    cookie.sets_cookie = true;
    assert_eq!(
        policy.eligibility(&cookie),
        Eligibility::Decline(DeclineReason::SetsCookie)
    );
    for status in [201, 204, 301, 302, 304, 400, 404, 500] {
        let mut other = get_html();
        other.status = status;
        assert_eq!(
            policy.eligibility(&other),
            Eligibility::Decline(DeclineReason::Status),
            "{status}"
        );
    }
    let mut hop = get_html();
    hop.header_names.push("transfer-encoding".to_owned());
    assert_eq!(
        policy.eligibility(&hop),
        Eligibility::Decline(DeclineReason::UnsafeHeader)
    );
}

#[test]
fn an_observed_private_read_downgrades_a_shared_policy_and_never_upgrades() {
    let policy = public_policy();
    let mut private = get_html();
    private.private_observed = true;
    assert_eq!(
        policy.eligibility(&private),
        Eligibility::Store(RepresentationClass::PrivateCached)
    );
    let uncacheable = RenderCachePolicy::builder(RepresentationClass::Uncacheable)
        .build()
        .expect("uncacheable policy");
    assert_eq!(
        uncacheable.eligibility(&get_html()),
        Eligibility::Decline(DeclineReason::PolicyUncacheable)
    );
}

#[test]
fn patches_apply_deterministically_and_can_only_narrow_the_class() {
    // Fix round 6: the engine's `PrivateCached`-empty-variance rule was
    // tightened from "some dimension declared" to "an identity-bearing
    // dimension declared" (see `validate`'s own doc), so narrowing
    // `public_policy()` (which declares only `Locale`) to `PrivateCached`
    // now needs an identity-bearing dimension too, even though this test is
    // about patch narrowing, not variance. Built locally rather than added
    // to `public_policy()` itself, so the other tests sharing that helper
    // are unaffected.
    let group = public_policy()
        .apply(
            &PolicyPatch::default().vary(
                [VarianceDimension::Locale, VarianceDimension::Principal]
                    .into_iter()
                    .collect(),
            ),
        )
        .expect("adding an identity-bearing dimension");
    let route = group
        .apply(&PolicyPatch::default().class(RepresentationClass::PrivateCached))
        .expect("narrowing patch");
    assert_eq!(route.class(), RepresentationClass::PrivateCached);
    let widening = RenderCachePolicy::builder(RepresentationClass::PrivateCached)
        // Fix round 5: `PrivateCached` with no declared variance is
        // rejected at build time (see `validate`'s own doc), so this needs
        // a dimension declared even though this test is about patch
        // narrowing, not variance.
        .vary(VarianceDimension::Principal)
        .build()
        .expect("private")
        .apply(&PolicyPatch::default().class(RepresentationClass::PublicShared));
    assert!(widening.is_err(), "a patch cannot widen sharing");
    let a = group
        .apply(&PolicyPatch::default().freshness(FreshnessPolicy::new(10, 0, 0).expect("f")))
        .expect("patch a");
    let b = group
        .apply(&PolicyPatch::default().freshness(FreshnessPolicy::new(10, 0, 0).expect("f")))
        .expect("patch b");
    assert_eq!(a, b, "the same patch yields the same policy");
    let declared: BTreeSet<&str> = route
        .query()
        .declared_names()
        .iter()
        .map(String::as_str)
        .collect();
    assert_eq!(declared, ["page", "sort"].into_iter().collect());
}

#[test]
fn apply_rejects_a_lease_patch_beyond_the_bound() {
    let group = public_policy();
    let patch = PolicyPatch::default().coherence(CoherenceMode::Lease {
        max_age_ms: 31 * 24 * 60 * 60 * 1000 + 1,
    });
    assert!(
        group.apply(&patch).is_err(),
        "a lease beyond the bound is rejected"
    );
}

#[test]
fn apply_rejects_a_query_patch_beyond_the_declared_bound() {
    let group = public_policy();
    let too_many: Vec<String> = (0..33).map(|index| format!("q{index}")).collect();
    let patch = PolicyPatch::default().query(QueryPolicy::declared(too_many));
    assert!(
        group.apply(&patch).is_err(),
        "more than the declared bound is rejected"
    );
}

// Fix round 6: neither of round 5's new engine rules had a direct test.
// This is the one that stayed in the engine (the other, "FeatureVersion /
// ConfigVersion / Application have no producer here," moved to the host's
// own `variance_descriptor`, since that fact is about the host, not this
// crate).
#[test]
fn private_cached_with_no_identity_bearing_dimension_is_rejected() {
    assert!(
        RenderCachePolicy::builder(RepresentationClass::PrivateCached)
            .build()
            .is_err(),
        "no declared variance at all must be rejected"
    );
    // Fix round 6: round 5's rule checked mere non-emptiness, so a
    // `PrivateCached` policy declaring only a dimension that never resolves
    // to `DimensionValue::Private` - `Media`, say - still built, promising
    // one representation per private key material set while declaring no
    // dimension that could ever hold one. The fixed rule checks for a
    // dimension that can actually hold private material, not just any
    // dimension.
    //
    // Iteration 006: `Media` now needs its own declared closed set
    // (`.vary_media`, not the bare `.vary`), which by itself would also
    // make this `build()` fail - for the *wrong* reason, missing the
    // closed set rather than lacking an identity-bearing dimension. Using
    // `.vary_media` here keeps this test failing for the reason its own
    // comment states.
    assert!(
        RenderCachePolicy::builder(RepresentationClass::PrivateCached)
            .vary_media(NegotiatedPolicy::declared(["text/html"], "text/html").expect("valid"))
            .build()
            .is_err(),
        "a declared dimension that never resolves to private material must still be rejected"
    );
    assert!(
        RenderCachePolicy::builder(RepresentationClass::PrivateCached)
            .vary(VarianceDimension::Principal)
            .build()
            .is_ok(),
        "Principal can hold private material and must be accepted"
    );
    assert!(
        RenderCachePolicy::builder(RepresentationClass::PrivateCached)
            .vary(VarianceDimension::Tenant)
            .build()
            .is_ok(),
        "Tenant can hold private material and must be accepted"
    );
}

#[test]
fn freshness_bounds_are_explicit_and_bounded() {
    assert!(FreshnessPolicy::new(0, 0, 0).is_ok(), "fresh may be zero");
    assert!(
        FreshnessPolicy::new(31 * 24 * 60 * 60 * 1000 + 1, 0, 0).is_err(),
        "fresh above 31 days"
    );
    assert!(FreshnessPolicy::new(1_000, 31 * 24 * 60 * 60 * 1000 + 1, 0).is_err());
}

#[test]
fn a_stitched_class_refuses_a_shared_max_age() {
    let error = RenderCachePolicy::builder(RepresentationClass::PublicShellStitched)
        .shared(SharedCachePolicy::SMaxAge { seconds: 30 })
        .build()
        .map(|_| ())
        .map_err(|e| e.kind());
    assert_eq!(error, Err(RenderCacheErrorKind::PolicyInvalid));
    RenderCachePolicy::builder(RepresentationClass::PublicShellStitched)
        .build()
        .expect("private sharing builds");
}

// ── NegotiatedPolicy: the closed set Media and Encoding negotiate against ──

#[test]
fn declared_rejects_an_empty_or_oversized_set_and_a_default_outside_it() {
    assert!(
        NegotiatedPolicy::declared(Vec::<&str>::new(), "text/html").is_err(),
        "an empty accepted set has nothing to negotiate against"
    );
    let too_many: Vec<String> = (0..17).map(|index| format!("type/{index}")).collect();
    assert!(
        NegotiatedPolicy::declared(too_many.iter().map(String::as_str), "type/0").is_err(),
        "more than MAX_NEGOTIATED_VALUES is rejected"
    );
    assert!(
        NegotiatedPolicy::declared(["text/html", "application/json"], "text/plain").is_err(),
        "a default that is not itself a declared member is rejected"
    );
    assert!(
        NegotiatedPolicy::declared(["Text/HTML"], "Text/HTML").is_err(),
        "an upper-case declared value is rejected: the closed set is canonically lower case"
    );
    assert!(
        NegotiatedPolicy::declared(["text/html"], "text/html").is_ok(),
        "a well-formed single-member set with a matching default builds"
    );
}

#[test]
fn negotiate_is_q_weighted_and_ties_keep_header_order() {
    let media =
        NegotiatedPolicy::declared(["text/html", "application/json"], "text/html").expect("valid");
    assert_eq!(
        media.negotiate(Some("text/html;q=0.5, application/json;q=0.9")),
        "application/json",
        "the higher-quality candidate wins even though it is listed second"
    );
    assert_eq!(
        media.negotiate(Some("application/json, text/html")),
        "application/json",
        "equal (implicit q=1) quality keeps the header's own left-to-right order: \
         the first-listed candidate wins"
    );
    assert_eq!(
        media.negotiate(Some("text/html, application/json")),
        "text/html",
        "and reversing the header order reverses which one wins the tie"
    );
}

#[test]
fn negotiate_falls_back_to_the_default_and_never_widens_the_set() {
    let media =
        NegotiatedPolicy::declared(["text/html", "application/json"], "text/html").expect("valid");
    assert_eq!(
        media.negotiate(None),
        "text/html",
        "an absent header resolves to the declared default"
    );
    assert_eq!(
        media.negotiate(Some("application/xml")),
        "text/html",
        "a value outside the declared set falls back to the default rather than \
         creating a variant for it"
    );
    assert_eq!(
        media.negotiate(Some("*/*")),
        "text/html",
        "a wildcard is compared as a literal token, not expanded against the set, \
         so it falls back to the default"
    );
    assert_eq!(
        media.negotiate(Some("application/json;q=0")),
        "text/html",
        "q=0 explicitly excludes that candidate, so the only other listed value \
         has nothing to beat and the default applies"
    );
    assert_eq!(
        media.negotiate(Some("application/json;q=2, application/json;q=abc")),
        "text/html",
        "an out-of-range or unparsable quality excludes the entry rather than \
         defaulting it to 1.0"
    );
}

#[test]
fn negotiate_never_panics_on_hostile_or_malformed_input() {
    let media = NegotiatedPolicy::declared(["text/html"], "text/html").expect("valid");
    for hostile in [
        "",
        ",",
        ";;;",
        "q=",
        "text/html;",
        "text/html;q=",
        "text/html;q=abc;q=1",
        "\u{0}\u{1}\u{2}",
        &",".repeat(500),
    ] {
        // The only assertion that matters here is that this returns at all
        // rather than panicking; every one of these still resolves to some
        // member of the declared set or the default.
        let value = media.negotiate(Some(hostile));
        assert!(
            value == "text/html",
            "hostile input {hostile:?} must degrade to the declared default, got {value:?}"
        );
    }
    // Well past MAX_NEGOTIATION_ENTRIES (64): only the first 64 comma
    // separated entries are considered, and none of the padding entries
    // name a declared value, so this still resolves to the default rather
    // than doing unbounded work.
    let long_header = format!("{}text/html;q=0.1", "application/json, ".repeat(200));
    assert_eq!(
        media.negotiate(Some(&long_header)),
        "text/html",
        "a header far longer than the considered-entries bound still resolves safely"
    );
}

#[test]
fn vary_media_and_vary_encoding_require_pairing_with_a_declared_set() {
    // A bare `.vary(Media)` with no matching `.vary_media(..)` has nothing
    // to negotiate against and must be rejected at build time.
    assert!(
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .vary(VarianceDimension::Media)
            .build()
            .is_err(),
        "Media declared without its closed set must be rejected"
    );
    assert!(
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .vary(VarianceDimension::Encoding)
            .build()
            .is_err(),
        "Encoding declared without its closed set must be rejected"
    );
    let policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .vary_media(
            NegotiatedPolicy::declared(["text/html", "application/json"], "text/html")
                .expect("valid"),
        )
        .vary_encoding(NegotiatedPolicy::declared(["identity", "gzip"], "identity").expect("valid"))
        .build()
        .expect("both dimensions paired with their declared sets build");
    assert!(policy.vary().contains(&VarianceDimension::Media));
    assert!(policy.vary().contains(&VarianceDimension::Encoding));
    assert_eq!(
        policy.media().map(NegotiatedPolicy::default_value),
        Some("text/html")
    );
    assert_eq!(
        policy.encoding().map(NegotiatedPolicy::default_value),
        Some("identity")
    );
}

#[test]
fn a_vary_patch_dropping_media_also_drops_its_stale_declared_set() {
    let base = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .vary_media(
            NegotiatedPolicy::declared(["text/html", "application/json"], "text/html")
                .expect("valid"),
        )
        .build()
        .expect("base policy");
    assert!(base.media().is_some());
    let narrowed = base
        .apply(&PolicyPatch::default().vary(BTreeSet::new()))
        .expect(
            "dropping Media from vary must also drop its declared set, not leave it \
             behind as inconsistent state",
        );
    assert!(!narrowed.vary().contains(&VarianceDimension::Media));
    assert!(narrowed.media().is_none());
}
