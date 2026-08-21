//! Shared deterministic wire-envelope values for protocol integration tests.

#![allow(
    dead_code,
    reason = "shared helpers are used by separate integration-test crates"
)]

#[path = "snapshot_support.rs"]
pub(crate) mod snapshot_support;

use suprnova_live::identity::UnixMillis;
use suprnova_live::limits::InputLimits;
use suprnova_live::protocol::{ProtocolLimitConfig, ProtocolLimits};
use suprnova_live::snapshot::{InstanceBodyV1, SeedBodyV1};

pub(crate) fn limits() -> ProtocolLimits {
    ProtocolLimits::new(ProtocolLimitConfig {
        input: InputLimits::new(64 * 1024, 12, 512, 40 * 1024).expect("input limits are valid"),
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
    .expect("protocol limits are valid")
}

pub(crate) fn instance_snapshot() -> String {
    let keys = snapshot_support::key_ring();
    let schemas = snapshot_support::schema_set();
    let limits = snapshot_support::snapshot_limits();
    let encoded = InstanceBodyV1::new(snapshot_support::instance_fields(&keys), &schemas, &limits)
        .expect("instance constructs")
        .sign(&keys, UnixMillis::new(1_010), &limits)
        .expect("instance signs");
    String::from_utf8(encoded).expect("snapshot is UTF-8")
}

pub(crate) fn seed_snapshot() -> String {
    let keys = snapshot_support::key_ring();
    let schemas = snapshot_support::schema_set();
    let limits = snapshot_support::snapshot_limits();
    let encoded = SeedBodyV1::new(snapshot_support::seed_fields(&keys), &schemas, &limits)
        .expect("seed constructs")
        .sign(&keys, UnixMillis::new(1_010), &limits)
        .expect("seed signs");
    String::from_utf8(encoded).expect("snapshot is UTF-8")
}

pub(crate) fn identity<const LENGTH: usize>(start: u8) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(std::array::from_fn::<_, LENGTH, _>(
        |index| start.wrapping_add(index as u8),
    ))
}

pub(crate) fn instance_request() -> Vec<u8> {
    format!(
        r#"{{"base_revision":"7","component":"catalog.search","correlation_id":"{}","extensions":{{}},"idempotency_key":"{}","model_proposals":{{"query":"rust"}},"operations":[{{"field":"query","kind":"sync_model"}},{{"arguments":{{"page":1}},"kind":"invoke_action","name":"search"}}],"protocol_version":1,"runtime_contract_version":1,"snapshot":{{"envelope":{},"kind":"instance"}},"snapshot_schema_version":1}}"#,
        identity::<16>(0x10),
        identity::<16>(0x30),
        instance_snapshot(),
    )
    .into_bytes()
}

pub(crate) fn accepted_html_response(html: &str) -> Vec<u8> {
    format!(
        r#"{{"accepted_revision":"8","correlation_id":"{}","effects":[{{"name":"focus","payload":{{"target":"query"}}}}],"events":[{{"name":"saved","payload":{{"id":"1"}}}}],"extensions":{{}},"outcome":"accepted","protocol_version":1,"render":{{"html":{},"kind":"html"}},"snapshot":{},"validation":{{}}}}"#,
        identity::<16>(0x10),
        serde_json::to_string(html).expect("HTML serializes"),
        instance_snapshot(),
    )
    .into_bytes()
}
