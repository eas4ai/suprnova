//! Checker limits, source stability, and parser-boundary regressions.

mod checker_support;

use proptest::prelude::*;
use suprnova_live::checker::{
    CheckerLimits, DiagnosticCode, DiagnosticSeverity, TemplateCatalog, TemplateChecker,
};

use checker_support::{CHILD_VIEW, ROOT_VIEW, registry, root_name, view};

#[test]
fn missing_view_include_and_parent_have_distinct_stable_diagnostics() {
    let registry = registry();
    let missing_root = TemplateCatalog::new(vec![(
        view(CHILD_VIEW),
        include_str!("fixtures/checker/pass/child.html"),
    )])
    .expect("template catalog");
    assert_code(
        TemplateChecker::new(&registry, &missing_root, CheckerLimits::default())
            .check_component(&root_name()),
        DiagnosticCode::MissingView,
    );

    for (source, expected) in [
        (
            include_str!("fixtures/checker/fail/missing_include.html"),
            DiagnosticCode::MissingTemplate,
        ),
        (
            include_str!("fixtures/checker/fail/missing_parent.html"),
            DiagnosticCode::MissingTemplate,
        ),
    ] {
        assert_code(check(source, CheckerLimits::default()), expected);
    }
}

#[test]
fn diagnostics_have_stable_machine_codes_and_source_locations_without_raw_values() {
    let report = check(
        include_str!("fixtures/checker/fail/unknown_action.html"),
        CheckerLimits::default(),
    );
    let diagnostic = report
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code() == DiagnosticCode::UnknownAction)
        .expect("unknown action diagnostic");

    assert_eq!(
        diagnostic.path().expect("diagnostic path").as_str(),
        ROOT_VIEW
    );
    assert_eq!((diagnostic.line(), diagnostic.column()), (2, 1));
    assert_eq!(diagnostic.component(), Some(&root_name()));
    assert!(!format!("{diagnostic:?}").contains("delete-everything"));
}

#[test]
fn every_checker_resource_dimension_is_hard_bounded() {
    let source = include_str!("fixtures/checker/pass/root.html");
    let dimensions = [
        (
            CheckerLimits::new(16, 128, 8, 32, 512, 64, 32, 32).expect("limits"),
            DiagnosticCode::SourceLimit,
        ),
        (
            CheckerLimits::new(64 * 1024, 1, 8, 32, 512, 64, 32, 32).expect("limits"),
            DiagnosticCode::NodeLimit,
        ),
        (
            CheckerLimits::new(64 * 1024, 128, 8, 1, 512, 64, 32, 32).expect("limits"),
            DiagnosticCode::BranchLimit,
        ),
        (
            CheckerLimits::new(64 * 1024, 128, 8, 32, 1, 64, 32, 32).expect("limits"),
            DiagnosticCode::HtmlTokenLimit,
        ),
        (
            CheckerLimits::new(64 * 1024, 128, 8, 32, 512, 1, 32, 32).expect("limits"),
            DiagnosticCode::AttributeLimit,
        ),
        (
            CheckerLimits::new(64 * 1024, 128, 8, 32, 512, 64, 1, 32).expect("limits"),
            DiagnosticCode::StackDepthLimit,
        ),
    ];
    for (limits, expected) in dimensions {
        assert_code(check(source, limits), expected);
    }

    let recursive = TemplateCatalog::new(vec![
        (
            view(ROOT_VIEW),
            include_str!("fixtures/checker/fail/recursive_include.html"),
        ),
        (
            view(CHILD_VIEW),
            include_str!("fixtures/checker/pass/child.html"),
        ),
    ])
    .expect("template catalog");
    let report = TemplateChecker::new(
        &registry(),
        &recursive,
        CheckerLimits::new(64 * 1024, 128, 1, 32, 512, 64, 32, 32).expect("limits"),
    )
    .check_component(&root_name());
    assert_code(report, DiagnosticCode::IncludeDepthLimit);

    let diagnostic_limited = check(
        "<section><button live:click=\"one\"></button><button live:click=\"two\"></button></section>",
        CheckerLimits::new(1024, 128, 8, 32, 512, 64, 32, 1).expect("limits"),
    );
    assert_eq!(diagnostic_limited.diagnostics().len(), 1);
    assert_code(diagnostic_limited, DiagnosticCode::DiagnosticLimit);
}

