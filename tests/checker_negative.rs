//! Negative checker fixtures.

mod checker_support;

use suprnova_live::checker::{
    CheckerLimits, DiagnosticCode, DiagnosticSeverity, TemplateCatalog, TemplateChecker,
};

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
