//! Ordered transport admission, authority verification, dispatch, and encoding.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use http::header::{
    ALLOW, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_SECURITY_POLICY, CONTENT_TYPE, REFERRER_POLICY,
    X_CONTENT_TYPE_OPTIONS,
};
use http::{HeaderMap, HeaderValue, Method, StatusCode};

use crate::clock::Clock;
use crate::crypto::SnapshotKeyRing;
use crate::host::TrustedLiveRequestContext;
use crate::identity::{ComponentName, CorrelationId, InstanceId, Revision};
use crate::protocol::{
    ResponseOutcome, SnapshotInput, VersionedUpdateRequest, VersionedUpdateResponse,
    encode_versioned_update_response, parse_versioned_update_request,
    parse_versioned_update_response,
};
use crate::registry::{ComponentDescriptor, ComponentRegistry};
use crate::snapshot::{
    ExpectedInstanceV1, VerifiedInstanceV1, VerifiedSeedV1, verify_instance, verify_seed,
};

use super::{
    EndpointDispatch, EndpointError, EndpointErrorKind, EndpointKernelError, EndpointOutcomeKind,
    LiveEndpointConfig, LiveEndpointRequest, LiveEndpointResponse, ParsedLiveMediaType,
};

/// Boxed host-neutral kernel future without an async-trait dependency.
pub type EndpointFuture<'request> =
    Pin<Box<dyn Future<Output = Result<EndpointDispatch, EndpointKernelError>> + Send + 'request>>;

/// Application-facing kernel invoked only after transport and signed authority validation.
pub trait EndpointKernel: Send + Sync {
    /// Dispatches one verified request capability into complete protocol bytes.
    fn dispatch<'request>(
        &'request self,
        request: VerifiedEndpointRequest<'request>,
    ) -> EndpointFuture<'request>;
}

/// Cryptographically verified snapshot form admitted to application execution.
pub enum VerifiedEndpointSnapshot {
    /// Ordinary scoped instance authority.
    Instance(VerifiedInstanceV1),
    /// Reusable public seed authority pending atomic promotion by the kernel.
    Seed(VerifiedSeedV1),
}

impl std::fmt::Debug for VerifiedEndpointSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Instance(_) => "VerifiedEndpointSnapshot::Instance(<redacted>)",
            Self::Seed(_) => "VerifiedEndpointSnapshot::Seed(<redacted>)",
        })
    }
}

/// Capability carrying one parsed request and trusted descriptor after all endpoint preflight.
pub struct VerifiedEndpointRequest<'request> {
    request: VersionedUpdateRequest,
    snapshot: VerifiedEndpointSnapshot,
    descriptor: &'request ComponentDescriptor,
    context: &'request TrustedLiveRequestContext,
}

impl<'request> VerifiedEndpointRequest<'request> {
    /// Returns the trusted registered component identity.
    #[must_use]
    pub const fn component(&self) -> &ComponentName {
        self.descriptor.metadata().identity()
    }

    /// Returns the fully parsed version-specific request.
    #[must_use]
    pub const fn request(&self) -> &VersionedUpdateRequest {
        &self.request
    }

    /// Returns the verified signed snapshot capability.
    #[must_use]
    pub const fn snapshot(&self) -> &VerifiedEndpointSnapshot {
        &self.snapshot
    }

    /// Returns current trusted host capabilities and scope facts.
    #[must_use]
    pub const fn context(&self) -> &TrustedLiveRequestContext {
        self.context
    }
}

impl std::fmt::Debug for VerifiedEndpointRequest<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedEndpointRequest")
            .field("component", &self.component().as_str())
            .field("request", &"<protocol:redacted>")
            .field("snapshot", &self.snapshot)
            .field("context", &"<trusted:redacted>")
            .finish()
    }
}

/// Host-neutral endpoint service; framework adapters only translate its typed input and output.
pub struct LiveEndpointService {
    config: LiveEndpointConfig,
    registry: Arc<ComponentRegistry>,
    clock: Arc<dyn Clock>,
    keys: Arc<SnapshotKeyRing>,
    kernel: Arc<dyn EndpointKernel>,
}

impl LiveEndpointService {
    /// Creates a service from explicit immutable registry, time, key, and kernel providers.
    #[must_use]
    pub fn new(
        config: LiveEndpointConfig,
        registry: Arc<ComponentRegistry>,
        clock: Arc<dyn Clock>,
        keys: Arc<SnapshotKeyRing>,
        kernel: Arc<dyn EndpointKernel>,
    ) -> Self {
        Self {
            config,
            registry,
            clock,
            keys,
            kernel,
        }
    }

    /// Performs the complete ordered endpoint operation and always returns HTTP intent.
    pub async fn handle(&self, request: LiveEndpointRequest) -> LiveEndpointResponse {
        match self.try_handle(request).await {
            Ok(response) => response,
            Err(error) => self.error_response(error),
        }
    }

