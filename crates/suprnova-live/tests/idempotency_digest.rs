//! Semantic idempotency digest profile tests.

use suprnova_live::identity::{ContentDigest, InstanceId, ScopeFingerprint};
use suprnova_live::protocol::{
    SemanticIdempotencyInputV1, parse_versioned_update_request, semantic_idempotency_digest_v1,
};

mod protocol_support;

fn bytes<const SIZE: usize>(value: u8) -> [u8; SIZE] {
    [value; SIZE]
}

fn request(
    correlation: &str,
    model_proposals: &str,
    operations: &str,
    extensions: &str,
) -> Vec<u8> {
    format!(
        r#"{{
          "snapshot_schema_version":1,
          "snapshot":{{"kind":"instance","envelope":{{"signature":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","body":{{}}}}}},
          "runtime_contract_version":1,
          "protocol_version":1,
          "operations":{operations},
          "model_proposals":{model_proposals},
          "idempotency_key":"MDEyMzQ1Njc4OTo7PD0-Pw",
          "extensions":{extensions},
          "correlation_id":"{correlation}",
          "component":"catalog.search",
          "base_revision":"7"
        }}"#
    )
    .into_bytes()
}

fn digest(encoded: &[u8], authority_byte: u8) -> ContentDigest {
    let request = parse_versioned_update_request(encoded, &protocol_support::limits())
        .expect("semantic request parses");
    let input = SemanticIdempotencyInputV1::new(
        ScopeFingerprint::from_bytes(&bytes::<32>(0x70)).expect("scope is valid"),
        InstanceId::from_bytes(&bytes::<16>(0x71)).expect("instance is valid"),
        ContentDigest::from_bytes(&bytes::<32>(0x72)).expect("contract digest is valid"),
        ContentDigest::from_bytes(&bytes::<32>(authority_byte)).expect("authority digest is valid"),
        &request,
    );
    semantic_idempotency_digest_v1(&input).expect("semantic digest succeeds")
}

#[test]
fn correlation_whitespace_and_object_key_presentation_do_not_change_meaning() {
    let operations = r#"[{"field":"query","kind":"sync_model"},{"arguments":{"page":1},"kind":"invoke_action","name":"search"}]"#;
    let compact = request(
        "EBESExQVFhcYGRobHB0eHw",
        r#"{"query":{"page":1,"text":"rust"}}"#,
        operations,
        r#"{"x_mode":{"a":1,"b":2}}"#,
    );
    let reordered = request(
        "ICEiIyQlJicoKSorLC0uLw",
        r#"{"query":{"text":"rust","page":1}}"#,
        operations,
        r#"{"x_mode":{"b":2,"a":1}}"#,
    );

    assert_eq!(digest(&compact, 0x73), digest(&reordered, 0x73));
}

#[test]
fn every_semantic_authority_proposal_operation_argument_and_extension_changes_the_digest() {
    let operations = r#"[{"field":"query","kind":"sync_model"},{"arguments":{"page":1},"kind":"invoke_action","name":"search"}]"#;
    let base = request(
        "EBESExQVFhcYGRobHB0eHw",
        r#"{"query":{"text":"rust"}}"#,
        operations,
        r#"{"x_mode":"full"}"#,
    );
    let expected = digest(&base, 0x73);

    assert_ne!(expected, digest(&base, 0x74));
    let changed_idempotency = String::from_utf8(base.clone())
        .expect("request is UTF-8")
        .replace("MDEyMzQ1Njc4OTo7PD0-Pw", "ICEiIyQlJicoKSorLC0uLw")
        .into_bytes();
    assert_ne!(expected, digest(&changed_idempotency, 0x73));
    assert_ne!(
        expected,
        digest(
            &request(
                "EBESExQVFhcYGRobHB0eHw",
                r#"{"query":{"text":"zig"}}"#,
                operations,
                r#"{"x_mode":"full"}"#
            ),
            0x73
        )
    );
    assert_ne!(
        expected,
        digest(
            &request(
                "EBESExQVFhcYGRobHB0eHw",
                r#"{"query":{"text":"rust"}}"#,
                r#"[{"arguments":{"page":2},"kind":"invoke_action","name":"search"}]"#,
                r#"{"x_mode":"full"}"#
            ),
            0x73
        )
    );
    assert_ne!(
        expected,
        digest(
            &request(
                "EBESExQVFhcYGRobHB0eHw",
                r#"{"query":{"text":"rust"}}"#,
                operations,
                r#"{"x_mode":"compact"}"#
            ),
            0x73
        )
    );
}

#[test]
fn ordered_operations_are_digest_significant() {
    let proposals = r#"{"page":1,"query":"rust"}"#;
    let first = request(
        "EBESExQVFhcYGRobHB0eHw",
        proposals,
        r#"[{"field":"query","kind":"sync_model"},{"field":"page","kind":"sync_model"},{"arguments":{},"kind":"invoke_action","name":"search"}]"#,
        "{}",
    );
    let reversed = request(
        "EBESExQVFhcYGRobHB0eHw",
        proposals,
        r#"[{"field":"page","kind":"sync_model"},{"field":"query","kind":"sync_model"},{"arguments":{},"kind":"invoke_action","name":"search"}]"#,
        "{}",
    );

    assert_ne!(digest(&first, 0x73), digest(&reversed, 0x73));
}
