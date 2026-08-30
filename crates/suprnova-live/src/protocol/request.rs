//! Bounded duplicate-aware parsing for Live v1 update requests.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Serialize;

use crate::canonical::{
    CanonicalErrorKind, CanonicalValue, parse_canonical_value, to_canonical_bytes,
};
use crate::identity::{
    ActionName, BrowserNonce, ComponentName, CorrelationId, IdempotencyKey, ModelField, Revision,
};

use super::{ProtocolError, ProtocolErrorKind, ProtocolLimits};

/// Browser-carried signed state form on one update request.
#[derive(Clone, Eq, PartialEq)]
pub enum SnapshotInput {
    /// Ordinary scoped instanced snapshot.
    Instance {
        /// Canonical signed snapshot envelope bytes.
        envelope: Vec<u8>,
    },
    /// First action on a reusable public cached island.
    SeedPromotion {
        /// Canonical signed public seed envelope bytes.
        envelope: Vec<u8>,
        /// Untrusted at-least-128-bit promotion identity input.
        browser_nonce: BrowserNonce,
    },
}

impl fmt::Debug for SnapshotInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<SnapshotInput:redacted>")
    }
}

/// Ordered operation syntax parsed but not resolved to a registered Rust target.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Operation {
    /// Apply one separately supplied model proposal before any action.
    SyncModel {
        /// Registered field-shaped identity pending component lookup.
        field: ModelField,
    },
    /// Invoke one action-shaped identity after model synchronization.
    InvokeAction {
        /// Registered action-shaped identity pending component lookup.
        name: ActionName,
        /// Bounded canonical arguments pending registered schema validation.
        arguments: BTreeMap<String, CanonicalValue>,
    },
}

/// Fully parsed but not dispatched Live v1 update request.
pub struct UpdateRequest {
    protocol_version: u16,
    runtime_contract_version: u16,
    snapshot_schema_version: u16,
    correlation_id: CorrelationId,
    idempotency_key: IdempotencyKey,
    component: ComponentName,
    base_revision: Revision,
    snapshot: SnapshotInput,
    model_proposals: BTreeMap<ModelField, CanonicalValue>,
    operations: Vec<Operation>,
    extensions: BTreeMap<String, CanonicalValue>,
}

impl UpdateRequest {
    /// Returns the control-protocol version.
    #[must_use]
    pub const fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    /// Returns the browser-runtime contract version.
    #[must_use]
    pub const fn runtime_contract_version(&self) -> u16 {
        self.runtime_contract_version
    }

    /// Returns the snapshot schema expected by this request.
    #[must_use]
    pub const fn snapshot_schema_version(&self) -> u16 {
        self.snapshot_schema_version
    }

    /// Returns the end-to-end correlation identity.
    #[must_use]
    pub const fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }

    /// Returns the retry identity distinct from correlation and revision.
    #[must_use]
    pub const fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }

    /// Returns the component-shaped identity pending registry lookup.
    #[must_use]
    pub const fn component(&self) -> &ComponentName {
        &self.component
    }

    /// Returns the expected island base revision.
    #[must_use]
    pub const fn base_revision(&self) -> Revision {
        self.base_revision
    }

    /// Returns the distinct signed-state input form.
    #[must_use]
    pub const fn snapshot(&self) -> &SnapshotInput {
        &self.snapshot
    }

    /// Returns bounded model proposals pending registered schema validation.
    #[must_use]
    pub const fn model_proposals(&self) -> &BTreeMap<ModelField, CanonicalValue> {
        &self.model_proposals
    }

    /// Returns deterministic operation order pending registered target lookup.
    #[must_use]
    pub fn operations(&self) -> &[Operation] {
        &self.operations
    }

    /// Returns explicit backward-compatible namespaced extensions.
    #[must_use]
    pub const fn extensions(&self) -> &BTreeMap<String, CanonicalValue> {
        &self.extensions
    }
}

impl fmt::Debug for UpdateRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UpdateRequest:redacted>")
    }
}

/// Parses and validates one bounded Live v1 update request without dispatching it.
pub fn parse_update_request(
    encoded: &[u8],
    limits: &ProtocolLimits,
) -> Result<UpdateRequest, ProtocolError> {
    let value = parse_canonical_value(encoded, limits.input()).map_err(map_canonical)?;
    let fields = object(value)?;
    if protocol_version_from_fields(&fields)? != 1 {
        return Err(ProtocolError::new(ProtocolErrorKind::UnsupportedVersion));
    }
    parse_update_request_fields(fields, limits)
}

