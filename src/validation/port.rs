//! Host-neutral application/framework validation boundary.

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

use crate::canonical::CanonicalValue;
use crate::identity::ActionName;
use crate::state::ModelPath;

use super::ValidationIssue;

/// Bounded boxed future returned by a host validation provider.
pub type ValidationFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Exact registered validation work selected for an action boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidationSelection {
    /// No validation provider call is required.
    None,
    /// Validate only the listed registered model paths.
    Selected(Vec<ModelPath>),
    /// Validate the whole component, including cross-field invariants.
    WholeComponent,
    /// Validate only the registered typed action argument object.
    ActionArguments,
    /// Validate component invariants and typed action arguments together.
    ComponentAndArguments,
}

/// Immutable bounded values passed to the host's validation implementation.
#[derive(Clone)]
pub struct ValidationRequest<'a> {
    selection: ValidationSelection,
    state: &'a CanonicalValue,
    arguments: &'a CanonicalValue,
    action: Option<&'a ActionName>,
}

impl<'a> ValidationRequest<'a> {
    /// Creates a request from already bounded component state and typed arguments.
    #[must_use]
    pub fn new(
        selection: ValidationSelection,
        state: &'a CanonicalValue,
        arguments: &'a CanonicalValue,
    ) -> Self {
        Self {
            selection,
            state,
            arguments,
            action: None,
        }
    }

    /// Associates this validation run with one registered action.
    #[must_use]
    pub fn with_action(mut self, action: &'a ActionName) -> Self {
        self.action = Some(action);
        self
    }

    /// Returns the exact declared validation selection.
    #[must_use]
    pub const fn selection(&self) -> &ValidationSelection {
        &self.selection
    }

    /// Returns bounded component state for application rule evaluation.
    #[must_use]
    pub const fn state(&self) -> &CanonicalValue {
        self.state
    }

    /// Returns bounded typed argument state for action-specific rules.
    #[must_use]
    pub const fn arguments(&self) -> &CanonicalValue {
        self.arguments
    }

    /// Returns the registered action identity when present.
    #[must_use]
    pub const fn action(&self) -> Option<&ActionName> {
        self.action
    }
}

/// Host validation service called only through the selected registered contract.
pub trait ValidationPort: Send + Sync {
    /// Evaluates application/framework rules and returns bounded stable issues.
    fn validate<'a>(
        &'a self,
        request: ValidationRequest<'a>,
    ) -> ValidationFuture<'a, Result<Vec<ValidationIssue>, ValidationPortError>>;
}

/// Redacted host validation provider failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidationPortError;

impl ValidationPortError {
    /// Creates a provider failure without carrying application data.
    #[must_use]
    pub const fn unavailable() -> Self {
        Self
    }
}

impl fmt::Display for ValidationPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("validation_port_failure")
    }
}

impl Error for ValidationPortError {}
