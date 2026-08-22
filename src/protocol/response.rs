//! Bounded parsing and semantic validation for Live v1 update responses.

use std::collections::BTreeMap;
use std::fmt;

use crate::canonical::{CanonicalValue, parse_canonical_value, to_canonical_bytes};
use crate::error::{ErrorCategory, LiveError, RecoveryInstruction, SafeDiagnosticCode};
use crate::identity::{BrowserOperationName, CorrelationId, Revision};

use super::request::{
    map_canonical, object, parse_extensions, protocol_version_from_fields, take, take_optional,
    take_string, take_u16,
};
use super::{ProtocolError, ProtocolErrorKind, ProtocolLimits};

/// Stable top-level response disposition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseOutcome {
    /// A new committed outcome is represented.
    Accepted,
    /// A prior compatible committed outcome is represented without re-execution.
    Duplicate,
    /// The request was not accepted and current DOM should normally remain.
    Rejected,
    /// Current island authority must be freshly rendered.
    RefreshRequired,
    /// Live processing cannot safely continue for this boundary.
    Fatal,
}

/// Explicit render result for a non-redirect accepted response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenderPayload {
    /// New server-rendered island HTML pending bounded morph.
    Html(String),
    /// Accepted state intentionally has no HTML morph.
    NoRender,
}

/// One declared event or registered browser effect with bounded canonical payload.
#[derive(Clone, Debug, PartialEq)]
pub struct Emission {
    name: BrowserOperationName,
    payload: CanonicalValue,
}

impl Emission {
    /// Returns the declared event or registered-effect identity.
    #[must_use]
    pub const fn name(&self) -> &BrowserOperationName {
        &self.name
    }

    /// Returns its bounded canonical payload.
    #[must_use]
    pub const fn payload(&self) -> &CanonicalValue {
        &self.payload
    }
}

/// Fully parsed response ready for correlation/revision validation and application planning.
pub struct UpdateResponse {
    protocol_version: u16,
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
}

impl UpdateResponse {
    /// Returns the control-protocol version.
    #[must_use]
    pub const fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    /// Returns the end-to-end request correlation identity.
    #[must_use]
    pub const fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }

    /// Returns the stable response disposition.
    #[must_use]
    pub const fn outcome(&self) -> ResponseOutcome {
        self.outcome
    }

    /// Returns the committed successor revision when required by the outcome.
    #[must_use]
    pub const fn accepted_revision(&self) -> Option<Revision> {
        self.accepted_revision
    }

    /// Returns canonical signed snapshot bytes after full envelope validation.
    #[must_use]
    pub fn snapshot(&self) -> Option<&[u8]> {
        self.snapshot.as_deref()
    }

    /// Returns the explicit HTML or no-render result.
    #[must_use]
    pub const fn render(&self) -> Option<&RenderPayload> {
        self.render.as_ref()
    }

    /// Returns a validated same-origin route target for terminal navigation.
    #[must_use]
    pub fn redirect(&self) -> Option<&str> {
        self.redirect.as_deref()
    }

    /// Returns the classified safe error for non-accepted outcomes.
    #[must_use]
    pub const fn error(&self) -> Option<&LiveError> {
        self.error.as_ref()
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

    /// Returns explicit backward-compatible namespaced extensions.
    #[must_use]
    pub const fn extensions(&self) -> &BTreeMap<String, CanonicalValue> {
        &self.extensions
    }
}

impl fmt::Debug for UpdateResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UpdateResponse:redacted>")
    }
}

pub(crate) struct ResponseEncodingParts<'value> {
    pub(crate) protocol_version: u16,
    pub(crate) correlation_id: &'value CorrelationId,
    pub(crate) outcome: ResponseOutcome,
    pub(crate) accepted_revision: Option<Revision>,
    pub(crate) snapshot: Option<&'value [u8]>,
    pub(crate) render: Option<&'value RenderPayload>,
    pub(crate) redirect: Option<&'value str>,
    pub(crate) validation: &'value BTreeMap<String, CanonicalValue>,
    pub(crate) events: &'value [Emission],
    pub(crate) effects: &'value [Emission],
    pub(crate) error: Option<&'value LiveError>,
    pub(crate) extensions: &'value BTreeMap<String, CanonicalValue>,
}

