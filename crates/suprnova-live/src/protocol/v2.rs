//! Protocol-v2 lifecycle requests and typed child/URL response fields.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Serialize;

use crate::canonical::{CanonicalValue, parse_canonical_value, to_canonical_bytes};
use crate::error::LiveError;
use crate::identity::{
    ActionName, ComponentName, ContentDigest, CorrelationId, IdempotencyKey, InstanceId,
    ModelField, Revision,
};

use super::request::{
    Operation, SnapshotInput, map_canonical, object, parse_extensions, parse_model_proposals,
    parse_operation, parse_snapshot, take, take_optional, take_string, take_u16,
};
use super::response::{
    Emission, RenderPayload, ResponseEncodingParts, ResponseOutcome, encode_response_object,
    parse_bounded_object, parse_emissions, parse_live_error, parse_outcome, parse_redirect,
    parse_render, parse_revision, parse_snapshot as parse_response_snapshot,
    response_encoding_object, validate_outcome,
};
use super::{ProtocolError, ProtocolErrorKind, ProtocolLimits};

/// Ordered operation set admitted only by protocol v2.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OperationV2 {
    /// Apply one separately supplied model proposal before an action.
    SyncModel {
        /// Registered field identity pending component lookup.
        field: ModelField,
    },
    /// Invoke one registered action after model synchronization.
    InvokeAction {
        /// Registered action identity pending component lookup.
        name: ActionName,
        /// Bounded canonical arguments pending registered schema validation.
        arguments: BTreeMap<String, CanonicalValue>,
    },
    /// Apply one separately signed parent-issued child parameter update.
    ParamsChanged,
    /// Complete one declared lazy lifecycle boundary.
    LazyComplete,
    /// Obtain current island state without retrying an earlier operation.
    FreshRender,
}

impl OperationV2 {
    /// Returns whether this operation is recovery that cannot replay the original request.
    #[must_use]
    pub const fn is_recovery_without_replay(&self) -> bool {
        matches!(self, Self::FreshRender)
    }

    const fn is_lifecycle(&self) -> bool {
        matches!(
            self,
            Self::ParamsChanged | Self::LazyComplete | Self::FreshRender
        )
    }
}

/// Fully parsed protocol-v2 request, still unresolved against the component registry.
pub struct UpdateRequestV2 {
    runtime_contract_version: u16,
    snapshot_schema_version: u16,
    correlation_id: CorrelationId,
    idempotency_key: IdempotencyKey,
    component: ComponentName,
    base_revision: Revision,
    snapshot: SnapshotInput,
    child_parameters: Option<Vec<u8>>,
    model_proposals: BTreeMap<ModelField, CanonicalValue>,
    operations: Vec<OperationV2>,
    extensions: BTreeMap<String, CanonicalValue>,
}

impl UpdateRequestV2 {
    /// Returns the v2 control-protocol version.
    #[must_use]
    pub const fn protocol_version(&self) -> u16 {
        2
    }

    /// Returns the required browser runtime contract version.
    #[must_use]
    pub const fn runtime_contract_version(&self) -> u16 {
        self.runtime_contract_version
    }

    /// Returns the independently versioned snapshot schema.
    #[must_use]
    pub const fn snapshot_schema_version(&self) -> u16 {
        self.snapshot_schema_version
    }

    /// Returns the request correlation identity.
    #[must_use]
    pub const fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }

    /// Returns the semantic retry identity.
    #[must_use]
    pub const fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }

    /// Returns the unresolved registered component identity.
    #[must_use]
    pub const fn component(&self) -> &ComponentName {
        &self.component
    }

    /// Returns the expected island base revision.
    #[must_use]
    pub const fn base_revision(&self) -> Revision {
        self.base_revision
    }

    /// Returns the signed snapshot input used to hydrate the child or island.
    #[must_use]
    pub const fn snapshot(&self) -> &SnapshotInput {
        &self.snapshot
    }

    /// Returns the separately signed parent-issued child parameter envelope, when required.
    #[must_use]
    pub fn child_parameters(&self) -> Option<&[u8]> {
        self.child_parameters.as_deref()
    }

    /// Returns bounded model proposals pending registered schema validation.
    #[must_use]
    pub const fn model_proposals(&self) -> &BTreeMap<ModelField, CanonicalValue> {
        &self.model_proposals
    }

    /// Returns deterministic v2 operation order.
    #[must_use]
    pub fn operations(&self) -> &[OperationV2] {
        &self.operations
    }

    /// Returns semantic namespaced request extensions.
    #[must_use]
    pub const fn extensions(&self) -> &BTreeMap<String, CanonicalValue> {
        &self.extensions
    }
}

