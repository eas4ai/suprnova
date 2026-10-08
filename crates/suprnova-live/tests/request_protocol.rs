//! Live v1 update-request grammar and bound tests.

mod protocol_support;

use protocol_support::{identity, instance_request, limits, seed_snapshot};
use suprnova_live::identity::Revision;
use suprnova_live::protocol::{Operation, ProtocolErrorKind, SnapshotInput, parse_update_request};

#[test]
fn parses_versioned_instanced_request_without_dispatching_names() {
    let request = parse_update_request(&instance_request(), &limits()).expect("request parses");

    assert_eq!(request.protocol_version(), 1);
    assert_eq!(request.runtime_contract_version(), 1);
    assert_eq!(request.snapshot_schema_version(), 1);
    assert_ne!(
        request.correlation_id().as_bytes(),
        request.idempotency_key().as_bytes()
    );
    assert_eq!(request.base_revision(), Revision::new(7));
    assert!(matches!(request.snapshot(), SnapshotInput::Instance { .. }));
    assert_eq!(request.model_proposals().len(), 1);
    assert!(matches!(
        request.operations()[0],
        Operation::SyncModel { .. }
    ));
    assert!(matches!(
        request.operations()[1],
        Operation::InvokeAction { .. }
    ));
}

#[test]
fn extensions_are_namespaced_and_bounded() {
    let unsafe_extension = String::from_utf8(instance_request())
        .expect("request is UTF-8")
        .replacen("\"extensions\":{}", "\"extensions\":{\"feature\":true}", 1);
    assert_eq!(
        parse_update_request(unsafe_extension.as_bytes(), &limits())
            .expect_err("extensions require explicit namespace")
            .kind(),
        ProtocolErrorKind::InvalidExtension
    );
}

#[test]
fn seed_promotion_is_a_distinct_snapshot_form_with_128_bit_nonce() {
    let encoded = format!(
        r#"{{"base_revision":"0","component":"catalog.search","correlation_id":"{}","extensions":{{}},"idempotency_key":"{}","model_proposals":{{}},"operations":[{{"arguments":{{}},"kind":"invoke_action","name":"search"}}],"protocol_version":1,"runtime_contract_version":1,"snapshot":{{"browser_nonce":"{}","envelope":{},"kind":"seed_promotion"}},"snapshot_schema_version":1}}"#,
        identity::<16>(0x11),
        identity::<16>(0x31),
        identity::<16>(0x51),
        seed_snapshot(),
    );
    let request = parse_update_request(encoded.as_bytes(), &limits()).expect("seed request parses");
    assert!(matches!(
        request.snapshot(),
        SnapshotInput::SeedPromotion { .. }
    ));
}

#[test]
fn duplicate_unknown_and_unsupported_versions_fail_closed() {
    let duplicate = br#"{"protocol_version":1,"protocol_version":1}"#;
    assert_eq!(
        parse_update_request(duplicate, &limits())
            .expect_err("duplicate keys fail")
            .kind(),
        ProtocolErrorKind::DuplicateField
    );

    let unknown = String::from_utf8(instance_request())
        .expect("request is UTF-8")
        .replacen(
            "\"extensions\":{}",
            "\"extensions\":{},\"surprise\":true",
            1,
        );
    assert_eq!(
        parse_update_request(unknown.as_bytes(), &limits())
            .expect_err("unknown fields fail")
            .kind(),
        ProtocolErrorKind::InvalidEnvelope
    );

    let unsupported = String::from_utf8(instance_request())
        .expect("request is UTF-8")
        .replacen("\"protocol_version\":1", "\"protocol_version\":2", 1);
    assert_eq!(
        parse_update_request(unsupported.as_bytes(), &limits())
            .expect_err("breaking version fails")
            .kind(),
        ProtocolErrorKind::UnsupportedVersion
    );
}

#[test]
fn ambiguous_and_semantically_incompatible_batches_are_rejected() {
    let ambiguous = String::from_utf8(instance_request())
        .expect("request is UTF-8")
        .replacen(
            r#"{"arguments":{"page":1},"kind":"invoke_action","name":"search"}"#,
            r#"{"arguments":{"page":1},"field":"query","kind":"invoke_action","name":"search"}"#,
            1,
        );
    assert_eq!(
        parse_update_request(ambiguous.as_bytes(), &limits())
            .expect_err("ambiguous operation fails")
            .kind(),
        ProtocolErrorKind::AmbiguousOperation
    );

    let invalid_order = String::from_utf8(instance_request())
        .expect("request is UTF-8")
        .replace(
            r#"{"field":"query","kind":"sync_model"},{"arguments":{"page":1},"kind":"invoke_action","name":"search"}"#,
            r#"{"arguments":{"page":1},"kind":"invoke_action","name":"search"},{"field":"query","kind":"sync_model"}"#,
        );
    assert_eq!(
        parse_update_request(invalid_order.as_bytes(), &limits())
            .expect_err("model synchronization cannot follow action")
            .kind(),
        ProtocolErrorKind::IncompatibleBatch
    );
}

#[test]
fn model_operation_and_snapshot_limits_are_enforced() {
    let mut small = protocol_support::limits();
    small = small
        .with_max_operations(1)
        .expect("test override is bounded");
    assert_eq!(
        parse_update_request(&instance_request(), &small)
            .expect_err("operation count is bounded")
            .kind(),
        ProtocolErrorKind::TooManyOperations
    );

    let small_snapshot = limits()
        .with_max_snapshot_bytes(32)
        .expect("test override is bounded");
    assert_eq!(
        parse_update_request(&instance_request(), &small_snapshot)
            .expect_err("snapshot bytes are bounded")
            .kind(),
        ProtocolErrorKind::SnapshotTooLarge
    );
}