pub(crate) fn parse_update_request_fields(
    mut fields: BTreeMap<String, CanonicalValue>,
    limits: &ProtocolLimits,
) -> Result<UpdateRequest, ProtocolError> {
    require_keys(
        &fields,
        &[
            "base_revision",
            "component",
            "correlation_id",
            "extensions",
            "idempotency_key",
            "model_proposals",
            "operations",
            "protocol_version",
            "runtime_contract_version",
            "snapshot",
            "snapshot_schema_version",
        ],
    )?;
    let protocol_version = take_u16(&mut fields, "protocol_version")?;
    let runtime_contract_version = take_u16(&mut fields, "runtime_contract_version")?;
    let snapshot_schema_version = take_u16(&mut fields, "snapshot_schema_version")?;
    if protocol_version != 1 || runtime_contract_version != 1 || snapshot_schema_version != 1 {
        return Err(ProtocolError::new(ProtocolErrorKind::UnsupportedVersion));
    }
    let correlation_id = CorrelationId::parse(&take_string(&mut fields, "correlation_id")?)
        .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))?;
    let idempotency_key = IdempotencyKey::parse(&take_string(&mut fields, "idempotency_key")?)
        .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))?;
    let component = ComponentName::parse(&take_string(&mut fields, "component")?)
        .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))?;
    let base_revision = Revision::parse(&take_string(&mut fields, "base_revision")?)
        .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))?;
    let snapshot = parse_snapshot(take(&mut fields, "snapshot")?, limits)?;
    if matches!(snapshot, SnapshotInput::SeedPromotion { .. }) && base_revision != Revision::new(0)
    {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidSnapshotForm));
    }
    let model_proposals = parse_model_proposals(
        take(&mut fields, "model_proposals")?,
        limits.max_model_proposals(),
    )?;
    let operations = parse_operations(take(&mut fields, "operations")?, limits)?;
    validate_batch(&operations, &model_proposals)?;
    let extensions = parse_extensions(take(&mut fields, "extensions")?, limits.max_extensions())?;

    Ok(UpdateRequest {
        protocol_version,
        runtime_contract_version,
        snapshot_schema_version,
        correlation_id,
        idempotency_key,
        component,
        base_revision,
        snapshot,
        model_proposals,
        operations,
        extensions,
    })
}

pub(crate) fn parse_snapshot(
    value: CanonicalValue,
    limits: &ProtocolLimits,
) -> Result<SnapshotInput, ProtocolError> {
    let mut fields = object(value)?;
    let kind = take_string(&mut fields, "kind")?;
    let envelope = take(&mut fields, "envelope")?;
    if !matches!(envelope, CanonicalValue::Object(_)) {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidSnapshotForm));
    }
    let envelope = to_canonical_bytes(&envelope, limits.input()).map_err(map_canonical)?;
    if envelope.len() > limits.max_snapshot_bytes() {
        return Err(ProtocolError::new(ProtocolErrorKind::SnapshotTooLarge));
    }
    match kind.as_str() {
        "instance" if fields.is_empty() => Ok(SnapshotInput::Instance { envelope }),
        "seed_promotion" => {
            let nonce = BrowserNonce::parse(&take_string(&mut fields, "browser_nonce")?)
                .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))?;
            if !fields.is_empty() {
                return Err(ProtocolError::new(ProtocolErrorKind::InvalidSnapshotForm));
            }
            Ok(SnapshotInput::SeedPromotion {
                envelope,
                browser_nonce: nonce,
            })
        }
        _ => Err(ProtocolError::new(ProtocolErrorKind::InvalidSnapshotForm)),
    }
}

pub(crate) fn parse_model_proposals(
    value: CanonicalValue,
    max: usize,
) -> Result<BTreeMap<ModelField, CanonicalValue>, ProtocolError> {
    let fields = object(value)?;
    if fields.len() > max {
        return Err(ProtocolError::new(ProtocolErrorKind::TooManyModelProposals));
    }
    fields
        .into_iter()
        .map(|(name, value)| {
            ModelField::parse(&name)
                .map(|name| (name, value))
                .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))
        })
        .collect()
}

fn parse_operations(
    value: CanonicalValue,
    limits: &ProtocolLimits,
) -> Result<Vec<Operation>, ProtocolError> {
    let CanonicalValue::Array(values) = value else {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
    };
    if values.is_empty() || values.len() > limits.max_operations() {
        return Err(ProtocolError::new(ProtocolErrorKind::TooManyOperations));
    }
    values
        .into_iter()
        .map(|value| parse_operation(value, limits))
        .collect()
}

