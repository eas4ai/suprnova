//! Signed snapshot-schema-v1 composition-lineage extension tests.

mod snapshot_support;

use std::collections::BTreeMap;

use snapshot_support::{
    bytes, component_contract, instance_fields, key_ring, public_value, route, schema_set,
    seed_fields,
};
use suprnova_live::canonical::{CanonicalValue, parse_canonical_value, to_canonical_bytes};
use suprnova_live::component::composition::ChildKey;
use suprnova_live::crypto::SnapshotPurpose;
use suprnova_live::identity::{BuildId, InstanceId, IslandSlot, Revision, UnixMillis};
use suprnova_live::limits::InputLimits;
use suprnova_live::snapshot::{
    COMPOSITION_LINEAGE_EXTENSION_V1, CompositionChildLineageV1, CompositionLineageV1,
    CompositionOwnerLineageV1, ExpectedInstanceV1, InstanceBodyV1,
    MAX_COMPOSITION_LINEAGE_CHILDREN_V1, MAX_COMPOSITION_LINEAGE_DEPTH_V1, SeedBodyV1,
    SnapshotErrorKind, SnapshotLimits, verify_instance,
};

fn composition_limits() -> SnapshotLimits {
    SnapshotLimits::new(
        InputLimits::new(64 * 1024, 8, 1_024, 512).expect("canonical limits are valid"),
        50,
        10_000,
        20_000,
        8,
        8,
    )
    .expect("snapshot limits are valid")
}

fn large_composition_limits() -> SnapshotLimits {
    SnapshotLimits::new(
        InputLimits::new(256 * 1024, 8, 4_096, 512).expect("canonical limits are valid"),
        50,
        10_000,
        20_000,
        8,
        8,
    )
    .expect("snapshot limits are valid")
}

fn indexed_instance(index: usize) -> InstanceId {
    let mut value = [0xe0; 16];
    value[0..2].copy_from_slice(&(index as u16).to_be_bytes());
    InstanceId::from_bytes(&value).expect("indexed instance is valid")
}

fn child_lineage(
    parent_instance: InstanceId,
    parent_revision: Revision,
    key: &str,
    child_instance: InstanceId,
    depth: u16,
) -> CompositionChildLineageV1 {
    CompositionChildLineageV1::new(
        parent_instance,
        parent_revision,
        ChildKey::parse(key).expect("child key is valid"),
        component_contract().contract_digest().clone(),
        child_instance,
        depth,
    )
    .expect("child lineage is valid")
}

fn signed_parent_snapshot() -> (Vec<u8>, ExpectedInstanceV1) {
    let keys = key_ring();
    let schemas = schema_set();
    let limits = composition_limits();
    let mut fields = instance_fields(&keys);
    let expected_scope = fields.scope.clone();
    let child = child_lineage(
        fields.instance_id.clone(),
        fields.revision,
        "line-item:7",
        indexed_instance(7),
        1,
    );
    fields
        .set_composition_lineage(
            CompositionLineageV1::new(None, vec![child]).expect("lineage is valid"),
        )
        .expect("lineage installs");
    let encoded = InstanceBodyV1::new(fields, &schemas, &limits)
        .expect("snapshot constructs")
        .sign(&keys, UnixMillis::new(1_010), &limits)
        .expect("snapshot signs");
    let expected = ExpectedInstanceV1::new(
        component_contract(),
        BuildId::parse("build-2026-08-21").expect("build id is valid"),
        route(1),
        IslandSlot::parse("search-results").expect("slot is valid"),
        expected_scope,
        schemas,
    );
    (encoded, expected)
}

fn replace_signed_child_binding(encoded: &[u8], field: &str, replacement: String) -> Vec<u8> {
    let limits = composition_limits();
    let mut envelope =
        parse_canonical_value(encoded, limits.input()).expect("snapshot is canonical");
    let CanonicalValue::Object(envelope_fields) = &mut envelope else {
        panic!("snapshot envelope is an object");
    };
    let Some(CanonicalValue::Object(body)) = envelope_fields.get_mut("body") else {
        panic!("snapshot body is an object");
    };
    let Some(CanonicalValue::Object(extensions)) = body.get_mut("extensions") else {
        panic!("snapshot extensions are an object");
    };
    let Some(CanonicalValue::Object(lineage)) =
        extensions.get_mut(COMPOSITION_LINEAGE_EXTENSION_V1)
    else {
        panic!("composition lineage is an object");
    };
    let Some(CanonicalValue::Array(children)) = lineage.get_mut("children") else {
        panic!("composition children are an array");
    };
    let Some(CanonicalValue::Object(binding)) = children.first_mut() else {
        panic!("composition child is an object");
    };
    binding.insert(field.to_owned(), CanonicalValue::String(replacement));
    to_canonical_bytes(&envelope, limits.input()).expect("tampered envelope remains canonical")
}

