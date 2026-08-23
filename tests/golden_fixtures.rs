//! Shared Rust-side golden fixture conformance.

mod protocol_support;

use std::collections::HashSet;
use std::fs;

use serde_json::Value;
use suprnova_live::SUPPORTED_PROTOCOL_VERSIONS;
use suprnova_live::canonical::{parse_canonical_value, to_canonical_bytes};
use suprnova_live::conformance::{
    FIXTURE_FILES_V4, FIXTURE_VERSIONS, FixtureVersion, expected_fixture_manifest_sha256_version,
    fixture_directory, fixture_manifest_sha256_version,
};
use suprnova_live::identity::{BuildId, IslandSlot, UnixMillis};
use suprnova_live::protocol::{
    ApplicationStep, CompatibilityDecision, CompatibilityWindow, MorphDisposition, VersionSet,
    application_plan, encode_versioned_update_response, parse_update_response,
    parse_versioned_update_request, parse_versioned_update_response,
};
use suprnova_live::snapshot::{ExpectedInstanceV1, verify_instance, verify_seed};

fn fixture(name: &str) -> Value {
    fixture_version(FixtureVersion::V1, name)
}

fn fixture_version(version: FixtureVersion, name: &str) -> Value {
    serde_json::from_slice(
        &fs::read(fixture_directory(version).join(name)).expect("fixture file is readable"),
    )
    .expect("fixture JSON is valid")
}

fn cases(value: &Value) -> &[Value] {
    assert_eq!(value["schema_version"], 1);
    value["cases"].as_array().expect("cases is an array")
}

fn string<'value>(value: &'value Value, key: &str) -> &'value str {
    value[key].as_str().expect("fixture field is a string")
}

fn array<'value>(value: &'value Value, key: &str) -> &'value [Value] {
    value[key].as_array().expect("fixture field is an array")
}

fn number(value: &Value, key: &str) -> u64 {
    value[key]
        .as_u64()
        .expect("fixture field is a nonnegative integer")
}

fn strings<'value>(value: &'value Value, key: &str) -> Vec<&'value str> {
    array(value, key)
        .iter()
        .map(|entry| entry.as_str().expect("fixture array entry is a string"))
        .collect()
}

fn numbers(value: &Value, key: &str) -> Vec<u64> {
    array(value, key)
        .iter()
        .map(|entry| entry.as_u64().expect("fixture array entry is an integer"))
        .collect()
}

fn assert_unique_case_ids(root: &Value, key: &str) {
    assert!(!array(root, key).is_empty(), "{key} must contain cases");
    let mut seen = HashSet::new();
    for case in array(root, key) {
        let id = string(case, "id");
        assert!(!id.is_empty(), "{key} contains an empty case id");
        assert!(seen.insert(id), "{key} contains duplicate case id {id}");
    }
}

fn json_metrics(
    value: &Value,
    depth: usize,
    entries: &mut usize,
    maximum_depth: &mut usize,
    maximum_string_bytes: &mut usize,
) {
    *maximum_depth = (*maximum_depth).max(depth);
    match value {
        Value::Array(values) => {
            *entries += values.len();
            for value in values {
                json_metrics(
                    value,
                    depth + 1,
                    entries,
                    maximum_depth,
                    maximum_string_bytes,
                );
            }
        }
        Value::Object(values) => {
            *entries += values.len();
            for (key, value) in values {
                *maximum_string_bytes = (*maximum_string_bytes).max(key.len());
                json_metrics(
                    value,
                    depth + 1,
                    entries,
                    maximum_depth,
                    maximum_string_bytes,
                );
            }
        }
        Value::String(value) => {
            *maximum_string_bytes = (*maximum_string_bytes).max(value.len());
        }
        Value::Bool(_) | Value::Null | Value::Number(_) => {}
    }
}

