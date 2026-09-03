//! Freshness intervals, leases, and HTTP metadata agree with the
//! represented variant and never label private output as shared.

use suprnova_live::render_cache::coherence::{
    FreshnessState, ValidationLease, age_seconds, evaluate_freshness, warning_header,
};
use suprnova_live::render_cache::entry::Validator;
use suprnova_live::render_cache::http::{
    ConditionalOutcome, cache_control_value, evaluate_conditional, vary_value,
};
use suprnova_live::render_cache::variance::{DimensionValue, VarianceDescriptor};
use suprnova_live::render_cache::{
    FreshnessPolicy, RepresentationClass, SharedCachePolicy, VarianceDimension,
};

#[test]
fn freshness_intervals_are_explicit_and_private_output_never_serves_stale() {
    let policy = FreshnessPolicy::new(60_000, 30_000, 120_000).expect("policy");
    let published = 1_000;
    assert_eq!(
        evaluate_freshness(
            &policy,
            RepresentationClass::PublicShared,
            published,
            50_000
        ),
        FreshnessState::Fresh
    );
    assert_eq!(
        evaluate_freshness(
            &policy,
            RepresentationClass::PublicShared,
            published,
            70_000
        ),
        FreshnessState::StaleServable
    );
    assert_eq!(
        evaluate_freshness(
            &policy,
            RepresentationClass::PublicShared,
            published,
            100_000
        ),
        FreshnessState::StaleOnError
    );
    assert_eq!(
        evaluate_freshness(
            &policy,
            RepresentationClass::PublicShared,
            published,
            200_000
        ),
        FreshnessState::Dead
    );
    assert_eq!(
        evaluate_freshness(
            &policy,
            RepresentationClass::PublicShared,
            published,
            published + policy.fresh_ms()
        ),
        FreshnessState::StaleServable,
        "the exact millisecond the fresh window ends is already stale-servable"
    );
    assert_eq!(
        evaluate_freshness(
            &policy,
            RepresentationClass::PublicShared,
            published,
            published + policy.fresh_ms() + policy.stale_servable_ms()
        ),
        FreshnessState::StaleOnError,
        "the exact millisecond the stale-servable window ends is already stale-on-error"
    );
    assert_eq!(
        evaluate_freshness(
            &policy,
            RepresentationClass::PublicShared,
            published,
            published + policy.fresh_ms() + policy.stale_on_error_ms()
        ),
        FreshnessState::Dead,
        "the exact millisecond the stale-on-error window ends is already dead"
    );
    assert_eq!(
        evaluate_freshness(
            &policy,
            RepresentationClass::PrivateCached,
            published,
            70_000
        ),
        FreshnessState::Dead,
        "private output is never served stale"
    );
    assert_eq!(age_seconds(published, 70_000), 69);
    assert_eq!(
        warning_header(FreshnessState::StaleServable),
        Some("110 - \"Response is Stale\"")
    );
    assert_eq!(warning_header(FreshnessState::Fresh), None);
}

#[test]
fn leases_bound_monotonic_age_and_hints_only_shorten() {
    let mut lease = ValidationLease::grant(10_000, 5_000);
    assert!(lease.valid_at(12_000));
    assert!(
        !lease.valid_at(15_000),
        "expiry requires an authority reread"
    );
    lease.hint_invalidate(11_000);
    assert!(!lease.valid_at(11_500), "a hint shortens the lease");
    let mut extended = ValidationLease::grant(10_000, 5_000);
    extended.hint_invalidate(30_000);
    assert!(
        !extended.valid_at(16_000),
        "a late hint never extends validity"
    );
    assert!(
        !ValidationLease::grant(10_000, 5_000).valid_at(9_000),
        "a clock that moves backwards fails closed"
    );
}

#[test]
fn conditional_requests_match_only_the_exact_strong_validator() {
    let validator = Validator::strong_for(b"body");
    let etag = validator.etag();
    assert_eq!(
        evaluate_conditional(Some(&etag), &validator),
        ConditionalOutcome::NotModified
    );
    assert_eq!(
        evaluate_conditional(Some(&format!("\"other\", {etag}")), &validator),
        ConditionalOutcome::NotModified
    );
    assert_eq!(
        evaluate_conditional(Some(&format!("W/{etag}")), &validator),
        ConditionalOutcome::Full,
        "weak comparison never satisfies a strong validator"
    );
    assert_eq!(
        evaluate_conditional(Some("*"), &validator),
        ConditionalOutcome::NotModified
    );
    assert_eq!(
        evaluate_conditional(None, &validator),
        ConditionalOutcome::Full
    );
}

#[test]
fn cache_control_and_vary_agree_with_class_variance_and_seed_deadline() {
    let fresh = FreshnessPolicy::new(60_000, 0, 0).expect("policy");
    assert_eq!(
        cache_control_value(
            RepresentationClass::PublicShared,
            SharedCachePolicy::Private,
            &fresh,
            None
        ),
        "private, max-age=60"
    );
    assert_eq!(
        cache_control_value(
            RepresentationClass::PublicShared,
            SharedCachePolicy::SMaxAge { seconds: 300 },
            &fresh,
            None
        ),
        "public, max-age=60, s-maxage=300"
    );
    assert_eq!(
        cache_control_value(
            RepresentationClass::PrivateCached,
            SharedCachePolicy::SMaxAge { seconds: 300 },
            &fresh,
            None
        ),
        "private, max-age=60",
        "private output is never publicly reusable"
    );
    assert_eq!(
        cache_control_value(
            RepresentationClass::PublicShared,
            SharedCachePolicy::SMaxAge { seconds: 300 },
            &fresh,
            Some(20_000)
        ),
        "public, max-age=20, s-maxage=20",
        "seed deadlines cap external freshness"
    );
    let mut descriptor = VarianceDescriptor::new();
    descriptor
        .declare(
            VarianceDimension::Locale,
            DimensionValue::Public("de".to_owned()),
        )
        .expect("locale");
    descriptor
        .declare(
            VarianceDimension::Encoding,
            DimensionValue::Public("br".to_owned()),
        )
        .expect("encoding");
    assert_eq!(
        vary_value(&descriptor),
        Some("Accept-Encoding, Accept-Language".to_owned())
    );
    assert_eq!(vary_value(&VarianceDescriptor::new()), None);
}
