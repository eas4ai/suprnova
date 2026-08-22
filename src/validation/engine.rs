//! Deterministic validation orchestration and issue bounds.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::future::poll_fn;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::task::Poll;

use super::error_bag::HARD_MAX_VALIDATION_ISSUES;
use super::{
    BagPolicy, ErrorBag, ValidationPort, ValidationRequest, ValidationSelection, ValidationStatus,
};

/// Closed validation engine failure reason.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidationEngineErrorKind {
    /// A selected-field contract was empty, duplicated, or exceeded its bound.
    InvalidSelection,
    /// The host validation provider failed.
    ProviderFailure,
    /// The provider returned too many issues.
    TooManyIssues,
}

/// Redacted validation engine failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ValidationEngineError {
    kind: ValidationEngineErrorKind,
}

impl ValidationEngineError {
    const fn new(kind: ValidationEngineErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed stable failure category.
    #[must_use]
    pub const fn kind(self) -> ValidationEngineErrorKind {
        self.kind
    }
}

impl fmt::Display for ValidationEngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ValidationEngineErrorKind::InvalidSelection => "invalid_validation_selection",
            ValidationEngineErrorKind::ProviderFailure => "validation_provider_failure",
            ValidationEngineErrorKind::TooManyIssues => "too_many_validation_issues",
        })
    }
}

impl fmt::Debug for ValidationEngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for ValidationEngineError {}

/// Bounded deterministic validation coordinator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidationEngine {
    max_issues: usize,
}

impl ValidationEngine {
    /// Creates a nonzero issue ceiling below the engine hard maximum.
    pub fn new(max_issues: usize) -> Result<Self, ValidationEngineError> {
        if max_issues == 0 || max_issues > HARD_MAX_VALIDATION_ISSUES {
            return Err(ValidationEngineError::new(
                ValidationEngineErrorKind::TooManyIssues,
            ));
        }
        Ok(Self { max_issues })
    }

    /// Runs the exact declared selection and applies its completed bag update.
    pub async fn validate(
        &self,
        port: &dyn ValidationPort,
        request: ValidationRequest<'_>,
        bag: &mut ErrorBag,
        policy: BagPolicy,
    ) -> Result<ValidationStatus, ValidationEngineError> {
        validate_selection(request.selection(), self.max_issues)?;
        let issues = if matches!(request.selection(), ValidationSelection::None) {
            Vec::new()
        } else {
            let future =
                catch_unwind(AssertUnwindSafe(|| port.validate(request))).map_err(|_| {
                    ValidationEngineError::new(ValidationEngineErrorKind::ProviderFailure)
                })?;
            poll_validation_future(future).await?
        };
        if issues.len() > self.max_issues {
            return Err(ValidationEngineError::new(
                ValidationEngineErrorKind::TooManyIssues,
            ));
        }
        let status = if issues.is_empty() {
            ValidationStatus::Valid
        } else {
            ValidationStatus::Invalid
        };
        bag.apply(policy, issues, self.max_issues)
            .map_err(|_| ValidationEngineError::new(ValidationEngineErrorKind::TooManyIssues))?;
        Ok(status)
    }
}

async fn poll_validation_future(
    mut future: super::ValidationFuture<
        '_,
        Result<Vec<super::ValidationIssue>, super::ValidationPortError>,
    >,
) -> Result<Vec<super::ValidationIssue>, ValidationEngineError> {
    let result =
        poll_fn(
            |context| match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(context))) {
                Ok(Poll::Ready(Ok(issues))) => Poll::Ready(Ok(issues)),
                Ok(Poll::Ready(Err(_))) | Err(_) => Poll::Ready(Err(ValidationEngineError::new(
                    ValidationEngineErrorKind::ProviderFailure,
                ))),
                Ok(Poll::Pending) => Poll::Pending,
            },
        )
        .await;
    if catch_unwind(AssertUnwindSafe(|| drop(future))).is_err() {
        return Err(ValidationEngineError::new(
            ValidationEngineErrorKind::ProviderFailure,
        ));
    }
    result
}

impl Default for ValidationEngine {
    fn default() -> Self {
        Self { max_issues: 128 }
    }
}

fn validate_selection(
    selection: &ValidationSelection,
    max_issues: usize,
) -> Result<(), ValidationEngineError> {
    if let ValidationSelection::Selected(paths) = selection {
        let unique = paths.iter().collect::<BTreeSet<_>>();
        let invalid = paths.is_empty() || paths.len() > max_issues || unique.len() != paths.len();
        if invalid {
            return Err(ValidationEngineError::new(
                ValidationEngineErrorKind::InvalidSelection,
            ));
        }
    }
    Ok(())
}
