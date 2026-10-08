//! Child-parameter body, expectation, and limit contracts.

use std::error::Error;
use std::fmt;

use serde::Serialize;

use crate::canonical::{CanonicalErrorKind, CanonicalValue, to_canonical_bytes};
use crate::component::composition::{ChildKey, ChildParameterSchema, PendingChildParameters};
use crate::identity::{ContentDigest, InstanceId, KeyId, Revision, ScopeFingerprint, UnixMillis};
use crate::limits::InputLimits;

const CHILD_PARAMETERS_FORM: &str = "child_parameters";
const HARD_MAX_CLOCK_SKEW_MS: u64 = 60_000;
const HARD_MAX_LIFETIME_MS: u64 = 300_000;

/// Canonical child-parameter body schema version.
pub const CHILD_PARAMETERS_SCHEMA_V1: u16 = 1;

/// Canonical exact-child-bound parameter body schema version.
pub const CHILD_PARAMETERS_SCHEMA_V2: u16 = 2;

/// Closed reason a child-parameter capability was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChildParameterErrorKind {
    /// Limit or body construction policy was invalid.
    InvalidConfiguration,
    /// Encoded input exceeded its byte bound.
    InputTooLarge,
    /// Encoded input exceeded its nesting bound.
    InputTooDeep,
    /// Encoded input exceeded its entry bound.
    TooManyEntries,
    /// An object repeated one field.
    DuplicateField,
    /// The envelope or canonical JSON shape was invalid.
    InvalidEnvelope,
    /// The capability form did not identify child parameters.
    WrongForm,
    /// The capability schema version is unsupported.
    UnsupportedSchema,
    /// Integrity verification, purpose selection, or key lookup failed.
    SignatureInvalid,
    /// A draft named a key other than the current signing key.
    SigningKeyMismatch,
    /// Scope, instance, child key, or child component contract did not match.
    BindingMismatch,
    /// The issuing parent revision is no longer the eligible accepted source.
    ParentRevisionMismatch,
    /// A rendered draft was not paired with its matching accepted parent outcome.
    ParentNotAccepted,
    /// The registered parameter schema version or digest did not match.
    ParameterSchemaMismatch,
    /// Canonical parameters did not match their signed value digest.
    ParameterValueMismatch,
    /// Parameters failed their registered typed schema.
    InvalidParameters,
    /// Issuance time exceeded permitted future skew.
    IssuedInFuture,
    /// The capability validity window elapsed.
    Expired,
    /// The requested validity interval exceeded configured policy.
    ValidityTooLong,
}

impl ChildParameterErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidConfiguration => "invalid_child_parameter_configuration",
            Self::InputTooLarge => "input_too_large",
            Self::InputTooDeep => "input_too_deep",
            Self::TooManyEntries => "too_many_entries",
            Self::DuplicateField => "duplicate_field",
            Self::InvalidEnvelope => "invalid_child_parameter_envelope",
            Self::WrongForm => "wrong_child_parameter_form",
            Self::UnsupportedSchema => "unsupported_child_parameter_schema",
            Self::SignatureInvalid => "child_parameter_signature_invalid",
            Self::SigningKeyMismatch => "child_parameter_signing_key_mismatch",
            Self::BindingMismatch => "child_parameter_binding_mismatch",
            Self::ParentRevisionMismatch => "parent_revision_mismatch",
            Self::ParentNotAccepted => "parent_outcome_not_accepted",
            Self::ParameterSchemaMismatch => "child_parameter_schema_mismatch",
            Self::ParameterValueMismatch => "child_parameter_value_mismatch",
            Self::InvalidParameters => "invalid_child_parameters",
            Self::IssuedInFuture => "child_parameters_issued_in_future",
            Self::Expired => "child_parameters_expired",
            Self::ValidityTooLong => "child_parameter_validity_too_long",
        }
    }
}

/// Redacted child-parameter construction or verification error.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ChildParameterError {
    kind: ChildParameterErrorKind,
}

impl ChildParameterError {
    pub(crate) const fn new(kind: ChildParameterErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed rejection reason.
    #[must_use]
    pub const fn kind(self) -> ChildParameterErrorKind {
        self.kind
    }
}

impl fmt::Display for ChildParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for ChildParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for ChildParameterError {}

/// Canonical input, clock-skew, and lifetime limits for child capabilities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChildParameterLimits {
    input: InputLimits,
    max_clock_skew_ms: u64,
    max_lifetime_ms: u64,
}