pub(crate) fn encode_update_response(
    response: &UpdateResponse,
    limits: &ProtocolLimits,
) -> Result<Vec<u8>, ProtocolError> {
    let fields = response_encoding_object(
        &ResponseEncodingParts {
            protocol_version: response.protocol_version,
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
    encode_response_object(fields, limits)
}

pub(crate) fn response_encoding_object(
    parts: &ResponseEncodingParts<'_>,
    limits: &ProtocolLimits,
) -> Result<serde_json::Map<String, serde_json::Value>, ProtocolError> {
    let mut fields = serde_json::Map::new();
    fields.insert(
        "protocol_version".to_owned(),
        serde_json::Value::from(parts.protocol_version),
    );
    fields.insert(
        "correlation_id".to_owned(),
        serde_json::Value::String(parts.correlation_id.to_base64url()),
    );
    fields.insert(
        "outcome".to_owned(),
        serde_json::Value::String(outcome_name(parts.outcome).to_owned()),
    );
    fields.insert(
        "validation".to_owned(),
        serde_json::to_value(parts.validation)
            .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidEnvelope))?,
    );
    fields.insert("events".to_owned(), emissions_json(parts.events)?);
    fields.insert("effects".to_owned(), emissions_json(parts.effects)?);
    fields.insert(
        "extensions".to_owned(),
        serde_json::to_value(parts.extensions)
            .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidEnvelope))?,
    );
    if let Some(revision) = parts.accepted_revision {
        fields.insert(
            "accepted_revision".to_owned(),
            serde_json::Value::String(revision.get().to_string()),
        );
    }
    if let Some(snapshot) = parts.snapshot {
        fields.insert("snapshot".to_owned(), embedded_json(snapshot, limits)?);
    }
    if let Some(render) = parts.render {
        let render = match render {
            RenderPayload::Html(html) => serde_json::json!({"kind": "html", "html": html}),
            RenderPayload::NoRender => serde_json::json!({"kind": "no_render"}),
        };
        fields.insert("render".to_owned(), render);
    }
    if let Some(redirect) = parts.redirect {
        fields.insert(
            "redirect".to_owned(),
            serde_json::Value::String(redirect.to_owned()),
        );
    }
    if let Some(error) = parts.error {
        fields.insert(
            "error".to_owned(),
            serde_json::json!({
                "category": error.category().as_str(),
                "recovery": error.recovery().as_str(),
                "detail": error.detail().as_str(),
            }),
        );
    }
    Ok(fields)
}

pub(crate) fn encode_response_object(
    fields: serde_json::Map<String, serde_json::Value>,
    limits: &ProtocolLimits,
) -> Result<Vec<u8>, ProtocolError> {
    let encoded = serde_json_canonicalizer::to_vec(&serde_json::Value::Object(fields))
        .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidEnvelope))?;
    if encoded.len() > limits.input().max_bytes() {
        return Err(ProtocolError::new(ProtocolErrorKind::InputTooLarge));
    }
    Ok(encoded)
}

fn embedded_json(
    encoded: &[u8],
    limits: &ProtocolLimits,
) -> Result<serde_json::Value, ProtocolError> {
    let value = parse_canonical_value(encoded, limits.input()).map_err(map_canonical)?;
    serde_json::to_value(value).map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidEnvelope))
}

fn emissions_json(emissions: &[Emission]) -> Result<serde_json::Value, ProtocolError> {
    emissions
        .iter()
        .map(|emission| {
            Ok(serde_json::json!({
                "name": emission.name().as_str(),
                "payload": emission.payload(),
            }))
        })
        .collect::<Result<Vec<_>, ProtocolError>>()
        .map(serde_json::Value::Array)
}

const fn outcome_name(outcome: ResponseOutcome) -> &'static str {
    match outcome {
        ResponseOutcome::Accepted => "accepted",
        ResponseOutcome::Duplicate => "duplicate",
        ResponseOutcome::Rejected => "rejected",
        ResponseOutcome::RefreshRequired => "refresh_required",
        ResponseOutcome::Fatal => "fatal",
    }
}

