//! Branch-aware Askama and HTML contract checking.

mod branch;
mod diagnostic;
mod directive;
mod html;
mod limits;
mod template;

pub use diagnostic::{CheckReport, DiagnosticCode, DiagnosticSeverity, TemplateDiagnostic};
pub use limits::{CheckerConfigError, CheckerLimits};
pub use template::{TemplateCatalog, TemplateCatalogError, TemplateChecker};

/// Version of the server-visible directive grammar checked by this module.
pub const DIRECTIVE_GRAMMAR_VERSION: u16 = 1;

/// Version of the Askama/HTML checker contract.
pub const VIEW_CHECKER_VERSION: u16 = 1;
