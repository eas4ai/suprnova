//! Positive checker contracts.

mod checker_support;

use serde_json::Value;
use suprnova_live::checker::{
    CheckerLimits, DIRECTIVE_ARGUMENT_FORMS, DIRECTIVE_CONTRACTS, DIRECTIVE_FALLBACKS,
    DIRECTIVE_FIXTURE_MANIFEST_SHA256, DIRECTIVE_GRAMMAR_VERSION, DIRECTIVE_LITERAL_KINDS,
    DIRECTIVE_TARGET_KINDS, DirectiveFallback, DirectiveOwner, DirectivePhase, DirectiveValue,
    RESERVED_DIRECTIVES, TemplateCatalog, TemplateChecker, directive_contract,
};
use suprnova_live::conformance::{
    FixtureVersion, expected_fixture_manifest_sha256_version, fixture_directory,
};

use checker_support::{CHILD_VIEW, ROOT_VIEW, registry, root_name, view};

#[test]
fn registered_directives_nested_ownership_and_compatible_branches_are_proved() {
    let registry = registry();
    let catalog = TemplateCatalog::new(vec![
        (
            view(ROOT_VIEW),
            include_str!("fixtures/checker/pass/root.html"),
        ),
        (
            view(CHILD_VIEW),
            include_str!("fixtures/checker/pass/child.html"),
        ),
    ])
    .expect("template catalog");
    let report = TemplateChecker::new(&registry, &catalog, CheckerLimits::default())
        .check_component(&root_name());

    assert!(report.is_proved(), "{:?}", report.diagnostics());
    assert!(report.diagnostics().is_empty());
}

#[test]
fn includes_and_inheritance_are_resolved_through_the_bounded_catalog() {
    let registry = registry();
    let catalog = TemplateCatalog::new(vec![
        (
            view(ROOT_VIEW),
            include_str!("fixtures/checker/pass/inherited.html"),
        ),
        (
            view("tests/layout.html"),
            include_str!("fixtures/checker/pass/layout.html"),
        ),
        (
            view("tests/shared.html"),
            include_str!("fixtures/checker/pass/shared.html"),
        ),
        (
            view(CHILD_VIEW),
            include_str!("fixtures/checker/pass/child.html"),
        ),
    ])
    .expect("template catalog");
    let report = TemplateChecker::new(&registry, &catalog, CheckerLimits::default())
        .check_component(&root_name());

    assert!(report.is_proved(), "{:?}", report.diagnostics());
}

#[test]
fn iteration_004_directives_are_generated_from_the_reviewed_fixture() {
    let fixture: Value = serde_json::from_slice(
        &std::fs::read(fixture_directory(FixtureVersion::V4).join("directive-grammar.json"))
            .expect("directive fixture is readable"),
    )
    .expect("directive fixture is valid JSON");
    let expected: Vec<_> = fixture["directives"]
        .as_array()
        .expect("directives are an array")
        .iter()
        .map(|entry| entry["name"].as_str().expect("directive name"))
        .collect();
    let actual: Vec<_> = DIRECTIVE_CONTRACTS
        .iter()
        .map(|contract| contract.name)
        .collect();

    assert_eq!(actual, expected);
    let syntax = &fixture["syntax"];
    assert_eq!(
        DIRECTIVE_TARGET_KINDS,
        fixture_strings(&syntax["target_kinds"])
    );
    assert_eq!(
        DIRECTIVE_LITERAL_KINDS,
        fixture_strings(&syntax["literal_kinds"])
    );
    assert_eq!(
        DIRECTIVE_ARGUMENT_FORMS,
        fixture_strings(&syntax["argument_forms"])
    );
    assert_eq!(DIRECTIVE_FALLBACKS, fixture_strings(&syntax["fallbacks"]));
    for (contract, entry) in DIRECTIVE_CONTRACTS.iter().zip(
        fixture["directives"]
            .as_array()
            .expect("directives are an array"),
    ) {
        assert_eq!(owner_name(contract.owner), entry["owner"]);
        assert_eq!(value_name(contract.value), entry["value"]);
        assert_eq!(phase_name(contract.phase), entry["phase"]);
        assert_eq!(fallback_name(contract.fallback), entry["fallback"]);
        assert_eq!(contract.roles, fixture_strings(&entry["roles"]));
        assert_eq!(contract.conflicts, fixture_strings(&entry["conflicts"]));
        assert_eq!(contract.capability, entry["capability"].as_str());
        let modifiers = if let Some(group) = entry["modifiers"].as_str() {
            fixture_strings(&fixture[format!("{group}_modifiers")])
        } else {
            fixture_strings(&entry["modifiers"])
        };
        assert_eq!(contract.modifiers, modifiers);
    }
    assert!(RESERVED_DIRECTIVES.is_empty());
    assert_eq!(
        DIRECTIVE_FIXTURE_MANIFEST_SHA256,
        expected_fixture_manifest_sha256_version(FixtureVersion::V4).expect("v4 manifest")
    );
}