impl fmt::Debug for UpdateRequestV2 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UpdateRequestV2:redacted>")
    }
}

pub(crate) fn parse_update_request_v2_fields(
    mut fields: BTreeMap<String, CanonicalValue>,
    limits: &ProtocolLimits,
) -> Result<UpdateRequestV2, ProtocolError> {
    require_exact_keys(
        &fields,
        &[
            "base_revision",
            "child_parameters",
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
    if protocol_version != 2 || runtime_contract_version != 2 || snapshot_schema_version != 1 {
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
    let child_parameters = parse_optional_envelope(
        take(&mut fields, "child_parameters")?,
        limits.max_snapshot_bytes(),
        limits,
    )?;
    let model_proposals = parse_model_proposals(
        take(&mut fields, "model_proposals")?,
        limits.max_model_proposals(),
    )?;
    let operations = parse_operations_v2(take(&mut fields, "operations")?, limits)?;
    validate_batch_v2(&operations, &model_proposals, child_parameters.is_some())?;
    let extensions = parse_extensions(take(&mut fields, "extensions")?, limits.max_extensions())?;
    Ok(UpdateRequestV2 {
        runtime_contract_version,
        snapshot_schema_version,
        correlation_id,
        idempotency_key,
        component,
        base_revision,
        snapshot,
        child_parameters,
        model_proposals,
        operations,
        extensions,
    })
}

fn parse_optional_envelope(
    value: CanonicalValue,
    max_bytes: usize,
    limits: &ProtocolLimits,
) -> Result<Option<Vec<u8>>, ProtocolError> {
    if matches!(value, CanonicalValue::Null) {
        return Ok(None);
    }
    if !matches!(value, CanonicalValue::Object(_)) {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
    }
    let encoded = to_canonical_bytes(&value, limits.input()).map_err(map_canonical)?;
    if encoded.len() > max_bytes {
        return Err(ProtocolError::new(ProtocolErrorKind::SnapshotTooLarge));
    }
    Ok(Some(encoded))
}

fn parse_operations_v2(
    value: CanonicalValue,
    limits: &ProtocolLimits,
) -> Result<Vec<OperationV2>, ProtocolError> {
    let CanonicalValue::Array(values) = value else {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
    };
    if values.is_empty() || values.len() > limits.max_operations() {
        return Err(ProtocolError::new(ProtocolErrorKind::TooManyOperations));
    }
    values
        .into_iter()
        .map(|value| parse_operation_v2(value, limits))
        .collect()
}

fn parse_operation_v2(
    value: CanonicalValue,
    limits: &ProtocolLimits,
) -> Result<OperationV2, ProtocolError> {
    let fields = object(value)?;
    let kind = fields.get("kind").and_then(|value| match value {
        CanonicalValue::String(value) => Some(value.as_str()),
        _ => None,
    });
    match kind {
        Some("sync_model" | "invoke_action") => {
            match parse_operation(CanonicalValue::Object(fields), limits)? {
                Operation::SyncModel { field } => Ok(OperationV2::SyncModel { field }),
                Operation::InvokeAction { name, arguments } => {
                    Ok(OperationV2::InvokeAction { name, arguments })
                }
            }
        }
        Some("params_changed") if fields.len() == 1 => Ok(OperationV2::ParamsChanged),
        Some("lazy_complete") if fields.len() == 1 => Ok(OperationV2::LazyComplete),
        Some("fresh_render") if fields.len() == 1 => Ok(OperationV2::FreshRender),
        _ => Err(ProtocolError::new(ProtocolErrorKind::AmbiguousOperation)),
    }
}

fn validate_batch_v2(
    operations: &[OperationV2],
    model_proposals: &BTreeMap<ModelField, CanonicalValue>,
    has_child_parameters: bool,
) -> Result<(), ProtocolError> {
    if let Some(lifecycle) = operations.iter().find(|operation| operation.is_lifecycle()) {
        return validate_lifecycle_batch(
            lifecycle,
            operations.len(),
            model_proposals,
            has_child_parameters,
        );
    }
    if has_child_parameters {
        return Err(ProtocolError::new(ProtocolErrorKind::IncompatibleBatch));
    }
    validate_action_batch_v2(operations, model_proposals)
}

fn validate_lifecycle_batch(
    operation: &OperationV2,
    operation_count: usize,
    model_proposals: &BTreeMap<ModelField, CanonicalValue>,
    has_child_parameters: bool,
) -> Result<(), ProtocolError> {
    let authority_matches = match operation {
        OperationV2::ParamsChanged => has_child_parameters,
        OperationV2::LazyComplete | OperationV2::FreshRender => !has_child_parameters,
        OperationV2::SyncModel { .. } | OperationV2::InvokeAction { .. } => false,
    };
    if operation_count == 1 && model_proposals.is_empty() && authority_matches {
        Ok(())
    } else {
        Err(ProtocolError::new(ProtocolErrorKind::IncompatibleBatch))
    }
}

fn validate_action_batch_v2(
    operations: &[OperationV2],
    model_proposals: &BTreeMap<ModelField, CanonicalValue>,
) -> Result<(), ProtocolError> {
    let mut invoked = false;
    let mut synchronized = BTreeSet::new();
    for operation in operations {
        match operation {
            OperationV2::SyncModel { field } if !invoked => {
                if !model_proposals.contains_key(field) || !synchronized.insert(field.clone()) {
                    return Err(ProtocolError::new(ProtocolErrorKind::IncompatibleBatch));
                }
            }
            OperationV2::InvokeAction { .. } if !invoked => invoked = true,
            OperationV2::SyncModel { .. }
            | OperationV2::InvokeAction { .. }
            | OperationV2::ParamsChanged
            | OperationV2::LazyComplete
            | OperationV2::FreshRender => {
                return Err(ProtocolError::new(ProtocolErrorKind::IncompatibleBatch));
            }
        }
    }
    Ok(())
}

/// Signed parent-issued child parameter delivery carried by one accepted response.
#[derive(Clone)]
pub struct ChildParameterDelivery {
    child_instance: InstanceId,
    parameter_hash: ContentDigest,
    envelope: Vec<u8>,
}

impl ChildParameterDelivery {
    /// Returns the independently scheduled child instance.
    #[must_use]
    pub const fn child_instance(&self) -> &InstanceId {
        &self.child_instance
    }

    /// Returns the comparable parameter hash emitted at the child boundary.
    #[must_use]
    pub const fn parameter_hash(&self) -> &ContentDigest {
        &self.parameter_hash
    }

    /// Returns the canonical signed child-parameter envelope.
    #[must_use]
    pub fn envelope(&self) -> &[u8] {
        &self.envelope
    }
}

impl fmt::Debug for ChildParameterDelivery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<ChildParameterDelivery:redacted>")
    }
}

/// Typed current-route reflection or real document navigation intent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UrlIntent {
    /// Replace the current same-route URL after committed island state is applied.
    Reflected {
        /// Validated same-origin route target.
        target: String,
    },
    /// Perform terminal ordinary document navigation.
    Navigated {
        /// Validated same-origin route target.
        target: String,
    },
}

