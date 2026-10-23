//! Pure semantic response-application ordering tests.

mod protocol_support;

use std::fs;

use protocol_support::{accepted_html_response, identity, instance_snapshot, limits};
use serde_json::{Value, json};
use suprnova_live::conformance::{FixtureVersion, fixture_directory};
use suprnova_live::protocol::{
    ApplicationStep, MorphDisposition, VersionedUpdateResponse, application_plan,
    application_plan_v2, parse_update_response, parse_versioned_update_response,
};

#[test]
fn redirect_is_the_only_application_step() {
    let encoded = format!(
        r#"{{"correlation_id":"{}","effects":[],"events":[],"extensions":{{}},"outcome":"accepted","protocol_version":1,"redirect":"/profiles","validation":{{}}}}"#,
        identity::<16>(0x10),
    );
    let response = parse_update_response(encoded.as_bytes(), &limits()).expect("response parses");
    assert_eq!(
        application_plan(&response, MorphDisposition::NotAttempted),
        vec![ApplicationStep::Navigate]
    );
}

#[test]
fn html_commits_browser_state_only_after_successful_morph() {
    let response = parse_update_response(&accepted_html_response("<div>ok</div>"), &limits())
        .expect("response parses");
    assert_eq!(
        application_plan(&response, MorphDisposition::Succeeded),
        vec![
            ApplicationStep::PreflightMorph,
            ApplicationStep::Morph,
            ApplicationStep::CommitSnapshotAndRevision,
            ApplicationStep::ReconcileModelsAndValidation,
            ApplicationStep::RestoreFocus,
            ApplicationStep::DispatchEvents,
            ApplicationStep::RunRegisteredEffects,
            ApplicationStep::SettleFeedback,
        ]
    );
    assert_eq!(
        application_plan(&response, MorphDisposition::FailedAfterAcceptance),
        vec![
            ApplicationStep::PreflightMorph,
            ApplicationStep::Morph,
            ApplicationStep::RequestFreshRenderWithoutReplay,
        ]
    );
}

#[test]
fn no_render_validation_precedes_commit_and_rejection_retains_dom() {
    let no_render = format!(
        r#"{{"accepted_revision":"8","correlation_id":"{}","effects":[],"events":[],"extensions":{{}},"outcome":"accepted","protocol_version":1,"render":{{"kind":"no_render"}},"snapshot":{},"validation":{{"query":"required"}}}}"#,
        identity::<16>(0x10),
        instance_snapshot(),
    );
    let response = parse_update_response(no_render.as_bytes(), &limits()).expect("response parses");
    assert_eq!(
        application_plan(&response, MorphDisposition::NotAttempted)[..2],
        [
            ApplicationStep::ValidateNoRender,
            ApplicationStep::CommitSnapshotAndRevision,
        ]
    );

    let rejected = format!(
        r#"{{"correlation_id":"{}","effects":[],"error":{{"category":"validation","detail":"invalid_json","recovery":"retain_dom"}},"events":[],"extensions":{{}},"outcome":"rejected","protocol_version":1,"validation":{{}}}}"#,
        identity::<16>(0x10),
    );
    let response = parse_update_response(rejected.as_bytes(), &limits()).expect("response parses");
    assert_eq!(
        application_plan(&response, MorphDisposition::NotAttempted),
        vec![ApplicationStep::RetainDom, ApplicationStep::SettleFeedback]
    );
}

#[test]
fn refresh_and_fatal_recovery_have_explicit_nonreplay_plans() {
    let refresh = format!(
        r#"{{"correlation_id":"{}","effects":[],"error":{{"category":"snapshot","detail":"invalid_json","recovery":"refresh_island"}},"events":[],"extensions":{{}},"outcome":"refresh_required","protocol_version":1,"validation":{{}}}}"#,
        identity::<16>(0x10),
    );
    let response = parse_update_response(refresh.as_bytes(), &limits()).expect("response parses");
    assert_eq!(
        application_plan(&response, MorphDisposition::NotAttempted),
        vec![
            ApplicationStep::RetainDom,
            ApplicationStep::RequestFreshIsland,
            ApplicationStep::SettleFeedback,
        ]
    );

    let fatal = format!(
        r#"{{"correlation_id":"{}","effects":[],"error":{{"category":"internal","detail":"invalid_json","recovery":"stop"}},"events":[],"extensions":{{}},"outcome":"fatal","protocol_version":1,"validation":{{}}}}"#,
        identity::<16>(0x10),
    );
    let response = parse_update_response(fatal.as_bytes(), &limits()).expect("response parses");
    assert_eq!(
        application_plan(&response, MorphDisposition::NotAttempted),
        vec![
            ApplicationStep::RetainDom,
            ApplicationStep::StopLive,
            ApplicationStep::SettleFeedback,
        ]
    );
}

