//! Live v1 response shape, outcome, and error/recovery tests.

mod protocol_support;

use protocol_support::{accepted_html_response, identity, instance_snapshot, limits};
use suprnova_live::error::{ErrorCategory, RecoveryInstruction};
use suprnova_live::protocol::{
    ProtocolErrorKind, RenderPayload, ResponseOutcome, parse_update_response,
};

#[test]
fn accepted_html_and_explicit_no_render_responses_require_committed_state() {
    let response = parse_update_response(&accepted_html_response("<div>ok</div>"), &limits())
        .expect("accepted response parses");
    assert_eq!(response.outcome(), ResponseOutcome::Accepted);
    assert!(matches!(response.render(), Some(RenderPayload::Html(_))));
    assert!(response.accepted_revision().is_some());
    assert!(response.snapshot().is_some());

    let no_render = format!(
        r#"{{"accepted_revision":"8","correlation_id":"{}","effects":[],"events":[],"extensions":{{}},"outcome":"accepted","protocol_version":1,"render":{{"kind":"no_render"}},"snapshot":{},"validation":{{"query":"required"}}}}"#,
        identity::<16>(0x10),
        instance_snapshot(),
    );
    let response =
        parse_update_response(no_render.as_bytes(), &limits()).expect("no-render response parses");
    assert!(matches!(response.render(), Some(RenderPayload::NoRender)));
}

#[test]
fn terminal_redirect_is_structurally_exclusive() {
    let redirect = format!(
        r#"{{"correlation_id":"{}","effects":[],"events":[],"extensions":{{}},"outcome":"accepted","protocol_version":1,"redirect":"/profiles","validation":{{}}}}"#,
        identity::<16>(0x10),
    );
    let response =
        parse_update_response(redirect.as_bytes(), &limits()).expect("terminal redirect parses");
    assert_eq!(response.redirect(), Some("/profiles"));
    assert!(response.snapshot().is_none());

    let confused = String::from_utf8(accepted_html_response("<div>ok</div>"))
        .expect("response is UTF-8")
        .replacen(
            "\"outcome\":\"accepted\"",
            "\"outcome\":\"accepted\",\"redirect\":\"/profiles\"",
            1,
        );
    assert_eq!(
        parse_update_response(confused.as_bytes(), &limits())
            .expect_err("redirect cannot carry morph state")
            .kind(),
        ProtocolErrorKind::OutcomeMismatch
    );
}

#[test]
fn rejected_refresh_and_fatal_outcomes_require_agreeing_safe_errors() {
    let rejected = format!(
        r#"{{"correlation_id":"{}","effects":[],"error":{{"category":"validation","detail":"invalid_json","recovery":"retain_dom"}},"events":[],"extensions":{{}},"outcome":"rejected","protocol_version":1,"validation":{{"query":"required"}}}}"#,
        identity::<16>(0x10),
    );
    let response =
        parse_update_response(rejected.as_bytes(), &limits()).expect("rejected response parses");
    let error = response.error().expect("rejection has error");
    assert_eq!(error.category(), ErrorCategory::Validation);
    assert_eq!(error.recovery(), RecoveryInstruction::RetainDom);

    let mismatch = rejected.replace("retain_dom", "refresh_island");
    assert_eq!(
        parse_update_response(mismatch.as_bytes(), &limits())
            .expect_err("outcome and recovery must agree")
            .kind(),
        ProtocolErrorKind::ErrorRecoveryMismatch
    );

    for (outcome, category, recovery) in [
        ("refresh_required", "snapshot", "refresh_island"),
        ("fatal", "internal", "stop"),
    ] {
        let encoded = format!(
            r#"{{"correlation_id":"{}","effects":[],"error":{{"category":"{category}","detail":"invalid_json","recovery":"{recovery}"}},"events":[],"extensions":{{}},"outcome":"{outcome}","protocol_version":1,"validation":{{}}}}"#,
            identity::<16>(0x10),
        );
        parse_update_response(encoded.as_bytes(), &limits())
            .expect("refresh and fatal responses parse with matching recovery");
    }

    let duplicate = String::from_utf8(accepted_html_response("<div>ok</div>"))
        .expect("response is UTF-8")
        .replacen("\"outcome\":\"accepted\"", "\"outcome\":\"duplicate\"", 1);
    assert_eq!(
        parse_update_response(duplicate.as_bytes(), &limits())
            .expect("compatible duplicate carries prior outcome")
            .outcome(),
        ResponseOutcome::Duplicate
    );
}

#[test]
fn unsafe_redirects_unknown_fields_and_oversized_html_are_rejected() {
    let unsafe_redirect = format!(
        r#"{{"correlation_id":"{}","effects":[],"events":[],"extensions":{{}},"outcome":"accepted","protocol_version":1,"redirect":"https://evil.example","validation":{{}}}}"#,
        identity::<16>(0x10),
    );
    assert_eq!(
        parse_update_response(unsafe_redirect.as_bytes(), &limits())
            .expect_err("external redirect is outside the route contract")
            .kind(),
        ProtocolErrorKind::UnsafeRedirect
    );

    let too_large = accepted_html_response(&"x".repeat(32 * 1024 + 1));
    assert_eq!(
        parse_update_response(&too_large, &limits())
            .expect_err("HTML bytes are bounded")
            .kind(),
        ProtocolErrorKind::InputTooLarge
    );

    let unknown = String::from_utf8(accepted_html_response("<div>ok</div>"))
        .expect("response is UTF-8")
        .replacen(
            "\"outcome\":\"accepted\"",
            "\"outcome\":\"accepted\",\"surprise\":true",
            1,
        );
    assert_eq!(
        parse_update_response(unknown.as_bytes(), &limits())
            .expect_err("unknown response field fails")
            .kind(),
        ProtocolErrorKind::InvalidEnvelope
    );
}

#[test]
fn nonaccepted_responses_cannot_smuggle_events_or_effects() {
    let rejected = format!(
        r#"{{"correlation_id":"{}","effects":[{{"name":"run","payload":{{}}}}],"error":{{"category":"validation","detail":"invalid_json","recovery":"retain_dom"}},"events":[],"extensions":{{}},"outcome":"rejected","protocol_version":1,"validation":{{}}}}"#,
        identity::<16>(0x10),
    );
    assert_eq!(
        parse_update_response(rejected.as_bytes(), &limits())
            .expect_err("rejection cannot carry executable output")
            .kind(),
        ProtocolErrorKind::OutcomeMismatch
    );
}

#[test]
fn a8_16_control_and_snapshot_framework_overheads_fit_the_hard_caps() {
    let html = "h".repeat(8 * 1024);
    let snapshot_payload = "s".repeat(16 * 1024);
    let encoded = format!(
        r#"{{"accepted_revision":"8","correlation_id":"{}","effects":[],"events":[],"extensions":{{}},"outcome":"accepted","protocol_version":1,"render":{{"html":"{html}","kind":"html"}},"snapshot":{{"body":{{"payload":"{snapshot_payload}"}},"signature":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}},"validation":{{}}}}"#,
        identity::<16>(0x10),
    );
    let control_overhead = encoded.len() - html.len() - snapshot_payload.len();
    assert!(
        control_overhead <= 1_024,
        "control overhead was {control_overhead} bytes"
    );

    let actual_snapshot = instance_snapshot();
    let application_state_bytes =
        r#"{"query":"rust","selected":"1"}"#.len() + r#"{"page":1}"#.len();
    let snapshot_overhead = actual_snapshot.len() - application_state_bytes;
    assert!(
        snapshot_overhead <= 768,
        "snapshot overhead was {snapshot_overhead} bytes"
    );
}
