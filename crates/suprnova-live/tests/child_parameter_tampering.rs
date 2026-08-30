//! Adversarial child-parameter binding, integrity, replay, and limit tests.

mod child_parameter_support;

use std::collections::BTreeMap;

use child_parameter_support::{
    EXPIRES, NOW, accepted_parent, digest, issued_child, key_ring, pending_parameters, scope,
};
use suprnova_live::canonical::{CanonicalValue, parse_canonical_value, to_canonical_bytes};
use suprnova_live::child::{
    ChildParameterErrorKind, ChildParameterLimits, ExpectedChildParametersV1,
    PreparedChildParametersV1, verify_child_parameters,
};
use suprnova_live::component::composition::{ChildKey, ChildParameterField, ChildParameterSchema};
use suprnova_live::crypto::{SnapshotKeyRing, SnapshotPurpose};
use suprnova_live::identity::{KeyId, ModelField, UnixMillis};
use suprnova_live::state::ModelCodec;

fn rewritten_envelope(
    encoded: &[u8],
    keys: &SnapshotKeyRing,
    limits: &ChildParameterLimits,
    purpose: SnapshotPurpose,
    mutate: impl FnOnce(&mut BTreeMap<String, CanonicalValue>),
) -> Vec<u8> {
    let mut envelope = parse_canonical_value(encoded, limits.input()).expect("fixture envelope");
    let CanonicalValue::Object(fields) = &mut envelope else {
        panic!("fixture envelope object");
    };
    let CanonicalValue::Object(body) = fields.get_mut("body").expect("fixture body") else {
        panic!("fixture body object");
    };
    mutate(body);
    let canonical_body =
        to_canonical_bytes(fields.get("body").expect("mutated body"), limits.input())
            .expect("canonical body");
    let signed = keys
        .sign(purpose, &canonical_body, NOW)
        .expect("test signature");
    fields.insert(
        "signature".to_owned(),
        CanonicalValue::String(signed.signature().to_base64url()),
    );
    to_canonical_bytes(&envelope, limits.input()).expect("rewritten envelope")
}

fn string_schema(version: u16) -> ChildParameterSchema {
    ChildParameterSchema::new(
        version,
        vec![ChildParameterField::new(
            ModelField::parse("query").expect("field"),
            ModelCodec::String,
            true,
        )],
    )
    .expect("schema")
}

#[tokio::test]
async fn wrong_purpose_key_id_and_root_material_all_fail_before_values_are_exposed() {
    let fixture = issued_child("zig").await;
    let wrong_purpose = rewritten_envelope(
        &fixture.encoded,
        &fixture.keys,
        &fixture.limits,
        SnapshotPurpose::InstanceV1,
        |_| {},
    );
    let error = verify_child_parameters(
        &wrong_purpose,
        &fixture.expected,
        &fixture.keys,
        NOW,
        &fixture.limits,
    )
    .expect_err("snapshot purpose cannot substitute");
    assert_eq!(error.kind(), ChildParameterErrorKind::SignatureInvalid);

    let wrong_key_id = rewritten_envelope(
        &fixture.encoded,
        &fixture.keys,
        &fixture.limits,
        SnapshotPurpose::ChildParametersV1,
        |body| {
            body.insert(
                "key_id".to_owned(),
                CanonicalValue::String("unknown-v1".to_owned()),
            );
        },
    );
    let error = verify_child_parameters(
        &wrong_key_id,
        &fixture.expected,
        &fixture.keys,
        NOW,
        &fixture.limits,
    )
    .expect_err("unknown body key id fails");
    assert_eq!(error.kind(), ChildParameterErrorKind::SignatureInvalid);

    let wrong_root = key_ring("child-v1", 0x99);
    let error = verify_child_parameters(
        &fixture.encoded,
        &fixture.expected,
        &wrong_root,
        NOW,
        &fixture.limits,
    )
    .expect_err("wrong root material fails");
    assert_eq!(error.kind(), ChildParameterErrorKind::SignatureInvalid);
}

