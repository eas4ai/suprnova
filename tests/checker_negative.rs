//! Negative checker fixtures.

mod checker_support;

use serde_json::Value;
use suprnova_live::checker::{
    CheckerLimits, DiagnosticCode, DiagnosticSeverity, TemplateCatalog, TemplateChecker,
};
use suprnova_live::conformance::{FixtureVersion, fixture_directory};

use checker_support::{CHILD_VIEW, ROOT_VIEW, registry, root_name, view};

#[test]
fn directive_metadata_and_nested_ownership_fail_closed() {
    let cases = [
        (
            include_str!("fixtures/checker/fail/unknown_action.html"),
            DiagnosticCode::UnknownAction,
        ),
        (
            include_str!("fixtures/checker/fail/forbidden_model.html"),
            DiagnosticCode::ForbiddenModel,
        ),
        (
            include_str!("fixtures/checker/fail/invalid_modifier.html"),
            DiagnosticCode::InvalidModifier,
        ),
        (
            include_str!("fixtures/checker/fail/invalid_error_modifier.html"),
            DiagnosticCode::InvalidModifier,
        ),
        (
            include_str!("fixtures/checker/fail/duplicate_key.html"),
            DiagnosticCode::DuplicateKey,
        ),
        (
            include_str!("fixtures/checker/fail/nested_ownership.html"),
            DiagnosticCode::OwnershipViolation,
        ),
        (
            include_str!("fixtures/checker/fail/invalid_url.html"),
            DiagnosticCode::InvalidUrlBinding,
        ),
        (
            include_str!("fixtures/checker/fail/forbidden_lifecycle.html"),
            DiagnosticCode::ForbiddenLifecycle,
        ),
        (
            include_str!("fixtures/checker/fail/unknown_effect.html"),
            DiagnosticCode::UnknownEffect,
        ),
        (
            include_str!("fixtures/checker/fail/unknown_event.html"),
            DiagnosticCode::UnknownEvent,
        ),
        (
            include_str!("fixtures/checker/fail/inaccessible_click.html"),
            DiagnosticCode::AccessibilityViolation,
        ),
        (
            include_str!("fixtures/checker/fail/unknown_component.html"),
            DiagnosticCode::UnknownComponent,
        ),
        (
            include_str!("fixtures/checker/fail/unknown_model.html"),
            DiagnosticCode::UnknownModel,
        ),
        (
            include_str!("fixtures/checker/fail/unstable_loop_key.html"),
            DiagnosticCode::InvalidKey,
        ),
    ];

    for (source, expected) in cases {
        let report = check(source);
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == expected),
            "missing {expected:?}: {:?}",
            report.diagnostics()
        );
    }
}

#[test]
fn askama_and_html_structure_are_checked_on_every_branch() {
    let cases = [
        (
            include_str!("fixtures/checker/fail/raw_safe.html"),
            DiagnosticCode::RawSafe,
        ),
        (
            include_str!("fixtures/checker/fail/filter_block.html"),
            DiagnosticCode::DynamicStructureUnproved,
        ),
        (
            include_str!("fixtures/checker/fail/branch_mismatch.html"),
            DiagnosticCode::BranchStackMismatch,
        ),
        (
            include_str!("fixtures/checker/fail/dynamic_tag.html"),
            DiagnosticCode::DynamicStructureUnproved,
        ),
        (
            include_str!("fixtures/checker/fail/dynamic_attribute.html"),
            DiagnosticCode::DynamicStructureUnproved,
        ),
        (
            include_str!("fixtures/checker/fail/malformed.html"),
            DiagnosticCode::HtmlSyntax,
        ),
        (
            include_str!("fixtures/checker/fail/match_mismatch.html"),
            DiagnosticCode::BranchStackMismatch,
        ),
        (
            include_str!("fixtures/checker/fail/loop_mismatch.html"),
            DiagnosticCode::BranchStackMismatch,
        ),
    ];

    for (source, expected) in cases {
        let report = check(source);
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == expected),
            "missing {expected:?}: {:?}",
            report.diagnostics()
        );
    }

    let report = check(include_str!("fixtures/checker/fail/dynamic_tag.html"));
    assert!(report.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == DiagnosticCode::DynamicStructureUnproved
            && diagnostic.severity() == DiagnosticSeverity::Unproved
    }));
}

#[test]
fn iteration_003_directive_failures_are_closed_and_source_oriented() {
    let report = check(include_str!(
        "fixtures/checker/fail/iteration-003-directives.html"
    ));
    for expected in [
        DiagnosticCode::UnknownDirective,
        DiagnosticCode::InvalidModifier,
        DiagnosticCode::DynamicStructureUnproved,
    ] {
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == expected),
            "missing {expected:?}: {:?}",
            report.diagnostics()
        );
    }
}