#[test]
fn iteration_four_directives_have_closed_capability_contracts() {
    assert_eq!(DIRECTIVE_GRAMMAR_VERSION, 2);
    let upload = directive_contract("upload").expect("upload contract");
    assert_eq!(upload.capability, Some("uploads@1"));
    assert_eq!(upload.roles, ["cancel", "retry", "remove"]);
    assert_eq!(upload.conflicts, ["model"]);
    assert_eq!(upload.fallback, DirectiveFallback::Native);

    let progress = directive_contract("progress").expect("progress contract");
    assert_eq!(progress.capability, Some("uploads@1"));
    assert!(progress.roles.is_empty());
    assert_eq!(progress.fallback, DirectiveFallback::Inert);

    let poll = directive_contract("poll").expect("poll contract");
    assert_eq!(poll.capability, Some("async@1"));
    assert_eq!(
        poll.modifiers,
        ["immediate", "visible", "always", "5s", "15s", "30s", "60s"]
    );
    assert_eq!(poll.fallback, DirectiveFallback::Inert);

    let stream = directive_contract("stream").expect("stream contract");
    assert_eq!(stream.capability, Some("async@1"));
    assert_eq!(stream.modifiers, ["push-only", "hybrid"]);
    assert_eq!(stream.fallback, DirectiveFallback::Inert);
}

#[test]
fn iteration_four_directive_roles_and_modifiers_are_statically_proved() {
    let registry = registry();
    let catalog = TemplateCatalog::new(vec![
        (
            view(ROOT_VIEW),
            r#"<section live:stream.hybrid="orders" live:poll.visible.30s="refresh">
                <input type="file" live:upload="avatar">
                <button live:upload.cancel="avatar">Cancel</button>
                <button live:upload.retry="avatar">Retry</button>
                <button live:upload.remove="avatar">Remove</button>
                <output live:progress="avatar"></output>
            </section>"#,
        ),
        (
            view(CHILD_VIEW),
            include_str!("fixtures/checker/pass/child.html"),
        ),
    ])
    .expect("template catalog");
    let report = TemplateChecker::new(&registry, &catalog, CheckerLimits::default())
        .check_component(&root_name());

    assert!(report.is_proved(), "{:?}", report.diagnostics());
}

fn fixture_strings(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .expect("fixture value is an array")
        .iter()
        .map(|entry| entry.as_str().expect("fixture value is a string"))
        .collect()
}

const fn owner_name(value: DirectiveOwner) -> &'static str {
    match value {
        DirectiveOwner::Island => "island",
        DirectiveOwner::KeyedScope => "keyed_scope",
        DirectiveOwner::Element => "element",
    }
}

const fn value_name(value: DirectiveValue) -> &'static str {
    match value {
        DirectiveValue::Empty => "empty",
        DirectiveValue::Identifier => "identifier",
        DirectiveValue::Literal => "literal",
        DirectiveValue::Field => "field",
        DirectiveValue::Action => "action",
        DirectiveValue::Target => "target",
        DirectiveValue::Mapping => "mapping",
    }
}

const fn phase_name(value: DirectivePhase) -> &'static str {
    match value {
        DirectivePhase::Local => "local",
        DirectivePhase::Schedule => "schedule",
        DirectivePhase::Feedback => "feedback",
        DirectivePhase::Morph => "morph",
        DirectivePhase::Navigation => "navigation",
    }
}

const fn fallback_name(value: DirectiveFallback) -> &'static str {
    match value {
        DirectiveFallback::Inert => "inert",
        DirectiveFallback::Native => "native",
        DirectiveFallback::RetainDom => "retain_dom",
    }
}

#[test]
fn every_iteration_003_directive_is_statically_proved() {
    let registry = registry();
    let catalog = TemplateCatalog::new(vec![
        (
            view(ROOT_VIEW),
            include_str!("fixtures/checker/pass/iteration-003-directives.html"),
        ),
        (
            view(CHILD_VIEW),
            include_str!("fixtures/checker/pass/child.html"),
        ),
    ])
    .expect("template catalog");
    let report = TemplateChecker::new(&registry, &catalog, CheckerLimits::default())
        .check_component(&root_name());

    assert!(report.is_proved(), "{:?}", report.diagnostics());
}

#[test]
fn iteration_003_signal_safe_integer_boundary_is_statically_proved() {
    let registry = registry();
    let catalog = TemplateCatalog::new(vec![
        (
            view(ROOT_VIEW),
            r#"<section live:signal="count:9007199254740991"></section>"#,
        ),
        (
            view(CHILD_VIEW),
            include_str!("fixtures/checker/pass/child.html"),
        ),
    ])
    .expect("template catalog");
    let report = TemplateChecker::new(&registry, &catalog, CheckerLimits::default())
        .check_component(&root_name());

    assert!(report.is_proved(), "{:?}", report.diagnostics());
}
