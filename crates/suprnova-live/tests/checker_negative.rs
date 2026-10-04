//! Negative checker fixtures.

mod checker_support;
#[path = "checker_support/freshness.rs"]
mod freshness;

use serde_json::Value;
use suprnova_live::checker::{
    CheckerLimits, DiagnosticCode, DiagnosticSeverity, TemplateCatalog, TemplateChecker,
};
use suprnova_live::conformance::{FixtureVersion, fixture_directory};

use checker_support::{
    CHILD_VIEW, ROOT_VIEW, registry, registry_with_checker_contract, root_name, view,
};
use freshness::freshness_source;

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
fn iteration_004_upload_model_conflicts_are_island_wide() {
    for source in [
        r#"<input type="file" live:upload="avatar"><input live:model.change="avatar">"#,
        r#"<input live:model.change="avatar"><button live:upload.remove="avatar">Remove</button>"#,
        r#"<section live:upload.remove="avatar"><input live:model.change="avatar"></section>"#,
    ] {
        let report = check(source);
        let conflicts = report
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code() == DiagnosticCode::InvalidModifier)
            .count();
        assert_eq!(
            conflicts,
            1,
            "island-wide upload/model conflict multiplicity for {source}: {:?}",
            report.diagnostics(),
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
fn iteration_004_upload_shape_metadata_ownership_and_accessibility_fail_closed() {
    let cases = [
        (
            r#"<input type="text" live:upload="avatar">"#,
            DiagnosticCode::AccessibilityViolation,
        ),
        (
            r#"<div live:upload="avatar"></div>"#,
            DiagnosticCode::AccessibilityViolation,
        ),
        (
            r#"<input type="file" live:upload="missing_upload">"#,
            DiagnosticCode::UnknownModel,
        ),
        (
            r#"<input type="file" live:upload="query">"#,
            DiagnosticCode::ForbiddenModel,
        ),
        (
            r#"<section live:component="tests.child" live:key="child"><button live:upload.cancel="avatar">Cancel</button></section>"#,
            DiagnosticCode::OwnershipViolation,
        ),
        (
            r#"<section live:component="tests.child" live:key="child"><output live:progress="avatar" role="progressbar" aria-label="Upload progress"></output></section>"#,
            DiagnosticCode::OwnershipViolation,
        ),
        (
            r#"<output live:progress="avatar"></output>"#,
            DiagnosticCode::AccessibilityViolation,
        ),
        (
            r#"<output live:progress="avatar" role="progressbar"></output>"#,
            DiagnosticCode::AccessibilityViolation,
        ),
        (
            r#"<output live:progress="avatar" aria-label="Upload progress"></output>"#,
            DiagnosticCode::AccessibilityViolation,
        ),
    ];

    for (source, expected) in cases {
        let report = check(source);
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == expected),
            "missing {expected:?} for {source}: {:?}",
            report.diagnostics()
        );
    }
}

