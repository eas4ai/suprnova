//! Branch-aware Askama and HTML contract checking.

mod branch;
mod diagnostic;
mod directive;
mod generated_directive_contract;
mod html;
mod limits;
mod template;

pub use diagnostic::{CheckReport, DiagnosticCode, DiagnosticSeverity, TemplateDiagnostic};
pub use generated_directive_contract::{
    DIRECTIVE_ARGUMENT_FORMS, DIRECTIVE_CONTRACTS, DIRECTIVE_FALLBACKS,
    DIRECTIVE_FIXTURE_MANIFEST_SHA256, DIRECTIVE_LITERAL_KINDS, DIRECTIVE_TARGET_KINDS,
    DirectiveContract, DirectiveFallback, DirectiveOwner, DirectivePhase, DirectiveValue,
    RESERVED_DIRECTIVES, directive_contract, is_reserved_directive,
};
pub use limits::{CheckerConfigError, CheckerLimits};
pub use template::{TemplateCatalog, TemplateCatalogError, TemplateChecker};

/// Version of the server-visible directive grammar checked by this module.
pub const DIRECTIVE_GRAMMAR_VERSION: u16 = 2;

/// Version of the Askama/HTML checker contract.
pub const VIEW_CHECKER_VERSION: u16 = 1;