#[test]
fn v3_fixture_covers_the_complete_v1_and_v2_application_order() {
    let fixture: Value = serde_json::from_slice(
        &fs::read(fixture_directory(FixtureVersion::V3).join("response-application.json"))
            .expect("v3 response fixture is readable"),
    )
    .expect("v3 response fixture is valid JSON");
    let cases = fixture["cases"].as_array().expect("cases are an array");
    assert_eq!(cases.len(), 11);

    for case in cases {
        let input = &case["input"];
        let morph = match input["morph"].as_str().expect("morph is a string") {
            "not_attempted" => MorphDisposition::NotAttempted,
            "succeeded" => MorphDisposition::Succeeded,
            "failed_after_acceptance" => MorphDisposition::FailedAfterAcceptance,
            other => panic!("unknown morph disposition: {other}"),
        };
        let response = parse_versioned_update_response(&response_bytes(input), &limits())
            .expect("fixture response parses");
        let actual: Vec<_> = match response {
            VersionedUpdateResponse::V1(response) => application_plan(&response, morph),
            VersionedUpdateResponse::V2(response) => application_plan_v2(&response, morph),
        }
        .into_iter()
        .map(step_name)
        .collect();
        let expected: Vec<_> = case["expected_steps"]
            .as_array()
            .expect("expected steps are an array")
            .iter()
            .map(|step| step.as_str().expect("step is a string"))
            .collect();
        assert_eq!(actual, expected, "fixture case {}", case["id"]);
    }
}

fn response_bytes(input: &Value) -> Vec<u8> {
    if input["protocol"] == 1 {
        return format!(
            r#"{{"correlation_id":"{}","effects":[],"events":[],"extensions":{{}},"outcome":"accepted","protocol_version":1,"redirect":"/next","validation":{{}}}}"#,
            identity::<16>(0x10),
        )
        .into_bytes();
    }

    let outcome = input["outcome"].as_str().expect("outcome is a string");
    let render = input["render"].as_str().expect("render is a string");
    let recovery = input["recovery"].as_str();
    let mut response = json!({
        "child_deliveries": [],
        "correlation_id": identity::<16>(0x10),
        "effects": [],
        "events": [],
        "extensions": {},
        "outcome": outcome,
        "protocol_version": 2,
        "url_intent": null,
        "validation": {},
    });
    let object = response.as_object_mut().expect("response is an object");

    if matches!(outcome, "accepted" | "duplicate") && !matches!(render, "navigated" | "none") {
        object.insert("accepted_revision".to_owned(), json!("8"));
        object.insert(
            "snapshot".to_owned(),
            serde_json::from_str(&instance_snapshot()).expect("snapshot fixture is JSON"),
        );
    }
    match render {
        "redirect" => {
            object.insert("redirect".to_owned(), json!("/next"));
        }
        "navigated" => {
            object.insert(
                "url_intent".to_owned(),
                json!({"kind": "navigated", "target": "/catalog?page=2"}),
            );
        }
        "html" => {
            object.insert(
                "render".to_owned(),
                json!({"html": "<div>ok</div>", "kind": "html"}),
            );
        }
        "no_render" => {
            object.insert("render".to_owned(), json!({"kind": "no_render"}));
        }
        "none" => {}
        other => panic!("unknown render: {other}"),
    }
    if input["hasReflectedUrl"] == true {
        object.insert(
            "url_intent".to_owned(),
            json!({"kind": "reflected", "target": "/catalog?q=rust"}),
        );
    }
    if input["hasChildDeliveries"] == true {
        object.insert(
            "child_deliveries".to_owned(),
            json!([{
                "child_instance": identity::<16>(0x10),
                "envelope": {
                    "body": {"parameters": {"query": "rust"}},
                    "signature": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
                },
                "parameter_hash": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            }]),
        );
    }
    if !matches!(outcome, "accepted" | "duplicate") {
        let category = match outcome {
            "rejected" => "validation",
            "refresh_required" => "snapshot",
            "fatal" => "internal",
            other => panic!("unknown failure outcome: {other}"),
        };
        object.insert(
            "error".to_owned(),
            json!({
                "category": category,
                "detail": "invalid_json",
                "recovery": recovery.expect("failure recovery is present"),
            }),
        );
    }

    serde_json::to_vec(&response).expect("response serializes")
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