#[test]
fn iteration_004_stream_event_signal_and_capability_metadata_fail_closed() {
    let sixty_five_bytes = format!("a{}", "z".repeat(64));
    for (source, expected) in [
        (
            r#"<section live:stream="missing"></section>"#.to_owned(),
            DiagnosticCode::InvalidModifier,
        ),
        (
            r#"<span live:on="orders.missing"></span>"#.to_owned(),
            DiagnosticCode::UnknownEvent,
        ),
        (
            r#"<section live:signal="1invalid:true"></section>"#.to_owned(),
            DiagnosticCode::InvalidModifier,
        ),
        (
            r#"<section live:signal="Progress:true"></section>"#.to_owned(),
            DiagnosticCode::InvalidModifier,
        ),
        (
            r#"<section live:signal="prøgress:true"></section>"#.to_owned(),
            DiagnosticCode::InvalidModifier,
        ),
        (
            format!(r#"<section live:signal="{sixty_five_bytes}:true"></section>"#),
            DiagnosticCode::InvalidModifier,
        ),
    ] {
        let report = check(source.clone());
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code() == expected),
            "missing {expected:?} for {source}: {:?}",
            report.diagnostics()
        );
    }

    let registry = registry_with_checker_contract(1);
    for source in [
        r#"<input type="file" live:upload="avatar">"#,
        r#"<output live:progress="avatar" role="progressbar" aria-label="Upload progress"></output>"#,
        r#"<section live:poll.visible.30s></section>"#,
        r#"<section live:stream.hybrid="orders"></section>"#,
    ] {
        let catalog = TemplateCatalog::new(vec![
            (view(ROOT_VIEW), source),
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
            "checker-contract v1 accepted {source}: {:?}",
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
        let source = freshness_source(combination);
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

fn check(source: impl Into<String>) -> suprnova_live::checker::CheckReport {
    let registry = registry();
    let catalog = TemplateCatalog::new(vec![
        (view(ROOT_VIEW), source.into()),
        (
            view(CHILD_VIEW),
            include_str!("fixtures/checker/pass/child.html").to_owned(),
        ),
    ])
    .expect("template catalog");
    TemplateChecker::new(&registry, &catalog, CheckerLimits::default())
        .check_component(&root_name())
}

#[test]
fn a_macro_call_is_checked_as_the_markup_it_expands_to() {
    let registry = registry();
    let catalog = TemplateCatalog::new(vec![
        (
            view(ROOT_VIEW),
            include_str!("fixtures/checker/fail/macro-unknown-model.html"),
        ),
        (
            view("tests/macros.html"),
            include_str!("fixtures/checker/pass/macros.html"),
        ),
        (
            view(CHILD_VIEW),
            include_str!("fixtures/checker/pass/child.html"),
        ),
    ])
    .expect("template catalog");
    let report = TemplateChecker::new(&registry, &catalog, CheckerLimits::default())
        .check_component(&root_name());
    assert!(!report.is_proved());
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == DiagnosticCode::UnknownModel),
        "a literal argument reaches the directive check: {:?}",
        report.diagnostics()
    );
}

#[test]
fn a_call_to_a_macro_the_catalog_cannot_resolve_stays_unproved() {
    let registry = registry();
    let catalog = TemplateCatalog::new(vec![
        (
            view(ROOT_VIEW),
            include_str!("fixtures/checker/fail/macro-missing.html"),
        ),
        (
            view(CHILD_VIEW),
            include_str!("fixtures/checker/pass/child.html"),
        ),
    ])
    .expect("template catalog");
    let report = TemplateChecker::new(&registry, &catalog, CheckerLimits::default())
        .check_component(&root_name());
    assert!(!report.is_proved());
    assert!(report.diagnostics().iter().any(|diagnostic| {
        diagnostic.code() == DiagnosticCode::DynamicStructureUnproved
            && diagnostic.severity() == DiagnosticSeverity::Unproved
    }));
}

fn raw_locations(report: &suprnova_live::checker::CheckReport) -> Vec<(u32, u32)> {
    report
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code() == DiagnosticCode::RawSafe)
        .map(|diagnostic| (diagnostic.line(), diagnostic.column()))
        .collect()
}