fn resign_self_directed_child_binding(encoded: &[u8]) -> Vec<u8> {
    let keys = key_ring();
    let limits = composition_limits();
    let mut envelope =
        parse_canonical_value(encoded, limits.input()).expect("snapshot is canonical");
    let CanonicalValue::Object(envelope_fields) = &mut envelope else {
        panic!("snapshot envelope is an object");
    };
    let Some(CanonicalValue::Object(body)) = envelope_fields.get_mut("body") else {
        panic!("snapshot body is an object");
    };
    let parent_instance = match body.get("instance_id") {
        Some(CanonicalValue::String(instance)) => instance.clone(),
        _ => panic!("snapshot has an instance identity"),
    };
    let Some(CanonicalValue::Object(extensions)) = body.get_mut("extensions") else {
        panic!("snapshot extensions are an object");
    };
    let Some(CanonicalValue::Object(lineage)) =
        extensions.get_mut(COMPOSITION_LINEAGE_EXTENSION_V1)
    else {
        panic!("composition lineage is an object");
    };
    let Some(CanonicalValue::Array(children)) = lineage.get_mut("children") else {
        panic!("composition children are an array");
    };
    let Some(CanonicalValue::Object(binding)) = children.first_mut() else {
        panic!("composition child is an object");
    };
    binding.insert(
        "child_instance".to_owned(),
        CanonicalValue::String(parent_instance),
    );

    let canonical_body = to_canonical_bytes(
        envelope_fields.get("body").expect("body remains present"),
        limits.input(),
    )
    .expect("self-directed body remains canonical");
    let signature = keys
        .sign(
            SnapshotPurpose::InstanceV1,
            &canonical_body,
            UnixMillis::new(1_010),
        )
        .expect("test key re-signs raw body");
    envelope_fields.insert(
        "signature".to_owned(),
        CanonicalValue::String(signature.signature().to_base64url()),
    );
    to_canonical_bytes(&envelope, limits.input()).expect("re-signed envelope remains canonical")
}

#[test]
fn public_seed_rejects_instance_only_composition_lineage() {
    let keys = key_ring();
    let schemas = schema_set();
    let limits = composition_limits();
    let mut fields = seed_fields(&keys);
    fields.extensions.insert(
        COMPOSITION_LINEAGE_EXTENSION_V1.to_owned(),
        public_value("null"),
    );

    let error = SeedBodyV1::new(fields, &schemas, &limits)
        .expect_err("public seeds cannot carry instance lineage");

    assert_eq!(error.kind().as_str(), "invalid_extension");
}

#[test]
fn typed_lineage_install_rejects_without_replacing_existing_raw_extension() {
    let keys = key_ring();
    let mut fields = instance_fields(&keys);
    let existing = CanonicalValue::String("preserve-existing-input".to_owned());
    fields.extensions.insert(
        COMPOSITION_LINEAGE_EXTENSION_V1.to_owned(),
        existing.clone(),
    );
    let lineage = CompositionLineageV1::new(
        None,
        vec![child_lineage(
            fields.instance_id.clone(),
            fields.revision,
            "replacement",
            indexed_instance(55),
            1,
        )],
    )
    .expect("lineage is valid");

    assert_eq!(
        fields
            .set_composition_lineage(lineage)
            .expect_err("typed installation never replaces raw extension input")
            .kind(),
        SnapshotErrorKind::InvalidExtension
    );
    assert_eq!(
        fields.extensions.get(COMPOSITION_LINEAGE_EXTENSION_V1),
        Some(&existing)
    );
}

#[test]
fn maximum_depth_child_without_descendants_remains_valid() {
    let child_instance =
        InstanceId::from_bytes(&bytes::<16>(0xd0)).expect("child instance is valid");
    let owner = CompositionOwnerLineageV1::new(
        InstanceId::from_bytes(&bytes::<16>(0xd1)).expect("parent instance is valid"),
        Revision::new(9),
        ChildKey::parse("deep-child").expect("child key is valid"),
        component_contract().contract_digest().clone(),
        child_instance,
        MAX_COMPOSITION_LINEAGE_DEPTH_V1,
    )
    .expect("maximum owner depth is valid");

    CompositionLineageV1::new(Some(owner), vec![])
        .expect("a maximum-depth child needs no impossible descendant depth");
}

