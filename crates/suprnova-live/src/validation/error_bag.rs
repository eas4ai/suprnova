//! Bounded stable issue identities and deterministic bag updates.

use std::error::Error;
use std::fmt;

use crate::state::ModelPath;

pub(crate) const HARD_MAX_VALIDATION_ISSUES: usize = 1_024;

/// A bounded localization key; free-form application-user messages never cross this boundary.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct ValidationMessageId(String);

impl ValidationMessageId {
    /// Parses a stable ASCII localization identifier.
    pub fn parse(value: &str) -> Result<Self, ValidationBagError> {
        let valid = !value.is_empty()
            && value.len() <= 128
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
            });
        if !valid {
            return Err(ValidationBagError);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the stable localization identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ValidationMessageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// One stable field/action path and localizable message identity.
#[derive(Clone, Eq, PartialEq)]
pub struct ValidationIssue {
    path: ModelPath,
    message: ValidationMessageId,
}

impl ValidationIssue {
    /// Creates one already-validated localizable issue.
    #[must_use]
    pub const fn new(path: ModelPath, message: ValidationMessageId) -> Self {
        Self { path, message }
    }

    /// Returns the stable semantic association path.
    #[must_use]
    pub const fn path(&self) -> &ModelPath {
        &self.path
    }

    /// Returns the localization message identity.
    #[must_use]
    pub const fn message(&self) -> &ValidationMessageId {
        &self.message
    }
}

impl fmt::Debug for ValidationIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidationIssue")
            .field("path", &self.path)
            .field("message", &self.message)
            .finish()
    }
}

/// How one completed validation run updates the component's current issue bag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BagPolicy {
    /// Clear stale state at the boundary, then publish the current run's issues.
    Clear,
    /// Retain existing issues and append new unique issues.
    Retain,
    /// Atomically replace existing issues after the validation port succeeds.
    Replace,
}

/// Whether the most recent validation run produced any issues.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidationStatus {
    /// No validation issue was returned.
    Valid,
    /// One or more validation issues were returned.
    Invalid,
}

/// Bounded validation issues owned by the Live component request.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct ErrorBag {
    issues: Vec<ValidationIssue>,
}

impl ErrorBag {
    /// Creates a bag after enforcing the engine hard issue ceiling.
    pub fn from_issues(issues: Vec<ValidationIssue>) -> Result<Self, ValidationBagError> {
        if issues.len() > HARD_MAX_VALIDATION_ISSUES {
            return Err(ValidationBagError);
        }
        Ok(Self { issues })
    }

    /// Returns current issues in deterministic insertion order.
    #[must_use]
    pub fn issues(&self) -> &[ValidationIssue] {
        &self.issues
    }

    /// Returns the number of current issues.
    #[must_use]
    pub fn len(&self) -> usize {
        self.issues.len()
    }

    /// Returns whether the bag has no current issues.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.issues.is_empty()
    }

    pub(crate) fn apply(
        &mut self,
        policy: BagPolicy,
        mut current: Vec<ValidationIssue>,
        max_issues: usize,
    ) -> Result<(), ValidationBagError> {
        match policy {
            BagPolicy::Clear | BagPolicy::Replace => {
                if current.len() > max_issues {
                    return Err(ValidationBagError);
                }
                self.issues = current;
            }
            BagPolicy::Retain => {
                let mut retained = self.issues.clone();
                for issue in current.drain(..) {
                    if !retained.contains(&issue) {
                        retained.push(issue);
                    }
                    if retained.len() > max_issues {
                        return Err(ValidationBagError);
                    }
                }
                self.issues = retained;
            }
        }
        Ok(())
    }
}

impl fmt::Debug for ErrorBag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ErrorBag")
            .field("issues", &self.issues)
            .finish()
    }
}

/// Invalid message identity or issue count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidationBagError;

impl fmt::Display for ValidationBagError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid_validation_bag")
    }
}

impl Error for ValidationBagError {}