#[test]
fn iteration_004_roles_modifiers_and_conflicts_fail_closed() {
    for source in [
        r#"<button live:upload.stream="avatar">Unsupported role</button>"#,
        r#"<button live:upload.cancel.retry="avatar">Multiple roles</button>"#,
        r#"<section live:poll.cancel></section>"#,
        r#"<section live:stream.visible="orders"></section>"#,
        r#"<input live:upload="avatar" live:model.blur="query">"#,
    ] {
        let report = check(source);
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == DiagnosticCode::InvalidModifier),
            "missing invalid modifier: {:?}",
            report.diagnostics()
        );
    }
}

#[test]
fn iteration_004_progress_rejects_endpoint_values() {
    let report = check(r#"<output live:progress="/uploads/chunk"></output>"#);
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::InvalidModifier),
        "missing invalid progress value: {:?}",
        report.diagnostics()
    );
}

#[test]
fn iteration_004_modifier_groups_fail_closed() {
    for source in [
        r#"<section live:stream.push-only.hybrid="orders"></section>"#,
        r#"<section live:poll.visible.always></section>"#,
        r#"<section live:poll.5s.30s></section>"#,
        r#"<section live:poll.visible.always.5s.30s></section>"#,
    ] {
        let report = check(source);
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == DiagnosticCode::InvalidModifier),
            "missing modifier conflict for {source}: {:?}",
            report.diagnostics()
        );
    }
}

#[test]
fn every_conflicting_v4_freshness_combination_is_rejected_by_the_real_checker() {
    let fixture: Value = serde_json::from_slice(
        &std::fs::read(fixture_directory(FixtureVersion::V4).join("directive-grammar.json"))
            .expect("directive fixture is readable"),
    )
    .expect("directive fixture is valid JSON");

    for combination in fixture["freshness_combinations"]
        .as_array()
        .expect("freshness combinations are an array")
    {
        if combination["result"] != "directive_conflict" {
            continue;
        }
        let poll = combination["poll"].as_bool().expect("poll flag");
        let stream = combination["stream"].as_str().expect("stream mode");
        let stream_attribute = match stream {
            "absent" => "",
            "default" => r#" live:stream="orders""#,
            "hybrid" => r#" live:stream.hybrid="orders""#,
            "push-only" => r#" live:stream.push-only="orders""#,
            other => panic!("unexpected stream mode {other}"),
        };
        let poll_attribute = if poll { " live:poll" } else { "" };
        let source = format!("<section{stream_attribute}{poll_attribute}></section>");
        let registry = registry();
        let catalog = TemplateCatalog::new(vec![
            (view(ROOT_VIEW), source.as_str()),
            (
                view(CHILD_VIEW),
                include_str!("fixtures/checker/pass/child.html"),
            ),
        ])
        .expect("template catalog");
        let report = TemplateChecker::new(&registry, &catalog, CheckerLimits::default())
            .check_component(&root_name());

        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == DiagnosticCode::InvalidModifier),
            "combination {combination:?} was not rejected: {:?}",
            report.diagnostics()
        );
    }
}

#[test]
fn freshness_combinations_are_enforced_across_the_whole_island() {
    let report =
        check(r#"<section live:stream.push-only="orders"></section><section live:poll></section>"#);

    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::InvalidModifier),
        "cross-element island conflict was not rejected: {:?}",
        report.diagnostics()
    );
}

#[test]
fn iteration_003_signal_integers_match_the_browser_safe_range() {
    for source in [
        r#"<section live:signal="count:9007199254740992"></section>"#,
        r#"<section live:signal="open:false"><div live:attr="onclick:open"></div></section>"#,
        r#"<section live:signal="open:false"><div live:attr="data-controller:open"></div></section>"#,
        r#"<section live:signal="open:false"><div live:attr="data-suprnova-live-snapshot:open"></div></section>"#,
        r#"<section live:signal="open:false"><div live:class="--unsafe:open"></div></section>"#,
    ] {
        let report = check(source);
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| { diagnostic.code() == DiagnosticCode::InvalidModifier })
        );
    }
}

#[test]
fn morph_controls_require_stable_identity_safe_modes_and_owned_structure() {
    let report = check(include_str!(
        "fixtures/checker/fail/invalid-morph-controls.html"
    ));
    for expected in [
        DiagnosticCode::InvalidKey,
        DiagnosticCode::InvalidModifier,
        DiagnosticCode::OwnershipViolation,
        DiagnosticCode::AccessibilityViolation,
    ] {
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == expected),
            "missing {expected:?}: {:?}",
            report.diagnostics()
        );
    }
}

fn check(source: &'static str) -> suprnova_live::checker::CheckReport {
    let registry = registry();
    let catalog = TemplateCatalog::new(vec![
        (view(ROOT_VIEW), source),
        (
            view(CHILD_VIEW),
            include_str!("fixtures/checker/pass/child.html"),
        ),
    ])
    .expect("template catalog");
    TemplateChecker::new(&registry, &catalog, CheckerLimits::default())
        .check_component(&root_name())
}
