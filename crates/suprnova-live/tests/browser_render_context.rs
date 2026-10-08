//! Typed browser render-context admission tests.

use serde_json::json;

use suprnova_live::limits::InputLimits;
use suprnova_live::mount::DocumentMountKey;
use suprnova_live::protocol::{
    BrowserRenderContext, ProtocolErrorKind, ProtocolLimitConfig, ProtocolLimits,
    parse_versioned_update_request,
};

fn limits() -> ProtocolLimits {
    ProtocolLimits::new(ProtocolLimitConfig {
        input: InputLimits::new(64 * 1024, 12, 512, 40 * 1024).expect("input limits"),
        max_snapshot_bytes: 32 * 1024,
        max_html_bytes: 32 * 1024,
        max_model_proposals: 8,
        max_operations: 8,
        max_arguments: 16,
        max_validation_entries: 16,
        max_events: 8,
        max_effects: 8,
        max_extensions: 8,
    })
    .expect("protocol limits")
}

fn request(
    version: u16,
    document_key: Option<&str>,
) -> suprnova_live::protocol::VersionedUpdateRequest {
    let mut extensions = serde_json::Map::new();
    if let Some(document_key) = document_key {
        extensions.insert(
            "x_suprnova_live_document_key_v1".to_owned(),
            json!(document_key),
        );
    }
    let mut value = json!({
        "base_revision": "7",
        "component": "catalog.search",
        "correlation_id": "EBESExQVFhcYGRobHB0eHw",
        "extensions": extensions,
        "idempotency_key": "MDEyMzQ1Njc4OTo7PD0-Pw",
        "model_proposals": {},
        "operations": [{"arguments": {}, "kind": "invoke_action", "name": "search"}],
        "protocol_version": version,
        "runtime_contract_version": version,
        "snapshot": {
            "envelope": {
                "body": {},
                "signature": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            },
            "kind": "instance"
        },
        "snapshot_schema_version": 1
    });
    if version == 2 {
        value
            .as_object_mut()
            .expect("request object")
            .insert("child_parameters".to_owned(), serde_json::Value::Null);
    }
    let encoded = serde_json_canonicalizer::to_vec(&value).expect("canonical request");
    parse_versioned_update_request(&encoded, &limits()).expect("parsed request")
}

#[test]
fn v1_and_v2_document_keys_become_typed_inert_render_context() {
    let expected = DocumentMountKey::parse("primary").expect("document key");
    for version in [1, 2] {
        let parsed = request(version, Some("primary"));
        let context =
            BrowserRenderContext::from_request(&parsed, &expected).expect("browser render context");
        assert_eq!(context.document_key(), &expected);
        assert_eq!(context.document_key().as_str(), "primary");
    }
}

#[test]
fn missing_malformed_and_cross_island_document_keys_fail_closed() {
    let expected = DocumentMountKey::parse("primary").expect("document key");
    for parsed in [
        request(1, None),
        request(1, Some("bad/key")),
        request(1, Some("other-island")),
    ] {
        let error = BrowserRenderContext::from_request(&parsed, &expected)
            .expect_err("browser context must fail");
        assert_eq!(error.kind(), ProtocolErrorKind::InvalidExtension);
    }
}