impl UrlIntent {
    /// Returns the validated same-origin route target.
    #[must_use]
    pub fn target(&self) -> &str {
        match self {
            Self::Reflected { target } | Self::Navigated { target } => target,
        }
    }
}

/// Fully parsed protocol-v2 response before browser application.
pub struct UpdateResponseV2 {
    correlation_id: CorrelationId,
    outcome: ResponseOutcome,
    accepted_revision: Option<Revision>,
    snapshot: Option<Vec<u8>>,
    render: Option<RenderPayload>,
    redirect: Option<String>,
    validation: BTreeMap<String, CanonicalValue>,
    events: Vec<Emission>,
    effects: Vec<Emission>,
    error: Option<LiveError>,
    extensions: BTreeMap<String, CanonicalValue>,
    child_deliveries: Vec<ChildParameterDelivery>,
    url_intent: Option<UrlIntent>,
}

impl UpdateResponseV2 {
    /// Returns the v2 control-protocol version.
    #[must_use]
    pub const fn protocol_version(&self) -> u16 {
        2
    }

    /// Returns the request correlation identity.
    #[must_use]
    pub const fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }

    /// Returns the stable response disposition.
    #[must_use]
    pub const fn outcome(&self) -> ResponseOutcome {
        self.outcome
    }

    /// Returns the accepted successor revision when present.
    #[must_use]
    pub const fn accepted_revision(&self) -> Option<Revision> {
        self.accepted_revision
    }

    /// Returns canonical signed snapshot bytes when present.
    #[must_use]
    pub fn snapshot(&self) -> Option<&[u8]> {
        self.snapshot.as_deref()
    }

    /// Returns the explicit render result when present.
    #[must_use]
    pub const fn render(&self) -> Option<&RenderPayload> {
        self.render.as_ref()
    }

    /// Returns the legacy terminal redirect target when present.
    #[must_use]
    pub fn redirect(&self) -> Option<&str> {
        self.redirect.as_deref()
    }

    /// Returns bounded validation metadata.
    #[must_use]
    pub const fn validation(&self) -> &BTreeMap<String, CanonicalValue> {
        &self.validation
    }

    /// Returns bounded declared events.
    #[must_use]
    pub fn events(&self) -> &[Emission] {
        &self.events
    }

    /// Returns bounded registered effects.
    #[must_use]
    pub fn effects(&self) -> &[Emission] {
        &self.effects
    }

    /// Returns the safe classified error when present.
    #[must_use]
    pub const fn error(&self) -> Option<&LiveError> {
        self.error.as_ref()
    }

    /// Returns semantic namespaced response extensions.
    #[must_use]
    pub const fn extensions(&self) -> &BTreeMap<String, CanonicalValue> {
        &self.extensions
    }

    /// Returns signed child deliveries scheduled only after parent commit.
    #[must_use]
    pub fn child_deliveries(&self) -> &[ChildParameterDelivery] {
        &self.child_deliveries
    }

    /// Returns typed reflected or navigated URL intent.
    #[must_use]
    pub const fn url_intent(&self) -> Option<&UrlIntent> {
        self.url_intent.as_ref()
    }
}

