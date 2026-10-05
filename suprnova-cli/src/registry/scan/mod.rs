//! Validation that gives a component's code no way to execute (REG-022):
//! the signature over the hash first, then the Rust, views and scripts
//! read as syntax trees against an allowlist of Suprnova's API. The scan
//! admits what it can classify and refuses everything else.

pub mod allowlist;
mod check;
mod contract;
mod limits;
pub mod rust;
pub mod script;
pub mod url;
pub mod view;

use std::collections::BTreeSet;
use std::fmt;

use super::{Capability, RegistryError, Result};

/// One refusal: the check, the file, the line when the syntax tree has one,
/// and what was refused (REG-022).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The check that refused, for example `rust-path`, `view-url`,
    /// `script-call`.
    pub check: &'static str,
    /// The file, by its manifest name.
    pub file: String,
    /// The line in that file, when known.
    pub line: Option<u32>,
    /// What was refused and why.
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(
                f,
                "{}:{}: [{}] {}",
                self.file, line, self.check, self.message
            ),
            None => write!(f, "{}: [{}] {}", self.file, self.check, self.message),
        }
    }
}

/// What the scan learned about a component.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    /// Every capability the admitted paths carry (REG-006).
    pub capabilities: BTreeSet<Capability>,
    /// Every refusal; an empty list is acceptance.
    pub findings: Vec<Finding>,
    /// Each Live component the Rust defines, as `<module>::<Type>` (REG-030).
    pub defined_components: Vec<String>,
}

impl ScanReport {
    /// Whether the scan admitted the component.
    pub fn accepted(&self) -> bool {
        self.findings.is_empty()
    }

    /// Folds another report into this one.
    pub fn merge(&mut self, other: ScanReport) {
        self.capabilities.extend(other.capabilities);
        self.findings.extend(other.findings);
        self.defined_components.extend(other.defined_components);
    }
}

/// One component as it arrived, ready to scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentFiles<'a> {
    /// The library's namespace (REG-004).
    pub namespace: &'a str,
    /// The component directory name.
    pub directory: &'a str,
    /// Every named file's bytes, by manifest name.
    pub files: &'a [(String, Vec<u8>)],
    /// The modules of the components it depends on, each as
    /// `<namespace_module>::<file stem>` relative to `crate::live`, which its
    /// Rust may name.
    pub dependency_modules: &'a [String],
    /// The views the components it depends on and the shipped library
    /// carry, which its views may include, import or extend.
    pub importable_views: &'a [String],
    /// The scripts the components it depends on carry, as
    /// `<namespace>-ui/<directory>/<file>`, which its scripts may import
    /// (REG-032). A shipped script is one of them only through a declared
    /// dependency.
    pub importable_scripts: &'a [String],
}

/// The stack the scans run on. The parsers and the walkers recurse once per
/// nesting level; the scans refuse nesting deeper than they can follow, and
/// this stack leaves room for the deepest input they accept.
const SCAN_STACK_BYTES: usize = 256 * 1024 * 1024;

/// Scans every file of a component (REG-030, REG-031, REG-032) and reports
/// its capabilities and refusals. Nothing in `files` runs.
pub fn scan_component(
    component: &ComponentFiles<'_>,
    allowlist: &allowlist::Allowlist,
) -> Result<ScanReport> {
    scan_component_in(component, &ScanContext::default(), allowlist)
}

/// What a caller knows about a component beyond its own files.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScanContext<'a> {
    /// The manifest's `register`: when given, the Live components the Rust
    /// defines must be exactly these (REG-005, REG-030).
    pub register: Option<&'a [String]>,
    /// The manifest's `elements`: when given, every element a script
    /// defines must be one of these (REG-032).
    pub elements: Option<&'a [String]>,
    /// The source of each view the components it depends on carry, by view
    /// path (`<namespace>-ui/<directory>/<file>`), so the view checker can
    /// follow an include of one. The shipped library's views come from the
    /// CLI itself.
    pub dependency_views: &'a [(String, String)],
}