impl ChildParameterLimits {
    /// Creates bounded child-parameter policy.
    pub fn new(
        input: InputLimits,
        max_clock_skew_ms: u64,
        max_lifetime_ms: u64,
    ) -> Result<Self, ChildParameterError> {
        if max_clock_skew_ms > HARD_MAX_CLOCK_SKEW_MS
            || max_lifetime_ms == 0
            || max_lifetime_ms > HARD_MAX_LIFETIME_MS
        {
            return Err(ChildParameterError::new(
                ChildParameterErrorKind::InvalidConfiguration,
            ));
        }
        Ok(Self {
            input,
            max_clock_skew_ms,
            max_lifetime_ms,
        })
    }

    /// Returns the bounded canonical input policy.
    #[must_use]
    pub const fn input(&self) -> &InputLimits {
        &self.input
    }

    pub(crate) const fn max_clock_skew_ms(&self) -> u64 {
        self.max_clock_skew_ms
    }

    pub(crate) const fn max_lifetime_ms(&self) -> u64 {
        self.max_lifetime_ms
    }
}

/// Canonical signed parent-to-child parameter body.
#[derive(Clone, Serialize)]
pub struct ChildParametersV1 {
    pub(crate) form: &'static str,
    pub(crate) schema_version: u16,
    pub(crate) parent_scope: ScopeFingerprint,
    pub(crate) parent_instance: InstanceId,
    pub(crate) parent_revision: Revision,
    pub(crate) child_key: String,
    pub(crate) child_contract: ContentDigest,
    pub(crate) parameter_schema_version: u16,
    pub(crate) parameter_schema_digest: ContentDigest,
    pub(crate) parameters: CanonicalValue,
    pub(crate) value_digest: ContentDigest,
    pub(crate) issued_at: UnixMillis,
    pub(crate) expires_at: UnixMillis,
    pub(crate) key_id: KeyId,
}

impl ChildParametersV1 {
    /// Returns the issuing accepted parent revision.
    #[must_use]
    pub const fn parent_revision(&self) -> Revision {
        self.parent_revision
    }

    /// Returns the stable child key.
    #[must_use]
    pub fn child_key(&self) -> &str {
        &self.child_key
    }

    /// Returns the typed canonical parameter object.
    #[must_use]
    pub const fn parameters(&self) -> &CanonicalValue {
        &self.parameters
    }
}

