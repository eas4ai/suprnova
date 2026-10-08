//! Canonical child-parameter signing and verification.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::schema::{form, map_canonical, validate_window};
use super::{
    AcceptedParentRevision, CHILD_PARAMETERS_SCHEMA_V1, CHILD_PARAMETERS_SCHEMA_V2,
    ChildParameterError, ChildParameterErrorKind, ChildParameterLimits, ChildParametersV1,
    ChildParametersV2, ExpectedChildParametersV1, ExpectedChildParametersV2,
    PreparedChildParametersV1, PreparedChildParametersV2, VerifiedChildParametersV1,
    VerifiedChildParametersV2,
};
use crate::canonical::{CanonicalValue, parse_canonical_value, to_canonical_bytes};
use crate::component::composition::{ChildKey, ChildParameterSchema};
use crate::crypto::{SnapshotKeyRing, SnapshotPurpose, SnapshotSignature};
use crate::identity::{ContentDigest, InstanceId, KeyId, Revision, ScopeFingerprint, UnixMillis};

/// Protocol version of a child-parameter capability.
///
/// Every step of the codec is shared between versions; only the schema constant, the signing
/// purpose, and the v2-only exact `child_instance` binding differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CodecVersion {
    V1,
    V2,
}

impl CodecVersion {
    const fn schema_version(self) -> u16 {
        match self {
            Self::V1 => CHILD_PARAMETERS_SCHEMA_V1,
            Self::V2 => CHILD_PARAMETERS_SCHEMA_V2,
        }
    }

    const fn purpose(self) -> SnapshotPurpose {
        match self {
            Self::V1 => SnapshotPurpose::ChildParametersV1,
            Self::V2 => SnapshotPurpose::ChildParametersV2,
        }
    }
}

/// Borrowed view of the fields every child-parameter body version shares.
struct BodyRef<'a> {
    parent_scope: &'a ScopeFingerprint,
    parent_instance: &'a InstanceId,
    parent_revision: Revision,
    child_key: &'a str,
    child_contract: &'a ContentDigest,
    child_instance: Option<&'a InstanceId>,
    parameter_schema_version: u16,
    parameter_schema_digest: &'a ContentDigest,
    parameters: &'a CanonicalValue,
    value_digest: &'a ContentDigest,
    issued_at: UnixMillis,
    expires_at: UnixMillis,
    key_id: &'a KeyId,
}

/// Borrowed view of the expectations every version shares.
struct ExpectedRef<'a> {
    parent_scope: &'a ScopeFingerprint,
    parent_instance: &'a InstanceId,
    parent_revision: Revision,
    child_key: &'a ChildKey,
    child_contract: &'a ContentDigest,
    child_instance: Option<&'a InstanceId>,
    parameter_schema: &'a ChildParameterSchema,
    last_applied_parent_revision: Option<Revision>,
}

trait ChildBody: Serialize {
    const VERSION: CodecVersion;

    fn as_ref(&self) -> BodyRef<'_>;
}

impl ChildBody for ChildParametersV1 {
    const VERSION: CodecVersion = CodecVersion::V1;

    fn as_ref(&self) -> BodyRef<'_> {
        BodyRef {
            parent_scope: &self.parent_scope,
            parent_instance: &self.parent_instance,
            parent_revision: self.parent_revision,
            child_key: &self.child_key,
            child_contract: &self.child_contract,
            child_instance: None,
            parameter_schema_version: self.parameter_schema_version,
            parameter_schema_digest: &self.parameter_schema_digest,
            parameters: &self.parameters,
            value_digest: &self.value_digest,
            issued_at: self.issued_at,
            expires_at: self.expires_at,
            key_id: &self.key_id,
        }
    }
}

impl ChildBody for ChildParametersV2 {
    const VERSION: CodecVersion = CodecVersion::V2;

    fn as_ref(&self) -> BodyRef<'_> {
        BodyRef {
            parent_scope: &self.parent_scope,
            parent_instance: &self.parent_instance,
            parent_revision: self.parent_revision,
            child_key: &self.child_key,
            child_contract: &self.child_contract,
            child_instance: Some(&self.child_instance),
            parameter_schema_version: self.parameter_schema_version,
            parameter_schema_digest: &self.parameter_schema_digest,
            parameters: &self.parameters,
            value_digest: &self.value_digest,
            issued_at: self.issued_at,
            expires_at: self.expires_at,
            key_id: &self.key_id,
        }
    }
}

