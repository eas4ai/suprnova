//! Checker limits, source stability, and parser-boundary regressions.

mod checker_support;

use proptest::prelude::*;
use suprnova_live::checker::{
    CheckerLimits, DiagnosticCode, DiagnosticSeverity, TemplateCatalog, TemplateChecker,
};

use checker_support::{CHILD_VIEW, MODEL_CHILD_VIEW, ROOT_VIEW, registry, root_name, view};

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

// Spec 11 scopes targeted feedback to an action, a field or the island; the
// checker once resolved every feedback target as an action.
#[test]
fn a_feedback_directive_targets_a_model_field_or_an_action() {
    let registry = registry();
    let nested = |feedback: &str| {
        TemplateCatalog::new(vec![
            (
                view(ROOT_VIEW),
                format!(
                    r#"<section live:component="tests.model-child" live:key="model-child">
                           <input live:model.change="avatar">
                           <span hidden live:{feedback}></span>
                       </section>"#
                ),
            ),
            (
                view(CHILD_VIEW),
                include_str!("fixtures/checker/pass/child.html").to_owned(),
            ),
            (view(MODEL_CHILD_VIEW), "<div></div>".to_owned()),
        ])
        .expect("nested island template catalog")
    };
    let proved = TemplateChecker::new(
        &registry,
        &nested(r#"queued.show="avatar""#),
        CheckerLimits::default(),
    )
    .check_component(&root_name());
    assert!(proved.is_proved(), "{:?}", proved.diagnostics());
    let unknown = TemplateChecker::new(
        &registry,
        &nested(r#"loading.show="nowhere""#),
        CheckerLimits::default(),
    )
    .check_component(&root_name());
    assert_code(unknown, DiagnosticCode::UnknownAction);
}

/// LIVE-024: a template keys a morph scope with `live:key` alone, the one
/// stable-key attribute the checker validates and the runtime reads; the
/// engine's `data-suprnova-live-key` spelling is not a directive and a
/// duplicate `live:key` is still refused.
#[test]
fn live_key_alone_names_a_keyed_scope_and_a_duplicate_is_refused() {
    let proved = check(
        r#"<details live:key="notes" live:preserve.self><summary>Notes</summary><p>Body</p></details>
           <ul><li live:key="one">One</li><li live:key="two">Two</li></ul>"#,
        CheckerLimits::default(),
    );
    assert!(proved.is_proved(), "{:?}", proved.diagnostics());
    let duplicate = check(
        r#"<details live:key="notes" live:preserve.self><summary>Notes</summary></details>
           <p live:key="notes">Twice</p>"#,
        CheckerLimits::default(),
    );
    assert!(!duplicate.is_proved());
    assert_code(duplicate, DiagnosticCode::DuplicateKey);
}

/// A macro that splices `caller()`, as the library's validation summary does.
const SUMMARY_VIEW: &str = "tests/summary.html";
const SUMMARY_MACRO: &str = r#"{% macro summary(action) %}<section live:error.live.polite="{{ action }}"><h2>Please correct</h2>{{ caller() }}</section>{% endmacro %}"#;

fn check_with_summary(root: &str) -> suprnova_live::checker::CheckReport {
    let registry = registry();
    let catalog = TemplateCatalog::new(vec![
        (view(ROOT_VIEW), root.to_owned()),
        (view(SUMMARY_VIEW), SUMMARY_MACRO.to_owned()),
        (
            view(CHILD_VIEW),
            include_str!("fixtures/checker/pass/child.html").to_owned(),
        ),
    ])
    .expect("template catalog");
    TemplateChecker::new(&registry, &catalog, CheckerLimits::default())
        .check_component(&root_name())
}

/// LIVE-025: an empty call block is empty caller content, so everything the
/// view renders after the call is still checked. Before the fix the empty
/// caller rendered zero branches and this view proved clean.
#[test]
fn checker_proof_covers_the_view_after_an_empty_call_block() {
    let report = check_with_summary(
        r#"{% import "tests/summary.html" as s %}{% call s::summary("save") %}{% endcall %}
           <button type="button" live:click="no_such_action">Probe</button>"#,
    );
    assert!(!report.is_proved());
    assert_code(report, DiagnosticCode::UnknownAction);

    let clean = check_with_summary(
        r#"{% import "tests/summary.html" as s %}{% call s::summary("save") %}{% endcall %}
           <button type="button" live:click="refresh">Refresh</button>"#,
    );
    assert!(clean.is_proved(), "{:?}", clean.diagnostics());
}

/// LIVE-025: a call block with content splices it once, and a failure inside
/// the caller content is still reported.
#[test]
fn checker_proof_checks_caller_content() {
    let report = check_with_summary(
        r#"{% import "tests/summary.html" as s %}{% call s::summary("save") %}<button type="button" live:click="no_such_action">Inside</button>{% endcall %}"#,
    );
    assert!(!report.is_proved());
    assert_code(report, DiagnosticCode::UnknownAction);
}

/// LIVE-027: error feedback may target an action the component declares, the
/// scope a validation summary names, or a declared field; an undeclared
/// name and a secret field are still refused.
#[test]
fn live_error_target_is_a_declared_field_or_action() {
    for source in [
        r#"<section live:error.live.polite="save"><h2>Please correct</h2></section>"#,
        r#"<p live:error="query"></p>"#,
    ] {
        let report = check(source, CheckerLimits::default());
        assert!(report.is_proved(), "{source}: {:?}", report.diagnostics());
    }
    let unknown = check(r#"<p live:error="nowhere"></p>"#, CheckerLimits::default());
    assert!(!unknown.is_proved());
    let secret = check(r#"<p live:error="secret"></p>"#, CheckerLimits::default());
    assert_code(secret, DiagnosticCode::ForbiddenModel);
}

/// LIVE-029: a `live:submit` form is refused past the 127 model fields one
/// request carries with its action. The registry declares one model field,
/// so the fixture binds the same field names the checker then reports as
/// unknown; the proposal bound is reported on its own code either way.
#[test]
fn submit_form_proposals_are_bounded_by_one_request() {
    // The test registry declares none of these fields, so each control also
    // reports an unknown model; room for all of them keeps the bound's own
    // report from being cut at the default diagnostic ceiling.
    let limits = CheckerLimits::new(256 * 1024, 8_192, 16, 128, 32_768, 2_048, 256, 1_024)
        .expect("checker limits within the engine maxima");
    let form = |fields: usize| {
        let controls: String = (0..fields)
            .map(|index| format!(r#"<input aria-label="f{index}" live:model="field_{index}">"#))
            .collect();
        format!(r#"<form live:submit.prevent="save">{controls}</form>"#)
    };
    let within = check(&form(127), limits);
    assert!(
        !within
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::SubmitProposalLimit),
        "127 fields fit one request"
    );
    let over = check(&form(128), limits);
    let reported = over
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code() == DiagnosticCode::SubmitProposalLimit)
        .count();
    assert_eq!(reported, 1, "one report per form: {:?}", over.diagnostics());

    let outside = format!(
        r#"<form live:submit.prevent="save"><input aria-label="q" live:model="query"></form>{}"#,
        (0..130)
            .map(|index| format!(r#"<input aria-label="o{index}" live:model="other_{index}">"#))
            .collect::<String>()
    );
    let outside = check(&outside, limits);
    assert!(
        !outside
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::SubmitProposalLimit),
        "controls outside the form do not count"
    );
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
