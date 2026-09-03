//! Variance is explicit, private material is opaque, and classification only
//! preserves or reduces sharing.

use suprnova_live::crypto::SnapshotKeyRing;
use suprnova_live::render_cache::variance::{
    ClassificationReason, DimensionValue, ObservedContext, PrivateMaterial, VarianceDescriptor,
    classify,
};
use suprnova_live::render_cache::{RepresentationClass, VarianceDimension};

fn keys() -> SnapshotKeyRing {
    SnapshotKeyRing::from_root_for_test([7_u8; 32])
}

#[test]
fn private_material_is_an_opaque_digest_that_changes_with_permission_version() {
    let keys = keys();
    let alice_v1 = PrivateMaterial::principal(&keys, "user-7", 1);
    let alice_v2 = PrivateMaterial::principal(&keys, "user-7", 2);
    let bob_v1 = PrivateMaterial::principal(&keys, "user-8", 1);
    assert_ne!(alice_v1, alice_v2, "a permission change invalidates");
    assert_ne!(alice_v1, bob_v1);
    assert_eq!(
        alice_v1,
        PrivateMaterial::principal(&keys, "user-7", 1),
        "stable"
    );
    let shown = format!("{alice_v1:?}");
    assert!(
        !shown.contains("user-7"),
        "debug output never shows the identifier: {shown}"
    );
    assert_ne!(
        PrivateMaterial::tenant(&keys, "user-7"),
        alice_v1,
        "purposes are separated"
    );
}

#[test]
fn a_descriptor_orders_dimensions_and_bounds_values() {
    let mut descriptor = VarianceDescriptor::new();
    descriptor
        .declare(
            VarianceDimension::Locale,
            DimensionValue::Public("de-DE".to_owned()),
        )
        .expect("locale");
    descriptor
        .declare(
            VarianceDimension::Encoding,
            DimensionValue::Public("br".to_owned()),
        )
        .expect("encoding");
    assert_eq!(
        descriptor.vary_headers(),
        vec!["Accept-Encoding", "Accept-Language"]
    );
    let oversized = DimensionValue::Public("x".repeat(257));
    assert!(
        descriptor
            .declare(
                VarianceDimension::Application("region".to_owned()),
                oversized
            )
            .is_err()
    );
    let canonical = descriptor.canonical_bytes();
    let mut reordered = VarianceDescriptor::new();
    reordered
        .declare(
            VarianceDimension::Encoding,
            DimensionValue::Public("br".to_owned()),
        )
        .expect("encoding");
    reordered
        .declare(
            VarianceDimension::Locale,
            DimensionValue::Public("de-DE".to_owned()),
        )
        .expect("locale");
    assert_eq!(
        canonical,
        reordered.canonical_bytes(),
        "declaration order does not matter"
    );
}

#[test]
fn classification_only_preserves_or_reduces_sharing() {
    let keys = keys();
    let anonymous = ObservedContext::default();
    let outcome = classify(RepresentationClass::PublicShared, &anonymous);
    assert_eq!(outcome.class, RepresentationClass::PublicShared);
    assert!(outcome.reasons.is_empty());

    let signed_in = ObservedContext {
        principal: Some(PrivateMaterial::principal(&keys, "user-7", 3)),
        ..ObservedContext::default()
    };
    let outcome = classify(RepresentationClass::PublicShared, &signed_in);
    assert_eq!(outcome.class, RepresentationClass::PrivateCached);
    assert_eq!(
        outcome.reasons,
        vec![ClassificationReason::PrincipalObserved]
    );

    let session_read = ObservedContext {
        session_read: true,
        ..ObservedContext::default()
    };
    let outcome = classify(RepresentationClass::PublicShared, &session_read);
    assert_eq!(outcome.class, RepresentationClass::Uncacheable);
    assert_eq!(
        outcome.reasons,
        vec![ClassificationReason::SessionValueRead]
    );

    let mut undeclared = ObservedContext::default();
    undeclared.undeclared_reads.push("cookie:theme".to_owned());
    let outcome = classify(RepresentationClass::PublicShared, &undeclared);
    assert_eq!(outcome.class, RepresentationClass::Uncacheable);
    assert_eq!(
        outcome.reasons,
        vec![ClassificationReason::UndeclaredContext]
    );

    let outcome = classify(RepresentationClass::PrivateCached, &anonymous);
    assert_eq!(
        outcome.class,
        RepresentationClass::PrivateCached,
        "a route never widens"
    );

    let secret = ObservedContext {
        secret_context_read: true,
        ..ObservedContext::default()
    };
    assert_eq!(
        classify(RepresentationClass::PrivateCached, &secret).class,
        RepresentationClass::Uncacheable
    );
}

#[test]
fn anonymous_and_authenticated_variants_cannot_collide() {
    let keys = keys();
    let mut descriptor = VarianceDescriptor::new();
    descriptor
        .declare(VarianceDimension::Principal, DimensionValue::Anonymous)
        .expect("anonymous");
    let mut authenticated = VarianceDescriptor::new();
    authenticated
        .declare(
            VarianceDimension::Principal,
            DimensionValue::Private(PrivateMaterial::principal(&keys, "user-7", 1)),
        )
        .expect("principal");
    assert_ne!(
        descriptor.canonical_bytes(),
        authenticated.canonical_bytes()
    );
}