fn assert_codec_semantics(
    root: &Value,
    cases_key: &str,
    expected_limits: [u64; 4],
    maximum_payload_bytes: Option<u64>,
) {
    let limits = &root["codec_limits"];
    let limit_keys = ["max_bytes", "max_depth", "max_entries", "max_string_bytes"];
    for (key, expected) in limit_keys.into_iter().zip(expected_limits) {
        assert_eq!(number(limits, key), expected, "unexpected {key}");
        assert!(expected > 0, "{key} must be positive");
    }
    assert_eq!(
        limits
            .as_object()
            .expect("codec limits are an object")
            .len(),
        if maximum_payload_bytes.is_some() {
            5
        } else {
            4
        }
    );
    if let Some(expected) = maximum_payload_bytes {
        assert_eq!(number(limits, "max_payload_bytes"), expected);
        assert!(expected > 0, "max_payload_bytes must be positive");
    }

    let canonical_limits = suprnova_live::limits::InputLimits::default();
    for case in array(root, cases_key) {
        let encoded = string(case, "encoded");
        assert!(
            encoded.len() <= number(limits, "max_bytes") as usize,
            "fixture {} exceeds max_bytes",
            string(case, "id")
        );
        let decoded: Value = serde_json::from_str(encoded).expect("encoded fixture parses as JSON");
        let mut entries = 0;
        let mut maximum_depth = 0;
        let mut maximum_string_bytes = 0;
        json_metrics(
            &decoded,
            1,
            &mut entries,
            &mut maximum_depth,
            &mut maximum_string_bytes,
        );
        assert!(entries <= number(limits, "max_entries") as usize);
        assert!(maximum_depth <= number(limits, "max_depth") as usize);
        assert!(maximum_string_bytes <= number(limits, "max_string_bytes") as usize);
        if let Some(maximum_payload_bytes) = maximum_payload_bytes {
            let payload = serde_json_canonicalizer::to_vec(&decoded["payload"])
                .expect("async payload canonicalizes");
            assert!(payload.len() <= maximum_payload_bytes as usize);
        }
        if string(case, "expected") == "accepted" {
            let parsed = parse_canonical_value(encoded.as_bytes(), &canonical_limits)
                .expect("accepted encoded fixture parses canonically");
            assert_eq!(
                to_canonical_bytes(&parsed, &canonical_limits)
                    .expect("accepted encoded fixture canonicalizes"),
                encoded.as_bytes(),
                "accepted fixture {} is not canonical",
                string(case, "id")
            );
        }
    }
}

#[test]
fn fixture_manifest_is_complete_and_hashable() {
    assert_eq!(FIXTURE_VERSIONS.len(), 4);
    assert_eq!(FixtureVersion::V3.files().len(), 9);
    assert_eq!(FixtureVersion::V4.files().len(), 7);
    for version in FIXTURE_VERSIONS {
        assert!(!version.files().is_empty());
        assert_eq!(
            fixture_manifest_sha256_version(*version).expect("fixtures hash"),
            expected_fixture_manifest_sha256_version(*version).expect("reviewed hash")
        );
    }
}

#[test]
fn version_four_is_an_independent_capability_fixture_set() {
    assert_eq!(
        FIXTURE_FILES_V4,
        &[
            "async-envelope.json",
            "compatibility.json",
            "diagnostics.json",
            "directive-grammar.json",
            "resource-lifecycle.json",
            "runtime-features.json",
            "upload-protocol.json",
        ]
    );
    assert_eq!(SUPPORTED_PROTOCOL_VERSIONS, &[1, 2]);
}