impl ExpectedChildParametersV1 {
    fn as_ref(&self) -> ExpectedRef<'_> {
        ExpectedRef {
            parent_scope: &self.parent_scope,
            parent_instance: &self.parent_instance,
            parent_revision: self.parent_revision,
            child_key: &self.child_key,
            child_contract: &self.child_contract,
            child_instance: None,
            parameter_schema: &self.parameter_schema,
            last_applied_parent_revision: self.last_applied_parent_revision,
        }
    }
}

impl ExpectedChildParametersV2 {
    fn as_ref(&self) -> ExpectedRef<'_> {
        ExpectedRef {
            parent_scope: &self.parent_scope,
            parent_instance: &self.parent_instance,
            parent_revision: self.parent_revision,
            child_key: &self.child_key,
            child_contract: &self.child_contract,
            child_instance: Some(&self.child_instance),
            parameter_schema: &self.parameter_schema,
            last_applied_parent_revision: self.last_applied_parent_revision,
        }
    }
}

#[derive(Serialize)]
struct EnvelopeRef<'a> {
    body: &'a CanonicalValue,
    signature: &'a SnapshotSignature,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChildBodyWire {
    form: String,
    schema_version: u16,
    parent_scope: String,
    parent_instance: String,
    parent_revision: String,
    child_key: String,
    child_contract: String,
    parameter_schema_version: u16,
    parameter_schema_digest: String,
    parameters: serde_json::Value,
    value_digest: String,
    issued_at: String,
    expires_at: String,
    key_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChildBodyWireV2 {
    form: String,
    schema_version: u16,
    parent_scope: String,
    parent_instance: String,
    parent_revision: String,
    child_key: String,
    child_contract: String,
    child_instance: String,
    parameter_schema_version: u16,
    parameter_schema_digest: String,
    parameters: serde_json::Value,
    value_digest: String,
    issued_at: String,
    expires_at: String,
    key_id: String,
}

impl ChildBodyWireV2 {
    fn split(self) -> (ChildBodyWire, String) {
        let Self {
            form,
            schema_version,
            parent_scope,
            parent_instance,
            parent_revision,
            child_key,
            child_contract,
            child_instance,
            parameter_schema_version,
            parameter_schema_digest,
            parameters,
            value_digest,
            issued_at,
            expires_at,
            key_id,
        } = self;
        (
            ChildBodyWire {
                form,
                schema_version,
                parent_scope,
                parent_instance,
                parent_revision,
                child_key,
                child_contract,
                parameter_schema_version,
                parameter_schema_digest,
                parameters,
                value_digest,
                issued_at,
                expires_at,
                key_id,
            },
            child_instance,
        )
    }
}

impl PreparedChildParametersV1 {
    /// Signs a rendered draft only after its exact parent outcome was accepted.
    pub fn publish(
        self,
        accepted: &AcceptedParentRevision,
        keys: &SnapshotKeyRing,
        now: UnixMillis,
        limits: &ChildParameterLimits,
    ) -> Result<Vec<u8>, ChildParameterError> {
        require_parent(
            &self.body,
            &accepted.scope,
            &accepted.instance,
            accepted.revision,
        )?;
        publish_child_parameters(&self.body, keys, now, limits)
    }
}

impl PreparedChildParametersV2 {
    /// Signs a rendered draft only after its exact parent outcome was accepted.
    pub fn publish(
        self,
        accepted: &AcceptedParentRevision,
        keys: &SnapshotKeyRing,
        now: UnixMillis,
        limits: &ChildParameterLimits,
    ) -> Result<Vec<u8>, ChildParameterError> {
        require_parent(
            &self.body,
            &accepted.scope,
            &accepted.instance,
            accepted.revision,
        )?;
        publish_child_parameters(&self.body, keys, now, limits)
    }

