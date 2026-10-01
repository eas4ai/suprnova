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
            50_000,
            None
        ),
        FreshnessState::Fresh
    );
    assert_eq!(
        evaluate_freshness(
            &policy,
            RepresentationClass::PublicShared,
            published,
            70_000,
            None
        ),
        FreshnessState::StaleServable
    );
    assert_eq!(
        evaluate_freshness(
            &policy,
            RepresentationClass::PublicShared,
            published,
            100_000,
            None
        ),
        FreshnessState::StaleOnError
    );
    assert_eq!(
        evaluate_freshness(
            &policy,
            RepresentationClass::PublicShared,
            published,
            200_000,
            None
        ),
        FreshnessState::Dead
    );
    assert_eq!(
        evaluate_freshness(
            &policy,
            RepresentationClass::PublicShared,
            published,
            published + policy.fresh_ms(),
            None
        ),
        FreshnessState::StaleServable,
        "the exact millisecond the fresh window ends is already stale-servable"
    );
    assert_eq!(
        evaluate_freshness(
            &policy,
            RepresentationClass::PublicShared,
            published,
            published + policy.fresh_ms() + policy.stale_servable_ms(),
            None
        ),
        FreshnessState::StaleOnError,
        "the exact millisecond the stale-servable window ends is already stale-on-error"
    );
    assert_eq!(
        evaluate_freshness(
            &policy,
            RepresentationClass::PublicShared,
            published,
            published + policy.fresh_ms() + policy.stale_on_error_ms(),
            None
        ),
        FreshnessState::Dead,
        "the exact millisecond the stale-on-error window ends is already dead"
    );
    assert_eq!(
        evaluate_freshness(
            &policy,
            RepresentationClass::PrivateCached,
            published,
            70_000,
            None
        ),
        FreshnessState::Dead,
        "private output is never served stale"
    );
    assert_eq!(
        evaluate_freshness(
            &policy,
            RepresentationClass::PublicShared,
            published,
            50_000,
            Some(50_000)
        ),
        FreshnessState::Dead,
        "a seed deadline reached exactly at now_ms is already dead even though the entry is otherwise fresh"
    );
    assert_eq!(
        evaluate_freshness(
            &policy,
            RepresentationClass::PublicShared,
            published,
            50_000,
            Some(50_001)
        ),
        FreshnessState::Fresh,
        "one millisecond before the seed deadline the entry is still fresh"
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

/// Fix round 1 (R93/F2): `dead_after_ms` is `fresh_ms +
/// max(stale_servable_ms, stale_on_error_ms)`, not `fresh_ms +
/// stale_on_error_ms` - `FreshnessPolicy::new` does not require
/// `stale_on_error_ms >= stale_servable_ms`, so a policy that declares the
/// stale-servable window wider than the stale-on-error window must still
/// use the stale-servable edge as the point of death. `evaluate_freshness`
/// must share this exact arithmetic (not merely agree with it by
/// coincidence), since a retention-based cleanup elsewhere in the system
/// (a file-backed L1's sweep) is built directly on `dead_after_ms` and
/// must never disagree with what this evaluator considers Dead.
#[test]
fn dead_after_ms_is_the_widest_stale_band_not_the_stale_on_error_band_alone() {
    // The ordinary case: stale_on_error_ms is the wider band, so it alone
    // determines the edge, same as `fresh_ms + stale_on_error_ms` would
    // suggest.
    let ordinary = FreshnessPolicy::new(60_000, 60_000, 120_000).expect("policy");
    assert_eq!(
        ordinary.dead_after_ms(RepresentationClass::PublicShared),
        180_000
    );
    assert_eq!(
        evaluate_freshness(
            &ordinary,
            RepresentationClass::PublicShared,
            0,
            179_999,
            None
        ),
        FreshnessState::StaleOnError,
        "one millisecond before the Dead edge, still StaleOnError"
    );
    assert_eq!(
        evaluate_freshness(
            &ordinary,
            RepresentationClass::PublicShared,
            0,
            180_000,
            None
        ),
        FreshnessState::Dead,
        "at the Dead edge exactly, Dead"
    );

    // The reviewer's exact case: FreshnessPolicy::new does NOT require
    // stale_on_error_ms >= stale_servable_ms, and when the stale-servable
    // window is the wider one, it alone determines when the entry is
    // truly dead - `fresh_ms + stale_on_error_ms` (60_000) would be wrong
    // here by a factor of two.
    let inverted = FreshnessPolicy::new(60_000, 120_000, 0).expect("policy accepts this ordering");
    assert_eq!(
        inverted.dead_after_ms(RepresentationClass::PublicShared),
        180_000,
        "the wider stale-servable window determines the Dead edge, not stale_on_error_ms"
    );
    assert_eq!(
        evaluate_freshness(
            &inverted,
            RepresentationClass::PublicShared,
            0,
            60_000 + 60_000 - 1,
            None
        ),
        FreshnessState::StaleServable,
        "at fresh_ms + stale_servable_ms - 1 the entry is still on disk and StaleServable"
    );
    assert_eq!(
        evaluate_freshness(
            &inverted,
            RepresentationClass::PublicShared,
            0,
            180_000,
            None
        ),
        FreshnessState::Dead,
        "at the true Dead edge (180_000, not 60_000), Dead"
    );
}

/// Fix round 2 (R99): `dead_after_ms` is class-aware. `PrivateCached` never
/// gets a stale grace period at all - `evaluate_freshness` puts it at
/// `Dead` the instant it stops being fresh (spec 16 line 78: private
/// entries have bounded retention and eviction independent of public
/// entries) - so for that class the Dead edge is `fresh_ms` alone, not
/// `fresh_ms + max(stale_servable_ms, stale_on_error_ms)`. A `PublicShared`
/// representation under the identical policy is not Dead until the wider
/// edge. This is the reviewer's N4 probe, made permanent.
#[test]
fn dead_after_ms_is_class_aware_private_dies_at_fresh_ms_alone() {
    let policy = FreshnessPolicy::new(60_000, 60_000, 120_000).expect("policy");
    assert_eq!(
        policy.dead_after_ms(RepresentationClass::PrivateCached),
        60_000,
        "PrivateCached has no stale grace period; its Dead edge is fresh_ms alone"
    );
    assert_eq!(
        policy.dead_after_ms(RepresentationClass::PublicShared),
        180_000,
        "PublicShared under the identical policy gets the full stale grace period"
    );
    assert_eq!(
        evaluate_freshness(&policy, RepresentationClass::PrivateCached, 0, 60_000, None),
        FreshnessState::Dead,
        "a private entry is Dead the instant it stops being fresh"
    );
    assert_eq!(
        evaluate_freshness(
            &policy,
            RepresentationClass::PrivateCached,
            0,
            60_000 - 1,
            None
        ),
        FreshnessState::Fresh,
        "one millisecond earlier it is still fresh"
    );
    assert_eq!(
        evaluate_freshness(&policy, RepresentationClass::PublicShared, 0, 60_000, None),
        FreshnessState::StaleServable,
        "a public entry with the same numbers is only stale-servable at the same age"
    );
}

/// Exhaustive equivalence of the rewritten evaluator (which folds the old
/// separate "PrivateCached is always Dead past fresh" branch into the
/// class-aware `dead_after_ms` check - fix round 2, R99) against the
/// original three-branch-plus-private-check logic, over a dense grid
/// covering both classes.
#[test]
fn evaluate_freshness_matches_the_original_three_branch_logic_for_every_class() {
    fn original(
        fresh: u64,
        ss: u64,
        soe: u64,
        class: RepresentationClass,
        age: u64,
    ) -> FreshnessState {
        if age < fresh {
            return FreshnessState::Fresh;
        }
        if class == RepresentationClass::PrivateCached {
            return FreshnessState::Dead;
        }
        let pf = age - fresh;
        if pf < ss {
            return FreshnessState::StaleServable;
        }
        if pf < soe {
            return FreshnessState::StaleOnError;
        }
        FreshnessState::Dead
    }
    let mut checked = 0_u64;
    for fresh in [0_u64, 1, 5, 10] {
        for ss in [0_u64, 1, 3, 7, 12] {
            for soe in [0_u64, 1, 3, 7, 12] {
                let policy = FreshnessPolicy::new(fresh, ss, soe).expect("policy");
                for class in [
                    RepresentationClass::PublicShared,
                    RepresentationClass::PrivateCached,
                ] {
                    for age in 0..40_u64 {
                        assert_eq!(
                            evaluate_freshness(&policy, class, 0, age, None),
                            original(fresh, ss, soe, class, age),
                            "divergence at fresh={fresh} ss={ss} soe={soe} class={class:?} age={age}"
                        );
                        checked += 1;
                    }
                }
            }
        }
    }
    assert_eq!(checked, 4 * 5 * 5 * 2 * 40, "sanity: every grid point ran");
}