#[test]
fn typed_child_and_owner_lineage_reject_exact_self_binding() {
    let instance = indexed_instance(61);
    let child_error = CompositionChildLineageV1::new(
        instance.clone(),
        Revision::new(7),
        ChildKey::parse("self-child").expect("child key"),
        component_contract().contract_digest().clone(),
        instance.clone(),
        1,
    )
    .expect_err("one island cannot be its own immediate child");
    assert_eq!(child_error.kind(), SnapshotErrorKind::InvalidExtension);

    let owner_error = CompositionOwnerLineageV1::new(
        instance.clone(),
        Revision::new(7),
        ChildKey::parse("self-owner").expect("child key"),
        component_contract().contract_digest().clone(),
        instance,
        1,
    )
    .expect_err("one island cannot name itself as its immediate owner");
    assert_eq!(owner_error.kind(), SnapshotErrorKind::InvalidExtension);
}

#[test]
fn verification_rejects_re_signed_raw_self_directed_lineage() {
    let keys = key_ring();
    let limits = composition_limits();
    let (encoded, expected) = signed_parent_snapshot();
    let self_directed = resign_self_directed_child_binding(&encoded);

    assert_eq!(
        verify_instance(
            &self_directed,
            &expected,
            &keys,
            UnixMillis::new(1_050),
            &limits,
        )
        .expect_err("a valid signature cannot authorize self-directed lineage")
        .kind(),
        SnapshotErrorKind::InvalidExtension
    );
}

#[test]
fn signed_lineage_rejects_tampering_and_cross_parent_or_child_substitution() {
    let keys = key_ring();
    let limits = composition_limits();
    let (encoded, expected) = signed_parent_snapshot();

    for tampered in [
        replace_signed_child_binding(
            &encoded,
            "parent_instance",
            indexed_instance(41).to_base64url(),
        ),
        replace_signed_child_binding(
            &encoded,
            "child_instance",
            indexed_instance(42).to_base64url(),
        ),
    ] {
        assert_eq!(
            verify_instance(&tampered, &expected, &keys, UnixMillis::new(1_050), &limits,)
                .expect_err("cross-instance substitution cannot retain the signature")
                .kind(),
            SnapshotErrorKind::SignatureInvalid
        );
    }
}

#[test]
fn duplicate_ambiguous_and_excessive_lineage_fails_before_snapshot_signing() {
    let keys = key_ring();
    let parent_fields = instance_fields(&keys);
    let parent_instance = parent_fields.instance_id;
    let parent_revision = parent_fields.revision;
    let first = child_lineage(
        parent_instance.clone(),
        parent_revision,
        "duplicate",
        indexed_instance(1),
        1,
    );
    let duplicate_key = child_lineage(
        parent_instance.clone(),
        parent_revision,
        "duplicate",
        indexed_instance(2),
        1,
    );
    assert_eq!(
        CompositionLineageV1::new(None, vec![first.clone(), duplicate_key])
            .expect_err("duplicate stable keys are ambiguous")
            .kind(),
        SnapshotErrorKind::InvalidExtension
    );

    let duplicate_instance = child_lineage(
        parent_instance.clone(),
        parent_revision,
        "different-key",
        indexed_instance(1),
        1,
    );
    assert_eq!(
        CompositionLineageV1::new(None, vec![first.clone(), duplicate_instance])
            .expect_err("one child instance cannot occupy two keys")
            .kind(),
        SnapshotErrorKind::InvalidExtension
    );

    let foreign_parent = child_lineage(
        indexed_instance(99),
        parent_revision,
        "foreign-parent",
        indexed_instance(3),
        1,
    );
    assert_eq!(
        CompositionLineageV1::new(None, vec![first, foreign_parent])
            .expect_err("one extension cannot name mixed parent authority")
            .kind(),
        SnapshotErrorKind::InvalidExtension
    );

    assert_eq!(
        CompositionChildLineageV1::new(
            parent_instance.clone(),
            parent_revision,
            ChildKey::parse("too-deep").expect("child key is valid"),
            component_contract().contract_digest().clone(),
            indexed_instance(4),
            MAX_COMPOSITION_LINEAGE_DEPTH_V1 + 1,
        )
        .expect_err("depth above the hard maximum is rejected")
        .kind(),
        SnapshotErrorKind::InvalidExtension
    );

    let children = (0..=MAX_COMPOSITION_LINEAGE_CHILDREN_V1)
        .map(|index| {
            child_lineage(
                parent_instance.clone(),
                parent_revision,
                &format!("child-{index}"),
                indexed_instance(index),
                1,
            )
        })
        .collect();
    assert_eq!(
        CompositionLineageV1::new(None, children)
            .expect_err("first child above the cardinality bound is rejected")
            .kind(),
        SnapshotErrorKind::InvalidExtension
    );
}