    /// Seals a capability during accepted-response preparation for an exact claimed successor.
    ///
    /// The caller must retain the bytes until the enclosing parent transaction and ledger
    /// acceptance complete. Production publication still occurs only through that accepted result.
    pub(crate) fn seal_claimed_successor(
        self,
        parent_scope: &ScopeFingerprint,
        parent_instance: &InstanceId,
        parent_revision: Revision,
        keys: &SnapshotKeyRing,
        now: UnixMillis,
        limits: &ChildParameterLimits,
    ) -> Result<Vec<u8>, ChildParameterError> {
        require_parent(&self.body, parent_scope, parent_instance, parent_revision)?;
        publish_child_parameters(&self.body, keys, now, limits)
    }
}

fn require_parent<B: ChildBody>(
    body: &B,
    parent_scope: &ScopeFingerprint,
    parent_instance: &InstanceId,
    parent_revision: Revision,
) -> Result<(), ChildParameterError> {
    let body = body.as_ref();
    if body.parent_scope != parent_scope
        || body.parent_instance != parent_instance
        || body.parent_revision != parent_revision
    {
        return Err(ChildParameterError::new(
            ChildParameterErrorKind::ParentNotAccepted,
        ));
    }
    Ok(())
}

fn publish_child_parameters<B: ChildBody>(
    body: &B,
    keys: &SnapshotKeyRing,
    now: UnixMillis,
    limits: &ChildParameterLimits,
) -> Result<Vec<u8>, ChildParameterError> {
    let fields = body.as_ref();
    if fields.key_id != keys.active_key_id() {
        return Err(ChildParameterError::new(
            ChildParameterErrorKind::SigningKeyMismatch,
        ));
    }
    validate_time(&fields, now, limits)?;
    let body = canonical_from_serializable(body, limits)?;
    let canonical_body = to_canonical_bytes(&body, limits.input()).map_err(map_canonical)?;
    let signed = keys
        .sign(B::VERSION.purpose(), &canonical_body, now)
        .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::SignatureInvalid))?;
    let envelope = canonical_from_serializable(
        &EnvelopeRef {
            body: &body,
            signature: signed.signature(),
        },
        limits,
    )?;
    to_canonical_bytes(&envelope, limits.input()).map_err(map_canonical)
}

/// Verifies integrity, current parent eligibility, child binding, and typed values.
pub fn verify_child_parameters(
    encoded: &[u8],
    expected: &ExpectedChildParametersV1,
    keys: &SnapshotKeyRing,
    now: UnixMillis,
    limits: &ChildParameterLimits,
) -> Result<VerifiedChildParametersV1, ChildParameterError> {
    let body_value = verify_envelope(CodecVersion::V1, encoded, keys, now, limits)?;
    let wire: ChildBodyWire = deserialize_body(&body_value)?;
    let body = body_from_wire(CodecVersion::V1, wire, limits)?;
    let child_key = verify_body(&body, &expected.as_ref(), now, limits)?;
    Ok(VerifiedChildParametersV1::new(body, child_key))
}

/// Verifies a v2 capability and its exact parent/child bindings.
pub fn verify_child_parameters_v2(
    encoded: &[u8],
    expected: &ExpectedChildParametersV2,
    keys: &SnapshotKeyRing,
    now: UnixMillis,
    limits: &ChildParameterLimits,
) -> Result<VerifiedChildParametersV2, ChildParameterError> {
    let body_value = verify_envelope(CodecVersion::V2, encoded, keys, now, limits)?;
    let (wire, child_instance) = deserialize_body::<ChildBodyWireV2>(&body_value)?.split();
    let child_instance = InstanceId::parse(&child_instance)
        .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?;
    let body = body_from_wire(CodecVersion::V2, wire, limits)?.with_child_instance(child_instance);
    let child_key = verify_body(&body, &expected.as_ref(), now, limits)?;
    Ok(VerifiedChildParametersV2::new(body, child_key))
}

impl ChildParametersV1 {
    fn with_child_instance(self, child_instance: InstanceId) -> ChildParametersV2 {
        let Self {
            form,
            schema_version,
            parent_scope,
            parent_instance,
            parent_revision,
            child_key,
            child_contract,
            parameter_schema_version,
            parameter_schema_digest,
            parameters,
            value_digest,
            issued_at,
            expires_at,
            key_id,
        } = self;
        ChildParametersV2 {
            form,
            schema_version,
            parent_scope,
            parent_instance,
            parent_revision,
            child_key,
            child_contract,
            child_instance,
            parameter_schema_version,
            parameter_schema_digest,
            parameters,
            value_digest,
            issued_at,
            expires_at,
            key_id,
        }
    }
}