pub(crate) fn parse_operation(
    value: CanonicalValue,
    limits: &ProtocolLimits,
) -> Result<Operation, ProtocolError> {
    let mut fields = object(value)?;
    let kind = take_string(&mut fields, "kind")?;
    match kind.as_str() {
        "sync_model" => {
            if fields.len() != 1 || !fields.contains_key("field") {
                return Err(ProtocolError::new(ProtocolErrorKind::AmbiguousOperation));
            }
            let field = ModelField::parse(&take_string(&mut fields, "field")?)
                .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))?;
            Ok(Operation::SyncModel { field })
        }
        "invoke_action" => {
            if fields.len() != 2
                || !fields.contains_key("name")
                || !fields.contains_key("arguments")
            {
                return Err(ProtocolError::new(ProtocolErrorKind::AmbiguousOperation));
            }
            let name = ActionName::parse(&take_string(&mut fields, "name")?)
                .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))?;
            let arguments = object(take(&mut fields, "arguments")?)?;
            if arguments.len() > limits.max_arguments() {
                return Err(ProtocolError::new(ProtocolErrorKind::TooManyArguments));
            }
            for key in arguments.keys() {
                ModelField::parse(key)
                    .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))?;
            }
            Ok(Operation::InvokeAction { name, arguments })
        }
        _ => Err(ProtocolError::new(ProtocolErrorKind::AmbiguousOperation)),
    }
}

fn validate_batch(
    operations: &[Operation],
    model_proposals: &BTreeMap<ModelField, CanonicalValue>,
) -> Result<(), ProtocolError> {
    let mut invoked = false;
    let mut synced = BTreeSet::new();
    for operation in operations {
        match operation {
            Operation::SyncModel { field } if !invoked => {
                if !model_proposals.contains_key(field) || !synced.insert(field.clone()) {
                    return Err(ProtocolError::new(ProtocolErrorKind::IncompatibleBatch));
                }
            }
            Operation::InvokeAction { .. } if !invoked => invoked = true,
            _ => return Err(ProtocolError::new(ProtocolErrorKind::IncompatibleBatch)),
        }
    }
    Ok(())
}

pub(crate) fn parse_extensions(
    value: CanonicalValue,
    max: usize,
) -> Result<BTreeMap<String, CanonicalValue>, ProtocolError> {
    let fields = object(value)?;
    if fields.len() > max
        || fields.keys().any(|name| {
            !name.starts_with("x_")
                || name.len() > 64
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        })
    {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidExtension));
    }
    Ok(fields)
}

pub(crate) fn object(
    value: CanonicalValue,
) -> Result<BTreeMap<String, CanonicalValue>, ProtocolError> {
    let CanonicalValue::Object(fields) = value else {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
    };
    Ok(fields)
}

pub(crate) fn take(
    fields: &mut BTreeMap<String, CanonicalValue>,
    name: &str,
) -> Result<CanonicalValue, ProtocolError> {
    fields
        .remove(name)
        .ok_or_else(|| ProtocolError::new(ProtocolErrorKind::InvalidEnvelope))
}

pub(crate) fn take_optional(
    fields: &mut BTreeMap<String, CanonicalValue>,
    name: &str,
) -> Option<CanonicalValue> {
    fields.remove(name)
}

pub(crate) fn take_string(
    fields: &mut BTreeMap<String, CanonicalValue>,
    name: &str,
) -> Result<String, ProtocolError> {
    let CanonicalValue::String(value) = take(fields, name)? else {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
    };
    Ok(value)
}

pub(crate) fn take_u16(
    fields: &mut BTreeMap<String, CanonicalValue>,
    name: &str,
) -> Result<u16, ProtocolError> {
    let CanonicalValue::Number(value) = take(fields, name)? else {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
    };
    let value = value.get();
    if value.fract() != 0.0 || value < 0.0 || value > f64::from(u16::MAX) {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
    }
    Ok(value as u16)
}

fn require_keys(
    fields: &BTreeMap<String, CanonicalValue>,
    expected: &[&str],
) -> Result<(), ProtocolError> {
    if fields.len() != expected.len() || expected.iter().any(|name| !fields.contains_key(*name)) {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
    }
    Ok(())
}

pub(crate) fn protocol_version_from_fields(
    fields: &BTreeMap<String, CanonicalValue>,
) -> Result<u16, ProtocolError> {
    let Some(CanonicalValue::Number(value)) = fields.get("protocol_version") else {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
    };
    let value = value.get();
    if value.fract() != 0.0 || value < 0.0 || value > f64::from(u16::MAX) {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
    }
    Ok(value as u16)
}

pub(crate) fn map_canonical(error: crate::canonical::CanonicalError) -> ProtocolError {
    let kind = match error.kind() {
        CanonicalErrorKind::TooLarge | CanonicalErrorKind::StringTooLong => {
            ProtocolErrorKind::InputTooLarge
        }
        CanonicalErrorKind::TooDeep => ProtocolErrorKind::InputTooDeep,
        CanonicalErrorKind::TooManyEntries => ProtocolErrorKind::TooManyEntries,
        CanonicalErrorKind::DuplicateKey => ProtocolErrorKind::DuplicateField,
        CanonicalErrorKind::InvalidUtf8
        | CanonicalErrorKind::InvalidNumber
        | CanonicalErrorKind::InvalidJson
        | CanonicalErrorKind::SerializationFailed => ProtocolErrorKind::InvalidEnvelope,
    };
    ProtocolError::new(kind)
}
