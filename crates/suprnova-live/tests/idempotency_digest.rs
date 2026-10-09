//! Semantic idempotency digest profile tests.

mod ledger_support;

use std::fs;
use std::sync::Arc;

use serde_json::Value;
use suprnova_live::conformance::fixture_directory_v2;
use suprnova_live::identity::{ContentDigest, InstanceId, Revision, ScopeFingerprint};
use suprnova_live::ledger::{
    AcceptedOutcome, AcceptedOutcomeKind, ClaimOutcome, ClaimRequest, LiveInstanceLedger,
};
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

#[tokio::test]
async fn v2_child_carrier_is_idempotency_significant_and_exact_replay_remains_accepted() {
    let fixture: Value = serde_json::from_slice(
        &fs::read(fixture_directory_v2().join("protocol-success.json")).expect("v2 fixture file"),
    )
    .expect("v2 fixture JSON");
    let encoded = fixture["cases"][0]["encoded"]
        .as_str()
        .expect("params request")
        .as_bytes()
        .to_vec();
    let exact_digest = digest(&encoded, 0x73);
    assert_eq!(exact_digest, digest(&encoded, 0x73));

    let mut changed: Value = serde_json::from_slice(&encoded).expect("params request JSON");
    changed["child_parameters"]["envelope"]["body"]["parameters"]["query"] =
        Value::String("different-authority".to_owned());
    let changed = serde_json_canonicalizer::to_vec(&changed).expect("changed canonical request");
    let changed_digest = digest(&changed, 0x73);

    let clock = Arc::new(ledger_support::ManualClock::new(1_000));
    let ledger = ledger_support::ledger(clock, 2);
    let scope = ScopeFingerprint::from_bytes(&bytes::<32>(0x70)).expect("scope is valid");
    let instance = InstanceId::from_bytes(&bytes::<16>(0x71)).expect("instance is valid");
    ledger_support::promote_default(&ledger, scope.clone(), instance.clone()).await;
    let idempotency = ledger_support::idempotency(0x62);
    let grant = match ledger
        .claim(ClaimRequest::new(
            scope.clone(),
            instance.clone(),
            Revision::new(0),
            idempotency.clone(),
            exact_digest.clone(),
        ))
        .await
        .expect("initial claim")
    {
        ClaimOutcome::Granted(grant) => grant,
        other => panic!("expected granted claim, got {other:?}"),
    };
    ledger
        .commit(
            &grant.into_token(),
            AcceptedOutcome::new(AcceptedOutcomeKind::Rendered, exact_digest.clone()),
        )
        .await
        .expect("initial claim commits");

    assert!(matches!(
        ledger
            .claim(ClaimRequest::new(
                scope.clone(),
                instance.clone(),
                Revision::new(0),
                idempotency.clone(),
                exact_digest,
            ))
            .await
            .expect("exact replay classifies"),
        ClaimOutcome::Accepted(_)
    ));
    assert!(matches!(
        ledger
            .claim(ClaimRequest::new(
                scope,
                instance,
                Revision::new(0),
                idempotency,
                changed_digest,
            ))
            .await
            .expect("changed carrier classifies"),
        ClaimOutcome::IdempotencyConflict
    ));
}
