//! Seed and instanced snapshot schema contract tests.

mod snapshot_support;

use std::collections::BTreeMap;

use snapshot_support::{
    component_contract, instance_fields, key_ring, public_value, route, schema_set, seed_fields,
    snapshot_limits,
};
use suprnova_live::identity::{BuildId, IslandSlot, Revision, UnixMillis};
use suprnova_live::snapshot::{
    ExpectedInstanceV1, ExpectedSeedV1, InstanceBodyV1, SeedBodyV1, verify_instance, verify_seed,
};

#[test]
fn seed_schema_round_trips_every_public_binding_without_instance_authority() {
    let keys = key_ring();
    let schemas = schema_set();
    let limits = snapshot_limits();
    let body = SeedBodyV1::new(seed_fields(&keys), &schemas, &limits)
        .expect("valid public seed constructs");
    let encoded = body
        .sign(&keys, UnixMillis::new(1_010), &limits)
        .expect("valid seed signs");
    let expected = ExpectedSeedV1::new(
        component_contract(),
        BuildId::parse("build-2026-08-21").expect("build id is valid"),
        route(1),
        IslandSlot::parse("search-results").expect("slot is valid"),
        schemas,
    );

    let verified = verify_seed(&encoded, &expected, &keys, UnixMillis::new(1_050), &limits)
        .expect("matching signed seed verifies");

    assert_eq!(verified.body().component(), &component_contract());
    assert_eq!(verified.body().route(), &route(1));
    assert_eq!(verified.body().slot().as_str(), "search-results");
    assert_eq!(verified.body().issued_at().get(), 1_000);
    assert_eq!(verified.body().max_age_ms(), 500);
    assert!(verified.body().refresh_on_promote());
    assert_eq!(verified.body().advisory_generations().len(), 1);

    let encoded_text = String::from_utf8(encoded).expect("snapshot envelope is UTF-8 JSON");
    assert!(!encoded_text.contains("instance_id"));
    assert!(!encoded_text.contains("scope"));
    assert!(!encoded_text.contains("tenant-private"));
}

#[test]
fn instanced_schema_round_trips_scope_instance_revision_and_expiry() {
    let keys = key_ring();
    let schemas = schema_set();
    let limits = snapshot_limits();
    let fields = instance_fields(&keys);
    let expected_scope = fields.scope.clone();
    let expected_instance = fields.instance_id.clone();
    let body =
        InstanceBodyV1::new(fields, &schemas, &limits).expect("valid instance snapshot constructs");
    let encoded = body
        .sign(&keys, UnixMillis::new(1_010), &limits)
        .expect("valid instance signs");
    let expected = ExpectedInstanceV1::new(
        component_contract(),
        BuildId::parse("build-2026-08-21").expect("build id is valid"),
        route(1),
        IslandSlot::parse("search-results").expect("slot is valid"),
        expected_scope,
        schemas,
    );

    let verified = verify_instance(&encoded, &expected, &keys, UnixMillis::new(1_050), &limits)
        .expect("matching signed instance verifies");

    assert_eq!(verified.body().instance_id(), &expected_instance);
    assert_eq!(verified.body().revision(), Revision::new(7));
    assert_eq!(verified.body().expires_at(), UnixMillis::new(2_000));
}

#[test]
fn schemas_are_deterministic_for_identical_inputs() {
    let keys = key_ring();
    let schemas = schema_set();
    let limits = snapshot_limits();
    let first = SeedBodyV1::new(seed_fields(&keys), &schemas, &limits)
        .expect("valid seed constructs")
        .sign(&keys, UnixMillis::new(1_010), &limits)
        .expect("valid seed signs");
    let second = SeedBodyV1::new(seed_fields(&keys), &schemas, &limits)
        .expect("valid seed constructs")
        .sign(&keys, UnixMillis::new(1_010), &limits)
        .expect("valid seed signs");

    assert_eq!(first, second);
}

#[test]
fn public_seed_construction_rejects_instance_and_private_shaped_state() {
    let keys = key_ring();
    let schemas = schema_set();
    let limits = snapshot_limits();
    let mut fields = seed_fields(&keys);
    fields.state =
        public_value(r#"{"query":"rust","selected":"1","tenant-private":"must-not-escape"}"#);

    let error = SeedBodyV1::new(fields, &schemas, &limits)
        .expect_err("unknown/private state cannot enter a public seed");

    assert_eq!(error.kind().as_str(), "unknown_state_field");
}

#[test]
fn unsafe_manual_extension_names_are_rejected() {
    let keys = key_ring();
    let schemas = schema_set();
    let limits = snapshot_limits();
    let mut fields = seed_fields(&keys);
    fields.extensions = BTreeMap::from([("signature".to_owned(), public_value("true"))]);

    assert!(SeedBodyV1::new(fields, &schemas, &limits).is_err());
}

#[test]
fn support_helpers_produce_strong_binary_bindings() {
    let fields = instance_fields(&key_ring());
    assert_eq!(fields.scope.as_bytes().len(), 32);
    assert_eq!(fields.instance_id.as_bytes().len(), 16);
}