impl fmt::Debug for ChildParametersV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChildParametersV1")
            .field("parent_scope", &self.parent_scope)
            .field("parent_instance", &self.parent_instance)
            .field("parent_revision", &self.parent_revision)
            .field("child_key", &self.child_key)
            .field("parameters", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// Canonical signed parent-to-exact-child parameter body.
#[derive(Clone, Serialize)]
pub struct ChildParametersV2 {
    pub(crate) form: &'static str,
    pub(crate) schema_version: u16,
    pub(crate) parent_scope: ScopeFingerprint,
    pub(crate) parent_instance: InstanceId,
    pub(crate) parent_revision: Revision,
    pub(crate) child_key: String,
    pub(crate) child_contract: ContentDigest,
    pub(crate) child_instance: InstanceId,
    pub(crate) parameter_schema_version: u16,
    pub(crate) parameter_schema_digest: ContentDigest,
    pub(crate) parameters: CanonicalValue,
    pub(crate) value_digest: ContentDigest,
    pub(crate) issued_at: UnixMillis,
    pub(crate) expires_at: UnixMillis,
    pub(crate) key_id: KeyId,
}

impl ChildParametersV2 {
    /// Returns the issuing accepted parent revision.
    #[must_use]
    pub const fn parent_revision(&self) -> Revision {
        self.parent_revision
    }

    /// Returns the stable child key.
    #[must_use]
    pub fn child_key(&self) -> &str {
        &self.child_key
    }

    /// Returns the exact child instance bound by this capability.
    #[must_use]
    pub const fn child_instance(&self) -> &InstanceId {
        &self.child_instance
    }

    /// Returns the typed canonical parameter object.
    #[must_use]
    pub const fn parameters(&self) -> &CanonicalValue {
        &self.parameters
    }
}

impl fmt::Debug for ChildParametersV2 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChildParametersV2")
            .field("parent_scope", &self.parent_scope)
            .field("parent_instance", &self.parent_instance)
            .field("parent_revision", &self.parent_revision)
            .field("child_key", &self.child_key)
            .field("child_instance", &self.child_instance)
            .field("parameters", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// Server-rendered child update awaiting its matching accepted parent outcome.
#[derive(Clone)]
pub struct PreparedChildParametersV1 {
    pub(crate) body: ChildParametersV1,
}

impl PreparedChildParametersV1 {
    /// Prepares a bounded child update during parent rendering without publishing it.
    #[allow(
        clippy::too_many_arguments,
        reason = "every authority and validity binding is explicit at this security boundary"
    )]
    pub fn new(
        parent_scope: ScopeFingerprint,
        parent_instance: InstanceId,
        parent_revision: Revision,
        pending: PendingChildParameters,
        issued_at: UnixMillis,
        expires_at: UnixMillis,
        key_id: KeyId,
        limits: &ChildParameterLimits,
    ) -> Result<Self, ChildParameterError> {
        validate_window(issued_at, expires_at, limits)?;
        to_canonical_bytes(pending.parameters(), limits.input()).map_err(map_canonical)?;
        let parameter_schema_digest =
            ContentDigest::from_bytes(pending.parameter_schema().as_bytes())
                .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?;
        let value_digest = ContentDigest::from_bytes(pending.parameter_value().as_bytes())
            .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?;
        Ok(Self {
            body: ChildParametersV1 {
                form: CHILD_PARAMETERS_FORM,
                schema_version: CHILD_PARAMETERS_SCHEMA_V1,
                parent_scope,
                parent_instance,
                parent_revision,
                child_key: pending.child().key().as_str().to_owned(),
                child_contract: pending.child().component_contract().clone(),
                parameter_schema_version: pending.parameter_schema_version(),
                parameter_schema_digest,
                parameters: pending.parameters().clone(),
                value_digest,
                issued_at,
                expires_at,
                key_id,
            },
        })
    }
}

impl fmt::Debug for PreparedChildParametersV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<PreparedChildParametersV1:redacted>")
    }
}

/// Server-rendered exact-child update awaiting its accepted parent outcome.
#[derive(Clone)]
pub struct PreparedChildParametersV2 {
    pub(crate) body: ChildParametersV2,
}

impl PreparedChildParametersV2 {
    /// Prepares a bounded exact-child update without publishing it.
    #[allow(
        clippy::too_many_arguments,
        reason = "every authority and validity binding is explicit at this security boundary"
    )]
    pub fn new(
        parent_scope: ScopeFingerprint,
        parent_instance: InstanceId,
        parent_revision: Revision,
        child_instance: InstanceId,
        pending: PendingChildParameters,
        issued_at: UnixMillis,
        expires_at: UnixMillis,
        key_id: KeyId,
        limits: &ChildParameterLimits,
    ) -> Result<Self, ChildParameterError> {
        validate_window(issued_at, expires_at, limits)?;
        to_canonical_bytes(pending.parameters(), limits.input()).map_err(map_canonical)?;
        let parameter_schema_digest =
            ContentDigest::from_bytes(pending.parameter_schema().as_bytes())
                .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?;
        let value_digest = ContentDigest::from_bytes(pending.parameter_value().as_bytes())
            .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?;
        Ok(Self {
            body: ChildParametersV2 {
                form: CHILD_PARAMETERS_FORM,
                schema_version: CHILD_PARAMETERS_SCHEMA_V2,
                parent_scope,
                parent_instance,
                parent_revision,
                child_key: pending.child().key().as_str().to_owned(),
                child_contract: pending.child().component_contract().clone(),
                child_instance,
                parameter_schema_version: pending.parameter_schema_version(),
                parameter_schema_digest,
                parameters: pending.parameters().clone(),
                value_digest,
                issued_at,
                expires_at,
                key_id,
            },
        })
    }
}

impl fmt::Debug for PreparedChildParametersV2 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<PreparedChildParametersV2:redacted>")
    }
}

