//! Stable error categories and redacted production diagnostics.

use std::error::Error;
use std::fmt;

use serde::Serialize;

/// Stable machine category for a Live failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCategory {
    /// The control envelope or operation grammar is invalid.
    Protocol,
    /// Submitted state failed application validation.
    Validation,
    /// No acceptable current identity is available.
    Authentication,
    /// The current identity is not permitted to perform the operation.
    Authorization,
    /// Request-authenticity verification failed.
    Csrf,
    /// A signed snapshot is invalid, stale, or unusable.
    Snapshot,
    /// Instance revision authority rejected the request.
    Revision,
    /// Server rendering failed.
    Render,
    /// Browser reconciliation failed after server acceptance.
    Morph,
    /// A configured provider failed its contract.
    Provider,
    /// RenderCache processing failed.
    Cache,
    /// File upload processing failed.
    Upload,
    /// Runtime, protocol, snapshot, or component versions are incompatible.
    Compatibility,
    /// A configured byte, count, depth, or allocation limit was exceeded.
    SizeLimit,
    /// A configured request or promotion rate was exceeded.
    RateLimit,
    /// An integrity or trust-boundary check failed closed.
    Security,
    /// A framework invariant failed without a safer specific category.
    Internal,
}

impl ErrorCategory {
    /// Returns the stable snake-case protocol value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Protocol => "protocol",
            Self::Validation => "validation",
            Self::Authentication => "authentication",
            Self::Authorization => "authorization",
            Self::Csrf => "csrf",
            Self::Snapshot => "snapshot",
            Self::Revision => "revision",
            Self::Render => "render",
            Self::Morph => "morph",
            Self::Provider => "provider",
            Self::Cache => "cache",
            Self::Upload => "upload",
            Self::Compatibility => "compatibility",
            Self::SizeLimit => "size_limit",
            Self::RateLimit => "rate_limit",
            Self::Security => "security",
            Self::Internal => "internal",
        }
    }
}

/// Safe browser recovery instruction paired with a failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RecoveryInstruction {
    /// Keep the current DOM and do not retry automatically.
    RetainDom,
    /// Retry under the explicit transport/idempotency policy.
    Retry,
    /// Obtain fresh authorized HTML and state for the island.
    RefreshIsland,
    /// Replace the island through a controlled fresh mount.
    RemountIsland,
    /// Perform a real document navigation.
    Navigate,
    /// Stop Live processing for this boundary.
    Stop,
}

impl RecoveryInstruction {
    /// Returns the stable snake-case protocol value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RetainDom => "retain_dom",
            Self::Retry => "retry",
            Self::RefreshIsland => "refresh_island",
            Self::RemountIsland => "remount_island",
            Self::Navigate => "navigate",
            Self::Stop => "stop",
        }
    }
}

/// Closed diagnostic detail safe for production responses and bounded telemetry.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SafeDiagnosticCode {
    /// Input exceeded its byte limit.
    InputTooLarge,
    /// Input exceeded its nesting limit.
    InputTooDeep,
    /// Input exceeded its total collection-entry limit.
    TooManyEntries,
    /// A string exceeded its byte limit.
    StringTooLong,
    /// An object repeated a key.
    DuplicateKey,
    /// Input was not valid UTF-8.
    InvalidUtf8,
    /// A JSON number was outside the supported interoperable profile.
    InvalidNumber,
    /// Input was not one valid JSON value.
    InvalidJson,
    /// Canonical serialization failed.
    SerializationFailed,
    /// A configured limit was zero or above its hard ceiling.
    InvalidLimitConfiguration,
    /// A typed text identifier violated its grammar or length.
    InvalidIdentifier,
    /// A binary identity was not canonical base64url or had the wrong strength.
    InvalidBase64Identity,
    /// Snapshot integrity verification failed.
    SignatureInvalid,
}

impl SafeDiagnosticCode {
    /// Returns the stable snake-case protocol value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InputTooLarge => "input_too_large",
            Self::InputTooDeep => "input_too_deep",
            Self::TooManyEntries => "too_many_entries",
            Self::StringTooLong => "string_too_long",
            Self::DuplicateKey => "duplicate_key",
            Self::InvalidUtf8 => "invalid_utf8",
            Self::InvalidNumber => "invalid_number",
            Self::InvalidJson => "invalid_json",
            Self::SerializationFailed => "serialization_failed",
            Self::InvalidLimitConfiguration => "invalid_limit_configuration",
            Self::InvalidIdentifier => "invalid_identifier",
            Self::InvalidBase64Identity => "invalid_base64_identity",
            Self::SignatureInvalid => "signature_invalid",
        }
    }
}

/// A classified Live failure whose normal formatting never includes payload data.
pub struct LiveError {
    category: ErrorCategory,
    recovery: RecoveryInstruction,
    detail: SafeDiagnosticCode,
    source: Option<Box<dyn Error + Send + Sync + 'static>>,
}

impl LiveError {
    /// Creates a classified error with no causal source.
    #[must_use]
    pub const fn new(
        category: ErrorCategory,
        recovery: RecoveryInstruction,
        detail: SafeDiagnosticCode,
    ) -> Self {
        Self {
            category,
            recovery,
            detail,
            source: None,
        }
    }

    /// Retains a causal source for explicitly trusted developer diagnostics.
    ///
    /// Normal [`Display`](fmt::Display) and [`Debug`](fmt::Debug) formatting stays
    /// redacted. A caller that traverses [`Error::source`] is responsible for
    /// enabling that richer chain only in an explicitly trusted diagnostic mode.
    #[must_use]
    pub fn with_source(mut self, source: impl Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// Returns the stable failure category.
    #[must_use]
    pub const fn category(&self) -> ErrorCategory {
        self.category
    }

    /// Returns the safe browser recovery instruction.
    #[must_use]
    pub const fn recovery(&self) -> RecoveryInstruction {
        self.recovery
    }

    /// Returns the closed safe diagnostic detail.
    #[must_use]
    pub const fn detail(&self) -> SafeDiagnosticCode {
        self.detail
    }
}

impl fmt::Display for LiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{} recovery={}",
            self.category.as_str(),
            self.detail.as_str(),
            self.recovery.as_str()
        )
    }
}

impl fmt::Debug for LiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for LiveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}
