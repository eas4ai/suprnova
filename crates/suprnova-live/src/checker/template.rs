//! Template catalog and checker orchestration.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use crate::identity::{ComponentName, ViewName};
use crate::registry::ComponentRegistry;

use super::branch::BranchRenderer;
use super::diagnostic::{CheckReport, DiagnosticCode, DiagnosticCollector, DiagnosticSeverity};
use super::html::check_html_branches;
use super::limits::CheckerLimits;

/// Duplicate template identity in a checker catalog.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TemplateCatalogError;

impl fmt::Display for TemplateCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("duplicate_template_identity")
    }
}

impl Error for TemplateCatalogError {}

/// Immutable bounded-source catalog used for includes and inheritance.
#[derive(Clone)]
pub struct TemplateCatalog {
    sources: BTreeMap<ViewName, Arc<str>>,
}

impl TemplateCatalog {
    /// Builds an immutable catalog and rejects duplicate registered views.
    pub fn new<S>(sources: Vec<(ViewName, S)>) -> Result<Self, TemplateCatalogError>
    where
        S: Into<Arc<str>>,
    {
        let mut catalog = BTreeMap::new();
        for (view, source) in sources {
            if catalog.insert(view, source.into()).is_some() {
                return Err(TemplateCatalogError);
            }
        }
        Ok(Self { sources: catalog })
    }

    pub(crate) fn source(&self, view: &ViewName) -> Option<&str> {
        self.sources.get(view).map(AsRef::as_ref)
    }

    pub(crate) fn contains(&self, view: &ViewName) -> bool {
        self.sources.contains_key(view)
    }
}

impl fmt::Debug for TemplateCatalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TemplateCatalog")
            .field("template_count", &self.sources.len())
            .finish()
    }
}

/// Host-neutral checker over an immutable registry and template catalog.
pub struct TemplateChecker<'checker> {
    registry: &'checker ComponentRegistry,
    catalog: &'checker TemplateCatalog,
    limits: CheckerLimits,
}

impl<'checker> TemplateChecker<'checker> {
    /// Creates a checker with explicit immutable inputs and hard limits.
    #[must_use]
    pub const fn new(
        registry: &'checker ComponentRegistry,
        catalog: &'checker TemplateCatalog,
        limits: CheckerLimits,
    ) -> Self {
        Self {
            registry,
            catalog,
            limits,
        }
    }

    /// Checks the registered view and every statically reachable template branch.
    #[must_use]
    pub fn check_component(&self, component: &ComponentName) -> CheckReport {
        let mut diagnostics = DiagnosticCollector::new(self.limits.max_diagnostics());
        let Ok(descriptor) = self.registry.resolve(component) else {
            diagnostics.push(
                DiagnosticCode::UnknownComponent,
                DiagnosticSeverity::Error,
                None,
                1,
                1,
                None,
            );
            return diagnostics.finish();
        };
        let metadata = descriptor.metadata();
        if !self.catalog.contains(metadata.view()) {
            diagnostics.push(
                DiagnosticCode::MissingView,
                DiagnosticSeverity::Error,
                Some(metadata.view()),
                1,
                1,
                Some(metadata.identity()),
            );
            return diagnostics.finish();
        }

        let mut renderer = BranchRenderer::new(
            self.catalog,
            self.limits,
            metadata.identity(),
            &mut diagnostics,
        );
        let branches = renderer.render(metadata.view());
        check_html_branches(
            &branches,
            self.registry,
            self.catalog,
            metadata,
            self.limits,
            &mut diagnostics,
        );
        diagnostics.finish()
    }
}