/// Parses and validates one complete Live v1 response before any field is applied.
pub fn parse_update_response(
    encoded: &[u8],
    limits: &ProtocolLimits,
) -> Result<UpdateResponse, ProtocolError> {
    let value = parse_canonical_value(encoded, limits.input()).map_err(map_canonical)?;
    let fields = object(value)?;
    if protocol_version_from_fields(&fields)? != 1 {
        return Err(ProtocolError::new(ProtocolErrorKind::UnsupportedVersion));
    }
    parse_update_response_fields(fields, limits)
}

pub(crate) fn parse_update_response_fields(
    mut fields: BTreeMap<String, CanonicalValue>,
    limits: &ProtocolLimits,
) -> Result<UpdateResponse, ProtocolError> {
    let allowed = [
        "accepted_revision",
        "correlation_id",
        "effects",
        "error",
        "events",
        "extensions",
        "outcome",
        "protocol_version",
        "redirect",
        "render",
        "snapshot",
        "validation",
    ];
    if fields.keys().any(|name| !allowed.contains(&name.as_str())) {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
    }
    let protocol_version = take_u16(&mut fields, "protocol_version")?;
    if protocol_version != 1 {
        return Err(ProtocolError::new(ProtocolErrorKind::UnsupportedVersion));
    }
    let correlation_id = CorrelationId::parse(&take_string(&mut fields, "correlation_id")?)
        .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))?;
    let outcome = parse_outcome(&take_string(&mut fields, "outcome")?)?;
    let accepted_revision = take_optional(&mut fields, "accepted_revision")
        .map(parse_revision)
        .transpose()?;
    let snapshot = take_optional(&mut fields, "snapshot")
        .map(|value| parse_snapshot(value, limits))
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
    if !fields.is_empty() {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
    }
    validate_outcome(
        outcome,
        accepted_revision,
        snapshot.as_deref(),
        render.as_ref(),
        redirect.as_deref(),
        &validation,
        &events,
        &effects,
        error.as_ref(),
    )?;
    Ok(UpdateResponse {
        protocol_version,
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
    })
}

pub(crate) fn parse_outcome(value: &str) -> Result<ResponseOutcome, ProtocolError> {
    match value {
        "accepted" => Ok(ResponseOutcome::Accepted),
        "duplicate" => Ok(ResponseOutcome::Duplicate),
        "rejected" => Ok(ResponseOutcome::Rejected),
        "refresh_required" => Ok(ResponseOutcome::RefreshRequired),
        "fatal" => Ok(ResponseOutcome::Fatal),
        _ => Err(ProtocolError::new(ProtocolErrorKind::OutcomeMismatch)),
    }
}

pub(crate) fn parse_revision(value: CanonicalValue) -> Result<Revision, ProtocolError> {
    let CanonicalValue::String(value) = value else {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
    };
    Revision::parse(&value).map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))
}

pub(crate) fn parse_snapshot(
    value: CanonicalValue,
    limits: &ProtocolLimits,
) -> Result<Vec<u8>, ProtocolError> {
    if !matches!(value, CanonicalValue::Object(_)) {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
    }
    let encoded = to_canonical_bytes(&value, limits.input()).map_err(map_canonical)?;
    if encoded.len() > limits.max_snapshot_bytes() {
        return Err(ProtocolError::new(ProtocolErrorKind::SnapshotTooLarge));
    }
    Ok(encoded)
}

pub(crate) fn parse_render(
    value: CanonicalValue,
    limits: &ProtocolLimits,
) -> Result<RenderPayload, ProtocolError> {
    let mut fields = object(value)?;
    let kind = take_string(&mut fields, "kind")?;
    match kind.as_str() {
        "html" => {
            if fields.len() != 1 || !fields.contains_key("html") {
                return Err(ProtocolError::new(ProtocolErrorKind::OutcomeMismatch));
            }
            let html = take_string(&mut fields, "html")?;
            if html.len() > limits.max_html_bytes() {
                return Err(ProtocolError::new(ProtocolErrorKind::InputTooLarge));
            }
            Ok(RenderPayload::Html(html))
        }
        "no_render" if fields.is_empty() => Ok(RenderPayload::NoRender),
        _ => Err(ProtocolError::new(ProtocolErrorKind::OutcomeMismatch)),
    }
}

