//! Snapshot tampering, substitution, time, and compatibility rejection tests.

mod snapshot_support;

use snapshot_support::{
    bytes, component_contract, instance_fields, key_ring, route, schema_set, seed_fields,
    snapshot_limits,
};
use suprnova_live::identity::{BuildId, IslandSlot, ScopeFingerprint, UnixMillis};
use suprnova_live::snapshot::{
    ExpectedInstanceV1, ExpectedSeedV1, InstanceBodyV1, SeedBodyV1, SnapshotErrorKind,
    verify_instance, verify_seed,
};

fn expected() -> ExpectedSeedV1 {
    ExpectedSeedV1::new(
        component_contract(),
        BuildId::parse("build-2026-08-21").expect("build id is valid"),
        route(1),
        IslandSlot::parse("search-results").expect("slot is valid"),
        schema_set(),
    )
}

#[test]
fn changing_any_signed_field_invalidates_integrity_before_hydration() {
    let keys = key_ring();
    let limits = snapshot_limits();
    let encoded = SeedBodyV1::new(seed_fields(&keys), &schema_set(), &limits)
        .expect("valid seed constructs")
        .sign(&keys, UnixMillis::new(1_010), &limits)
        .expect("valid seed signs");
    let tampered = String::from_utf8(encoded)
        .expect("snapshot is UTF-8")
        .replace("rust", "evil")
        .into_bytes();

    let error = verify_seed(
        &tampered,
        &expected(),
        &keys,
        UnixMillis::new(1_050),
        &limits,
    )
    .expect_err("field tampering must fail integrity");

    assert_eq!(error.kind(), SnapshotErrorKind::SignatureInvalid);
}

#[test]
fn valid_snapshots_cannot_cross_component_route_slot_or_build_expectations() {
    let keys = key_ring();
    let limits = snapshot_limits();
    let encoded = SeedBodyV1::new(seed_fields(&keys), &schema_set(), &limits)
        .expect("valid seed constructs")
        .sign(&keys, UnixMillis::new(1_010), &limits)
        .expect("valid seed signs");

    let wrong_route = ExpectedSeedV1::new(
        component_contract(),
        BuildId::parse("build-2026-08-21").expect("build id is valid"),
        route(2),
        IslandSlot::parse("search-results").expect("slot is valid"),
        schema_set(),
    );
    let route_error = verify_seed(
        &encoded,
        &wrong_route,
        &keys,
        UnixMillis::new(1_050),
        &limits,
    )
    .expect_err("signed snapshot is bound to its route");
    assert_eq!(route_error.kind(), SnapshotErrorKind::BindingMismatch);

    let wrong_build = ExpectedSeedV1::new(
        component_contract(),
        BuildId::parse("other-build").expect("build id is valid"),
        route(1),
        IslandSlot::parse("search-results").expect("slot is valid"),
        schema_set(),
    );
    let build_error = verify_seed(
        &encoded,
        &wrong_build,
        &keys,
        UnixMillis::new(1_050),
        &limits,
    )
    .expect_err("signed snapshot is bound to its build");
    assert_eq!(build_error.kind(), SnapshotErrorKind::CompatibilityMismatch);
}

#[test]
fn expired_future_and_overlong_seed_windows_fail_closed() {
    let keys = key_ring();
    let limits = snapshot_limits();
    let encoded = SeedBodyV1::new(seed_fields(&keys), &schema_set(), &limits)
        .expect("valid seed constructs")
        .sign(&keys, UnixMillis::new(1_010), &limits)
        .expect("valid seed signs");

    let expired = verify_seed(
        &encoded,
        &expected(),
        &keys,
        UnixMillis::new(1_551),
        &limits,
    )
    .expect_err("seed is outside age plus skew");
    assert_eq!(expired.kind(), SnapshotErrorKind::Expired);

    let future = verify_seed(&encoded, &expected(), &keys, UnixMillis::new(900), &limits)
        .expect_err("issuance is too far in the future");
    assert_eq!(future.kind(), SnapshotErrorKind::IssuedInFuture);

    let mut fields = seed_fields(&keys);
    fields.max_age_ms = 10_001;
    let overlong =
        SeedBodyV1::new(fields, &schema_set(), &limits).expect_err("seed age cannot exceed policy");
    assert_eq!(overlong.kind(), SnapshotErrorKind::ValidityTooLong);
}