    /// Converts a normalization failure into the endpoint's closed HTTP mapping.
    #[must_use]
    pub fn error_response(&self, error: EndpointError) -> LiveEndpointResponse {
        let status = error_status(error.kind());
        let allow_post = error.kind() == EndpointErrorKind::MethodNotAllowed;
        build_response(status, None, Bytes::new(), allow_post)
    }

    async fn try_handle(
        &self,
        request: LiveEndpointRequest,
    ) -> Result<LiveEndpointResponse, EndpointError> {
        if request.method != Method::POST {
            return Err(EndpointError::new(EndpointErrorKind::MethodNotAllowed));
        }
        if request.body.len() > self.config.max_request_bytes() {
            return Err(EndpointError::new(EndpointErrorKind::RequestTooLarge));
        }
        let now = self
            .clock
            .now()
            .map_err(|_| EndpointError::new(EndpointErrorKind::ClockUnavailable))?;
        if !request.context.is_current(now) {
            return Err(EndpointError::new(EndpointErrorKind::ContextExpired));
        }

        let parsed = parse_versioned_update_request(&request.body, self.config.protocol())
            .map_err(|_| EndpointError::new(EndpointErrorKind::MalformedProtocol))?;
        let identity = request_identity(&parsed);
        if identity.protocol_version != request.content_type.protocol_version()
            || identity.protocol_version != request.context.mount().protocol()
            || identity.protocol_version < request.context.mount().minimum_protocol()
        {
            return Err(EndpointError::new(EndpointErrorKind::UnsupportedVersion));
        }
        if identity.component != request.context.mount().component() {
            return Err(EndpointError::new(EndpointErrorKind::ContextInconsistent));
        }
        let base_revision = identity.base_revision;
        let descriptor = self
            .registry
            .require_contract(
                request.context.mount().component(),
                request.context.mount().contract_digest(),
            )
            .map_err(|_| EndpointError::new(EndpointErrorKind::RegistryMismatch))?;

        let snapshot =
            self.verify_snapshot(&request, identity.snapshot, identity.base_revision, now)?;
        let expected_instance_id = match &snapshot {
            VerifiedEndpointSnapshot::Instance(snapshot) => {
                Some(snapshot.body().instance_id().clone())
            }
            VerifiedEndpointSnapshot::Seed(_) => None,
        };
        let expected_correlation = identity.correlation_id.clone();
        let verified = VerifiedEndpointRequest {
            request: parsed,
            snapshot,
            descriptor,
            context: &request.context,
        };
        let dispatch = self
            .kernel
            .dispatch(verified)
            .await
            .map_err(|_| EndpointError::new(EndpointErrorKind::KernelUnavailable))?;
        let completed_at = self
            .clock
            .now()
            .map_err(|_| EndpointError::new(EndpointErrorKind::ClockUnavailable))?;
        if completed_at < now {
            return Err(EndpointError::new(EndpointErrorKind::ClockUnavailable));
        }
        if !request.context.is_current(completed_at) {
            return Err(EndpointError::new(EndpointErrorKind::ContextExpired));
        }
        if dispatch.outcome == EndpointOutcomeKind::Concealed {
            return Ok(build_response(
                dispatch.outcome.status(),
                None,
                Bytes::new(),
                false,
            ));
        }
        if dispatch.outcome == EndpointOutcomeKind::Duplicate && dispatch.body.is_empty() {
            return Err(EndpointError::new(EndpointErrorKind::SnapshotRejected));
        }
        if dispatch.body.len() > self.config.max_response_bytes() {
            return Err(EndpointError::new(EndpointErrorKind::ResponseTooLarge));
        }
        let response = parse_versioned_update_response(&dispatch.body, self.config.protocol())
            .map_err(|_| EndpointError::new(EndpointErrorKind::InvalidKernelResponse))?;
        validate_kernel_response(
            &response,
            dispatch.outcome,
            request.content_type,
            &expected_correlation,
        )?;
        self.validate_response_snapshot(
            &request,
            &response,
            expected_instance_id.as_ref(),
            base_revision,
            completed_at,
        )?;
        let encoded = encode_versioned_update_response(&response, self.config.protocol())
            .map_err(|_| EndpointError::new(EndpointErrorKind::InvalidKernelResponse))?;
        if encoded.len() > self.config.max_response_bytes() {
            return Err(EndpointError::new(EndpointErrorKind::ResponseTooLarge));
        }
        Ok(build_response(
            dispatch.outcome.status(),
            Some(request.content_type),
            Bytes::from(encoded),
            false,
        ))
    }

