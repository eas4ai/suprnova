//! Shared Rust-side golden fixture conformance.

mod protocol_support;

use std::fs;

use serde_json::Value;
use suprnova_live::canonical::{parse_canonical_value, to_canonical_bytes};
use suprnova_live::conformance::{
    FIXTURE_VERSIONS, FixtureVersion, expected_fixture_manifest_sha256_version, fixture_directory,
    fixture_manifest_sha256_version,
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

#[test]
fn fixture_manifest_is_complete_and_hashable() {
    assert_eq!(FIXTURE_VERSIONS.len(), 2);
    for version in FIXTURE_VERSIONS {
        assert!(!version.files().is_empty());
        assert_eq!(
            fixture_manifest_sha256_version(*version).expect("fixtures hash"),
            expected_fixture_manifest_sha256_version(*version).expect("reviewed hash")
        );
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
    for version in FIXTURE_VERSIONS {
        let root = fixture_version(*version, "protocol-success.json");
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
    for version in FIXTURE_VERSIONS {
        let root = fixture_version(*version, "protocol-failure.json");
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
        ApplicationStep::DispatchEvents => "dispatch_events",
        ApplicationStep::RunRegisteredEffects => "run_registered_effects",
        ApplicationStep::SettleFeedback => "settle_feedback",
        ApplicationStep::RetainDom => "retain_dom",
        ApplicationStep::RequestFreshRenderWithoutReplay => "request_fresh_render_without_replay",
        ApplicationStep::RequestFreshIsland => "request_fresh_island",
        ApplicationStep::StopLive => "stop_live",
    }
}