#[test]
fn version_four_case_ids_and_hard_bounds_are_closed() {
    for (name, collections) in [
        ("compatibility.json", &["cases"][..]),
        ("diagnostics.json", &["redaction_cases"][..]),
        ("resource-lifecycle.json", &["cases"][..]),
        (
            "upload-protocol.json",
            &["codec_cases", "transition_cases"][..],
        ),
        (
            "async-envelope.json",
            &["envelope_cases", "continuity_cases"][..],
        ),
    ] {
        let root = fixture_version(FixtureVersion::V4, name);
        for collection in collections {
            assert_unique_case_ids(&root, collection);
        }
    }

    let resources = fixture_version(FixtureVersion::V4, "resource-lifecycle.json");
    let bounds = &resources["bounds"];
    assert_eq!(
        [
            number(bounds, "max_items"),
            number(bounds, "max_bytes"),
            number(bounds, "max_active"),
        ],
        [2, 8, 1]
    );
    for case in array(&resources, "cases") {
        let (mut retained_items, mut retained_bytes, mut active) = (0_u64, 0_u64, 0_u64);
        for operation in array(case, "operations") {
            match string(operation, "operation") {
                "enqueue" if string(operation, "expected") == "accepted" => {
                    retained_items += 1;
                    retained_bytes += number(operation, "bytes");
                    assert!(retained_items <= number(bounds, "max_items"));
                    assert!(retained_bytes <= number(bounds, "max_bytes"));
                }
                "acquire" if string(operation, "expected") == "acquired" => {
                    active += 1;
                    assert!(active <= number(bounds, "max_active"));
                }
                "release" => active = active.checked_sub(1).expect("permit was acquired"),
                "retire" => {
                    let expected = &operation["expected"];
                    assert_eq!(number(expected, "drained_items"), retained_items);
                    assert_eq!(number(expected, "drained_bytes"), retained_bytes);
                    assert_eq!(number(expected, "released_permits"), active);
                    (retained_items, retained_bytes, active) = (0, 0, 0);
                }
                _ => {}
            }
        }
    }

    let features = fixture_version(FixtureVersion::V4, "runtime-features.json");
    let registry = &features["registry"];
    assert_eq!(number(registry, "maximum_features"), 2);
    assert_eq!(number(registry, "maximum_pending_registrations"), 2);
    assert!(number(registry, "maximum_features") > 0);
    assert!(number(registry, "maximum_pending_registrations") > 0);
    assert!(array(&features, "features").len() <= number(registry, "maximum_features") as usize);

    let diagnostics = fixture_version(FixtureVersion::V4, "diagnostics.json");
    let retention = &diagnostics["retention"];
    assert_eq!(number(retention, "maximum_entries"), 256);
    assert!(number(retention, "maximum_entries") > 0);
    assert!(
        array(&diagnostics, "redaction_cases").len()
            <= number(retention, "maximum_entries") as usize
    );
}

#[test]
fn version_four_encoded_cases_are_canonical_and_within_exact_limits() {
    let uploads = fixture_version(FixtureVersion::V4, "upload-protocol.json");
    assert_codec_semantics(&uploads, "codec_cases", [16_384, 8, 64, 4_096], None);

    let asynchronous = fixture_version(FixtureVersion::V4, "async-envelope.json");
    assert_codec_semantics(
        &asynchronous,
        "envelope_cases",
        [65_536, 8, 64, 4_096],
        Some(32_768),
    );
}

#[test]
fn version_four_idempotent_upload_retries_are_self_contained() {
    let uploads = fixture_version(FixtureVersion::V4, "upload-protocol.json");
    let retries: Vec<_> = array(&uploads, "transition_cases")
        .iter()
        .filter(|case| string(case, "expected") == "existing_outcome")
        .collect();
    assert!(!retries.is_empty(), "fixture must cover idempotent retry");

    for case in retries {
        let retry = case["retry"]
            .as_object()
            .expect("existing outcome includes retry context");
        let request = retry["request"]
            .as_object()
            .expect("retry includes the complete repeated request");
        let recorded = retry["recorded_outcome"]
            .as_object()
            .expect("retry includes its recorded prior outcome");
        assert_eq!(request["operation"], case["operation"]);
        assert_eq!(request["expected_revision"], case["expected_revision"]);
        assert_eq!(request["chunk_index"], case["chunk_index"]);
        assert_eq!(request["idempotency_key"], case["idempotency_key"]);
        assert!(request["chunk_index"].as_u64().is_some());
        assert!(
            !request["idempotency_key"]
                .as_str()
                .expect("retry idempotency key is a string")
                .is_empty()
        );
        assert_eq!(recorded["disposition"], "applied");
        assert_eq!(recorded["to"], case["to"]);
        assert_eq!(recorded["next_revision"], retry["current_revision"]);
        assert_eq!(retry["current_revision"], case["next_revision"]);
        let expected_revision: u64 = string(case, "expected_revision")
            .parse()
            .expect("expected revision is decimal");
        let current_revision: u64 = retry["current_revision"]
            .as_str()
            .expect("current revision is a string")
            .parse()
            .expect("current revision is decimal");
        assert!(expected_revision < current_revision);
    }
}

