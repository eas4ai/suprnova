//! Host-neutral application/framework validation boundary.

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

use crate::action::ActionTarget;
use crate::action::PreparedActionArguments;
use crate::canonical::CanonicalValue;
use crate::identity::{ActionName, ComponentName};
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
pub struct ValidationRequest<'a> {
    component: &'a ComponentName,
    selection: ValidationSelection,
    state: &'a CanonicalValue,
    arguments: &'a CanonicalValue,
    action: Option<&'a ActionName>,
    prepared_arguments: Option<&'a PreparedActionArguments>,
    target: Option<&'a mut dyn ActionTarget>,
}

impl<'a> ValidationRequest<'a> {
    /// Creates a request from already bounded component state and typed arguments.
    #[must_use]
    pub fn new(
        component: &'a ComponentName,
        selection: ValidationSelection,
        state: &'a CanonicalValue,
        arguments: &'a CanonicalValue,
    ) -> Self {
        Self {
            component,
            selection,
            state,
            arguments,
            action: None,
            prepared_arguments: None,
            target: None,
        }
    }

    /// Returns the exact registered component identity that owns the contract.
    #[must_use]
    pub const fn component(&self) -> &ComponentName {
        self.component
    }

    /// Associates this validation run with one registered action.
    #[must_use]
    pub fn with_action(mut self, action: &'a ActionName) -> Self {
        self.action = Some(action);
        self
    }

    /// Binds the schema-checked argument object used by generated typed hooks.
    #[must_use]
    pub fn with_prepared_arguments(mut self, arguments: &'a PreparedActionArguments) -> Self {
        self.prepared_arguments = Some(arguments);
        self
    }

    /// Binds validation to the exact request-owned hydrated component.
    #[must_use]
    pub fn with_target(mut self, target: &'a mut dyn ActionTarget) -> Self {
        self.target = Some(target);
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

    /// Decodes one already schema-checked action argument for a generated hook.
    pub fn decode_argument<T: serde::de::DeserializeOwned + 'static>(
        &self,
        name: &str,
    ) -> Result<T, ValidationPortError> {
        self.prepared_arguments
            .ok_or_else(ValidationPortError::unavailable)?
            .decode(name)
            .map_err(|_| ValidationPortError::unavailable())
    }

    /// Returns the registered action identity when present.
    #[must_use]
    pub const fn action(&self) -> Option<&ActionName> {
        self.action
    }

    /// Returns the exact request-owned target for generated typed validation.
    pub fn target_mut(&mut self) -> Option<&mut dyn ActionTarget> {
        self.target.as_deref_mut()
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