impl fmt::Debug for UpdateResponseV2 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UpdateResponseV2:redacted>")
    }
}

pub(crate) fn parse_update_response_v2_fields(
    mut fields: BTreeMap<String, CanonicalValue>,
    limits: &ProtocolLimits,
) -> Result<UpdateResponseV2, ProtocolError> {
    require_allowed_keys(
        &fields,
        &[
            "child_deliveries",
            "correlation_id",
            "effects",
            "events",
            "extensions",
            "outcome",
            "protocol_version",
            "url_intent",
            "validation",
        ],
        &[
            "accepted_revision",
            "error",
            "redirect",
            "render",
            "snapshot",
        ],
    )?;
    if take_u16(&mut fields, "protocol_version")? != 2 {
        return Err(ProtocolError::new(ProtocolErrorKind::UnsupportedVersion));
    }
    let correlation_id = CorrelationId::parse(&take_string(&mut fields, "correlation_id")?)
        .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))?;
    let outcome = parse_outcome(&take_string(&mut fields, "outcome")?)?;
    let accepted_revision = take_optional(&mut fields, "accepted_revision")
        .map(parse_revision)
        .transpose()?;
    let snapshot = take_optional(&mut fields, "snapshot")
        .map(|value| parse_response_snapshot(value, limits))
        .transpose()?;
    let render = take_optional(&mut fields, "render")
        .map(|value| parse_render(value, limits))
        .transpose()?;
    let redirect = take_optional(&mut fields, "redirect")
        .map(parse_redirect)
        .transpose()?;
    let validation = parse_bounded_object(
        take(&mut fields, "validation")?,
        limits.max_validation_entries(),
    )?;
    let events = parse_emissions(take(&mut fields, "events")?, limits.max_events())?;
    let effects = parse_emissions(take(&mut fields, "effects")?, limits.max_effects())?;
    let error = take_optional(&mut fields, "error")
        .map(parse_live_error)
        .transpose()?;
    let extensions = parse_extensions(take(&mut fields, "extensions")?, limits.max_extensions())?;
    let child_deliveries = parse_child_deliveries(
        take(&mut fields, "child_deliveries")?,
        limits.max_events(),
        limits,
    )?;
    let url_intent = parse_url_intent(take(&mut fields, "url_intent")?)?;
    validate_v2_outcome(
        outcome,
        accepted_revision,
        snapshot.as_deref(),
        render.as_ref(),
        redirect.as_deref(),
        &validation,
        &events,
        &effects,
        error.as_ref(),
        &child_deliveries,
        url_intent.as_ref(),
    )?;
    Ok(UpdateResponseV2 {
        correlation_id,
        outcome,
        accepted_revision,
        snapshot,
        render,
        redirect,
        validation,
        events,
        effects,
        error,
        extensions,
        child_deliveries,
        url_intent,
    })
}

