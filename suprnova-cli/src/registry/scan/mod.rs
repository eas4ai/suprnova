//! Validation that gives a component's code no way to execute (REG-022):
//! the signature over the hash first, then the Rust, views and scripts
//! read as syntax trees against an allowlist of Suprnova's API. The scan
//! admits what it can classify and refuses everything else.

pub mod allowlist;
pub mod rust;
pub mod script;
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
    /// The modules of the components it depends on, as paths under
    /// `crate::live::<namespace_module>::`, which its Rust may name.
    pub dependency_modules: &'a [String],
    /// The views the components it depends on and the shipped library
    /// carry, which its views may include, import or extend.
    pub importable_views: &'a [String],
}

/// Scans every file of a component (REG-030, REG-031, REG-032) and reports
/// its capabilities and refusals. Nothing in `files` runs.
pub fn scan_component(
    component: &ComponentFiles<'_>,
    allowlist: &allowlist::Allowlist,
) -> Result<ScanReport> {
    let _ = (component, allowlist);
    Err(RegistryError::NotBuilt("the component scan"))
}
