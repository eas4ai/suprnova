//! Stable, redacted checker diagnostics.

use std::fmt;

use crate::identity::{ComponentName, ViewName};

/// Stable checker diagnostic category.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DiagnosticCode {
    /// The component's registered root view was absent.
    MissingView,
    /// An included or inherited template was absent.
    MissingTemplate,
    /// Askama rejected the template source.
    AskamaSyntax,
    /// Static HTML was malformed or structurally incompatible.
    HtmlSyntax,
    /// Askama branches did not produce compatible HTML stacks.
    BranchStackMismatch,
    /// Dynamic tag or attribute structure could not be proved.
    DynamicStructureUnproved,
    /// Askama's untyped safe filter crossed the Live view boundary.
    RawSafe,
    /// A directive name is not in the shipped grammar.
    UnknownDirective,
    /// A lifecycle hook was exposed as a browser directive.
    ForbiddenLifecycle,
    /// An action identity was not registered by the owning component.
    UnknownAction,
    /// A model identity was not registered.
    UnknownModel,
    /// A field exists but is not browser-bindable.
    ForbiddenModel,
    /// A directive modifier is unknown or disagrees with metadata.
    InvalidModifier,
    /// A directive escaped its island owner.
    OwnershipViolation,
    /// A nested component identity was not registered.
    UnknownComponent,
    /// A key was missing, dynamic, malformed, or repeated.
    InvalidKey,
    /// A stable key was repeated in one rendered branch.
    DuplicateKey,
    /// A URL directive disagreed with registered URL policy.
    InvalidUrlBinding,
    /// An event identity was not registered.
    UnknownEvent,
    /// An effect identity was not registered.
    UnknownEffect,
    /// A selected accessibility or component-anatomy invariant failed.
    AccessibilityViolation,
    /// Template source or one expanded branch exceeded the byte ceiling.
    SourceLimit,
    /// Parsed Askama nodes exceeded the configured ceiling.
    NodeLimit,
    /// Include/inheritance traversal exceeded its depth ceiling or cycled.
    IncludeDepthLimit,
    /// Control-flow expansion exceeded the branch-state ceiling.
    BranchLimit,
    /// HTML tokenization exceeded the token ceiling.
    HtmlTokenLimit,
    /// HTML attributes exceeded the attribute ceiling.
    AttributeLimit,
    /// HTML nesting exceeded the stack ceiling.
    StackDepthLimit,
    /// Diagnostic production reached its hard ceiling.
    DiagnosticLimit,
}

impl DiagnosticCode {
    /// Returns the stable machine-readable code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingView => "missing_view",
            Self::MissingTemplate => "missing_template",
            Self::AskamaSyntax => "askama_syntax",
            Self::HtmlSyntax => "html_syntax",
            Self::BranchStackMismatch => "branch_stack_mismatch",
            Self::DynamicStructureUnproved => "dynamic_structure_unproved",
            Self::RawSafe => "raw_safe",
            Self::UnknownDirective => "unknown_directive",
            Self::ForbiddenLifecycle => "forbidden_lifecycle",
            Self::UnknownAction => "unknown_action",
            Self::UnknownModel => "unknown_model",
            Self::ForbiddenModel => "forbidden_model",
            Self::InvalidModifier => "invalid_modifier",
            Self::OwnershipViolation => "ownership_violation",
            Self::UnknownComponent => "unknown_component",
            Self::InvalidKey => "invalid_key",
            Self::DuplicateKey => "duplicate_key",
            Self::InvalidUrlBinding => "invalid_url_binding",
            Self::UnknownEvent => "unknown_event",
            Self::UnknownEffect => "unknown_effect",
            Self::AccessibilityViolation => "accessibility_violation",
            Self::SourceLimit => "source_limit",
            Self::NodeLimit => "node_limit",
            Self::IncludeDepthLimit => "include_depth_limit",
            Self::BranchLimit => "branch_limit",
            Self::HtmlTokenLimit => "html_token_limit",
            Self::AttributeLimit => "attribute_limit",
            Self::StackDepthLimit => "stack_depth_limit",
            Self::DiagnosticLimit => "diagnostic_limit",
        }
    }
}

