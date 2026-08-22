//! Positive checker contracts.

mod checker_support;

use suprnova_live::checker::{CheckerLimits, TemplateCatalog, TemplateChecker};

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
