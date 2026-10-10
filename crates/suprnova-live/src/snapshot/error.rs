//! Redacted snapshot schema and state errors.

use std::error::Error;
use std::fmt;

/// Closed reason a snapshot or state boundary rejected input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotErrorKind {
    /// Encoded input exceeded its byte limit.
    InputTooLarge,
    /// Encoded input exceeded its depth limit.
    InputTooDeep,
    /// Encoded input exceeded its collection limit.
    TooManyEntries,
    /// An object repeated a field.
    DuplicateField,
    /// The signed envelope shape or JSON syntax was invalid.
    InvalidEnvelope,
    /// The snapshot form did not match the verification entry point.
    WrongForm,
    /// The snapshot schema version is unsupported.
    UnsupportedSchema,
    /// HMAC verification failed or key selection was not acceptable.
    SignatureInvalid,
    /// The key ID in the body is not the current signing key.
    SigningKeyMismatch,
    /// Route, slot, component name, scope, or instance expectations differ.
    BindingMismatch,
    /// Build, component contract, or state schema compatibility differs.
    CompatibilityMismatch,
    /// Issuance time exceeds the allowed future clock skew.
    IssuedInFuture,
    /// Snapshot validity has elapsed.
    Expired,
    /// Requested seed or instance validity exceeds configured policy.
    ValidityTooLong,
    /// A state, memo, or mount value is not an object of the expected shape.
    InvalidStateShape,
    /// State contained a field not registered by its schema.
    UnknownStateField,
    /// State omitted a required registered field.
    MissingStateField,
    /// State attempted to serialize a secret, transient, computed, or server-only field.
    ForbiddenStateField,
    /// A registered state field did not match its codec.
    InvalidStateCodec,
    /// A state schema or component contract was invalid.
    InvalidSchema,
    /// Advisory generations exceeded their configured count.
    TooManyGenerations,
    /// Extension fields exceeded policy or violated the namespaced grammar.
    InvalidExtension,
    /// Trusted dehydration could not produce the registered canonical state.
    DehydrationFailed,
    /// Verified state could not hydrate into the caller-selected registered type.
    HydrationFailed,
}

impl SnapshotErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InputTooLarge => "input_too_large",
            Self::InputTooDeep => "input_too_deep",
            Self::TooManyEntries => "too_many_entries",
            Self::DuplicateField => "duplicate_field",
            Self::InvalidEnvelope => "invalid_envelope",
            Self::WrongForm => "wrong_snapshot_form",
            Self::UnsupportedSchema => "unsupported_snapshot_schema",
            Self::SignatureInvalid => "signature_invalid",
            Self::SigningKeyMismatch => "signing_key_mismatch",
            Self::BindingMismatch => "binding_mismatch",
            Self::CompatibilityMismatch => "compatibility_mismatch",
            Self::IssuedInFuture => "issued_in_future",
            Self::Expired => "expired",
            Self::ValidityTooLong => "validity_too_long",
            Self::InvalidStateShape => "invalid_state_shape",
            Self::UnknownStateField => "unknown_state_field",
            Self::MissingStateField => "missing_state_field",
            Self::ForbiddenStateField => "forbidden_state_field",
            Self::InvalidStateCodec => "invalid_state_codec",
            Self::InvalidSchema => "invalid_schema",
            Self::TooManyGenerations => "too_many_generations",
            Self::InvalidExtension => "invalid_extension",
            Self::DehydrationFailed => "dehydration_failed",
            Self::HydrationFailed => "hydration_failed",
        }
    }
}

/// Redacted snapshot error that never includes state, signatures, or hostile fields.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct SnapshotError {
    kind: SnapshotErrorKind,
}

impl SnapshotError {
    pub(crate) const fn new(kind: SnapshotErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed rejection reason.
    #[must_use]
    pub const fn kind(self) -> SnapshotErrorKind {
        self.kind
    }
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for SnapshotError {}