#[tokio::test]
async fn parent_scope_instance_revision_and_child_bindings_cannot_cross() {
    let fixture = issued_child("zig").await;
    let wrong_scope = ExpectedChildParametersV1::new(
        scope(0x11),
        fixture.parent_instance.clone(),
        fixture.parent_revision,
        fixture.child_key.clone(),
        fixture.child_contract.clone(),
        string_schema(1),
    );
    let error = verify_child_parameters(
        &fixture.encoded,
        &wrong_scope,
        &fixture.keys,
        NOW,
        &fixture.limits,
    )
    .expect_err("scope cannot cross");
    assert_eq!(error.kind(), ChildParameterErrorKind::BindingMismatch);

    let wrong_instance = ExpectedChildParametersV1::new(
        fixture.parent_scope.clone(),
        child_parameter_support::instance(0x41),
        fixture.parent_revision,
        fixture.child_key.clone(),
        fixture.child_contract.clone(),
        string_schema(1),
    );
    let error = verify_child_parameters(
        &fixture.encoded,
        &wrong_instance,
        &fixture.keys,
        NOW,
        &fixture.limits,
    )
    .expect_err("instance cannot cross");
    assert_eq!(error.kind(), ChildParameterErrorKind::BindingMismatch);

    let superseded = ExpectedChildParametersV1::new(
        fixture.parent_scope.clone(),
        fixture.parent_instance.clone(),
        fixture.parent_revision.checked_next().expect("successor"),
        fixture.child_key.clone(),
        fixture.child_contract.clone(),
        string_schema(1),
    );
    let error = verify_child_parameters(
        &fixture.encoded,
        &superseded,
        &fixture.keys,
        NOW,
        &fixture.limits,
    )
    .expect_err("superseded parent revision cannot replay");
    assert_eq!(
        error.kind(),
        ChildParameterErrorKind::ParentRevisionMismatch
    );

    let wrong_child = ExpectedChildParametersV1::new(
        fixture.parent_scope.clone(),
        fixture.parent_instance.clone(),
        fixture.parent_revision,
        ChildKey::parse("other-child").expect("key"),
        fixture.child_contract.clone(),
        string_schema(1),
    );
    let error = verify_child_parameters(
        &fixture.encoded,
        &wrong_child,
        &fixture.keys,
        NOW,
        &fixture.limits,
    )
    .expect_err("child key cannot cross");
    assert_eq!(error.kind(), ChildParameterErrorKind::BindingMismatch);

    let wrong_contract = ExpectedChildParametersV1::new(
        fixture.parent_scope,
        fixture.parent_instance,
        fixture.parent_revision,
        fixture.child_key,
        digest(0xa0),
        string_schema(1),
    );
    let error = verify_child_parameters(
        &fixture.encoded,
        &wrong_contract,
        &fixture.keys,
        NOW,
        &fixture.limits,
    )
    .expect_err("component contract cannot cross");
    assert_eq!(error.kind(), ChildParameterErrorKind::BindingMismatch);
}

#[tokio::test]
async fn parameter_schema_and_value_hashes_are_independently_enforced() {
    let fixture = issued_child("zig").await;
    let wrong_schema = ExpectedChildParametersV1::new(
        fixture.parent_scope.clone(),
        fixture.parent_instance.clone(),
        fixture.parent_revision,
        fixture.child_key.clone(),
        fixture.child_contract.clone(),
        string_schema(2),
    );
    let error = verify_child_parameters(
        &fixture.encoded,
        &wrong_schema,
        &fixture.keys,
        NOW,
        &fixture.limits,
    )
    .expect_err("schema version and digest are bound");
    assert_eq!(
        error.kind(),
        ChildParameterErrorKind::ParameterSchemaMismatch
    );

    let changed_schema_digest = rewritten_envelope(
        &fixture.encoded,
        &fixture.keys,
        &fixture.limits,
        SnapshotPurpose::ChildParametersV1,
        |body| {
            body.insert(
                "parameter_schema_digest".to_owned(),
                CanonicalValue::String(digest(0xb0).to_base64url()),
            );
        },
    );
    let error = verify_child_parameters(
        &changed_schema_digest,
        &fixture.expected,
        &fixture.keys,
        NOW,
        &fixture.limits,
    )
    .expect_err("re-signed schema digest must match the registered schema");
    assert_eq!(
        error.kind(),
        ChildParameterErrorKind::ParameterSchemaMismatch
    );

    let changed_value = rewritten_envelope(
        &fixture.encoded,
        &fixture.keys,
        &fixture.limits,
        SnapshotPurpose::ChildParametersV1,
        |body| {
            body.insert(
                "parameters".to_owned(),
                child_parameter_support::parameters("substituted"),
            );
        },
    );
    let error = verify_child_parameters(
        &changed_value,
        &fixture.expected,
        &fixture.keys,
        NOW,
        &fixture.limits,
    )
    .expect_err("re-signed values must match the signed digest");
    assert_eq!(
        error.kind(),
        ChildParameterErrorKind::ParameterValueMismatch
    );

    let wrong_type = rewritten_envelope(
        &fixture.encoded,
        &fixture.keys,
        &fixture.limits,
        SnapshotPurpose::ChildParametersV1,
        |body| {
            body.insert(
                "parameters".to_owned(),
                CanonicalValue::Object(BTreeMap::from([(
                    "query".to_owned(),
                    CanonicalValue::Bool(true),
                )])),
            );
        },
    );
    let error = verify_child_parameters(
        &wrong_type,
        &fixture.expected,
        &fixture.keys,
        NOW,
        &fixture.limits,
    )
    .expect_err("signed values still must pass the registered typed schema");
    assert_eq!(error.kind(), ChildParameterErrorKind::InvalidParameters);
}

