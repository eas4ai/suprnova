//! Typed kernel disposition and complete HTTP response intent.

use bytes::Bytes;
use http::header::{
    ALLOW, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_SECURITY_POLICY, CONTENT_TYPE, REFERRER_POLICY,
    X_CONTENT_TYPE_OPTIONS,
};
use http::{HeaderMap, HeaderValue, StatusCode};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

use crate::action::{ActionOutcome, ActionResult};
use crate::execution::{ExecutionResult, RefreshRequiredExecution};
use crate::identity::{
    BrowserNonce, ComponentName, ContentDigest, CorrelationId, InstanceId, IslandSlot, Revision,
    RouteIdentity, ScopeFingerprint,
};
use crate::protocol::{
    ProtocolLimits, ResponseOutcome, VersionedUpdateRequest, VersionedUpdateResponse,
    encode_versioned_update_response, parse_versioned_update_response,
};
use crate::snapshot::{ComponentContract, MountedDocumentPath};

use super::{EndpointErrorKind, EndpointKernelError, ParsedLiveMediaType};

/// Bounded browser-parity root-relative same-origin navigation target.
#[derive(Clone, Eq, PartialEq)]
pub struct EndpointNavigationTarget(String);

impl EndpointNavigationTarget {
    /// Validates the browser's bounded root-relative same-origin target profile.
    pub fn parse(target: &str) -> Result<Self, EndpointNavigationTargetError> {
        if target.len() > 2_048
            || target
                .chars()
                .any(|character| character == '\\' || character.is_control())
        {
            return Err(EndpointNavigationTargetError);
        }
        let path_end = target.find(['?', '#']).unwrap_or(target.len());
        MountedDocumentPath::parse(&target[..path_end])
            .map_err(|_| EndpointNavigationTargetError)?;
        Ok(Self(target.to_owned()))
    }

    /// Returns the exact validated target bytes.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for EndpointNavigationTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("EndpointNavigationTarget(<validated>)")
    }
}

/// Redacted unsafe-navigation rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EndpointNavigationTargetError;

impl std::fmt::Display for EndpointNavigationTargetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("unsafe_endpoint_navigation_target")
    }
}

impl std::error::Error for EndpointNavigationTargetError {}

/// Host-resolved navigation and URL-reflection facts for one accepted result.
///
/// The engine owns protocol serialization; the host owns resolving registered
/// routes and the current route's query state into same-origin targets.
#[derive(Debug, Default)]
pub struct EndpointResponseIntents {
    redirect: Option<EndpointNavigationTarget>,
    reflected_url: Option<EndpointNavigationTarget>,
}

impl EndpointResponseIntents {
    /// Supplies the same-origin target resolved from a registered route intent.
    #[must_use]
    pub fn with_redirect(mut self, target: EndpointNavigationTarget) -> Self {
        self.redirect = Some(target);
        self
    }

    /// Supplies the same-route target resolved from typed URL-bound state.
    #[must_use]
    pub fn with_reflected_url(mut self, target: EndpointNavigationTarget) -> Self {
        self.reflected_url = Some(target);
        self
    }

    pub(crate) fn is_valid_for(&self, result: &ActionResult, protocol_version: u16) -> bool {
        let redirect_required = matches!(result.outcome(), ActionOutcome::Redirect(_));
        redirect_required == self.redirect.is_some()
            && !(redirect_required && self.reflected_url.is_some())
            && (protocol_version == 2 || self.reflected_url.is_none())
    }
}

const ACCEPTED_REQUEST_BINDING_DOMAIN: &[u8] = b"suprnova-live/accepted-request-binding/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AcceptedRequestBindingDigest([u8; 32]);

/// Opaque verified-request binding carried independently of its one-shot sealer.
///
/// The value can only be obtained from endpoint admission. Passing a binding
/// from another request fails closed during pre-commit response sealing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AcceptedResponseRequestBinding {
    digest: AcceptedRequestBindingDigest,
}

impl AcceptedResponseRequestBinding {
    pub(crate) const fn new(digest: AcceptedRequestBindingDigest) -> Self {
        Self { digest }
    }

    pub(crate) const fn digest(self) -> AcceptedRequestBindingDigest {
        self.digest
    }
}

#[derive(Debug)]
pub(crate) enum AcceptedRequestSnapshotBinding {
    Instance(InstanceId),
    SeedPromotion(BrowserNonce),
}