#[test]
fn version_four_protocols_and_promoted_directives_are_consistent() {
    let uploads = fixture_version(FixtureVersion::V4, "upload-protocol.json");
    let asynchronous = fixture_version(FixtureVersion::V4, "async-envelope.json");
    assert_eq!(numbers(&uploads, "protocol_versions"), [1]);
    assert_eq!(numbers(&asynchronous, "protocol_versions"), [1]);
    assert_eq!(numbers(&uploads, "live_protocol_versions"), [1, 2]);
    assert_eq!(numbers(&asynchronous, "live_protocol_versions"), [1, 2]);
    assert_eq!(SUPPORTED_PROTOCOL_VERSIONS, &[1, 2]);

    let features = fixture_version(FixtureVersion::V4, "runtime-features.json");
    let capabilities: HashSet<_> = array(&features, "features")
        .iter()
        .map(|feature| string(feature, "capability"))
        .collect();
    let grammar = fixture_version(FixtureVersion::V4, "directive-grammar.json");
    assert_eq!(number(&grammar, "schema_version"), 2);
    assert_eq!(number(&grammar, "contract_version"), 2);
    let directives = array(&grammar, "directives");
    let mut names = HashSet::new();
    for directive in directives {
        assert!(names.insert(string(directive, "name")));
        let roles = strings(directive, "roles");
        assert_eq!(
            roles.iter().copied().collect::<HashSet<_>>().len(),
            roles.len()
        );
        if let Some(capability) = directive["capability"].as_str() {
            assert!(capabilities.contains(capability));
        } else {
            assert!(roles.is_empty());
        }
    }
    assert_eq!(
        directives
            .iter()
            .filter(|directive| directive["capability"].is_string())
            .map(|directive| string(directive, "name"))
            .collect::<Vec<_>>(),
        ["upload", "progress", "poll", "stream"]
    );
    for (name, capability, roles) in [
        ("upload", "uploads@1", &["cancel", "retry", "remove"][..]),
        ("progress", "uploads@1", &[][..]),
        ("poll", "async@1", &[][..]),
        ("stream", "async@1", &[][..]),
    ] {
        let directive = directives
            .iter()
            .find(|directive| string(directive, "name") == name)
            .expect("promoted directive exists");
        assert_eq!(directive["capability"], capability);
        assert_eq!(strings(directive, "roles"), roles);
        assert!(!strings(&grammar, "reserved").contains(&name));
    }
}

#[test]
fn v1_protocol_success_bytes_are_already_canonical_and_stable() {
    let limits = suprnova_live::limits::InputLimits::default();
    for case in cases(&fixture("protocol-success.json")) {
        let encoded = string(case, "encoded").as_bytes();
        let parsed = parse_canonical_value(encoded, &limits).expect("v1 protocol JSON parses");
        assert_eq!(
            to_canonical_bytes(&parsed, &limits).expect("v1 protocol JSON canonicalizes"),
            encoded,
            "v1 fixture {} changed its canonical bytes",
            string(case, "id")
        );
        if string(case, "kind") == "response" {
            let response = parse_versioned_update_response(encoded, &protocol_support::limits())
                .expect("v1 response parses through version dispatch");
            assert_eq!(
                encode_versioned_update_response(&response, &protocol_support::limits())
                    .expect("v1 response re-encodes"),
                encoded,
                "v1 response fixture {} changed its encoded bytes",
                string(case, "id")
            );
        }
    }
}

