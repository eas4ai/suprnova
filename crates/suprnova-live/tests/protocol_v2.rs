//! Protocol-v2 schema separation, lifecycle, child delivery, URL, and compatibility tests.

use std::fs;

use serde_json::{Value, json};
use suprnova_live::conformance::fixture_directory_v2;
use suprnova_live::protocol::{
    CompatibilityDecision, CompatibilityWindow, OperationV2, UrlIntent, VersionSet,
    VersionedUpdateRequest, VersionedUpdateResponse, encode_versioned_update_response,
    parse_versioned_update_request, parse_versioned_update_response,
};

mod protocol_support;

fn fixture(name: &str) -> Value {
    serde_json::from_slice(
        &fs::read(fixture_directory_v2().join(name)).expect("v2 fixture file is readable"),
    )
    .expect("v2 fixture JSON is valid")
}

fn cases(value: &Value) -> &[Value] {
    value["cases"].as_array().expect("cases is an array")
}

fn encoded(case: &Value) -> &[u8] {
    case["encoded"]
        .as_str()
        .expect("encoded is a string")
        .as_bytes()
}

#[test]
fn lifecycle_operations_resolve_only_inside_the_v2_model() {
    let root = fixture("protocol-success.json");
    let expected = [
        OperationV2::ParamsChanged,
        OperationV2::LazyComplete,
        OperationV2::FreshRender,
    ];

    for (case, expected_operation) in cases(&root).iter().take(3).zip(expected) {
        let request = parse_versioned_update_request(encoded(case), &protocol_support::limits())
            .expect("v2 lifecycle request parses");
        let VersionedUpdateRequest::V2(request) = request else {
            panic!("v2 bytes cannot resolve as v1");
        };
        assert_eq!(request.protocol_version(), 2);
        assert_eq!(request.runtime_contract_version(), 2);
        assert_eq!(request.snapshot_schema_version(), 1);
        assert_eq!(request.operations(), &[expected_operation]);
    }
}

#[test]
fn params_changed_requires_signed_child_authority_and_fresh_render_never_replays() {
    let root = fixture("protocol-success.json");
    let params =
        parse_versioned_update_request(encoded(&cases(&root)[0]), &protocol_support::limits())
            .expect("params_changed parses");
    let VersionedUpdateRequest::V2(params) = params else {
        panic!("expected v2");
    };
    assert!(params.child_parameters().is_some());

    let fresh =
        parse_versioned_update_request(encoded(&cases(&root)[2]), &protocol_support::limits())
            .expect("fresh_render parses");
    let VersionedUpdateRequest::V2(fresh) = fresh else {
        panic!("expected v2");
    };
    assert!(fresh.operations()[0].is_recovery_without_replay());
}

#[test]
fn params_changed_parses_the_exact_child_admission_carrier() {
    let root = fixture("protocol-success.json");
    let mut request: Value =
        serde_json::from_slice(encoded(&cases(&root)[0])).expect("params_changed fixture is JSON");
    let envelope = request["child_parameters"].take();
    let parent_snapshot = request["snapshot"]["envelope"].clone();
    request["child_parameters"] = json!({
        "envelope": envelope,
        "parent_snapshot": parent_snapshot,
    });

    let request = parse_versioned_update_request(
        &serde_json::to_vec(&request).expect("carrier request serializes"),
        &protocol_support::limits(),
    )
    .expect("exact child admission carrier parses");
    let VersionedUpdateRequest::V2(request) = request else {
        panic!("expected v2");
    };
    let carrier = request
        .child_parameters()
        .expect("params_changed carries admission authority");
    assert_eq!(
        serde_json::from_slice::<Value>(carrier.envelope()).expect("envelope is canonical JSON"),
        request_fixture_child_envelope(&root),
    );
    assert_eq!(
        serde_json::from_slice::<Value>(carrier.parent_snapshot())
            .expect("parent snapshot is canonical JSON"),
        request_fixture_snapshot(&root),
    );
}

fn request_fixture_child_envelope(root: &Value) -> Value {
    serde_json::from_slice::<Value>(encoded(&cases(root)[0]))
        .expect("fixture is JSON")["child_parameters"]
        .clone()
}

fn request_fixture_snapshot(root: &Value) -> Value {
    serde_json::from_slice::<Value>(encoded(&cases(root)[0])).expect("fixture is JSON")["snapshot"]
        ["envelope"]
        .clone()
}

#[test]
fn v2_responses_carry_typed_children_and_url_intent_and_round_trip() {
    let root = fixture("protocol-success.json");
    for case in cases(&root)
        .iter()
        .filter(|case| case["kind"].as_str() == Some("response"))
    {
        let response = parse_versioned_update_response(encoded(case), &protocol_support::limits())
            .expect("v2 response parses");
        let VersionedUpdateResponse::V2(v2_response) = &response else {
            panic!("v2 bytes cannot resolve as v1");
        };
        match case["id"].as_str().expect("id is a string") {
            "child-delivery-response" => assert_eq!(v2_response.child_deliveries().len(), 1),
            "reflected-url-response" => assert!(matches!(
                v2_response.url_intent(),
                Some(UrlIntent::Reflected { .. })
            )),
            "navigated-url-response" => assert!(matches!(
                v2_response.url_intent(),
                Some(UrlIntent::Navigated { .. })
            )),
            other => panic!("unknown response fixture: {other}"),
        }
        assert_eq!(
            encode_versioned_update_response(&response, &protocol_support::limits())
                .expect("response re-encodes"),
            encoded(case)
        );
    }
}

#[test]
fn v2_window_supports_whole_prior_or_current_triplets_but_rejects_mixed_nodes() {
    let root = fixture("compatibility.json");
    for case in cases(&root) {
        let number = |name| case[name].as_u64().expect("version is an integer") as u16;
        let actual = CompatibilityWindow::v2().evaluate(VersionSet::new(
            number("protocol"),
            number("runtime"),
            number("snapshot"),
        ));
        let expected = match case["expected"].as_str().expect("expected is a string") {
            "compatible" => CompatibilityDecision::Compatible,
            "refresh_document" => CompatibilityDecision::RefreshDocument,
            other => panic!("unknown compatibility result: {other}"),
        };
        assert_eq!(actual, expected);
    }
}