#[derive(Debug)]
pub(crate) struct AcceptedRequestBinding {
    digest: AcceptedRequestBindingDigest,
    component: ComponentName,
    contract_digest: ContentDigest,
    route: RouteIdentity,
    slot: IslandSlot,
    scope: ScopeFingerprint,
    base_revision: Revision,
    snapshot: AcceptedRequestSnapshotBinding,
}

impl AcceptedRequestBinding {
    #[allow(
        clippy::too_many_arguments,
        reason = "the binding deliberately names every verified request authority dimension"
    )]
    pub(crate) fn new(
        media: ParsedLiveMediaType,
        correlation: &CorrelationId,
        component: ComponentName,
        contract_digest: ContentDigest,
        route: RouteIdentity,
        slot: IslandSlot,
        scope: ScopeFingerprint,
        base_revision: Revision,
        semantic_request_digest: ContentDigest,
        snapshot: AcceptedRequestSnapshotBinding,
    ) -> Self {
        let mut digest = Sha256::new();
        digest.update(ACCEPTED_REQUEST_BINDING_DOMAIN);
        update_binding_part(&mut digest, &media.protocol_version().to_be_bytes());
        update_binding_part(&mut digest, correlation.as_bytes());
        update_binding_part(&mut digest, component.as_str().as_bytes());
        update_binding_part(&mut digest, contract_digest.as_bytes());
        update_binding_part(&mut digest, route.as_bytes());
        update_binding_part(&mut digest, slot.as_str().as_bytes());
        update_binding_part(&mut digest, scope.as_bytes());
        update_binding_part(&mut digest, &base_revision.get().to_be_bytes());
        update_binding_part(&mut digest, semantic_request_digest.as_bytes());
        match &snapshot {
            AcceptedRequestSnapshotBinding::Instance(instance_id) => {
                update_binding_part(&mut digest, b"instance");
                update_binding_part(&mut digest, instance_id.as_bytes());
            }
            AcceptedRequestSnapshotBinding::SeedPromotion(browser_nonce) => {
                update_binding_part(&mut digest, b"seed_promotion");
                update_binding_part(&mut digest, browser_nonce.as_bytes());
            }
        }
        Self {
            digest: AcceptedRequestBindingDigest(digest.finalize().into()),
            component,
            contract_digest,
            route,
            slot,
            scope,
            base_revision,
            snapshot,
        }
    }

    const fn digest(&self) -> AcceptedRequestBindingDigest {
        self.digest
    }

    fn matches(&self, candidate: &AcceptedResponseCandidate<'_>) -> bool {
        let Ok(expected_revision) = self.base_revision.checked_next() else {
            return false;
        };
        candidate.revision == expected_revision
            && candidate.request_binding.digest() == self.digest
            && candidate.authority.component.name() == &self.component
            && candidate.authority.component.contract_digest() == &self.contract_digest
            && candidate.authority.route == &self.route
            && candidate.authority.slot == &self.slot
            && candidate.authority.scope == &self.scope
            && match (&self.snapshot, candidate.authority.snapshot) {
                (
                    AcceptedRequestSnapshotBinding::Instance(expected),
                    AcceptedResponseSnapshotAuthority::Instance(actual),
                ) => expected == actual,
                (
                    AcceptedRequestSnapshotBinding::SeedPromotion(expected),
                    AcceptedResponseSnapshotAuthority::SeedPromotion(actual),
                ) => expected == actual,
                _ => false,
            }
    }
}

fn update_binding_part(digest: &mut Sha256, value: &[u8]) {
    digest.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    digest.update(value);
}

/// Opaque one-shot engine capability derived from one verified endpoint request.
#[derive(Debug)]
pub struct AcceptedResponseSealer {
    media: ParsedLiveMediaType,
    correlation: CorrelationId,
    protocol: ProtocolLimits,
    max_response_bytes: usize,
    binding: AcceptedRequestBinding,
}

impl AcceptedResponseSealer {
    pub(crate) fn new(
        media: ParsedLiveMediaType,
        correlation: CorrelationId,
        protocol: ProtocolLimits,
        max_response_bytes: usize,
        binding: AcceptedRequestBinding,
    ) -> Self {
        Self {
            media,
            correlation,
            protocol,
            max_response_bytes,
            binding,
        }
    }

