//! Closed and redacted Live control-protocol failures.

use std::error::Error;
use std::fmt;

/// Closed reason a request, response, or compatibility contract failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolErrorKind {
    /// Encoded control input exceeded its byte limit.
    InputTooLarge,
    /// Encoded control input exceeded its nesting limit.
    InputTooDeep,
    /// Encoded control input exceeded its total entry limit.
    TooManyEntries,
    /// An object repeated a key.
    DuplicateField,
    /// The control envelope shape, field type, or JSON syntax was invalid.
    InvalidEnvelope,
    /// A breaking protocol, runtime, or snapshot version is unsupported.
    UnsupportedVersion,
    /// A correlation, idempotency, nonce, component, field, or operation identity was invalid.
    InvalidIdentity,
    /// Snapshot form or its required fields were inconsistent.
    InvalidSnapshotForm,
    /// Embedded canonical snapshot bytes exceeded their independent limit.
    SnapshotTooLarge,
    /// Model proposals exceeded their independent count limit.
    TooManyModelProposals,
    /// Ordered operations exceeded their independent count limit.
    TooManyOperations,
    /// Action arguments exceeded their independent count limit.
    TooManyArguments,
    /// An operation mixed fields from more than one operation form.
    AmbiguousOperation,
    /// Operation ordering or batching could change semantics.
    IncompatibleBatch,
    /// Extension names or counts violated the explicit extension contract.
    InvalidExtension,
    /// Response outcome fields were incomplete or mutually exclusive.
    OutcomeMismatch,
    /// Error category, response outcome, and recovery instruction disagreed.
    ErrorRecoveryMismatch,
    /// Redirect target was not one bounded same-origin route location.
    UnsafeRedirect,
}

impl ProtocolErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InputTooLarge => "protocol_input_too_large",
            Self::InputTooDeep => "protocol_input_too_deep",
            Self::TooManyEntries => "protocol_too_many_entries",
            Self::DuplicateField => "protocol_duplicate_field",
            Self::InvalidEnvelope => "invalid_protocol_envelope",
            Self::UnsupportedVersion => "unsupported_protocol_version",
            Self::InvalidIdentity => "invalid_protocol_identity",
            Self::InvalidSnapshotForm => "invalid_snapshot_input_form",
            Self::SnapshotTooLarge => "protocol_snapshot_too_large",
            Self::TooManyModelProposals => "too_many_model_proposals",
            Self::TooManyOperations => "too_many_operations",
            Self::TooManyArguments => "too_many_action_arguments",
            Self::AmbiguousOperation => "ambiguous_operation",
            Self::IncompatibleBatch => "incompatible_operation_batch",
            Self::InvalidExtension => "invalid_protocol_extension",
            Self::OutcomeMismatch => "response_outcome_mismatch",
            Self::ErrorRecoveryMismatch => "error_recovery_mismatch",
            Self::UnsafeRedirect => "unsafe_redirect",
        }
    }
}

/// Redacted protocol error that never formats payload or hostile identity data.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ProtocolError {
    kind: ProtocolErrorKind,
}

impl ProtocolError {
    pub(crate) const fn new(kind: ProtocolErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed rejection reason.
    #[must_use]
    pub const fn kind(self) -> ProtocolErrorKind {
        self.kind
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for ProtocolError {}