#[tokio::test]
async fn issue_expiry_and_canonical_input_are_strictly_bounded() {
    let fixture = issued_child("zig").await;
    let expired = verify_child_parameters(
        &fixture.encoded,
        &fixture.expected,
        &fixture.keys,
        UnixMillis::new(EXPIRES.get() + 51),
        &fixture.limits,
    )
    .expect_err("expired envelope fails");
    assert_eq!(expired.kind(), ChildParameterErrorKind::Expired);

    let parent = accepted_parent().await;
    let (pending, _) = pending_parameters("zig");
    let future = PreparedChildParametersV1::new(
        parent.scope.clone(),
        parent.instance.clone(),
        parent.revision,
        pending.clone(),
        UnixMillis::new(NOW.get() + 51),
        UnixMillis::new(NOW.get() + 151),
        fixture.keys.active_key_id().clone(),
        &fixture.limits,
    )
    .expect("bounded future draft");
    let error = future
        .publish(&parent.accepted, &fixture.keys, NOW, &fixture.limits)
        .expect_err("future issue outside skew fails");
    assert_eq!(error.kind(), ChildParameterErrorKind::IssuedInFuture);

    let too_long = PreparedChildParametersV1::new(
        parent.scope.clone(),
        parent.instance.clone(),
        parent.revision,
        pending.clone(),
        NOW,
        UnixMillis::new(NOW.get() + 2_001),
        fixture.keys.active_key_id().clone(),
        &fixture.limits,
    )
    .expect_err("validity is bounded during preparation");
    assert_eq!(too_long.kind(), ChildParameterErrorKind::ValidityTooLong);

    let wrong_key = PreparedChildParametersV1::new(
        parent.scope,
        parent.instance,
        parent.revision,
        pending,
        NOW,
        EXPIRES,
        KeyId::parse("other-v1").expect("key id"),
        &fixture.limits,
    )
    .expect("draft records declared key id");
    let error = wrong_key
        .publish(&parent.accepted, &fixture.keys, NOW, &fixture.limits)
        .expect_err("only active key id may publish");
    assert_eq!(error.kind(), ChildParameterErrorKind::SigningKeyMismatch);

    assert!(ChildParameterLimits::new(*fixture.limits.input(), 60_001, 1).is_err());
    assert!(ChildParameterLimits::new(*fixture.limits.input(), 0, 0).is_err());
    assert!(ChildParameterLimits::new(*fixture.limits.input(), 0, 300_001).is_err());

    let duplicate = br#"{"body":{},"body":{},"signature":"x"}"#;
    let error = verify_child_parameters(
        duplicate,
        &fixture.expected,
        &fixture.keys,
        NOW,
        &fixture.limits,
    )
    .expect_err("duplicate canonical keys fail");
    assert_eq!(error.kind(), ChildParameterErrorKind::DuplicateField);

    let raw_parameters = to_canonical_bytes(&fixture.parameters, fixture.limits.input())
        .expect("raw parameter map canonicalizes");
    let error = verify_child_parameters(
        &raw_parameters,
        &fixture.expected,
        &fixture.keys,
        NOW,
        &fixture.limits,
    )
    .expect_err("raw browser map is not child authority");
    assert_eq!(error.kind(), ChildParameterErrorKind::InvalidEnvelope);
}