    /// Returns the admitted whole protocol version.
    #[must_use]
    pub const fn protocol_version(&self) -> u16 {
        self.media.protocol_version()
    }

    pub(crate) fn binding_digest(&self) -> AcceptedRequestBindingDigest {
        self.binding.digest()
    }

    pub(crate) fn seal(
        self,
        candidate: AcceptedResponseCandidate<'_>,
    ) -> Result<SealedAcceptedResponse, EndpointKernelError> {
        if !self.binding.matches(&candidate)
            || !candidate
                .intents
                .is_valid_for(candidate.result, self.protocol_version())
        {
            return Err(EndpointKernelError::unavailable());
        }
        let protocol = self.protocol_version();
        let mut fields = base_fields(protocol, &self.correlation, ResponseOutcome::Accepted);
        if let Some(target) = candidate.intents.redirect.as_ref() {
            if protocol == 1 {
                fields.insert(
                    "redirect".to_owned(),
                    Value::String(target.as_str().to_owned()),
                );
            } else {
                fields.insert(
                    "url_intent".to_owned(),
                    json!({"kind": "navigated", "target": target.as_str()}),
                );
            }
        } else {
            fields.insert(
                "accepted_revision".to_owned(),
                Value::String(candidate.revision.get().to_string()),
            );
            fields.insert(
                "snapshot".to_owned(),
                serde_json::from_slice(candidate.signed_snapshot)
                    .map_err(|_| EndpointKernelError::unavailable())?,
            );
            let render = match candidate.result.outcome() {
                ActionOutcome::Render => {
                    let render = candidate
                        .render
                        .ok_or_else(EndpointKernelError::unavailable)?;
                    let html = std::str::from_utf8(&render.body)
                        .map_err(|_| EndpointKernelError::unavailable())?;
                    json!({"kind": "html", "html": html})
                }
                ActionOutcome::NoRender => json!({"kind": "no_render"}),
                ActionOutcome::Redirect(_) => return Err(EndpointKernelError::unavailable()),
            };
            fields.insert("render".to_owned(), render);
            if protocol == 2 {
                fields.insert(
                    "url_intent".to_owned(),
                    candidate
                        .intents
                        .reflected_url
                        .as_ref()
                        .map(|target| json!({"kind": "reflected", "target": target.as_str()}))
                        .unwrap_or(Value::Null),
                );
            }
        }
        fields.insert(
            "validation".to_owned(),
            validation_json(candidate.validation),
        );
        fields.insert(
            "events".to_owned(),
            emissions_json(candidate.result.metadata().events()),
        );
        fields.insert(
            "effects".to_owned(),
            emissions_json(candidate.result.metadata().effects()),
        );
        if protocol == 2 {
            let child_deliveries = candidate
                .child_deliveries
                .iter()
                .map(|delivery| {
                    Ok(json!({
                        "child_instance": delivery.child_instance().to_base64url(),
                        "envelope": serde_json::from_slice::<Value>(delivery.envelope())
                            .map_err(|_| EndpointKernelError::unavailable())?,
                        "parameter_hash": delivery.parameter_hash().to_base64url(),
                    }))
                })
                .collect::<Result<Vec<_>, EndpointKernelError>>()?;
            fields.insert(
                "child_deliveries".to_owned(),
                Value::Array(child_deliveries),
            );
            fields.entry("url_intent".to_owned()).or_insert(Value::Null);
        } else if !candidate.child_deliveries.is_empty() {
            return Err(EndpointKernelError::unavailable());
        }
        let candidate_body = serde_json_canonicalizer::to_vec(&Value::Object(fields))
            .map_err(|_| EndpointKernelError::unavailable())?;
        if candidate_body.len() > self.max_response_bytes {
            return Err(EndpointKernelError::unavailable());
        }
        let parsed = parse_versioned_update_response(&candidate_body, &self.protocol)
            .map_err(|_| EndpointKernelError::unavailable())?;
        validate_sealed_accepted(&parsed, self.media, &self.correlation)?;
        let encoded = encode_versioned_update_response(&parsed, &self.protocol)
            .map_err(|_| EndpointKernelError::unavailable())?;
        if encoded.len() > self.max_response_bytes {
            return Err(EndpointKernelError::unavailable());
        }
        Ok(SealedAcceptedResponse {
            response: LiveEndpointResponse::complete(
                EndpointOutcomeKind::Accepted.status(),
                Some(self.media),
                Bytes::from(encoded),
                false,
            ),
            request_binding: self.binding.digest(),
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) enum AcceptedResponseSnapshotAuthority<'a> {
    Instance(&'a InstanceId),
    SeedPromotion(&'a BrowserNonce),
}

pub(crate) struct AcceptedResponseAuthority<'a> {
    pub(crate) component: &'a ComponentContract,
    pub(crate) route: &'a RouteIdentity,
    pub(crate) slot: &'a IslandSlot,
    pub(crate) scope: &'a ScopeFingerprint,
    pub(crate) snapshot: AcceptedResponseSnapshotAuthority<'a>,
}

pub(crate) struct AcceptedResponseCandidate<'a> {
    pub(crate) request_binding: AcceptedResponseRequestBinding,
    pub(crate) revision: Revision,
    pub(crate) signed_snapshot: &'a [u8],
    pub(crate) render: Option<&'a crate::view::IslandRender>,
    pub(crate) result: &'a ActionResult,
    pub(crate) intents: &'a EndpointResponseIntents,
    pub(crate) validation: &'a crate::validation::ErrorBag,
    pub(crate) child_deliveries: &'a [crate::protocol::ChildParameterDelivery],
    pub(crate) authority: AcceptedResponseAuthority<'a>,
}

pub(crate) struct SealedAcceptedResponse {
    response: LiveEndpointResponse,
    request_binding: AcceptedRequestBindingDigest,
}

fn validate_sealed_accepted(
    response: &VersionedUpdateResponse,
    media: ParsedLiveMediaType,
    expected_correlation: &CorrelationId,
) -> Result<(), EndpointKernelError> {
    let (version, correlation, outcome) = match response {
        VersionedUpdateResponse::V1(response) => (
            response.protocol_version(),
            response.correlation_id(),
            response.outcome(),
        ),
        VersionedUpdateResponse::V2(response) => (
            response.protocol_version(),
            response.correlation_id(),
            response.outcome(),
        ),
    };
    if version != media.protocol_version()
        || correlation != expected_correlation
        || outcome != ResponseOutcome::Accepted
    {
        return Err(EndpointKernelError::unavailable());
    }
    Ok(())
}

/// Encodes an execution result through the engine-owned protocol state machine.
///
/// Framework adapters prepare host route/session concerns before acceptance;
/// this function only serializes the intents already carried by the result.
pub fn dispatch_execution_result(
    request: &VersionedUpdateRequest,
    result: ExecutionResult,
) -> Result<EndpointDispatch, EndpointKernelError> {
    if let ExecutionResult::Accepted(accepted) = result {
        return Ok(EndpointDispatch::sealed((*accepted).into_sealed_response()));
    }
    let (protocol, correlation) = request_identity(request);
    let (outcome, mut fields) = match result {
        ExecutionResult::Accepted(_) => unreachable!("accepted results return sealed bytes"),
        ExecutionResult::InProgress { .. } => (
            EndpointOutcomeKind::Conflict,
            nonaccepted_fields(
                protocol,
                correlation,
                ResponseOutcome::Rejected,
                "revision",
                "retry",
            ),
        ),
        ExecutionResult::RefreshRequired(refresh) => (
            EndpointOutcomeKind::RefreshRequired,
            refresh_fields(protocol, correlation, &refresh),
        ),
        ExecutionResult::IdempotencyConflict => (
            EndpointOutcomeKind::Conflict,
            nonaccepted_fields(
                protocol,
                correlation,
                ResponseOutcome::Rejected,
                "revision",
                "retain_dom",
            ),
        ),
    };
    if protocol == 2 {
        fields
            .entry("child_deliveries".to_owned())
            .or_insert_with(|| Value::Array(Vec::new()));
        fields.entry("url_intent".to_owned()).or_insert(Value::Null);
    }
    let body = serde_json_canonicalizer::to_vec(&Value::Object(fields))
        .map_err(|_| EndpointKernelError::unavailable())?;
    Ok(EndpointDispatch::new(outcome, Bytes::from(body)))
}

fn request_identity(request: &VersionedUpdateRequest) -> (u16, &crate::identity::CorrelationId) {
    match request {
        VersionedUpdateRequest::V1(request) => (1, request.correlation_id()),
        VersionedUpdateRequest::V2(request) => (2, request.correlation_id()),
    }
}

fn base_fields(
    protocol: u16,
    correlation: &crate::identity::CorrelationId,
    outcome: ResponseOutcome,
) -> Map<String, Value> {
    let mut fields = Map::new();
    fields.insert("protocol_version".to_owned(), Value::from(protocol));
    fields.insert(
        "correlation_id".to_owned(),
        Value::String(correlation.to_base64url()),
    );
    fields.insert(
        "outcome".to_owned(),
        Value::String(
            match outcome {
                ResponseOutcome::Accepted => "accepted",
                ResponseOutcome::Duplicate => "duplicate",
                ResponseOutcome::Rejected => "rejected",
                ResponseOutcome::RefreshRequired => "refresh_required",
                ResponseOutcome::Fatal => "fatal",
            }
            .to_owned(),
        ),
    );
    fields.insert("validation".to_owned(), Value::Object(Map::new()));
    fields.insert("events".to_owned(), Value::Array(Vec::new()));
    fields.insert("effects".to_owned(), Value::Array(Vec::new()));
    fields.insert("extensions".to_owned(), Value::Object(Map::new()));
    fields
}

fn nonaccepted_fields(
    protocol: u16,
    correlation: &crate::identity::CorrelationId,
    outcome: ResponseOutcome,
    category: &str,
    recovery: &str,
) -> Map<String, Value> {
    let mut fields = base_fields(protocol, correlation, outcome);
    fields.insert(
        "error".to_owned(),
        json!({
            "category": category,
            "detail": "signature_invalid",
            "recovery": recovery,
        }),
    );
    fields
}

fn refresh_fields(
    protocol: u16,
    correlation: &crate::identity::CorrelationId,
    _refresh: &RefreshRequiredExecution,
) -> Map<String, Value> {
    nonaccepted_fields(
        protocol,
        correlation,
        ResponseOutcome::RefreshRequired,
        "snapshot",
        "refresh_island",
    )
}

fn validation_json(validation: &crate::validation::ErrorBag) -> Value {
    let mut fields = Map::new();
    for issue in validation.issues() {
        fields.insert(
            issue.path().as_str().to_owned(),
            Value::String(issue.message().as_str().to_owned()),
        );
    }
    Value::Object(fields)
}

fn emissions_json(emissions: &[crate::action::RegisteredEmission]) -> Value {
    Value::Array(
        emissions
            .iter()
            .map(|emission| {
                json!({
                    "name": emission.name().as_str(),
                    "payload": emission.payload(),
                })
            })
            .collect(),
    )
}

/// Closed semantic disposition returned by the endpoint kernel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndpointOutcomeKind {
    /// A new committed outcome is returned.
    Accepted,
    /// A retained prior committed outcome is returned without re-execution.
    Duplicate,
    /// Validation or ordinary request policy rejected the operation.
    Rejected,
    /// Authentication or authorization failed under resource-concealment policy.
    Concealed,
    /// Revision or idempotency authority conflicted with this request.
    Conflict,
    /// Browser authority must fresh-render without replaying the operation.
    RefreshRequired,
    /// Live processing cannot safely continue for the island.
    Fatal,
}

impl EndpointOutcomeKind {
    pub(crate) const fn status(self) -> StatusCode {
        match self {
            Self::Accepted | Self::Duplicate => StatusCode::OK,
            Self::Rejected => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Concealed => StatusCode::NOT_FOUND,
            Self::Conflict | Self::RefreshRequired => StatusCode::CONFLICT,
            Self::Fatal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Complete protocol bytes paired with their closed semantic HTTP disposition.
pub struct EndpointDispatch {
    pub(crate) outcome: EndpointOutcomeKind,
    pub(crate) body: Option<Bytes>,
    pub(crate) sealed_response: Option<SealedAcceptedResponse>,
}

impl EndpointDispatch {
    /// Creates a kernel result pending endpoint validation and canonical re-encoding.
    #[must_use]
    pub const fn new(outcome: EndpointOutcomeKind, body: Bytes) -> Self {
        Self {
            outcome,
            body: Some(body),
            sealed_response: None,
        }
    }

    pub(super) fn sealed(response: SealedAcceptedResponse) -> Self {
        Self {
            outcome: EndpointOutcomeKind::Accepted,
            body: None,
            sealed_response: Some(response),
        }
    }

    pub(crate) fn into_bound_accepted(
        self,
        expected: &AcceptedRequestBindingDigest,
    ) -> Result<LiveEndpointResponse, EndpointKernelError> {
        if self.outcome != EndpointOutcomeKind::Accepted || self.body.is_some() {
            return Err(EndpointKernelError::unavailable());
        }
        let sealed = self
            .sealed_response
            .ok_or_else(EndpointKernelError::unavailable)?;
        if &sealed.request_binding != expected {
            return Err(EndpointKernelError::unavailable());
        }
        Ok(sealed.response)
    }
}

impl std::fmt::Debug for EndpointDispatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EndpointDispatch")
            .field("outcome", &self.outcome)
            .field("body_bytes", &self.body.as_ref().map_or(0, Bytes::len))
            .field("sealed", &self.sealed_response.is_some())
            .finish()
    }
}

/// Complete host-neutral HTTP response intent for the Suprnova adapter.
pub struct LiveEndpointResponse {
    /// Exact status selected by the endpoint's closed mapping.
    pub status: StatusCode,
    /// Endpoint-owned cache, media, length, and bounded security headers.
    pub headers: HeaderMap,
    /// Fully encoded bytes; partial protocol output is never represented.
    pub body: Bytes,
}

impl LiveEndpointResponse {
    /// Builds the endpoint's closed, payload-free HTTP mapping for an admission failure.
    #[must_use]
    pub fn from_error_kind(kind: EndpointErrorKind) -> Self {
        let status = error_status(kind);
        let allow_post = kind == EndpointErrorKind::MethodNotAllowed;
        Self::complete(status, None, Bytes::new(), allow_post)
    }

