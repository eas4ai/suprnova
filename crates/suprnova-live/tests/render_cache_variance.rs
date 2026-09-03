//! Variance is explicit, private material is opaque, and classification only
//! preserves or reduces sharing.

use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{KeyId, UnixMillis};
use suprnova_live::render_cache::variance::{
    ClassificationReason, DimensionValue, ObservedContext, PrivateMaterial, VarianceDescriptor,
    classify,
};
use suprnova_live::render_cache::{RepresentationClass, VarianceDimension};

fn keys_from(root: u8) -> SnapshotKeyRing {
    let active = KeyRecord::new(
        KeyId::parse("render-cache-test").expect("key id"),
        RootKey::new(vec![root; 32]).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(u64::MAX / 2),
        UnixMillis::new(u64::MAX),
    )
    .expect("key record");
    SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
}

#[test]
fn private_material_is_an_opaque_digest_that_changes_with_permission_version() {
    let keys = keys_from(7);
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
    assert_eq!(
        shown, "<private-material>",
        "debug output never shows digest bytes either: {shown}"
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
fn a_rejected_duplicate_declaration_leaves_the_descriptor_unchanged() {
    let mut descriptor = VarianceDescriptor::new();
    descriptor
        .declare(
            VarianceDimension::Locale,
            DimensionValue::Public("de-DE".to_owned()),
        )
        .expect("first declaration");
    let after_first = descriptor.canonical_bytes();
    let result = descriptor.declare(
        VarianceDimension::Locale,
        DimensionValue::Public("fr-FR".to_owned()),
    );
    assert!(result.is_err(), "a duplicate dimension is rejected");
    assert_eq!(
        descriptor.canonical_bytes(),
        after_first,
        "a rejected declaration must not overwrite the stored value"
    );
}

#[test]
fn classification_only_preserves_or_reduces_sharing() {
    let keys = keys_from(7);
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
    let keys = keys_from(7);
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

#[test]
fn every_variance_dimension_round_trips_through_its_canonical_name_and_rejects_unknown_text() {
    let dimensions = [
        VarianceDimension::Host,
        VarianceDimension::Locale,
        VarianceDimension::Media,
        VarianceDimension::Encoding,
        VarianceDimension::Tenant,
        VarianceDimension::Principal,
        VarianceDimension::FeatureVersion,
        VarianceDimension::ConfigVersion,
        VarianceDimension::Application("checkout".to_owned()),
    ];
    for dimension in dimensions {
        let json = serde_json::to_string(&dimension).expect("serialize");
        let decoded: VarianceDimension = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, dimension, "round trip for {json}");
    }
    assert!(
        serde_json::from_str::<VarianceDimension>("\"nonsense\"").is_err(),
        "an unknown name never parses"
    );
    assert!(
        serde_json::from_str::<VarianceDimension>("\"app:\"").is_err(),
        "a bare app: with no name never parses"
    );
}

#[test]
fn tenant_and_authorization_observations_both_accumulate() {
    let keys = keys_from(7);
    let observed = ObservedContext {
        tenant: Some(PrivateMaterial::tenant(&keys, "tenant-1")),
        authorization_read: true,
        ..ObservedContext::default()
    };
    let outcome = classify(RepresentationClass::PublicShared, &observed);
    assert_eq!(outcome.class, RepresentationClass::PrivateCached);
    assert_eq!(
        outcome.reasons,
        vec![
            ClassificationReason::TenantObserved,
            ClassificationReason::AuthorizationRead,
        ],
        "both reasons are recorded, in evaluation order"
    );
}