#[test]
fn duplicate_unknown_oversized_and_deep_envelopes_are_rejected_safely() {
    let keys = key_ring();
    let limits = snapshot_limits();

    let duplicate = br#"{"body":null,"body":null,"signature":"x"}"#;
    let duplicate_error = verify_seed(
        duplicate,
        &expected(),
        &keys,
        UnixMillis::new(1_000),
        &limits,
    )
    .expect_err("duplicate envelope keys are rejected");
    assert_eq!(duplicate_error.kind(), SnapshotErrorKind::DuplicateField);

    let unknown = br#"{"body":null,"signature":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","surprise":true}"#;
    let unknown_error = verify_seed(unknown, &expected(), &keys, UnixMillis::new(1_000), &limits)
        .expect_err("unknown envelope fields are rejected");
    assert_eq!(unknown_error.kind(), SnapshotErrorKind::InvalidEnvelope);

    let oversized = vec![b' '; limits.input().max_bytes() + 1];
    let oversized_error = verify_seed(
        &oversized,
        &expected(),
        &keys,
        UnixMillis::new(1_000),
        &limits,
    )
    .expect_err("bytes are bounded before parsing");
    assert_eq!(oversized_error.kind(), SnapshotErrorKind::InputTooLarge);

    let deep =
        br#"{"body":{"x":[[[[[0]]]]]},"signature":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#;
    let deep_error = verify_seed(deep, &expected(), &keys, UnixMillis::new(1_000), &limits)
        .expect_err("depth is bounded before signature work");
    assert_eq!(deep_error.kind(), SnapshotErrorKind::InputTooDeep);
}

#[test]
fn instanced_snapshots_cannot_cross_trusted_scope_and_expire_without_recreation() {
    let keys = key_ring();
    let schemas = schema_set();
    let limits = snapshot_limits();
    let fields = instance_fields(&keys);
    let correct_scope = fields.scope.clone();
    let encoded = InstanceBodyV1::new(fields, &schemas, &limits)
        .expect("valid instance constructs")
        .sign(&keys, UnixMillis::new(1_010), &limits)
        .expect("valid instance signs");
    let base_expected = |scope| {
        ExpectedInstanceV1::new(
            component_contract(),
            BuildId::parse("build-2026-08-21").expect("build id is valid"),
            route(1),
            IslandSlot::parse("search-results").expect("slot is valid"),
            scope,
            schemas.clone(),
        )
    };

    let other_scope = ScopeFingerprint::from_bytes(&bytes::<32>(0xd0)).expect("scope is valid");
    let scope_error = verify_instance(
        &encoded,
        &base_expected(other_scope),
        &keys,
        UnixMillis::new(1_050),
        &limits,
    )
    .expect_err("a valid snapshot cannot cross trusted scope");
    assert_eq!(scope_error.kind(), SnapshotErrorKind::BindingMismatch);

    let expired = verify_instance(
        &encoded,
        &base_expected(correct_scope),
        &keys,
        UnixMillis::new(2_051),
        &limits,
    )
    .expect_err("expired instance requires fresh-render recovery");
    assert_eq!(expired.kind(), SnapshotErrorKind::Expired);
}

#[test]
fn snapshot_debug_output_redacts_signed_state_and_verified_capabilities() {
    let keys = key_ring();
    let limits = snapshot_limits();
    let body =
        SeedBodyV1::new(seed_fields(&keys), &schema_set(), &limits).expect("valid seed constructs");
    let encoded = body
        .clone()
        .sign(&keys, UnixMillis::new(1_010), &limits)
        .expect("valid seed signs");
    let verified = verify_seed(
        &encoded,
        &expected(),
        &keys,
        UnixMillis::new(1_050),
        &limits,
    )
    .expect("valid seed verifies");

    for debug_output in [format!("{body:?}"), format!("{verified:?}")] {
        assert!(debug_output.contains("redacted"));
        assert!(!debug_output.contains("rust"));
        assert!(!debug_output.contains("build-2026-08-21"));
    }
}