fn parse_child_deliveries(
    value: CanonicalValue,
    max: usize,
    limits: &ProtocolLimits,
) -> Result<Vec<ChildParameterDelivery>, ProtocolError> {
    let CanonicalValue::Array(values) = value else {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
    };
    if values.len() > max {
        return Err(ProtocolError::new(ProtocolErrorKind::TooManyEntries));
    }
    values
        .into_iter()
        .map(|value| {
            let mut fields = object(value)?;
            require_exact_keys(&fields, &["child_instance", "envelope", "parameter_hash"])?;
            let child_instance = InstanceId::parse(&take_string(&mut fields, "child_instance")?)
                .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))?;
            let parameter_hash = ContentDigest::parse(&take_string(&mut fields, "parameter_hash")?)
                .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))?;
            let envelope = parse_optional_envelope(
                take(&mut fields, "envelope")?,
                limits.max_snapshot_bytes(),
                limits,
            )?
            .ok_or_else(|| ProtocolError::new(ProtocolErrorKind::InvalidEnvelope))?;
            Ok(ChildParameterDelivery {
                child_instance,
                parameter_hash,
                envelope,
            })
        })
        .collect()
}

fn parse_url_intent(value: CanonicalValue) -> Result<Option<UrlIntent>, ProtocolError> {
    if matches!(value, CanonicalValue::Null) {
        return Ok(None);
    }
    let mut fields = object(value)?;
    require_exact_keys(&fields, &["kind", "target"])?;
    let kind = take_string(&mut fields, "kind")?;
    let target = parse_redirect(take(&mut fields, "target")?)?;
    match kind.as_str() {
        "reflected" => Ok(Some(UrlIntent::Reflected { target })),
        "navigated" => Ok(Some(UrlIntent::Navigated { target })),
        _ => Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope)),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "v2 validates base response classes together with child and URL exclusivity"
)]
fn validate_v2_outcome(
    outcome: ResponseOutcome,
    revision: Option<Revision>,
    snapshot: Option<&[u8]>,
    render: Option<&RenderPayload>,
    redirect: Option<&str>,
    validation: &BTreeMap<String, CanonicalValue>,
    events: &[Emission],
    effects: &[Emission],
    error: Option<&LiveError>,
    child_deliveries: &[ChildParameterDelivery],
    url_intent: Option<&UrlIntent>,
) -> Result<(), ProtocolError> {
    if redirect.is_some() && url_intent.is_some() {
        return Err(ProtocolError::new(ProtocolErrorKind::OutcomeMismatch));
    }
    let navigated = url_intent.and_then(|intent| match intent {
        UrlIntent::Navigated { target } => Some(target.as_str()),
        UrlIntent::Reflected { .. } => None,
    });
    validate_outcome(
        outcome,
        revision,
        snapshot,
        render,
        redirect.or(navigated),
        validation,
        events,
        effects,
        error,
    )?;
    let accepted = matches!(
        outcome,
        ResponseOutcome::Accepted | ResponseOutcome::Duplicate
    );
    let committed = revision.is_some() && snapshot.is_some() && render.is_some();
    if !accepted && (!child_deliveries.is_empty() || url_intent.is_some()) {
        return Err(ProtocolError::new(ProtocolErrorKind::OutcomeMismatch));
    }
    if navigated.is_some() && (committed || !child_deliveries.is_empty()) {
        return Err(ProtocolError::new(ProtocolErrorKind::OutcomeMismatch));
    }
    if matches!(url_intent, Some(UrlIntent::Reflected { .. })) && !committed {
        return Err(ProtocolError::new(ProtocolErrorKind::OutcomeMismatch));
    }
    if !child_deliveries.is_empty() && !committed {
        return Err(ProtocolError::new(ProtocolErrorKind::OutcomeMismatch));
    }
    Ok(())
}