#[test]
fn expanded_includes_and_inherited_blocks_remain_bounded_and_structural() {
    let registry = registry();
    let inherited = TemplateCatalog::new(vec![
        (
            view(ROOT_VIEW),
            include_str!("fixtures/checker/fail/inherited_mismatch.html"),
        ),
        (
            view("tests/layout.html"),
            include_str!("fixtures/checker/pass/layout.html"),
        ),
        (
            view(CHILD_VIEW),
            include_str!("fixtures/checker/pass/child.html"),
        ),
    ])
    .expect("template catalog");
    assert_code(
        TemplateChecker::new(&registry, &inherited, CheckerLimits::default())
            .check_component(&root_name()),
        DiagnosticCode::BranchStackMismatch,
    );

    let expanded = TemplateCatalog::new(vec![
        (
            view(ROOT_VIEW),
            "<section>{% include \"tests/shared.html\" %}</section>",
        ),
        (
            view("tests/shared.html"),
            "<article><p>included content expands the complete checked branch</p></article>",
        ),
        (
            view(CHILD_VIEW),
            include_str!("fixtures/checker/pass/child.html"),
        ),
    ])
    .expect("template catalog");
    assert_code(
        TemplateChecker::new(
            &registry,
            &expanded,
            CheckerLimits::new(80, 128, 8, 32, 512, 64, 32, 32).expect("limits"),
        )
        .check_component(&root_name()),
        DiagnosticCode::SourceLimit,
    );
}

#[test]
fn dynamic_upload_attribute_structure_is_explicitly_unproved_never_statically_proved() {
    let report = check(r#"<input {{ attrs }}>"#, CheckerLimits::default());

    assert!(!report.is_proved());
    assert!(report.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == DiagnosticCode::DynamicStructureUnproved
            && diagnostic.severity() == DiagnosticSeverity::Unproved
    }));
}

#[test]
fn dynamic_iteration_004_directive_values_are_explicitly_unproved() {
    for source in [
        r#"<input type="file" live:upload="{{ field }}">"#,
        r#"<output live:progress="{{ field }}" role="progressbar" aria-label="Upload progress"></output>"#,
        r#"<section live:stream="{{ subscription }}"></section>"#,
    ] {
        let report = check(source, CheckerLimits::default());
        assert!(!report.is_proved(), "dynamic source was proved: {source}");
        assert!(
            report.diagnostics().iter().any(|diagnostic| {
                diagnostic.code() == DiagnosticCode::DynamicStructureUnproved
                    && diagnostic.severity() == DiagnosticSeverity::Unproved
            }),
            "missing explicit unproved diagnostic for {source}: {:?}",
            report.diagnostics()
        );
    }
}

proptest! {
    #[test]
    fn arbitrary_bounded_utf8_never_panics_or_exposes_unbounded_diagnostics(
        source in ".{0,512}"
    ) {
        let report = check(
            &source,
            CheckerLimits::new(1024, 64, 4, 16, 128, 32, 16, 8).expect("limits"),
        );
        prop_assert!(report.diagnostics().len() <= 8);
    }
}

fn check(source: &str, limits: CheckerLimits) -> suprnova_live::checker::CheckReport {
    let registry = registry();
    let catalog = TemplateCatalog::new(vec![
        (view(ROOT_VIEW), source.to_owned()),
        (
            view(CHILD_VIEW),
            include_str!("fixtures/checker/pass/child.html").to_owned(),
        ),
    ])
    .expect("template catalog");
    TemplateChecker::new(&registry, &catalog, limits).check_component(&root_name())
}

fn assert_code(report: suprnova_live::checker::CheckReport, expected: DiagnosticCode) {
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == expected),
        "missing {expected:?}: {:?}",
        report.diagnostics()
    );
}