/// Unescaped output is classified from the parsed template, not from the
/// expression's text, and reported where the raw value is written out: a
/// binding, a macro argument, a loop variable, or caller content carries the
/// raw value to the `{{ }}` that writes it, and an escaper that does not
/// escape HTML is as raw as `safe`.
#[test]
fn raw_output_is_traced_to_the_place_it_is_written() {
    let cases: &[(&str, (u32, u32))] = &[
        ("{% let y = body|safe %}\n<p>{{ y }}</p>", (2, 7)),
        ("{% set y = body|safe %}\n<p>{{ y }}</p>", (2, 7)),
        (
            "{% let (y, z) = (body|safe, name) %}\n<p>{{ y }}</p>",
            (2, 7),
        ),
        ("<p>{{ body|safe|lower }}</p>", (1, 7)),
        ("<p>{{ body|escape(\"none\") }}</p>", (1, 7)),
        ("<p>{{ body|e(\"none\") }}</p>", (1, 7)),
        ("<p>{{ body|escape(\"txt\") }}</p>", (1, 7)),
        ("<p>{{ body|escape(\"md\") }}</p>", (1, 7)),
        ("<p>{{ body|escape(\"yml\") }}</p>", (1, 7)),
        ("<p>{{ body|escape(\"\") }}</p>", (1, 7)),
        ("<p>{{ body|escape(escaper = \"none\") }}</p>", (1, 7)),
        (
            "{% macro show(value) %}\n<p>{{ value }}</p>\n{% endmacro %}\n{% call show(body|safe) %}{% endcall %}",
            (2, 7),
        ),
        (
            "{% macro show(value = body|safe) %}\n<p>{{ value }}</p>\n{% endmacro %}\n{% call show() %}{% endcall %}",
            (2, 7),
        ),
        (
            "{% let y = body|safe %}{% macro show() %}\n<p>{{ y }}</p>\n{% endmacro %}\n{% call show() %}{% endcall %}",
            (2, 7),
        ),
        (
            "{% macro frame() %}<div>{{ caller() }}</div>{% endmacro %}\n{% call frame() %}<p>{{ body|safe }}</p>{% endcall %}",
            (2, 25),
        ),
        (
            "{% for item in rows|safe %}\n<p>{{ item }}</p>\n{% endfor %}",
            (2, 7),
        ),
        (
            "{% let rows = list|safe %}{% for (key, item) in rows %}\n<p>{{ item }}</p>\n{% endfor %}",
            (2, 7),
        ),
        (
            "{% set y %}<b>{{ body|safe }}</b>{% endset %}<p>{{ y }}</p>",
            (1, 18),
        ),
        (
            "{% decl y %}{% if c %}{% let y = body|safe %}{% else %}{% let y = name %}{% endif %}\n<p>{{ y }}</p>",
            (2, 7),
        ),
        (
            "{% let mut y = name %}{% if c %}{% mut y = body|safe %}{% endif %}\n<p>{{ y }}</p>",
            (2, 7),
        ),
        (
            "{% let mut y = name %}{% mut y = body|safe %}\n<p>{{ y }}</p>",
            (2, 7),
        ),
    ];
    let mismatches: Vec<_> = cases
        .iter()
        .filter_map(|(source, location)| {
            let report = check(*source);
            let found = raw_locations(&report);
            (found != vec![*location]).then_some((*source, *location, found))
        })
        .collect();
    assert!(mismatches.is_empty(), "{mismatches:#?}");

    let filtered = check("{% filter escape(\"none\") %}<p>{{ body }}</p>{% endfilter %}");
    assert!(
        !raw_locations(&filtered).is_empty(),
        "{:?}",
        filtered.diagnostics()
    );
}

/// The same classification proves what does escape: an HTML escaper, a
/// binding that rebinds the name to an escaped value, a macro parameter that
/// shadows a raw local, a `{% set %}` block (Askama stores its rendered body
/// as a string and escapes it on output), and the framework's own
/// `trusted_html` filter, which accepts only an audited `TrustedHtml` value.
#[test]
fn escaped_output_is_not_mistaken_for_raw_output() {
    for source in [
        "<p>{{ body|escape(\"html\") }}</p>",
        "<p>{{ body|e }}</p>",
        "<p>{{ body|escape }}</p>",
        "{% let y = body|safe %}{% let y = name %}<p>{{ y }}</p>",
        "{% let v = body|safe %}{% macro show(v) %}<p>{{ v }}</p>{% endmacro %}{% call show(name) %}{% endcall %}",
        "{% set y %}<b>{{ name }}</b>{% endset %}<p>{{ y }}</p>",
        "<p>{{ island|trusted_html }}</p>",
    ] {
        let report = check(source);
        assert!(report.is_proved(), "{source}: {:?}", report.diagnostics());
    }
}

fn locations_of(
    report: &suprnova_live::checker::CheckReport,
    code: DiagnosticCode,
) -> Vec<(String, u32, u32)> {
    report
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code() == code)
        .map(|diagnostic| {
            (
                diagnostic
                    .path()
                    .map(|path| path.as_str().to_owned())
                    .unwrap_or_default(),
                diagnostic.line(),
                diagnostic.column(),
            )
        })
        .collect()
}

