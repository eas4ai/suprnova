//! `live:check` and `live:inspect` are thin, fail-closed clients of the
//! application's Live tooling helper.

mod live_support;

use std::io::Cursor;
use std::process::Command;

use live_support::{
    BIN, IDENTITY, begin, check_stream, combined, diagnostic, end_failed, end_ok, envelope,
    envelope_with, inspect_stream, run_cli, summary,
};
use suprnova_cli::commands::live_tool::{
    MAX_DIAGNOSTICS, MAX_LINE_BYTES, Operation, Outcome, ToolFailure, consume,
};

fn consume_text(
    text: &str,
    operation: Operation,
) -> Result<suprnova_cli::commands::live_tool::Session, ToolFailure> {
    consume(Cursor::new(text.as_bytes()), operation)
}

#[test]
fn help_lists_the_live_commands() {
    let output = Command::new(BIN).arg("--help").output().expect("help");
    let text = combined(&output);
    for command in ["live:make", "live:check", "live:inspect", "live:assets"] {
        assert!(text.contains(command), "help lists {command}: {text}");
    }
}

#[test]
fn live_check_requires_a_project_before_spawning_anything() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let output = Command::new(BIN)
        .arg("live:check")
        .current_dir(tmp.path())
        .output()
        .expect("spawn");
    assert_eq!(output.status.code(), Some(1));
    assert!(combined(&output).contains("Cargo.toml"));
}

#[test]
fn live_check_rejects_a_missing_template_root_before_building() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").expect("manifest");
    let output = Command::new(BIN)
        .args(["live:check", "--templates", "nowhere"])
        .current_dir(tmp.path())
        .output()
        .expect("spawn");
    assert_eq!(output.status.code(), Some(1));
    assert!(combined(&output).contains("nowhere"));
}

#[test]
fn a_complete_stream_is_consumed_into_a_session() {
    let session = consume_text(
        &check_stream(
            &[(
                "demo.broken",
                "live/broken.html",
                "unknown_action",
                "error",
                3,
                5,
            )],
            1,
        ),
        Operation::Check,
    )
    .expect("valid stream");
    assert_eq!(session.outcome, Outcome::Ok);
    assert_eq!(session.framework, "1.3.7");
    assert_eq!(session.assets.as_deref(), Some(IDENTITY));
    assert_eq!(session.diagnostics.len(), 1);
    assert_eq!(session.diagnostics[0].code, "unknown_action");
    let summary = session.summary.expect("summary");
    assert_eq!(
        (summary.components, summary.proved, summary.errors),
        (2, 1, 1)
    );
}

#[test]
fn a_failed_end_marker_is_reported_as_the_helper_failure() {
    let stream = format!(
        "{}{}",
        begin("check"),
        end_failed(1, "check", "live_tooling_registry_unavailable")
    );
    let session = consume_text(&stream, Operation::Check).expect("well-formed failure");
    assert_eq!(session.outcome, Outcome::Failed);
    assert_eq!(
        session.error.as_deref(),
        Some("live_tooling_registry_unavailable")
    );
}

#[test]
fn hostile_streams_fail_closed_without_echoing_content() {
    let secret = "SECRET_TOKEN=do-not-print";
    let cases: Vec<(String, &str)> = vec![
        (format!("{secret}\n{}", begin("check")), "unexpected"),
        (
            begin("check").replace("\"protocol\":1", "\"protocol\":2"),
            "protocol",
        ),
        (
            format!("{}{}", begin("check"), end_ok(1, "check"))
                .replace("\"operation\":\"check\"", "\"operation\":\"inspect\""),
            "operation",
        ),
        (
            format!("{}{}", begin("check"), end_ok(5, "check")),
            "sequence",
        ),
        (
            format!(
                "{}{}",
                begin("check"),
                envelope_with(
                    1,
                    "check",
                    "9.9.9",
                    Some(IDENTITY),
                    "{\"kind\":\"end\",\"payload\":{\"status\":\"ok\",\"error\":null}}"
                )
            ),
            "identity",
        ),
        (
            format!(
                "{}{}",
                begin("check"),
                envelope_with(
                    1,
                    "check",
                    "1.3.7",
                    None,
                    "{\"kind\":\"end\",\"payload\":{\"status\":\"ok\",\"error\":null}}"
                )
            ),
            "identity",
        ),
        (begin("check"), "end marker"),
        (
            format!(
                "{}{}{}",
                begin("check"),
                end_ok(1, "check"),
                end_ok(2, "check")
            ),
            "after the end",
        ),
        (
            format!(
                "{}{}",
                begin("check"),
                envelope(
                    1,
                    "check",
                    &format!(
                        "{{\"kind\":\"diagnostic\",\"payload\":{{\"component\":\"{secret}\",\"view\":null,\"code\":\"x\",\"severity\":\"error\",\"line\":1,\"column\":1,\"extra\":1}}}}"
                    )
                )
            ),
            "malformed",
        ),
        (
            format!("{}{}", end_ok(0, "check"), end_ok(1, "check")),
            "begin",
        ),
        (
            format!(
                "{}{}",
                begin("check"),
                envelope(
                    1,
                    "check",
                    &format!(
                        "{{\"kind\":\"diagnostic\",\"payload\":{{\"component\":\"{}\",\"view\":null,\"code\":\"x\",\"severity\":\"error\",\"line\":1,\"column\":1}}}}",
                        "c".repeat(300)
                    )
                )
            ),
            "too long",
        ),
        (
            format!(
                "{}{}",
                begin("check"),
                envelope(
                    1,
                    "check",
                    &format!("{{\"kind\":\"begin\"}}{}", " ".repeat(MAX_LINE_BYTES))
                )
            ),
            "line",
        ),
    ];
    for (stream, expected) in cases {
        let error = consume_text(&stream, Operation::Check)
            .expect_err(&format!("stream must fail closed: expected {expected}"));
        let message = error.to_string();
        assert!(
            message.to_lowercase().contains(expected),
            "{message:?} mentions {expected}"
        );
        assert!(
            !message.contains(secret),
            "failure messages never echo stdout"
        );
        assert!(!message.contains("do-not-print"));
    }
}