#[test]
fn canonical_fixtures_have_no_unconsumed_case_kind() {
    let limits = suprnova_live::limits::InputLimits::default();
    for case in cases(&fixture("canonical-success.json")) {
        let parsed = parse_canonical_value(string(case, "input").as_bytes(), &limits)
            .expect("success fixture parses");
        assert_eq!(
            to_canonical_bytes(&parsed, &limits).expect("fixture canonicalizes"),
            string(case, "canonical").as_bytes()
        );
    }
    for case in cases(&fixture("canonical-failure.json")) {
        let error = parse_canonical_value(string(case, "input").as_bytes(), &limits)
            .expect_err("failure fixture rejects");
        assert_eq!(error.kind().as_str(), string(case, "expected_error"));
    }
}

#[test]
fn snapshot_fixtures_match_rust_verification_and_failure_classes() {
    let keys = protocol_support::snapshot_support::key_ring();
    let schemas = protocol_support::snapshot_support::schema_set();
    let limits = protocol_support::snapshot_support::snapshot_limits();
    for name in ["snapshot-success.json", "snapshot-failure.json"] {
        let root = fixture(name);
        for case in cases(&root) {
            let encoded = serde_json_canonicalizer::to_vec(&case["encoded"])
                .expect("encoded snapshot canonicalizes");
            let now = UnixMillis::new(string(case, "now").parse().expect("time is decimal"));
            let result = match string(case, "purpose") {
                "seed" => verify_seed(
                    &encoded,
                    &protocol_support::snapshot_support::expected_seed(schemas.clone()),
                    &keys,
                    now,
                    &limits,
                )
                .map(|_| ()),
                "instance" => verify_instance(
                    &encoded,
                    &ExpectedInstanceV1::new(
                        protocol_support::snapshot_support::component_contract(),
                        BuildId::parse("build-2026-08-21").expect("build is valid"),
                        protocol_support::snapshot_support::route(1),
                        IslandSlot::parse("search-results").expect("slot is valid"),
                        protocol_support::snapshot_support::instance_fields(&keys).scope,
                        schemas.clone(),
                    ),
                    &keys,
                    now,
                    &limits,
                )
                .map(|_| ()),
                other => panic!("unknown snapshot fixture purpose: {other}"),
            };
            if name == "snapshot-success.json" {
                result.expect("success snapshot fixture verifies");
            } else {
                assert_eq!(
                    result
                        .expect_err("failure snapshot fixture rejects")
                        .kind()
                        .as_str(),
                    string(case, "expected_error")
                );
            }
        }
    }
}

#[test]
fn protocol_fixtures_are_exhaustively_accepted_or_rejected() {
    let limits = protocol_support::limits();
    for version in [FixtureVersion::V1, FixtureVersion::V2] {
        let root = fixture_version(version, "protocol-success.json");
        for case in cases(&root) {
            match string(case, "kind") {
                "request" => {
                    parse_versioned_update_request(string(case, "encoded").as_bytes(), &limits)
                        .map(|_| ())
                        .expect("success request fixture parses")
                }
                "response" => {
                    parse_versioned_update_response(string(case, "encoded").as_bytes(), &limits)
                        .map(|_| ())
                        .expect("success response fixture parses")
                }
                other => panic!("unknown protocol fixture kind: {other}"),
            }
        }
    }
    for version in [FixtureVersion::V1, FixtureVersion::V2] {
        let root = fixture_version(version, "protocol-failure.json");
        for case in cases(&root) {
            let error = match string(case, "kind") {
                "request" => {
                    parse_versioned_update_request(string(case, "encoded").as_bytes(), &limits)
                        .expect_err("failure request rejects")
                }
                "response" => {
                    parse_versioned_update_response(string(case, "encoded").as_bytes(), &limits)
                        .expect_err("failure response rejects")
                }
                other => panic!("unknown protocol fixture kind: {other}"),
            };
            assert_eq!(error.kind().as_str(), string(case, "expected_error"));
        }
    }
}