fn require_exact_keys(
    fields: &BTreeMap<String, CanonicalValue>,
    expected: &[&str],
) -> Result<(), ProtocolError> {
    if fields.len() != expected.len() || expected.iter().any(|name| !fields.contains_key(*name)) {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
    }
    Ok(())
}

fn require_allowed_keys(
    fields: &BTreeMap<String, CanonicalValue>,
    required: &[&str],
    optional: &[&str],
) -> Result<(), ProtocolError> {
    if required.iter().any(|name| !fields.contains_key(*name))
        || fields
            .keys()
            .any(|name| !required.contains(&name.as_str()) && !optional.contains(&name.as_str()))
    {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
    }
    Ok(())
}

pub(crate) fn encode_update_response_v2(
    response: &UpdateResponseV2,
    limits: &ProtocolLimits,
) -> Result<Vec<u8>, ProtocolError> {
    let mut fields = response_encoding_object(
        &ResponseEncodingParts {
            protocol_version: 2,
            correlation_id: &response.correlation_id,
            outcome: response.outcome,
            accepted_revision: response.accepted_revision,
            snapshot: response.snapshot.as_deref(),
            render: response.render.as_ref(),
            redirect: response.redirect.as_deref(),
            validation: &response.validation,
            events: &response.events,
            effects: &response.effects,
            error: response.error.as_ref(),
            extensions: &response.extensions,
        },
        limits,
    )?;
    let child_deliveries = response
        .child_deliveries
        .iter()
        .map(|delivery| {
            let envelope =
                parse_canonical_value(&delivery.envelope, limits.input()).map_err(map_canonical)?;
            Ok(serde_json::json!({
                "child_instance": delivery.child_instance.to_base64url(),
                "parameter_hash": delivery.parameter_hash.to_base64url(),
                "envelope": envelope,
            }))
        })
        .collect::<Result<Vec<_>, ProtocolError>>()?;
    fields.insert(
        "child_deliveries".to_owned(),
        serde_json::Value::Array(child_deliveries),
    );
    let url_intent = match response.url_intent.as_ref() {
        Some(UrlIntent::Reflected { target }) => {
            serde_json::json!({"kind": "reflected", "target": target})
        }
        Some(UrlIntent::Navigated { target }) => {
            serde_json::json!({"kind": "navigated", "target": target})
        }
        None => serde_json::Value::Null,
    };
    fields.insert("url_intent".to_owned(), url_intent);
    encode_response_object(fields, limits)
}

pub(crate) fn parse_response_envelope(
    encoded: &[u8],
    limits: &ProtocolLimits,
) -> Result<BTreeMap<String, CanonicalValue>, ProtocolError> {
    let value = parse_canonical_value(encoded, limits.input()).map_err(map_canonical)?;
    object(value)
}
