//! `live:check`'s view checks on a component's views, before install and
//! without compiling the application (REG-022). The checker is the one the
//! application's Live tooling helper runs, from the Live engine crate; the
//! registry it checks against is built from the contracts `contract.rs`
//! reads from the component's Rust, and its template catalog holds the
//! component's views, the views of the components it depends on and the
//! shipped library's.

use suprnova_live::checker::{CheckerLimits, DiagnosticSeverity, TemplateCatalog, TemplateChecker};
use suprnova_live::identity::ViewName;
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};

use super::contract::Contracts;
use super::{ComponentFiles, Finding};

/// Runs the view checker for every component the Rust declares and turns
/// each error and unproved structure into a refusal naming the view and
/// line.
pub(crate) fn check_views(
    component: &ComponentFiles<'_>,
    contracts: &Contracts,
    dependency_views: &[(String, String)],
) -> Vec<Finding> {
    if contracts.contracts.is_empty() {
        return Vec::new();
    }
    let own_prefix = format!("{}-ui/{}/", component.namespace, component.directory);
    let mut findings = Vec::new();
    let mut sources: Vec<(ViewName, String)> = Vec::new();
    let mut add =
        |path: String, source: String, findings: &mut Vec<Finding>| match ViewName::parse(&path) {
            Ok(view) => {
                if !sources.iter().any(|(known, _)| known == &view) {
                    sources.push((view, source));
                }
            }
            Err(_) => findings.push(Finding {
                check: "view-contract",
                file: path.clone(),
                line: None,
                message: format!("`{path}` is not a view name the checker accepts"),
            }),
        };
    for (name, bytes) in component.files {
        if !name.ends_with(".html") {
            continue;
        }
        // A view that is not UTF-8 is refused by the view scan.
        if let Ok(text) = std::str::from_utf8(bytes) {
            add(
                format!("{own_prefix}{name}"),
                text.to_string(),
                &mut findings,
            );
        }
    }
    for (path, source) in dependency_views {
        add(path.clone(), source.clone(), &mut findings);
    }
    for shipped in crate::registry::fetch::COMPONENTS {
        for (name, source) in shipped.files {
            if name.ends_with(".html") {
                add(
                    format!("suprnova-ui/{}/{name}", shipped.directory),
                    (*source).to_string(),
                    &mut findings,
                );
            }
        }
    }
    let catalog = match TemplateCatalog::new(sources) {
        Ok(catalog) => catalog,
        Err(_) => {
            findings.push(Finding {
                check: "view-contract",
                file: own_prefix,
                line: None,
                message: "the checker's template catalog holds one view twice".to_string(),
            });
            return findings;
        }
    };
    let mut builder = ComponentRegistryBuilder::new();
    for contract in &contracts.contracts {
        builder = match builder.register(ComponentDescriptor::new(contract.metadata.clone())) {
            Ok(builder) => builder,
            Err(error) => {
                findings.push(Finding {
                    check: "view-contract",
                    file: file_of(contract.metadata.view(), &own_prefix),
                    line: None,
                    message: format!(
                        "`{}` cannot be registered: {error}",
                        contract.metadata.identity().as_str()
                    ),
                });
                return findings;
            }
        };
    }
    let registry = builder.build();
    let checker = TemplateChecker::new(&registry, &catalog, CheckerLimits::default());
    for contract in &contracts.contracts {
        let identity = contract.metadata.identity();
        let report = checker.check_component(identity);
        for diagnostic in report.diagnostics() {
            let (check, what) = match diagnostic.severity() {
                DiagnosticSeverity::Error => ("view-contract", "error"),
                DiagnosticSeverity::Unproved => ("view-unproved", "unproved structure"),
            };
            let view = diagnostic.path().unwrap_or(contract.metadata.view());
            let finding = Finding {
                check,
                file: file_of(view, &own_prefix),
                line: Some(diagnostic.line().max(1)),
                message: format!(
                    "`live:check` reports {what} `{}` in `{}` for `{}`",
                    diagnostic.code().as_str(),
                    view.as_str(),
                    identity.as_str()
                ),
            };
            if !findings.contains(&finding) {
                findings.push(finding);
            }
        }
    }
    findings
}

/// The manifest file name of one of the component's own views, or the
/// view's path for a view it includes from elsewhere.
fn file_of(view: &ViewName, own_prefix: &str) -> String {
    match view.as_str().strip_prefix(own_prefix) {
        Some(name) => name.to_string(),
        None => view.as_str().to_string(),
    }
}
