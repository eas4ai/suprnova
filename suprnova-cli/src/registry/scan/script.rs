//! The script scan (REG-032): each `.js` file parsed as a JavaScript
//! module; every call must resolve to a function the script defines or a
//! standard browser API, and every URL must stay on the application's
//! origin.

mod lists;
mod walk;

use std::collections::BTreeSet;

use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

use super::{ComponentFiles, Finding, ScanReport};
use crate::registry::Result;

pub use lists::{
    ADMITTED_CONSTRUCTORS, ADMITTED_GLOBALS, ADMITTED_METHODS, ARGUMENT_RULES, CONSTRUCTOR_RULES,
    GLOBAL_OBJECTS, IMPLICITLY_CALLED, READ_ONLY_PROPERTIES, REFUSED_ELEMENTS, REFUSED_PROPERTIES,
    Rule, URL_ATTRIBUTES, URL_CSS_PROPERTIES, URL_PROPERTIES,
};

/// Scans the component's scripts.
pub fn scan(component: &ComponentFiles<'_>) -> Result<ScanReport> {
    scan_with_elements(component, None)
}

/// Scans the component's scripts, also checking that every element
/// `customElements.define` defines is one the manifest declares.
pub fn scan_with_elements(
    component: &ComponentFiles<'_>,
    elements: Option<&[String]>,
) -> Result<ScanReport> {
    let mut report = ScanReport::default();
    let own_scripts: BTreeSet<String> = component
        .files
        .iter()
        .filter(|(name, _)| name.ends_with(".js"))
        .map(|(name, _)| name.clone())
        .collect();
    let importable_directories: BTreeSet<String> = component
        .importable_views
        .iter()
        .filter_map(|view| {
            view.rsplit_once('/')
                .map(|(directory, _)| directory.to_string())
        })
        .collect();
    let element_prefix = if component.namespace == "suprnova" {
        "sn-".to_string()
    } else {
        format!("{}-", component.namespace)
    };
    for (name, bytes) in component.files {
        if !name.ends_with(".js") {
            continue;
        }
        let Ok(text) = std::str::from_utf8(bytes) else {
            report.findings.push(finding(
                name,
                None,
                "script-parse",
                "the file is not UTF-8".to_string(),
            ));
            continue;
        };
        let line_starts: Vec<usize> = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(index, _)| index + 1))
            .collect();
        if let Some(line) = html_like_comment(text) {
            report.findings.push(finding(
                name,
                Some(line),
                "script-parse",
                "an HTML-like comment (`<!--` or `-->`) hides code from a module parser that a classic script runs, or the reverse".to_string(),
            ));
            continue;
        }
        if let Err(too_deep) = super::limits::check(text, super::limits::Language::Script) {
            report.findings.push(finding(
                name,
                Some(too_deep.line),
                "script-limit",
                too_deep.message,
            ));
            continue;
        }
        let context = walk::Context {
            file: name.clone(),
            line_starts,
            element_prefix: element_prefix.clone(),
            elements,
            own_scripts: own_scripts.clone(),
            own_directory: format!("{}-ui/{}", component.namespace, component.directory),
            importable_directories: importable_directories.clone(),
        };
        report.findings.extend(scan_script(text, &context));
    }
    Ok(report)
}

fn scan_script(text: &str, context: &walk::Context<'_>) -> Vec<Finding> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, text, SourceType::mjs()).parse();
    if parsed.panicked || !parsed.diagnostics.is_empty() {
        let offset = parsed
            .diagnostics
            .first()
            .and_then(|diagnostic| {
                diagnostic
                    .labels
                    .as_slice()
                    .first()
                    .map(|label| label.offset())
            })
            .unwrap_or(0);
        let offset = offset as usize;
        let line = context
            .line_starts
            .partition_point(|start| *start <= offset)
            .max(1);
        let message = parsed
            .diagnostics
            .first()
            .map(|diagnostic| diagnostic.message.to_string())
            .unwrap_or_else(|| "the parser gave up".to_string());
        return vec![finding(
            &context.file,
            Some(u32::try_from(line).unwrap_or(u32::MAX)),
            "script-parse",
            format!("the file does not parse as a JavaScript module: {message}"),
        )];
    }
    let program = &parsed.program;
    let mut collect = walk::Walker::new(walk::Phase::Collect, walk::Facts::default(), context);
    collect.program(program);
    let facts = collect.facts;
    let mut check = walk::Walker::new(walk::Phase::Check, facts, context);
    check.program(program);
    check.findings
}

fn finding(file: &str, line: Option<u32>, check: &'static str, message: String) -> Finding {
    Finding {
        check,
        file: file.to_string(),
        line,
        message,
    }
}

/// The line of the first `<!--`, or of a `-->` that starts a line: in a
/// classic script both open a comment a module parser does not see.
fn html_like_comment(text: &str) -> Option<u32> {
    for (index, line) in text.lines().enumerate() {
        if line.contains("<!--") || line.trim_start().starts_with("-->") {
            return Some(u32::try_from(index + 1).unwrap_or(u32::MAX));
        }
    }
    None
}