/// A diagnostic names the file, line, and column of what it is about: the
/// attribute for a directive or attribute rule, the tag for an element rule,
/// and the unclosed element when the input ends inside it.
#[test]
fn diagnostics_report_the_real_column_of_the_attribute_or_element() {
    let directive = check(
        "<section>\n  <button type=\"button\" live:click=\"missing\">Go</button>\n</section>",
    );
    assert_eq!(
        locations_of(&directive, DiagnosticCode::UnknownAction),
        vec![(ROOT_VIEW.to_owned(), 2, 25)],
        "{:?}",
        directive.diagnostics()
    );

    let mismatched = check("<section>\n  <div></span></div>\n</section>");
    assert_eq!(
        locations_of(&mismatched, DiagnosticCode::HtmlSyntax),
        vec![(ROOT_VIEW.to_owned(), 2, 8)],
        "{:?}",
        mismatched.diagnostics()
    );

    let unclosed = check("<p>Open</p>\n    <section>");
    assert_eq!(
        locations_of(&unclosed, DiagnosticCode::HtmlSyntax),
        vec![(ROOT_VIEW.to_owned(), 2, 5)],
        "{:?}",
        unclosed.diagnostics()
    );

    let keyed =
        check("<section>\n<p live:key=\"same\">A</p>  <p   live:key=\"same\">B</p>\n</section>");
    assert_eq!(
        locations_of(&keyed, DiagnosticCode::DuplicateKey),
        vec![(ROOT_VIEW.to_owned(), 2, 32)],
        "{:?}",
        keyed.diagnostics()
    );

    let missing = check("<section>\n  {% include \"tests/nowhere.html\" %}\n</section>");
    assert_eq!(
        locations_of(&missing, DiagnosticCode::MissingTemplate),
        vec![(ROOT_VIEW.to_owned(), 2, 3)],
        "{:?}",
        missing.diagnostics()
    );

    let dynamic = check("<section><input {{ attrs }}></section>");
    assert_eq!(
        locations_of(&dynamic, DiagnosticCode::DynamicStructureUnproved),
        vec![(ROOT_VIEW.to_owned(), 1, 20)],
        "{:?}",
        dynamic.diagnostics()
    );

    let registry = registry();
    let included = TemplateCatalog::new(vec![
        (
            view(ROOT_VIEW),
            "<section>\n{% include \"tests/shared.html\" %}\n</section>".to_owned(),
        ),
        (
            view("tests/shared.html"),
            "<p>Shared</p>\n<p><button live:click=\"missing\">Go</button></p>".to_owned(),
        ),
        (
            view(CHILD_VIEW),
            include_str!("fixtures/checker/pass/child.html").to_owned(),
        ),
    ])
    .expect("template catalog");
    let report = TemplateChecker::new(&registry, &included, CheckerLimits::default())
        .check_component(&root_name());
    assert_eq!(
        locations_of(&report, DiagnosticCode::UnknownAction),
        vec![("tests/shared.html".to_owned(), 2, 12)],
        "{:?}",
        report.diagnostics()
    );
}

fn coded(report: &suprnova_live::checker::CheckReport, code: &str) -> Vec<(u32, u32)> {
    report
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code().as_str() == code)
        .map(|diagnostic| (diagnostic.line(), diagnostic.column()))
        .collect()
}

