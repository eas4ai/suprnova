//! Pure semantic response-application ordering tests.

mod protocol_support;

use protocol_support::{accepted_html_response, identity, instance_snapshot, limits};
use suprnova_live::protocol::{
    ApplicationStep, MorphDisposition, application_plan, parse_update_response,
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
