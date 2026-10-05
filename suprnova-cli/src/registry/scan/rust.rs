//! The Rust scan (REG-030): `syn` parses each `.rs` file; every path is
//! resolved through its `use`, alias and rename, then classified against
//! the allowlist, the component's own items, its dependencies' modules and
//! the admitted part of `std`. Anything else, and every construct that
//! could run or read at build time, is refused.

mod attrs;
mod modules;
mod ty;
mod walk;

use std::collections::BTreeSet;

use super::allowlist::Allowlist;
use super::{ComponentFiles, Finding, ScanReport};
use crate::registry::Result;

pub(crate) use walk::DefinedComponent;

/// Scans the component's Rust files.
pub fn scan(component: &ComponentFiles<'_>, allowlist: &Allowlist) -> Result<ScanReport> {
    scan_detailed(component, allowlist).map(|(report, _)| report)
}

/// Scans the component's Rust files and also returns where each Live
/// component it defines sits, for the check against the manifest's
/// `register`.
pub(crate) fn scan_detailed(
    component: &ComponentFiles<'_>,
    allowlist: &Allowlist,
) -> Result<(ScanReport, Vec<DefinedComponent>)> {
    let namespace_module = component.namespace.replace('-', "_");
    let mut report = ScanReport::default();
    let mut parsed = Vec::new();
    for (name, bytes) in component.files {
        let Some(stem) = name.strip_suffix(".rs") else {
            continue;
        };
        let Ok(text) = std::str::from_utf8(bytes) else {
            report.findings.push(finding(
                "rust-parse",
                name,
                None,
                "the file is not UTF-8".to_string(),
            ));
            continue;
        };
        if let Err(too_deep) = super::limits::check(text, super::limits::Language::Rust) {
            report.findings.push(finding(
                "rust-limit",
                name,
                Some(too_deep.line),
                too_deep.message,
            ));
            continue;
        }
        match syn::parse_file(text) {
            Ok(file) => parsed.push((name.clone(), stem.to_string(), file)),
            Err(error) => report.findings.push(finding(
                "rust-parse",
                name,
                modules::line_of(error.span()),
                format!("the file is not Rust the scan can parse: {error}"),
            )),
        }
    }
    if parsed.is_empty() {
        return Ok((report, Vec::new()));
    }
    let mut table = modules::ModuleTable::default();
    for (_, stem, file) in &parsed {
        let module = vec![
            "crate".to_string(),
            "live".to_string(),
            namespace_module.clone(),
            stem.clone(),
        ];
        table.add_file(module, &file.items);
    }
    let mut views: BTreeSet<String> = component
        .files
        .iter()
        .filter(|(name, _)| name.ends_with(".html"))
        .map(|(name, _)| format!("{}-ui/{}/{name}", component.namespace, component.directory))
        .collect();
    views.extend(component.importable_views.iter().cloned());
    let dependency_modules: Vec<Vec<String>> = component
        .dependency_modules
        .iter()
        .map(|module| {
            let segments: Vec<String> = module.split("::").map(str::to_string).collect();
            // Callers list each dependency as `<namespace_module>::<stem>`,
            // relative to `crate::live`; a full `crate::` path is read as is.
            if segments.first().is_some_and(|first| first == "crate") {
                segments
            } else {
                let mut full = vec!["crate".to_string(), "live".to_string()];
                full.extend(segments);
                full
            }
        })
        .collect();
    let mut walker = walk::Walker::new(
        allowlist,
        component.namespace,
        views,
        dependency_modules,
        &table,
    );
    walker.gather_impls();
    for (name, stem, file) in &parsed {
        walker.file = name.clone();
        walker.module = vec![
            "crate".to_string(),
            "live".to_string(),
            namespace_module.clone(),
            stem.clone(),
        ];
        walker.check_attrs(&file.attrs, walk::Site::Other);
        walker.visit_items(&file.items);
    }
    report
        .capabilities
        .extend(walker.capabilities.iter().copied());
    report.findings.extend(walker.findings.iter().cloned());
    report
        .defined_components
        .extend(walker.defined.iter().map(|defined| defined.path.clone()));
    Ok((report, walker.defined.clone()))
}

/// The 1-based line a span starts on, for findings outside the walker.
pub(crate) fn line_of_span(span: proc_macro2::Span) -> Option<u32> {
    modules::line_of(span)
}

fn finding(check: &'static str, file: &str, line: Option<u32>, message: String) -> Finding {
    Finding {
        check,
        file: file.to_string(),
        line,
        message,
    }
}