/// An action directive carries positional literal arguments, checked
/// against the action's declared parameters in order: `remove(id: u64)`,
/// `rename(title: String, publish: bool)`, and `note(text: Option<String>)`.
#[test]
fn action_directive_arguments_are_checked_against_the_action_signature() {
    for source in [
        r#"<section><button type="button" live:click="remove(42)">Remove</button></section>"#,
        r#"<section><button type="button" live:click="rename('draft', true)">Rename</button></section>"#,
        r#"<section><button type="button" live:click="rename(&quot;draft&quot;, false)">Rename</button></section>"#,
        r#"<section><button type="button" live:click="note()">Note</button></section>"#,
        r#"<section><button type="button" live:click="note">Note</button></section>"#,
        r#"<section><button type="button" live:click="note(null)">Note</button></section>"#,
        r#"<section><button type="button" live:click="note('hi')">Note</button></section>"#,
        r#"<section><button type="button" live:click="save()">Save</button></section>"#,
        r#"<form live:submit.prevent="remove(7)"><button type="submit">Go</button></form>"#,
    ] {
        let report = check(source);
        assert!(report.is_proved(), "{source}: {:?}", report.diagnostics());
    }

    let cases = [
        ("remove", "a required parameter left out"),
        ("remove()", "a required parameter left out"),
        ("remove(1, 2)", "one argument too many"),
        ("save(1)", "an argument to an action without parameters"),
        ("remove(true)", "a boolean for an integer"),
        ("remove(-1)", "a negative number for an unsigned integer"),
        ("remove(1.5)", "a fraction for an integer"),
        ("remove(null)", "null for a required parameter"),
        ("rename(1, true)", "a number for a string"),
        ("rename('draft')", "the second required parameter left out"),
        ("rename(true, 'draft')", "arguments in the wrong order"),
    ];
    for (value, why) in cases {
        let source = format!(
            "<section>\n  <button type=\"button\" live:click=\"{value}\">Go</button>\n</section>"
        );
        let report = check(source);
        assert_eq!(
            coded(&report, "invalid_action_arguments"),
            vec![(2, 25)],
            "{value} ({why}): {:?}",
            report.diagnostics()
        );
    }

    let malformed = check(
        "<section>\n  <button type=\"button\" live:click=\"remove(42\">Go</button>\n</section>",
    );
    assert_eq!(
        coded(&malformed, "invalid_modifier"),
        vec![(2, 25)],
        "{:?}",
        malformed.diagnostics()
    );
}

fn check_with(templates: Vec<(&str, &str)>) -> suprnova_live::checker::CheckReport {
    let registry = registry();
    let mut sources: Vec<(suprnova_live::identity::ViewName, String)> = templates
        .into_iter()
        .map(|(name, source)| (view(name), source.to_owned()))
        .collect();
    sources.push((
        view(CHILD_VIEW),
        include_str!("fixtures/checker/pass/child.html").to_owned(),
    ));
    let catalog = TemplateCatalog::new(sources).expect("template catalog");
    TemplateChecker::new(&registry, &catalog, CheckerLimits::default())
        .check_component(&root_name())
}

fn has_code(report: &suprnova_live::checker::CheckReport, code: DiagnosticCode) -> bool {
    report
        .diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.code() == code)
}

/// Askama treats `{{ show(x) }}` and `{{ ui::show(x) }}` as a macro call, so
/// the macro's markup is written there and is checked there.
#[test]
fn a_macro_called_as_an_expression_is_expanded_and_checked() {
    let macro_source = "{% macro show(v) %}\n<p>{{ v|safe }}</p>\n{% endmacro %}";
    for call in ["{{ show(notice) }}", "{{ (show(notice)) }}"] {
        let report = check(format!("{macro_source}<section>{call}</section>"));
        assert_eq!(
            raw_locations(&report),
            vec![(2, 7)],
            "{call}: {:?}",
            report.diagnostics()
        );
    }
    let directive = check(
        "{% macro go() %}<button type=\"button\" live:click=\"missing\">Go</button>{% endmacro %}<section>{{ go() }}</section>",
    );
    assert!(
        has_code(&directive, DiagnosticCode::UnknownAction),
        "{:?}",
        directive.diagnostics()
    );
    let scoped = check_with(vec![
        (
            ROOT_VIEW,
            "{% import \"tests/macros.html\" as ui %}<form live:submit=\"save\">{{ ui::input(\"nope\") }}</form>",
        ),
        (
            "tests/macros.html",
            include_str!("fixtures/checker/pass/macros.html"),
        ),
    ]);
    assert!(
        has_code(&scoped, DiagnosticCode::UnknownModel),
        "{:?}",
        scoped.diagnostics()
    );
}