#[test]
fn ordering_and_compatibility_fixtures_enumerate_every_case() {
    let limits = protocol_support::limits();
    for case in cases(&fixture("response-ordering.json")) {
        let encoded = match string(case, "render") {
            "redirect" => format!(
                r#"{{"correlation_id":"{}","effects":[],"events":[],"extensions":{{}},"outcome":"accepted","protocol_version":1,"redirect":"/next","validation":{{}}}}"#,
                protocol_support::identity::<16>(0x10)
            )
            .into_bytes(),
            "html" => protocol_support::accepted_html_response("<div>ok</div>"),
            "no_render" => format!(
                r#"{{"accepted_revision":"8","correlation_id":"{}","effects":[],"events":[],"extensions":{{}},"outcome":"accepted","protocol_version":1,"render":{{"kind":"no_render"}},"snapshot":{},"validation":{{}}}}"#,
                protocol_support::identity::<16>(0x10),
                protocol_support::instance_snapshot()
            )
            .into_bytes(),
            other => panic!("unknown ordering render: {other}"),
        };
        let morph = match string(case, "morph") {
            "not_attempted" => MorphDisposition::NotAttempted,
            "succeeded" => MorphDisposition::Succeeded,
            "failed_after_acceptance" => MorphDisposition::FailedAfterAcceptance,
            other => panic!("unknown morph disposition: {other}"),
        };
        let response = parse_update_response(&encoded, &limits).expect("ordering response parses");
        let actual: Vec<_> = application_plan(&response, morph)
            .into_iter()
            .map(step_name)
            .collect();
        let expected: Vec<_> = case["expected_steps"]
            .as_array()
            .expect("steps are array")
            .iter()
            .map(|step| step.as_str().expect("step is string"))
            .collect();
        assert_eq!(actual, expected);
    }

    for case in cases(&fixture("compatibility.json")) {
        let number = |key| case[key].as_u64().expect("version is integer") as u16;
        let actual = CompatibilityWindow::v1().evaluate(VersionSet::new(
            number("protocol"),
            number("runtime"),
            number("snapshot"),
        ));
        let expected = match string(case, "expected") {
            "compatible" => CompatibilityDecision::Compatible,
            "refresh_document" => CompatibilityDecision::RefreshDocument,
            other => panic!("unknown compatibility result: {other}"),
        };
        assert_eq!(actual, expected);
    }
}

fn step_name(step: ApplicationStep) -> &'static str {
    match step {
        ApplicationStep::Navigate => "navigate",
        ApplicationStep::PreflightMorph => "preflight_morph",
        ApplicationStep::Morph => "morph",
        ApplicationStep::ValidateNoRender => "validate_no_render",
        ApplicationStep::CommitSnapshotAndRevision => "commit_snapshot_and_revision",
        ApplicationStep::ReconcileModelsAndValidation => "reconcile_models_and_validation",
        ApplicationStep::RestoreFocus => "restore_focus",
        ApplicationStep::QueueChildDeliveries => "queue_child_deliveries",
        ApplicationStep::ReflectUrl => "reflect_url",
        ApplicationStep::DispatchEvents => "dispatch_events",
        ApplicationStep::RunRegisteredEffects => "run_registered_effects",
        ApplicationStep::SettleFeedback => "settle_feedback",
        ApplicationStep::RetainDom => "retain_dom",
        ApplicationStep::RequestFreshRenderWithoutReplay => "request_fresh_render_without_replay",
        ApplicationStep::RequestFreshIsland => "request_fresh_island",
        ApplicationStep::StopLive => "stop_live",
    }
}