/// Runs the version-independent checks and returns the parsed child key.
fn verify_body<B: ChildBody>(
    body: &B,
    expected: &ExpectedRef<'_>,
    now: UnixMillis,
    limits: &ChildParameterLimits,
) -> Result<ChildKey, ChildParameterError> {
    let body = body.as_ref();
    validate_time(&body, now, limits)?;
    validate_expectations(&body, expected, limits)?;
    ChildKey::parse(body.child_key)
        .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::BindingMismatch))
}

fn verify_envelope(
    version: CodecVersion,
    encoded: &[u8],
    keys: &SnapshotKeyRing,
    now: UnixMillis,
    limits: &ChildParameterLimits,
) -> Result<CanonicalValue, ChildParameterError> {
    let envelope = parse_canonical_value(encoded, limits.input()).map_err(map_canonical)?;
    let CanonicalValue::Object(fields) = envelope else {
        return Err(ChildParameterError::new(
            ChildParameterErrorKind::InvalidEnvelope,
        ));
    };
    if fields.len() != 2 || !fields.contains_key("body") || !fields.contains_key("signature") {
        return Err(ChildParameterError::new(
            ChildParameterErrorKind::InvalidEnvelope,
        ));
    }
    let body = fields
        .get("body")
        .ok_or_else(|| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?;
    let CanonicalValue::Object(body_fields) = body else {
        return Err(ChildParameterError::new(
            ChildParameterErrorKind::InvalidEnvelope,
        ));
    };
    if string_field(body_fields, "form")? != form() {
        return Err(ChildParameterError::new(ChildParameterErrorKind::WrongForm));
    }
    if integer_field(body_fields, "schema_version")? != u64::from(version.schema_version()) {
        return Err(ChildParameterError::new(
            ChildParameterErrorKind::UnsupportedSchema,
        ));
    }
    let key_id = KeyId::parse(string_field(body_fields, "key_id")?)
        .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?;
    let CanonicalValue::String(signature) = fields
        .get("signature")
        .ok_or_else(|| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?
    else {
        return Err(ChildParameterError::new(
            ChildParameterErrorKind::InvalidEnvelope,
        ));
    };
    let signature = SnapshotSignature::parse(signature)
        .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::SignatureInvalid))?;
    let canonical_body = to_canonical_bytes(body, limits.input()).map_err(map_canonical)?;
    keys.verify(&key_id, version.purpose(), &canonical_body, &signature, now)
        .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::SignatureInvalid))?;
    Ok(body.clone())
}

/// Parses the version-independent body fields; v2 attaches `child_instance` afterwards.
fn body_from_wire(
    version: CodecVersion,
    wire: ChildBodyWire,
    limits: &ChildParameterLimits,
) -> Result<ChildParametersV1, ChildParameterError> {
    if wire.form != form() {
        return Err(ChildParameterError::new(ChildParameterErrorKind::WrongForm));
    }
    if wire.schema_version != version.schema_version() {
        return Err(ChildParameterError::new(
            ChildParameterErrorKind::UnsupportedSchema,
        ));
    }
    let body = ChildParametersV1 {
        form: form(),
        schema_version: wire.schema_version,
        parent_scope: ScopeFingerprint::parse(&wire.parent_scope)
            .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?,
        parent_instance: InstanceId::parse(&wire.parent_instance)
            .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?,
        parent_revision: Revision::parse(&wire.parent_revision)
            .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?,
        child_key: ChildKey::parse(&wire.child_key)
            .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?
            .as_str()
            .to_owned(),
        child_contract: ContentDigest::parse(&wire.child_contract)
            .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?,
        parameter_schema_version: wire.parameter_schema_version,
        parameter_schema_digest: ContentDigest::parse(&wire.parameter_schema_digest)
            .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?,
        parameters: CanonicalValue::from_serde_value(wire.parameters).map_err(map_canonical)?,
        value_digest: ContentDigest::parse(&wire.value_digest)
            .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?,
        issued_at: UnixMillis::parse(&wire.issued_at)
            .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?,
        expires_at: UnixMillis::parse(&wire.expires_at)
            .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?,
        key_id: KeyId::parse(&wire.key_id)
            .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?,
    };
    validate_window(body.issued_at, body.expires_at, limits)?;
    Ok(body)
}

