//! Small persisted hostile-input cases promoted from the fuzz boundary contract.

mod protocol_support;

use suprnova_live::canonical::parse_canonical_value;
use suprnova_live::identity::UnixMillis;
use suprnova_live::limits::InputLimits;
use suprnova_live::protocol::{parse_update_request, parse_update_response};
use suprnova_live::snapshot::{ExpectedInstanceV1, verify_instance, verify_seed};

const MALFORMED_CORPUS: &[&[u8]] = &[
    b"",
    b"\0",
    b"{",
    b"[]",
    br#"{"a":1,"a":2}"#,
    br#"{"body":{},"signature":""}"#,
    br#"{"protocol_version":1}"#,
    &[0xff, 0xfe, 0xfd],
];

#[test]
fn persisted_malformed_inputs_are_rejected_without_panicking() {
    let canonical_limits = InputLimits::new(2_048, 8, 128, 512).expect("limits are valid");
    let protocol_limits = protocol_support::limits();
    let keys = protocol_support::snapshot_support::key_ring();
    let schemas = protocol_support::snapshot_support::schema_set();
    let seed_expected = protocol_support::snapshot_support::expected_seed(schemas.clone());
    let instance_expected = ExpectedInstanceV1::new(
        protocol_support::snapshot_support::component_contract(),
        suprnova_live::identity::BuildId::parse("build-2026-08-21").expect("build ID is valid"),
        protocol_support::snapshot_support::route(1),
        suprnova_live::identity::IslandSlot::parse("search-results").expect("slot is valid"),
        suprnova_live::identity::ScopeFingerprint::from_bytes(
            &protocol_support::snapshot_support::bytes::<32>(0x90),
        )
        .expect("scope is valid"),
        schemas,
    );
    let snapshot_limits = protocol_support::snapshot_support::snapshot_limits();

    for bytes in MALFORMED_CORPUS {
        let _ = parse_canonical_value(bytes, &canonical_limits);
        assert!(parse_update_request(bytes, &protocol_limits).is_err());
        assert!(parse_update_response(bytes, &protocol_limits).is_err());
        assert!(
            verify_seed(
                bytes,
                &seed_expected,
                &keys,
                UnixMillis::new(1_010),
                &snapshot_limits,
            )
            .is_err()
        );
        assert!(
            verify_instance(
                bytes,
                &instance_expected,
                &keys,
                UnixMillis::new(1_010),
                &snapshot_limits,
            )
            .is_err()
        );
    }
}

#[test]
fn every_single_byte_signed_seed_mutation_is_rejected() {
    let keys = protocol_support::snapshot_support::key_ring();
    let schemas = protocol_support::snapshot_support::schema_set();
    let expected = protocol_support::snapshot_support::expected_seed(schemas);
    let original = protocol_support::seed_snapshot().into_bytes();
    let limits = protocol_support::snapshot_support::snapshot_limits();

    for index in 0..original.len() {
        let mut mutated = original.clone();
        mutated[index] ^= 1;
        assert!(
            verify_seed(&mutated, &expected, &keys, UnixMillis::new(1_010), &limits,).is_err(),
            "mutation at byte {index} was accepted"
        );
    }
}