pub(crate) fn parse_redirect(value: CanonicalValue) -> Result<String, ProtocolError> {
    let CanonicalValue::String(value) = value else {
        return Err(ProtocolError::new(ProtocolErrorKind::UnsafeRedirect));
    };
    let valid = value.starts_with('/')
        && !value.starts_with("//")
        && value.len() <= 2_048
        && !value.contains('\\')
        && !value.chars().any(char::is_control);
    if !valid {
        return Err(ProtocolError::new(ProtocolErrorKind::UnsafeRedirect));
    }
    Ok(value)
}

pub(crate) fn parse_bounded_object(
    value: CanonicalValue,
    max: usize,
) -> Result<BTreeMap<String, CanonicalValue>, ProtocolError> {
    let object = object(value)?;
    if object.len() > max {
        return Err(ProtocolError::new(ProtocolErrorKind::TooManyEntries));
    }
    Ok(object)
}

pub(crate) fn parse_emissions(
    value: CanonicalValue,
    max: usize,
) -> Result<Vec<Emission>, ProtocolError> {
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
            if fields.len() != 2 || !fields.contains_key("name") || !fields.contains_key("payload")
            {
                return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
            }
            let name = BrowserOperationName::parse(&take_string(&mut fields, "name")?)
                .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidIdentity))?;
            let payload = take(&mut fields, "payload")?;
            Ok(Emission { name, payload })
        })
        .collect()
}

pub(crate) fn parse_live_error(value: CanonicalValue) -> Result<LiveError, ProtocolError> {
    let mut fields = object(value)?;
    if fields.len() != 3 {
        return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope));
    }
    let category = parse_category(&take_string(&mut fields, "category")?)?;
    let recovery = parse_recovery(&take_string(&mut fields, "recovery")?)?;
    let detail = parse_detail(&take_string(&mut fields, "detail")?)?;
    Ok(LiveError::new(category, recovery, detail))
}

fn parse_category(value: &str) -> Result<ErrorCategory, ProtocolError> {
    let category = match value {
        "protocol" => ErrorCategory::Protocol,
        "validation" => ErrorCategory::Validation,
        "authentication" => ErrorCategory::Authentication,
        "authorization" => ErrorCategory::Authorization,
        "csrf" => ErrorCategory::Csrf,
        "snapshot" => ErrorCategory::Snapshot,
        "revision" => ErrorCategory::Revision,
        "render" => ErrorCategory::Render,
        "morph" => ErrorCategory::Morph,
        "provider" => ErrorCategory::Provider,
        "cache" => ErrorCategory::Cache,
        "upload" => ErrorCategory::Upload,
        "compatibility" => ErrorCategory::Compatibility,
        "size_limit" => ErrorCategory::SizeLimit,
        "rate_limit" => ErrorCategory::RateLimit,
        "security" => ErrorCategory::Security,
        "internal" => ErrorCategory::Internal,
        _ => return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope)),
    };
    Ok(category)
}

fn parse_recovery(value: &str) -> Result<RecoveryInstruction, ProtocolError> {
    let recovery = match value {
        "retain_dom" => RecoveryInstruction::RetainDom,
        "retry" => RecoveryInstruction::Retry,
        "refresh_island" => RecoveryInstruction::RefreshIsland,
        "remount_island" => RecoveryInstruction::RemountIsland,
        "navigate" => RecoveryInstruction::Navigate,
        "stop" => RecoveryInstruction::Stop,
        _ => return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope)),
    };
    Ok(recovery)
}

fn parse_detail(value: &str) -> Result<SafeDiagnosticCode, ProtocolError> {
    let detail = match value {
        "input_too_large" => SafeDiagnosticCode::InputTooLarge,
        "input_too_deep" => SafeDiagnosticCode::InputTooDeep,
        "too_many_entries" => SafeDiagnosticCode::TooManyEntries,
        "string_too_long" => SafeDiagnosticCode::StringTooLong,
        "duplicate_key" => SafeDiagnosticCode::DuplicateKey,
        "invalid_utf8" => SafeDiagnosticCode::InvalidUtf8,
        "invalid_number" => SafeDiagnosticCode::InvalidNumber,
        "invalid_json" => SafeDiagnosticCode::InvalidJson,
        "serialization_failed" => SafeDiagnosticCode::SerializationFailed,
        "invalid_limit_configuration" => SafeDiagnosticCode::InvalidLimitConfiguration,
        "invalid_identifier" => SafeDiagnosticCode::InvalidIdentifier,
        "invalid_base64_identity" => SafeDiagnosticCode::InvalidBase64Identity,
        "signature_invalid" => SafeDiagnosticCode::SignatureInvalid,
        _ => return Err(ProtocolError::new(ProtocolErrorKind::InvalidEnvelope)),
    };
    Ok(detail)
}

