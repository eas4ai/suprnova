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

// The streamed canonical form is what the lookup key length-prefixes, so a
// disagreement between the declared length and the written bytes would move
// every digest. Cover every dimension shape, including an application
// dimension whose canonical name is built from two pieces.
fn every_shape() -> VarianceDescriptor {
    let keys = keys_from(3);
    let mut descriptor = VarianceDescriptor::new();
    descriptor
        .declare(
            VarianceDimension::Locale,
            DimensionValue::Public("de-DE".to_owned()),
        )
        .expect("public");
    descriptor
        .declare(
            VarianceDimension::Principal,
            DimensionValue::Private(PrivateMaterial::principal(&keys, "user-7", 1)),
        )
        .expect("private");
    descriptor
        .declare(VarianceDimension::Tenant, DimensionValue::Anonymous)
        .expect("anonymous");
    descriptor
        .declare(
            VarianceDimension::Application("theme_variant".to_owned()),
            DimensionValue::Public("dark".to_owned()),
        )
        .expect("application");
    descriptor
}

// Both canonical forms are compared against bytes written out by hand, not
// against each other: `canonical_bytes` is implemented by `write_canonical`,
// so comparing the two would pass no matter what either of them emitted.
#[test]
fn the_streamed_canonical_form_matches_its_declared_length_and_the_expected_bytes() {
    let descriptor = every_shape();
    let material = PrivateMaterial::principal(&keys_from(3), "user-7", 1);

    // Dimension order is `VarianceDimension`'s declaration order: locale,
    // tenant, principal, then the application dimension.
    let mut expected: Vec<u8> = Vec::new();
    expected.extend_from_slice(&6_u32.to_be_bytes());
    expected.extend_from_slice(b"locale");
    expected.push(1);
    expected.extend_from_slice(&5_u32.to_be_bytes());
    expected.extend_from_slice(b"de-DE");
    expected.extend_from_slice(&6_u32.to_be_bytes());
    expected.extend_from_slice(b"tenant");
    expected.push(3);
    expected.extend_from_slice(&9_u32.to_be_bytes());
    expected.extend_from_slice(b"principal");
    expected.push(2);
    expected.extend_from_slice(material.as_bytes());
    expected.extend_from_slice(&17_u32.to_be_bytes());
    expected.extend_from_slice(b"app:theme_variant");
    expected.push(1);
    expected.extend_from_slice(&4_u32.to_be_bytes());
    expected.extend_from_slice(b"dark");

    let mut streamed: Vec<u8> = Vec::new();
    descriptor.write_canonical(&mut |bytes| streamed.extend_from_slice(bytes));
    assert_eq!(streamed, expected, "the writer emits exactly these bytes");
    assert_eq!(
        descriptor.canonical_bytes(),
        expected,
        "the built form is the same bytes"
    );
    assert_eq!(
        descriptor.canonical_len(),
        expected.len(),
        "canonical_len must predict exactly what write_canonical writes"
    );
}

// The 32 digest bytes in `expected` are recomputed from the same
// derivation, so what this pins is the format and the position of the
// material - a name length, the name, one marker byte, then exactly 32
// bytes - never the digest value itself. The digest is pinned where it
// belongs, against the key fixtures in `render_cache_key.rs`.
#[test]
fn a_private_dimension_writes_its_marker_byte_and_the_material_digest() {
    let material = PrivateMaterial::principal(&keys_from(3), "user-7", 1);
    let mut descriptor = VarianceDescriptor::new();
    descriptor
        .declare(
            VarianceDimension::Principal,
            DimensionValue::Private(material),
        )
        .expect("private");
    let name = "principal";
    let mut expected: Vec<u8> = Vec::new();
    expected.extend_from_slice(&(name.len() as u32).to_be_bytes());
    expected.extend_from_slice(name.as_bytes());
    expected.push(2);
    expected.extend_from_slice(material.as_bytes());
    assert_eq!(
        expected.len(),
        4 + name.len() + 1 + 32,
        "a private dimension is the name, one marker byte, and the digest"
    );
    assert_eq!(descriptor.canonical_bytes(), expected);
    assert_eq!(descriptor.canonical_len(), expected.len());
}

#[test]
fn an_application_dimension_writes_its_prefixed_canonical_name_exactly_once() {
    let mut descriptor = VarianceDescriptor::new();
    descriptor
        .declare(
            VarianceDimension::Application("theme_variant".to_owned()),
            DimensionValue::Public("dark".to_owned()),
        )
        .expect("application");
    let name = "app:theme_variant";
    let mut expected: Vec<u8> = Vec::new();
    expected.extend_from_slice(&(name.len() as u32).to_be_bytes());
    expected.extend_from_slice(name.as_bytes());
    expected.push(1);
    expected.extend_from_slice(&4_u32.to_be_bytes());
    expected.extend_from_slice(b"dark");
    assert_eq!(descriptor.canonical_bytes(), expected);
    assert_eq!(descriptor.canonical_len(), expected.len());
}