/// Trusted current expectations used to reject cross-parent or superseded replay.
#[derive(Clone, Debug)]
pub struct ExpectedChildParametersV1 {
    pub(crate) parent_scope: ScopeFingerprint,
    pub(crate) parent_instance: InstanceId,
    pub(crate) parent_revision: Revision,
    pub(crate) child_key: ChildKey,
    pub(crate) child_contract: ContentDigest,
    pub(crate) parameter_schema: ChildParameterSchema,
    pub(crate) last_applied_parent_revision: Option<Revision>,
}

impl ExpectedChildParametersV1 {
    /// Binds verification to the current parent and registered child contract.
    #[must_use]
    pub const fn new(
        parent_scope: ScopeFingerprint,
        parent_instance: InstanceId,
        parent_revision: Revision,
        child_key: ChildKey,
        child_contract: ContentDigest,
        parameter_schema: ChildParameterSchema,
    ) -> Self {
        Self {
            parent_scope,
            parent_instance,
            parent_revision,
            child_key,
            child_contract,
            parameter_schema,
            last_applied_parent_revision: None,
        }
    }

    /// Records the last parent revision already applied by this child scheduler.
    #[must_use]
    pub fn after_applied_parent_revision(mut self, revision: Revision) -> Self {
        self.last_applied_parent_revision = Some(revision);
        self
    }
}

/// Trusted exact-child expectations used to reject substitution and replay.
#[derive(Clone, Debug)]
pub struct ExpectedChildParametersV2 {
    pub(crate) parent_scope: ScopeFingerprint,
    pub(crate) parent_instance: InstanceId,
    pub(crate) parent_revision: Revision,
    pub(crate) child_key: ChildKey,
    pub(crate) child_contract: ContentDigest,
    pub(crate) child_instance: InstanceId,
    pub(crate) parameter_schema: ChildParameterSchema,
    pub(crate) last_applied_parent_revision: Option<Revision>,
}

impl ExpectedChildParametersV2 {
    /// Binds verification to one parent and one exact child instance.
    #[must_use]
    pub const fn new(
        parent_scope: ScopeFingerprint,
        parent_instance: InstanceId,
        parent_revision: Revision,
        child_key: ChildKey,
        child_contract: ContentDigest,
        child_instance: InstanceId,
        parameter_schema: ChildParameterSchema,
    ) -> Self {
        Self {
            parent_scope,
            parent_instance,
            parent_revision,
            child_key,
            child_contract,
            child_instance,
            parameter_schema,
            last_applied_parent_revision: None,
        }
    }

    /// Records the last parent revision already applied by this child scheduler.
    #[must_use]
    pub fn after_applied_parent_revision(mut self, revision: Revision) -> Self {
        self.last_applied_parent_revision = Some(revision);
        self
    }
}

pub(crate) fn validate_window(
    issued_at: UnixMillis,
    expires_at: UnixMillis,
    limits: &ChildParameterLimits,
) -> Result<(), ChildParameterError> {
    let Some(lifetime) = expires_at.get().checked_sub(issued_at.get()) else {
        return Err(ChildParameterError::new(
            ChildParameterErrorKind::ValidityTooLong,
        ));
    };
    if lifetime == 0 || lifetime > limits.max_lifetime_ms() {
        return Err(ChildParameterError::new(
            ChildParameterErrorKind::ValidityTooLong,
        ));
    }
    Ok(())
}

pub(crate) fn map_canonical(error: crate::canonical::CanonicalError) -> ChildParameterError {
    let kind = match error.kind() {
        CanonicalErrorKind::TooLarge => ChildParameterErrorKind::InputTooLarge,
        CanonicalErrorKind::TooDeep => ChildParameterErrorKind::InputTooDeep,
        CanonicalErrorKind::TooManyEntries => ChildParameterErrorKind::TooManyEntries,
        CanonicalErrorKind::DuplicateKey => ChildParameterErrorKind::DuplicateField,
        CanonicalErrorKind::StringTooLong
        | CanonicalErrorKind::InvalidUtf8
        | CanonicalErrorKind::InvalidNumber
        | CanonicalErrorKind::InvalidJson
        | CanonicalErrorKind::SerializationFailed => ChildParameterErrorKind::InvalidEnvelope,
    };
    ChildParameterError::new(kind)
}

pub(crate) const fn form() -> &'static str {
    CHILD_PARAMETERS_FORM
}