#[allow(
    clippy::too_many_arguments,
    reason = "the function validates mutual exclusion across every response field class"
)]
pub(crate) fn validate_outcome(
    outcome: ResponseOutcome,
    revision: Option<Revision>,
    snapshot: Option<&[u8]>,
    render: Option<&RenderPayload>,
    redirect: Option<&str>,
    validation: &BTreeMap<String, CanonicalValue>,
    events: &[Emission],
    effects: &[Emission],
    error: Option<&LiveError>,
) -> Result<(), ProtocolError> {
    let committed =
        redirect.is_none() && revision.is_some() && snapshot.is_some() && render.is_some();
    let terminal_redirect = redirect.is_some()
        && revision.is_none()
        && snapshot.is_none()
        && render.is_none()
        && validation.is_empty()
        && events.is_empty()
        && effects.is_empty();
    match outcome {
        ResponseOutcome::Accepted | ResponseOutcome::Duplicate => {
            if error.is_none() && (committed || terminal_redirect) {
                Ok(())
            } else {
                Err(ProtocolError::new(ProtocolErrorKind::OutcomeMismatch))
            }
        }
        ResponseOutcome::Rejected => {
            require_nonaccepted_shape(
                revision, snapshot, render, redirect, events, effects, error,
            )?;
            if error.is_some_and(|error| {
                matches!(
                    error.recovery(),
                    RecoveryInstruction::RetainDom | RecoveryInstruction::Retry
                )
            }) {
                Ok(())
            } else {
                Err(ProtocolError::new(ProtocolErrorKind::ErrorRecoveryMismatch))
            }
        }
        ResponseOutcome::RefreshRequired => {
            require_nonaccepted_shape(
                revision, snapshot, render, redirect, events, effects, error,
            )?;
            if !validation.is_empty() {
                return Err(ProtocolError::new(ProtocolErrorKind::OutcomeMismatch));
            }
            if error.is_some_and(|error| {
                matches!(
                    error.recovery(),
                    RecoveryInstruction::RefreshIsland
                        | RecoveryInstruction::RemountIsland
                        | RecoveryInstruction::Navigate
                )
            }) {
                Ok(())
            } else {
                Err(ProtocolError::new(ProtocolErrorKind::ErrorRecoveryMismatch))
            }
        }
        ResponseOutcome::Fatal => {
            require_nonaccepted_shape(
                revision, snapshot, render, redirect, events, effects, error,
            )?;
            if !validation.is_empty() {
                return Err(ProtocolError::new(ProtocolErrorKind::OutcomeMismatch));
            }
            if error.is_some_and(|error| {
                matches!(
                    error.recovery(),
                    RecoveryInstruction::Stop | RecoveryInstruction::Navigate
                )
            }) {
                Ok(())
            } else {
                Err(ProtocolError::new(ProtocolErrorKind::ErrorRecoveryMismatch))
            }
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the helper checks every forbidden state-bearing response class together"
)]
fn require_nonaccepted_shape(
    revision: Option<Revision>,
    snapshot: Option<&[u8]>,
    render: Option<&RenderPayload>,
    redirect: Option<&str>,
    events: &[Emission],
    effects: &[Emission],
    error: Option<&LiveError>,
) -> Result<(), ProtocolError> {
    if revision.is_some()
        || snapshot.is_some()
        || render.is_some()
        || redirect.is_some()
        || !events.is_empty()
        || !effects.is_empty()
        || error.is_none()
    {
        return Err(ProtocolError::new(ProtocolErrorKind::OutcomeMismatch));
    }
    Ok(())
}