    fn verify_snapshot(
        &self,
        request: &LiveEndpointRequest,
        snapshot: &SnapshotInput,
        base_revision: Revision,
        now: crate::identity::UnixMillis,
    ) -> Result<VerifiedEndpointSnapshot, EndpointError> {
        let expected_seed = request.context.mount().expected_seed();
        match snapshot {
            SnapshotInput::Instance { envelope } => {
                let expected = ExpectedInstanceV1::new(
                    expected_seed.component.clone(),
                    expected_seed.build_id.clone(),
                    expected_seed.route.clone(),
                    expected_seed.slot.clone(),
                    request.context.scope().clone(),
                    expected_seed.schemas.clone(),
                );
                let verified =
                    verify_instance(envelope, &expected, &self.keys, now, self.config.snapshot())
                        .map_err(|_| EndpointError::new(EndpointErrorKind::SnapshotRejected))?;
                if verified.body().revision() != base_revision {
                    return Err(EndpointError::new(EndpointErrorKind::SnapshotRejected));
                }
                Ok(VerifiedEndpointSnapshot::Instance(verified))
            }
            SnapshotInput::SeedPromotion { envelope, .. } => verify_seed(
                envelope,
                expected_seed,
                &self.keys,
                now,
                self.config.snapshot(),
            )
            .map(VerifiedEndpointSnapshot::Seed)
            .map_err(|_| EndpointError::new(EndpointErrorKind::SnapshotRejected)),
        }
    }

    fn validate_response_snapshot(
        &self,
        request: &LiveEndpointRequest,
        response: &VersionedUpdateResponse,
        expected_instance_id: Option<&InstanceId>,
        base_revision: Revision,
        now: crate::identity::UnixMillis,
    ) -> Result<(), EndpointError> {
        let (revision, snapshot) = match response {
            VersionedUpdateResponse::V1(response) => {
                (response.accepted_revision(), response.snapshot())
            }
            VersionedUpdateResponse::V2(response) => {
                (response.accepted_revision(), response.snapshot())
            }
        };
        let Some(snapshot) = snapshot else {
            return Ok(());
        };
        let expected_seed = request.context.mount().expected_seed();
        let expected = ExpectedInstanceV1::new(
            expected_seed.component.clone(),
            expected_seed.build_id.clone(),
            expected_seed.route.clone(),
            expected_seed.slot.clone(),
            request.context.scope().clone(),
            expected_seed.schemas.clone(),
        );
        let verified =
            verify_instance(snapshot, &expected, &self.keys, now, self.config.snapshot())
                .map_err(|_| EndpointError::new(EndpointErrorKind::InvalidKernelResponse))?;
        let expected_revision = base_revision
            .checked_next()
            .map_err(|_| EndpointError::new(EndpointErrorKind::InvalidKernelResponse))?;
        if revision != Some(expected_revision)
            || verified.body().revision() != expected_revision
            || expected_instance_id
                .is_some_and(|instance_id| instance_id != verified.body().instance_id())
        {
            return Err(EndpointError::new(EndpointErrorKind::InvalidKernelResponse));
        }
        Ok(())
    }
}

struct RequestIdentity<'request> {
    protocol_version: u16,
    correlation_id: &'request CorrelationId,
    component: &'request ComponentName,
    base_revision: Revision,
    snapshot: &'request SnapshotInput,
}

const fn request_identity(request: &VersionedUpdateRequest) -> RequestIdentity<'_> {
    match request {
        VersionedUpdateRequest::V1(request) => RequestIdentity {
            protocol_version: request.protocol_version(),
            correlation_id: request.correlation_id(),
            component: request.component(),
            base_revision: request.base_revision(),
            snapshot: request.snapshot(),
        },
        VersionedUpdateRequest::V2(request) => RequestIdentity {
            protocol_version: request.protocol_version(),
            correlation_id: request.correlation_id(),
            component: request.component(),
            base_revision: request.base_revision(),
            snapshot: request.snapshot(),
        },
    }
}

fn validate_kernel_response(
    response: &VersionedUpdateResponse,
    endpoint_outcome: EndpointOutcomeKind,
    media: ParsedLiveMediaType,
    expected_correlation: &CorrelationId,
) -> Result<(), EndpointError> {
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
    let class_matches = match endpoint_outcome {
        EndpointOutcomeKind::Accepted => outcome == ResponseOutcome::Accepted,
        EndpointOutcomeKind::Duplicate => outcome == ResponseOutcome::Duplicate,
        EndpointOutcomeKind::Rejected | EndpointOutcomeKind::Conflict => {
            outcome == ResponseOutcome::Rejected
        }
        EndpointOutcomeKind::Concealed => false,
        EndpointOutcomeKind::RefreshRequired => outcome == ResponseOutcome::RefreshRequired,
        EndpointOutcomeKind::Fatal => outcome == ResponseOutcome::Fatal,
    };
    if version != media.protocol_version() || correlation != expected_correlation || !class_matches
    {
        return Err(EndpointError::new(EndpointErrorKind::InvalidKernelResponse));
    }
    Ok(())
}

fn build_response(
    status: StatusCode,
    media: Option<ParsedLiveMediaType>,
    body: Bytes,
    allow_post: bool,
) -> LiveEndpointResponse {
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
    LiveEndpointResponse {
        status,
        headers,
        body,
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