    pub(crate) fn complete(
        status: StatusCode,
        media: Option<ParsedLiveMediaType>,
        body: Bytes,
        allow_post: bool,
    ) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
        headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
        headers.insert(
            CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
        );
        headers.insert(CONTENT_LENGTH, HeaderValue::from(body.len() as u64));
        if let Some(media) = media {
            headers.insert(
                CONTENT_TYPE,
                HeaderValue::from_static(media.response_value()),
            );
        }
        if allow_post {
            headers.insert(ALLOW, HeaderValue::from_static("POST"));
        }
        Self {
            status,
            headers,
            body,
        }
    }
}

impl std::fmt::Debug for LiveEndpointResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LiveEndpointResponse")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

const fn error_status(kind: EndpointErrorKind) -> StatusCode {
    match kind {
        EndpointErrorKind::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
        EndpointErrorKind::UnsupportedMediaType | EndpointErrorKind::UnsupportedCharset => {
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        }
        EndpointErrorKind::RequestTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        EndpointErrorKind::MalformedProtocol
        | EndpointErrorKind::CacheAttempt
        | EndpointErrorKind::UnsupportedVersion => StatusCode::BAD_REQUEST,
        EndpointErrorKind::MissingContext
        | EndpointErrorKind::ContextInconsistent
        | EndpointErrorKind::RegistryMismatch => StatusCode::NOT_FOUND,
        EndpointErrorKind::ContextExpired | EndpointErrorKind::SnapshotRejected => {
            StatusCode::CONFLICT
        }
        EndpointErrorKind::InvalidKernelResponse
        | EndpointErrorKind::ResponseTooLarge
        | EndpointErrorKind::KernelUnavailable
        | EndpointErrorKind::ClockUnavailable
        | EndpointErrorKind::InvalidConfiguration => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::limits::InputLimits;
    use crate::protocol::ProtocolLimitConfig;
    use crate::validation::ErrorBag;
    use crate::view::{AssetSet, IslandRender};

    struct TestAuthority {
        component: ComponentContract,
        route: RouteIdentity,
        slot: IslandSlot,
        scope: ScopeFingerprint,
        instance: InstanceId,
    }

    fn test_authority() -> TestAuthority {
        let component = ComponentName::parse("tests.response").expect("component");
        let contract_digest = ContentDigest::from_bytes(&[0x20; 32]).expect("contract digest");
        TestAuthority {
            component: ComponentContract::new(component, contract_digest, 1, 1, 1)
                .expect("component contract"),
            route: RouteIdentity::from_bytes(&[0x30; 32]).expect("route"),
            slot: IslandSlot::parse("root").expect("slot"),
            scope: ScopeFingerprint::from_bytes(&[0x40; 32]).expect("scope"),
            instance: InstanceId::from_bytes(&[0x50; 16]).expect("instance"),
        }
    }

    fn candidate_authority(authority: &TestAuthority) -> AcceptedResponseAuthority<'_> {
        AcceptedResponseAuthority {
            component: &authority.component,
            route: &authority.route,
            slot: &authority.slot,
            scope: &authority.scope,
            snapshot: AcceptedResponseSnapshotAuthority::Instance(&authority.instance),
        }
    }

    fn sealer() -> (AcceptedResponseSealer, AcceptedResponseRequestBinding) {
        let protocol = ProtocolLimits::new(ProtocolLimitConfig {
            input: InputLimits::new(64 * 1024, 12, 512, 40 * 1024).expect("input limits"),
            max_snapshot_bytes: 32 * 1024,
            max_html_bytes: 32 * 1024,
            max_model_proposals: 8,
            max_operations: 8,
            max_arguments: 16,
            max_validation_entries: 16,
            max_events: 8,
            max_effects: 8,
            max_extensions: 8,
        })
        .expect("protocol limits");
        let media = ParsedLiveMediaType::parse(super::super::LIVE_MEDIA_TYPE_V1).expect("media");
        let correlation = CorrelationId::from_bytes(&[0x45; 16]).expect("correlation");
        let authority = test_authority();
        let binding = AcceptedRequestBinding::new(
            media,
            &correlation,
            authority.component.name().clone(),
            authority.component.contract_digest().clone(),
            authority.route,
            authority.slot,
            authority.scope,
            Revision::new(0),
            ContentDigest::from_bytes(&[0x46; 32]).expect("semantic request digest"),
            AcceptedRequestSnapshotBinding::Instance(authority.instance),
        );
        let request_binding = AcceptedResponseRequestBinding::new(binding.digest());
        (
            AcceptedResponseSealer::new(media, correlation, protocol, 64 * 1024, binding),
            request_binding,
        )
    }

    #[test]
    fn sealing_rejects_invalid_snapshot_json_before_acceptance() {
        let result = ActionResult::no_render();
        let authority = test_authority();
        let (sealer, request_binding) = sealer();
        assert!(
            sealer
                .seal(AcceptedResponseCandidate {
                    request_binding,
                    revision: Revision::new(1),
                    signed_snapshot: b"{",
                    render: None,
                    result: &result,
                    intents: &EndpointResponseIntents::default(),
                    validation: &ErrorBag::default(),
                    child_deliveries: &[],
                    authority: candidate_authority(&authority),
                })
                .is_err()
        );
    }

    #[test]
    fn sealing_rejects_invalid_render_utf8_before_acceptance() {
        let result = ActionResult::render();
        let render = IslandRender {
            body: Bytes::from_static(&[0xff]),
            assets: AssetSet::default(),
            children: Vec::new(),
        };
        let authority = test_authority();
        let (sealer, request_binding) = sealer();
        assert!(
            sealer
                .seal(AcceptedResponseCandidate {
                    request_binding,
                    revision: Revision::new(1),
                    signed_snapshot: b"{}",
                    render: Some(&render),
                    result: &result,
                    intents: &EndpointResponseIntents::default(),
                    validation: &ErrorBag::default(),
                    child_deliveries: &[],
                    authority: candidate_authority(&authority),
                })
                .is_err()
        );
    }

    #[test]
    fn accepted_dispatch_never_releases_another_verified_requests_sealed_body() {
        let body = Bytes::from_static(b"already-sealed");
        let dispatch = EndpointDispatch::sealed(SealedAcceptedResponse {
            response: LiveEndpointResponse::complete(
                StatusCode::OK,
                Some(ParsedLiveMediaType::parse(super::super::LIVE_MEDIA_TYPE_V1).expect("media")),
                body.clone(),
                false,
            ),
            request_binding: AcceptedRequestBindingDigest([0x11; 32]),
        });
        assert!(dispatch.body.is_none());
        assert!(
            dispatch
                .into_bound_accepted(&AcceptedRequestBindingDigest([0x22; 32]))
                .is_err()
        );
    }
}