/// Scans a component with what the caller knows about it: the three scans,
/// the manifest checks the context asks for, and `live:check`'s view checks
/// against the contract read from the component's Rust (REG-022).
pub fn scan_component_in(
    component: &ComponentFiles<'_>,
    context: &ScanContext<'_>,
    allowlist: &allowlist::Allowlist,
) -> Result<ScanReport> {
    on_scan_stack(|| scan_on_this_thread(component, allowlist, context))
}

/// Scans a component as [`scan_component`] does and checks that the Live
/// components its Rust defines are exactly the manifest's `register`
/// entries (REG-005, REG-030): a defined component the manifest does not
/// declare, a declared one the Rust does not define, or one declared twice
/// is refused.
pub fn scan_component_with_register(
    component: &ComponentFiles<'_>,
    register: &[String],
    allowlist: &allowlist::Allowlist,
) -> Result<ScanReport> {
    let context = ScanContext {
        register: Some(register),
        ..ScanContext::default()
    };
    scan_component_in(component, &context, allowlist)
}

/// Scans a component as [`scan_component_with_register`] does, and also
/// checks that every element its scripts define with
/// `customElements.define` is one of the manifest's `elements` (REG-032).
pub fn scan_component_with_manifest(
    component: &ComponentFiles<'_>,
    register: &[String],
    elements: &[String],
    allowlist: &allowlist::Allowlist,
) -> Result<ScanReport> {
    let context = ScanContext {
        register: Some(register),
        elements: Some(elements),
        ..ScanContext::default()
    };
    scan_component_in(component, &context, allowlist)
}

fn on_scan_stack<F>(scan: F) -> Result<ScanReport>
where
    F: FnOnce() -> Result<ScanReport> + Send,
{
    std::thread::scope(|scope| {
        let handle = std::thread::Builder::new()
            .name("suprnova-registry-scan".to_string())
            .stack_size(SCAN_STACK_BYTES)
            .spawn_scoped(scope, scan)
            .map_err(|error| {
                RegistryError::Io(format!("could not start the component scan: {error}"))
            })?;
        handle
            .join()
            .map_err(|_| RegistryError::Invalid("the component scan failed".to_string()))?
    })
}

fn scan_on_this_thread(
    component: &ComponentFiles<'_>,
    allowlist: &allowlist::Allowlist,
    context: &ScanContext<'_>,
) -> Result<ScanReport> {
    let mut report = ScanReport::default();
    for (name, _) in component.files {
        let known = [".rs", ".html", ".css", ".js"]
            .iter()
            .any(|extension| name.ends_with(extension));
        if !known {
            report.findings.push(Finding {
                check: "scan-file",
                file: name.clone(),
                line: None,
                message: "the scan reads only `.rs`, `.html`, `.css` and `.js` files".to_string(),
            });
        }
    }
    let (rust_report, defined) = rust::scan_detailed(component, allowlist)?;
    report.merge(rust_report);
    report.merge(view::scan(component, allowlist)?);
    report.merge(script::scan_with_elements(component, context.elements)?);
    if let Some(register) = context.register {
        report
            .findings
            .extend(register_findings(&defined, register));
    }
    let contracts = contract::read(component);
    report.findings.extend(check::check_views(
        component,
        &contracts,
        context.dependency_views,
    ));
    report.findings.extend(contracts.findings);
    Ok(report)
}

/// The refusals of a manifest `register` that differs from the Live
/// components the Rust defines.
fn register_findings(defined: &[rust::DefinedComponent], register: &[String]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for component in defined {
        let declared = register
            .iter()
            .filter(|entry| **entry == component.path)
            .count();
        if declared != 1 {
            findings.push(Finding {
                check: "rust-component",
                file: component.file.clone(),
                line: component.line,
                message: if declared == 0 {
                    format!(
                        "`{}` is a Live component the manifest's `register` does not declare",
                        component.path
                    )
                } else {
                    format!(
                        "`{}` is declared more than once in the manifest's `register`",
                        component.path
                    )
                },
            });
        }
    }
    for entry in register {
        if !defined.iter().any(|component| component.path == *entry) {
            findings.push(Finding {
                check: "rust-component",
                file: "manifest.json".to_string(),
                line: None,
                message: format!("`register` names `{entry}`, which the component's Rust does not define with `#[live]`"),
            });
        }
    }
    findings
}
