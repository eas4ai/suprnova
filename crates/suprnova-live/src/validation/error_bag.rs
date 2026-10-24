//! Bounded stable issue identities and deterministic bag updates.

use std::error::Error;
use std::fmt;

use crate::state::ModelPath;

pub(crate) const HARD_MAX_VALIDATION_ISSUES: usize = crate::limits::HARD_MAX_COLLECTION_ITEMS;

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

/// The words the application user reads for one issue, resolved by the server
/// for the request's locale: a rule's own `message`, a catalog entry, or the
/// catalog fallback. One line of text; a control character or an empty string
/// is not a message.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct ValidationText(String);

impl ValidationText {
    /// Parses resolved message text: non-empty and free of control characters.
    pub fn parse(value: &str) -> Result<Self, ValidationBagError> {
        if value.is_empty() || value.chars().any(char::is_control) {
            return Err(ValidationBagError);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ValidationText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// One stable field/action path and localizable message identity, with the
/// text the server resolved for it when it had one.
#[derive(Clone, Eq, PartialEq)]
pub struct ValidationIssue {
    path: ModelPath,
    message: ValidationMessageId,
    text: Option<ValidationText>,
}

impl ValidationIssue {
    /// Creates one already-validated localizable issue.
    #[must_use]
    pub const fn new(path: ModelPath, message: ValidationMessageId) -> Self {
        Self {
            path,
            message,
            text: None,
        }
    }

    /// Attaches the text the application user reads for this issue. The
    /// browser shows it under the field in place of its generic announcement.
    #[must_use]
    pub fn with_text(mut self, text: ValidationText) -> Self {
        self.text = Some(text);
        self
    }

    /// Returns the resolved text, when the server attached one.
    #[must_use]
    pub const fn text(&self) -> Option<&ValidationText> {
        self.text.as_ref()
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
            .field("text", &self.text)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_text_is_one_line_of_words() {
        assert_eq!(
            ValidationText::parse("Enter your name.").map(|text| text.as_str().to_owned()),
            Ok("Enter your name.".to_owned())
        );
        assert_eq!(ValidationText::parse(""), Err(ValidationBagError));
        assert_eq!(
            ValidationText::parse("line\nbreak"),
            Err(ValidationBagError)
        );
        assert_eq!(ValidationText::parse("tab\there"), Err(ValidationBagError));
    }

    #[test]
    fn an_issue_without_text_stays_the_identity_alone() {
        let issue = ValidationIssue::new(
            ModelPath::parse("full_name").expect("path"),
            ValidationMessageId::parse("validation.length").expect("identity"),
        );
        assert!(issue.text().is_none());
        let worded = issue
            .clone()
            .with_text(ValidationText::parse("Enter your name.").expect("text"));
        assert_eq!(
            worded.text().map(ValidationText::as_str),
            Some("Enter your name.")
        );
        assert_ne!(issue, worded);
    }
}