fn validate_time(
    body: &BodyRef<'_>,
    now: UnixMillis,
    limits: &ChildParameterLimits,
) -> Result<(), ChildParameterError> {
    validate_window(body.issued_at, body.expires_at, limits)?;
    let latest_issue = now.get().saturating_add(limits.max_clock_skew_ms());
    if body.issued_at.get() > latest_issue {
        return Err(ChildParameterError::new(
            ChildParameterErrorKind::IssuedInFuture,
        ));
    }
    let expiry_with_skew = body
        .expires_at
        .get()
        .saturating_add(limits.max_clock_skew_ms());
    if now.get() > expiry_with_skew {
        return Err(ChildParameterError::new(ChildParameterErrorKind::Expired));
    }
    Ok(())
}

fn validate_expectations(
    body: &BodyRef<'_>,
    expected: &ExpectedRef<'_>,
    limits: &ChildParameterLimits,
) -> Result<(), ChildParameterError> {
    if body.parent_scope != expected.parent_scope
        || body.parent_instance != expected.parent_instance
        || body.child_key != expected.child_key.as_str()
        || body.child_contract != expected.child_contract
        || body.child_instance != expected.child_instance
    {
        return Err(ChildParameterError::new(
            ChildParameterErrorKind::BindingMismatch,
        ));
    }
    if body.parent_revision != expected.parent_revision
        || expected
            .last_applied_parent_revision
            .is_some_and(|revision| body.parent_revision <= revision)
    {
        return Err(ChildParameterError::new(
            ChildParameterErrorKind::ParentRevisionMismatch,
        ));
    }
    let expected_schema_digest =
        ContentDigest::from_bytes(expected.parameter_schema.digest().as_bytes()).map_err(|_| {
            ChildParameterError::new(ChildParameterErrorKind::ParameterSchemaMismatch)
        })?;
    if body.parameter_schema_version != expected.parameter_schema.version()
        || *body.parameter_schema_digest != expected_schema_digest
    {
        return Err(ChildParameterError::new(
            ChildParameterErrorKind::ParameterSchemaMismatch,
        ));
    }
    let value_digest = expected
        .parameter_schema
        .validate_and_digest(body.parameters, limits.input())
        .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidParameters))?;
    if body.value_digest.as_bytes() != value_digest.as_bytes() {
        return Err(ChildParameterError::new(
            ChildParameterErrorKind::ParameterValueMismatch,
        ));
    }
    Ok(())
}

fn canonical_from_serializable<T: Serialize>(
    value: &T,
    limits: &ChildParameterLimits,
) -> Result<CanonicalValue, ChildParameterError> {
    let serde_value = serde_json::to_value(value)
        .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))?;
    let canonical = CanonicalValue::from_serde_value(serde_value).map_err(map_canonical)?;
    to_canonical_bytes(&canonical, limits.input()).map_err(map_canonical)?;
    Ok(canonical)
}

fn deserialize_body<W: for<'de> Deserialize<'de>>(
    value: &CanonicalValue,
) -> Result<W, ChildParameterError> {
    let value = value.to_serde_value().map_err(map_canonical)?;
    serde_json::from_value(value)
        .map_err(|_| ChildParameterError::new(ChildParameterErrorKind::InvalidEnvelope))
}

fn string_field<'a>(
    fields: &'a BTreeMap<String, CanonicalValue>,
    name: &str,
) -> Result<&'a str, ChildParameterError> {
    match fields.get(name) {
        Some(CanonicalValue::String(value)) => Ok(value),
        _ => Err(ChildParameterError::new(
            ChildParameterErrorKind::InvalidEnvelope,
        )),
    }
}

fn integer_field(
    fields: &BTreeMap<String, CanonicalValue>,
    name: &str,
) -> Result<u64, ChildParameterError> {
    match fields.get(name) {
        Some(CanonicalValue::Number(value)) if value.get().fract() == 0.0 && value.get() >= 0.0 => {
            Ok(value.get() as u64)
        }
        _ => Err(ChildParameterError::new(
            ChildParameterErrorKind::InvalidEnvelope,
        )),
    }
}