#[test]
fn diagnostic_and_envelope_counts_are_capped() {
    let mut stream = begin("check");
    for index in 0..=MAX_DIAGNOSTICS as u32 {
        stream.push_str(&diagnostic(
            index + 1,
            "demo.c",
            "live/c.html",
            "raw_safe",
            "error",
            1,
            1,
        ));
    }
    stream.push_str(&summary(MAX_DIAGNOSTICS as u32 + 2, 1, 0, 1, 0, 1));
    stream.push_str(&end_ok(MAX_DIAGNOSTICS as u32 + 3, "check"));
    let error = consume_text(&stream, Operation::Check).expect_err("too many diagnostics");
    assert!(error.to_string().contains("diagnostic"));
}

#[test]
fn live_check_passes_a_proved_application() {
    let output = run_cli(&["live:check"], &check_stream(&[], 2), 0);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("2 component"), "{text}");
    assert!(text.contains("proved"), "{text}");
}

#[test]
fn live_check_fails_on_errors_and_distinguishes_unproved() {
    let stream = check_stream(
        &[
            (
                "demo.broken",
                "live/broken.html",
                "unknown_action",
                "error",
                3,
                5,
            ),
            (
                "demo.dynamic",
                "live/dynamic.html",
                "dynamic_structure_unproved",
                "unproved",
                7,
                1,
            ),
        ],
        0,
    );
    let output = run_cli(&["live:check"], &stream, 0);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("live/broken.html:3:5"), "{text}");
    assert!(text.contains("unknown_action"), "{text}");
    assert!(text.contains("demo.broken"), "{text}");
    assert!(text.contains("dynamic_structure_unproved"), "{text}");

    let unproved_only = check_stream(
        &[(
            "demo.dynamic",
            "live/dynamic.html",
            "dynamic_structure_unproved",
            "unproved",
            7,
            1,
        )],
        1,
    );
    let strict = run_cli(&["live:check"], &unproved_only, 0);
    assert_eq!(strict.status.code(), Some(1), "{}", combined(&strict));
    assert!(combined(&strict).contains("--allow-unproved"));
    let relaxed = run_cli(&["live:check", "--allow-unproved"], &unproved_only, 0);
    assert_eq!(relaxed.status.code(), Some(0), "{}", combined(&relaxed));
}

#[test]
fn live_check_explains_an_unbound_registry() {
    let stream = format!(
        "{}{}",
        begin("check"),
        end_failed(1, "check", "live_tooling_registry_unavailable")
    );
    let output = run_cli(&["live:check"], &stream, 1);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.contains("registry"), "{text}");
    assert!(text.contains("bootstrap"), "{text}");
}

#[test]
fn live_check_reports_a_missing_helper_and_hostile_stdout() {
    let missing = run_cli(&["live:check"], "", 1);
    let text = combined(&missing);
    assert_eq!(missing.status.code(), Some(1), "{text}");
    assert!(text.contains("helper"), "{text}");

    let hostile = run_cli(
        &["live:check"],
        &format!("SECRET_TOKEN=do-not-print\n{}", check_stream(&[], 2)),
        0,
    );
    let text = combined(&hostile);
    assert_eq!(hostile.status.code(), Some(1), "{text}");
    assert!(!text.contains("do-not-print"), "{text}");
}

#[test]
fn live_inspect_prints_safe_state_and_optional_json() {
    let human = run_cli(&["live:inspect"], &inspect_stream(), 0);
    let text = combined(&human);
    assert_eq!(human.status.code(), Some(0), "{text}");
    assert!(text.contains(IDENTITY), "{text}");
    assert!(text.contains("demo.counter"), "{text}");
    assert!(text.contains("live/counter.html"), "{text}");

    let json = run_cli(&["live:inspect", "--json"], &inspect_stream(), 0);
    assert_eq!(json.status.code(), Some(0), "{}", combined(&json));
    let value: serde_json::Value =
        serde_json::from_slice(&json.stdout).expect("stdout is exactly one JSON document");
    assert_eq!(value["assets"], IDENTITY);
    assert_eq!(value["runtime"]["registry_bound"], true);
    assert_eq!(value["components"][0]["name"], "demo.counter");
}