#[test]
fn composition_byte_budget_and_unknown_extension_compatibility_are_explicit() {
    let keys = key_ring();
    let schemas = schema_set();
    let large_limits = large_composition_limits();
    let mut oversized_fields = instance_fields(&keys);
    let parent_instance = oversized_fields.instance_id.clone();
    let parent_revision = oversized_fields.revision;
    let children = (0..MAX_COMPOSITION_LINEAGE_CHILDREN_V1)
        .map(|index| {
            let prefix = format!("k{index:03}-");
            let key = format!("{prefix}{}", "a".repeat(128 - prefix.len()));
            child_lineage(
                parent_instance.clone(),
                parent_revision,
                &key,
                indexed_instance(index),
                1,
            )
        })
        .collect();
    oversized_fields
        .set_composition_lineage(
            CompositionLineageV1::new(None, children).expect("cardinality remains bounded"),
        )
        .expect("lineage installs");
    assert_eq!(
        InstanceBodyV1::new(oversized_fields, &schemas, &large_limits)
            .expect_err("composition extension has an independent byte budget")
            .kind(),
        SnapshotErrorKind::InvalidExtension
    );

    let limits = composition_limits();
    let mut future_fields = instance_fields(&keys);
    let expected_scope = future_fields.scope.clone();
    future_fields.extensions.insert(
        "x_example_future_v9".to_owned(),
        CanonicalValue::Object(BTreeMap::from([(
            "future".to_owned(),
            CanonicalValue::String("value".to_owned()),
        )])),
    );
    let body = InstanceBodyV1::new(future_fields, &schema_set(), &limits)
        .expect("unknown namespaced extensions remain compatible");
    let encoded = body
        .sign(&keys, UnixMillis::new(1_010), &limits)
        .expect("unknown extension signs inside the canonical body");
    let expected = ExpectedInstanceV1::new(
        component_contract(),
        BuildId::parse("build-2026-08-21").expect("build id is valid"),
        route(1),
        IslandSlot::parse("search-results").expect("slot is valid"),
        expected_scope,
        schema_set(),
    );
    let verified = verify_instance(&encoded, &expected, &keys, UnixMillis::new(1_050), &limits)
        .expect("unknown namespaced extension verifies under v1 compatibility rules");
    assert!(verified.body().composition_lineage().is_none());
}

#[test]
fn composition_lineage_round_trips_deterministically_inside_the_signed_body() {
    let keys = key_ring();
    let schemas = schema_set();
    let limits = composition_limits();
    let mut fields = instance_fields(&keys);
    let parent_instance = fields.instance_id.clone();
    let parent_revision = fields.revision;
    let child_instance =
        InstanceId::from_bytes(&bytes::<16>(0xc0)).expect("child instance is valid");
    let child = CompositionChildLineageV1::new(
        parent_instance,
        parent_revision,
        ChildKey::parse("line-item:7").expect("child key is valid"),
        component_contract().contract_digest().clone(),
        child_instance.clone(),
        1,
    )
    .expect("child lineage is valid");
    let lineage =
        CompositionLineageV1::new(None, vec![child]).expect("composition lineage is valid");
    fields
        .set_composition_lineage(lineage.clone())
        .expect("typed lineage installs once");
    let expected_scope = fields.scope.clone();

    let body =
        InstanceBodyV1::new(fields, &schemas, &limits).expect("instance snapshot constructs");
    let first = body
        .sign(&keys, UnixMillis::new(1_010), &limits)
        .expect("instance snapshot signs");
    let second = body
        .sign(&keys, UnixMillis::new(1_010), &limits)
        .expect("identical instance snapshot signs");
    assert_eq!(first, second);

    let expected = ExpectedInstanceV1::new(
        component_contract(),
        BuildId::parse("build-2026-08-21").expect("build id is valid"),
        route(1),
        IslandSlot::parse("search-results").expect("slot is valid"),
        expected_scope,
        schemas,
    );
    let verified = verify_instance(&first, &expected, &keys, UnixMillis::new(1_050), &limits)
        .expect("signed lineage verifies");

    assert_eq!(verified.body().composition_lineage(), Some(&lineage));
    assert_eq!(
        verified.body().composition_lineage().unwrap().children()[0].child_instance(),
        &child_instance
    );
}