/// Whether a diagnostic proves invalidity or records an unproved dynamic case.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticSeverity {
    /// The checked contract is invalid.
    Error,
    /// The checker deliberately makes no proof claim for this dynamic structure.
    Unproved,
}

/// One bounded, source-oriented checker diagnostic.
#[derive(Clone, Eq, PartialEq)]
pub struct TemplateDiagnostic {
    code: DiagnosticCode,
    severity: DiagnosticSeverity,
    path: Option<ViewName>,
    line: u32,
    column: u32,
    component: Option<ComponentName>,
}

impl TemplateDiagnostic {
    pub(crate) const fn new(
        code: DiagnosticCode,
        severity: DiagnosticSeverity,
        path: Option<ViewName>,
        line: u32,
        column: u32,
        component: Option<ComponentName>,
    ) -> Self {
        Self {
            code,
            severity,
            path,
            line,
            column,
            component,
        }
    }

    /// Returns the stable machine-readable category.
    #[must_use]
    pub const fn code(&self) -> DiagnosticCode {
        self.code
    }

    /// Returns error or explicitly unproved severity.
    #[must_use]
    pub const fn severity(&self) -> DiagnosticSeverity {
        self.severity
    }

    /// Returns the registered template identity when available.
    #[must_use]
    pub const fn path(&self) -> Option<&ViewName> {
        self.path.as_ref()
    }

    /// Returns the one-based source line.
    #[must_use]
    pub const fn line(&self) -> u32 {
        self.line
    }

    /// Returns the one-based source column.
    #[must_use]
    pub const fn column(&self) -> u32 {
        self.column
    }

    /// Returns the safe registered component identity when available.
    #[must_use]
    pub const fn component(&self) -> Option<&ComponentName> {
        self.component.as_ref()
    }
}

impl fmt::Debug for TemplateDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TemplateDiagnostic")
            .field("code", &self.code.as_str())
            .field("severity", &self.severity)
            .field("path", &self.path.as_ref().map(ViewName::as_str))
            .field("line", &self.line)
            .field("column", &self.column)
            .field(
                "component",
                &self.component.as_ref().map(ComponentName::as_str),
            )
            .finish()
    }
}

/// Complete bounded result of checking one component view contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckReport {
    diagnostics: Vec<TemplateDiagnostic>,
}

impl CheckReport {
    pub(crate) fn new(diagnostics: Vec<TemplateDiagnostic>) -> Self {
        Self { diagnostics }
    }

    /// Returns true only when every static contract was proved.
    #[must_use]
    pub fn is_proved(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// Returns bounded stable diagnostics in deterministic discovery order.
    #[must_use]
    pub fn diagnostics(&self) -> &[TemplateDiagnostic] {
        &self.diagnostics
    }
}

pub(crate) struct DiagnosticCollector {
    diagnostics: Vec<TemplateDiagnostic>,
    max: usize,
    limit_hit: bool,
}

impl DiagnosticCollector {
    pub(crate) fn new(max: usize) -> Self {
        Self {
            diagnostics: Vec::new(),
            max,
            limit_hit: false,
        }
    }

    pub(crate) fn push(
        &mut self,
        code: DiagnosticCode,
        severity: DiagnosticSeverity,
        path: Option<&ViewName>,
        line: u32,
        column: u32,
        component: Option<&ComponentName>,
    ) {
        if self.diagnostics.len() < self.max {
            self.diagnostics.push(TemplateDiagnostic::new(
                code,
                severity,
                path.cloned(),
                line.max(1),
                column.max(1),
                component.cloned(),
            ));
            return;
        }
        if self.limit_hit {
            return;
        }
        self.limit_hit = true;
        let diagnostic = TemplateDiagnostic::new(
            DiagnosticCode::DiagnosticLimit,
            DiagnosticSeverity::Error,
            path.cloned(),
            line.max(1),
            column.max(1),
            component.cloned(),
        );
        if let Some(last) = self.diagnostics.last_mut() {
            *last = diagnostic;
        }
    }

    pub(crate) fn finish(self) -> CheckReport {
        CheckReport::new(self.diagnostics)
    }
}
