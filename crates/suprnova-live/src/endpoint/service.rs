//! Ordered transport admission, authority verification, dispatch, and encoding.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use http::Method;

use crate::clock::Clock;
use crate::crypto::SnapshotKeyRing;
use crate::host::TrustedLiveRequestContext;
use crate::identity::{ComponentName, CorrelationId, InstanceId, Revision};
use crate::protocol::{
    ResponseOutcome, SnapshotInput, VersionedUpdateRequest, VersionedUpdateResponse,
    encode_versioned_update_response, parse_versioned_update_request,
    parse_versioned_update_response, semantic_request_digest_v1,
};
use crate::registry::{ComponentDescriptor, ComponentRegistry};
use crate::snapshot::{
    ExpectedInstanceV1, VerifiedInstanceV1, VerifiedSeedV1, verify_instance, verify_seed,
};

use super::{
    AcceptedRequestBinding, AcceptedRequestSnapshotBinding, AcceptedResponseRequestBinding,
    AcceptedResponseSealer, EndpointDispatch, EndpointError, EndpointErrorKind,
    EndpointKernelError, EndpointOutcomeKind, LiveEndpointConfig, LiveEndpointRequest,
    LiveEndpointResponse, ParsedLiveMediaType,
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
    response_sealer: AcceptedResponseSealer,
}

impl<'request> VerifiedEndpointRequest<'request> {
    /// Consumes endpoint admission into a reusable execution view and one-shot sealer.
    #[must_use]
    pub fn into_execution_parts(
        self,
    ) -> (
        VerifiedEndpointExecutionRequest<'request>,
        AcceptedResponseSealer,
    ) {
        let response_binding =
            AcceptedResponseRequestBinding::new(self.response_sealer.binding_digest());
        (
            VerifiedEndpointExecutionRequest {
                request: self.request,
                snapshot: self.snapshot,
                descriptor: self.descriptor,
                context: self.context,
                response_binding,
            },
            self.response_sealer,
        )
    }
}

/// Verified endpoint facts retained after consuming the one-shot sealing capability.
pub struct VerifiedEndpointExecutionRequest<'request> {
    request: VersionedUpdateRequest,
    snapshot: VerifiedEndpointSnapshot,
    descriptor: &'request ComponentDescriptor,
    context: &'request TrustedLiveRequestContext,
    response_binding: AcceptedResponseRequestBinding,
}

impl<'request> VerifiedEndpointExecutionRequest<'request> {
    /// Returns the exact immutable descriptor selected by endpoint admission.
    #[must_use]
    pub const fn descriptor(&self) -> &ComponentDescriptor {
        self.descriptor
    }

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

    /// Returns the opaque binding for this exact verified endpoint request.
    #[must_use]
    pub const fn response_binding(&self) -> AcceptedResponseRequestBinding {
        self.response_binding
    }
}

impl std::fmt::Debug for VerifiedEndpointRequest<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedEndpointRequest")
            .field("component", &self.descriptor.metadata().identity().as_str())
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
        LiveEndpointResponse::from_error_kind(error.kind())
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
        let semantic_request_digest = semantic_request_digest_v1(&parsed)
            .map_err(|_| EndpointError::new(EndpointErrorKind::MalformedProtocol))?;
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
        let snapshot_binding = match (&snapshot, identity.snapshot) {
            (VerifiedEndpointSnapshot::Instance(snapshot), SnapshotInput::Instance { .. }) => {
                AcceptedRequestSnapshotBinding::Instance(snapshot.body().instance_id().clone())
            }
            (
                VerifiedEndpointSnapshot::Seed(_),
                SnapshotInput::SeedPromotion { browser_nonce, .. },
            ) => AcceptedRequestSnapshotBinding::SeedPromotion(browser_nonce.clone()),
            _ => return Err(EndpointError::new(EndpointErrorKind::ContextInconsistent)),
        };
        let binding = AcceptedRequestBinding::new(
            request.content_type,
            &expected_correlation,
            descriptor.metadata().identity().clone(),
            descriptor.contract_digest().clone(),
            request.context.mount().route().clone(),
            request.context.mount().slot().clone(),
            request.context.scope().clone(),
            base_revision,
            semantic_request_digest,
            snapshot_binding,
        );
        let response_sealer = AcceptedResponseSealer::new(
            request.content_type,
            expected_correlation.clone(),
            self.config.protocol().clone(),
            self.config.max_response_bytes(),
            binding,
        );
        let expected_request_binding = response_sealer.binding_digest();
        let verified = VerifiedEndpointRequest {
            request: parsed,
            snapshot,
            descriptor,
            context: &request.context,
            response_sealer,
        };
        let dispatch = self
            .kernel
            .dispatch(verified)
            .await
            .map_err(|error| EndpointError::new(error.kind()))?;
        if dispatch.outcome == EndpointOutcomeKind::Accepted {
            return dispatch
                .into_bound_accepted(&expected_request_binding)
                .map_err(|_| EndpointError::new(EndpointErrorKind::InvalidKernelResponse));
        }
        if dispatch.sealed_response.is_some() {
            return Err(EndpointError::new(EndpointErrorKind::InvalidKernelResponse));
        }
        let body = dispatch
            .body
            .ok_or_else(|| EndpointError::new(EndpointErrorKind::InvalidKernelResponse))?;
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
            return Ok(LiveEndpointResponse::complete(
                dispatch.outcome.status(),
                None,
                Bytes::new(),
                false,
            ));
        }
        if dispatch.outcome == EndpointOutcomeKind::Duplicate && body.is_empty() {
            return Err(EndpointError::new(EndpointErrorKind::SnapshotRejected));
        }
        if body.len() > self.config.max_response_bytes() {
            return Err(EndpointError::new(EndpointErrorKind::ResponseTooLarge));
        }
        let response = parse_versioned_update_response(&body, self.config.protocol())
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
        Ok(LiveEndpointResponse::complete(
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use http::{Method, StatusCode};

    use super::*;
    use crate::action::ActionResult;
    use crate::canonical::CanonicalValue;
    use crate::clock::ClockError;
    use crate::crypto::{KeyRecord, RootKey};
    use crate::endpoint::{
        AcceptedResponseAuthority, AcceptedResponseCandidate, AcceptedResponseSnapshotAuthority,
        EndpointResponseIntents, LIVE_MEDIA_TYPE_V1, RequestCachePolicy,
    };
    use crate::host::{
        CheckDisposition, CheckFact, CheckKind, HostCapabilities, HostCheckFacts, HostScopeFacts,
        LiveRequestContextCandidate, LiveRequestContextValidator, MountCatalogBuilder,
        MountCatalogEntry, MountScopeRequirements, MountSelection, PrincipalFingerprint,
        ScopeRequirement, SessionFingerprint, TenantFingerprint,
    };
    use crate::identity::{
        BuildId, InstanceId, IslandSlot, KeyId, ModelField, RouteIdentity, ScopeFingerprint,
        UnixMillis, ViewName,
    };
    use crate::limits::InputLimits;
    use crate::metadata::{ComponentMetadata, ContractVersions, FieldMetadata};
    use crate::protocol::{ProtocolLimitConfig, ProtocolLimits};
    use crate::registry::ComponentRegistryBuilder;
    use crate::snapshot::state::{FieldCategory, FieldSpec, StateCodec, StateSchema};
    use crate::snapshot::{
        ComponentContract, ExpectedSeedV1, InstanceBodyV1, InstanceFieldsV1, SnapshotLimits,
        SnapshotSchemaSet,
    };
    use crate::validation::ErrorBag;
    use crate::view::{AssetSet, IslandRender};

    struct FixedClock;

    impl Clock for FixedClock {
        fn now(&self) -> Result<UnixMillis, ClockError> {
            Ok(UnixMillis::new(1_200))
        }
    }

    struct ReplayAcceptedKernel {
        calls: AtomicUsize,
        replay: Mutex<Option<EndpointDispatch>>,
    }

    impl EndpointKernel for ReplayAcceptedKernel {
        fn dispatch<'request>(
            &'request self,
            request: VerifiedEndpointRequest<'request>,
        ) -> EndpointFuture<'request> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                let (request, sealer) = request.into_execution_parts();
                let request_binding = request.response_binding();
                let VerifiedEndpointSnapshot::Instance(snapshot) = request.snapshot() else {
                    return Box::pin(async { Err(EndpointKernelError::unavailable()) });
                };
                let body = snapshot.body();
                let result = ActionResult::render();
                let render = IslandRender {
                    body: Bytes::from_static(b"<div>private-request-a</div>"),
                    assets: AssetSet::default(),
                    children: Vec::new(),
                };
                let intents = EndpointResponseIntents::default();
                let validation = ErrorBag::default();
                let sealed = sealer.seal(AcceptedResponseCandidate {
                    request_binding,
                    revision: body.revision().checked_next().expect("successor revision"),
                    signed_snapshot: br#"{"private":"snapshot-a"}"#,
                    render: Some(&render),
                    result: &result,
                    intents: &intents,
                    validation: &validation,
                    authority: AcceptedResponseAuthority {
                        component: body.component(),
                        route: body.route(),
                        slot: body.slot(),
                        scope: body.scope(),
                        snapshot: AcceptedResponseSnapshotAuthority::Instance(body.instance_id()),
                    },
                });
                let stored = sealed
                    .and_then(|sealed| {
                        self.replay
                            .lock()
                            .map_err(|_| EndpointKernelError::unavailable())?
                            .replace(EndpointDispatch::sealed(sealed));
                        Ok(())
                    })
                    .is_ok();
                return Box::pin(async move {
                    if !stored {
                        return Err(EndpointKernelError::unavailable());
                    }
                    Ok(EndpointDispatch::new(
                        EndpointOutcomeKind::Concealed,
                        Bytes::new(),
                    ))
                });
            }

            let _ = request.into_execution_parts();
            let replay = self.replay.lock().ok().and_then(|mut replay| replay.take());
            Box::pin(async move { replay.ok_or_else(EndpointKernelError::unavailable) })
        }
    }

    #[tokio::test]
    async fn accepted_response_from_another_verified_request_is_never_exposed() {
        let descriptor = replay_descriptor();
        let registry = Arc::new(
            ComponentRegistryBuilder::new()
                .register(descriptor.clone())
                .expect("replay descriptor")
                .build(),
        );
        let keys = Arc::new(replay_keys());
        let (context_a, snapshot_a) =
            replay_authority(&registry, &descriptor, &keys, 0x30, 0x40, 0x50);
        let (context_b, snapshot_b) =
            replay_authority(&registry, &descriptor, &keys, 0x31, 0x60, 0x70);
        let kernel = Arc::new(ReplayAcceptedKernel {
            calls: AtomicUsize::new(0),
            replay: Mutex::new(None),
        });
        let service = LiveEndpointService::new(
            LiveEndpointConfig::new(replay_protocol_limits(), replay_snapshot_limits())
                .expect("endpoint config"),
            registry,
            Arc::new(FixedClock),
            keys,
            kernel,
        );

        let captured = service
            .handle(replay_request(context_a, snapshot_a, 0x10))
            .await;
        assert_eq!(captured.status, StatusCode::NOT_FOUND);
        assert!(captured.body.is_empty());

        let replayed = service
            .handle(replay_request(context_b, snapshot_b, 0x20))
            .await;
        assert_eq!(replayed.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(replayed.body.is_empty());
        assert_eq!(replayed.headers[http::header::CONTENT_LENGTH], "0");
    }

    #[tokio::test]
    async fn accepted_response_from_another_semantic_request_is_never_exposed() {
        let descriptor = replay_descriptor();
        let registry = Arc::new(
            ComponentRegistryBuilder::new()
                .register(descriptor.clone())
                .expect("replay descriptor")
                .build(),
        );
        let keys = Arc::new(replay_keys());
        let (context_a, snapshot_a) =
            replay_authority(&registry, &descriptor, &keys, 0x30, 0x40, 0x50);
        let (context_b, snapshot_b) =
            replay_authority(&registry, &descriptor, &keys, 0x30, 0x40, 0x50);
        let kernel = Arc::new(ReplayAcceptedKernel {
            calls: AtomicUsize::new(0),
            replay: Mutex::new(None),
        });
        let service = LiveEndpointService::new(
            LiveEndpointConfig::new(replay_protocol_limits(), replay_snapshot_limits())
                .expect("endpoint config"),
            registry,
            Arc::new(FixedClock),
            keys,
            kernel,
        );

        let captured = service
            .handle(replay_request_with_semantics(
                context_a,
                snapshot_a,
                0x10,
                0x11,
                "execute",
                "request-a",
                "model-a",
            ))
            .await;
        assert_eq!(captured.status, StatusCode::NOT_FOUND);
        assert!(captured.body.is_empty());

        let replayed = service
            .handle(replay_request_with_semantics(
                context_b,
                snapshot_b,
                0x10,
                0x12,
                "cancel",
                "request-b",
                "model-b",
            ))
            .await;
        assert_eq!(replayed.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(replayed.body.is_empty());
        assert_eq!(replayed.headers[http::header::CONTENT_LENGTH], "0");
    }

    fn replay_descriptor() -> ComponentDescriptor {
        let metadata = ComponentMetadata::new(
            ComponentName::parse("tests.endpoint-replay").expect("component"),
            ViewName::parse("tests/endpoint-replay.html").expect("view"),
            ContractVersions::new(1, 1, 1, 1, 1).expect("versions"),
            vec![FieldMetadata::new(
                ModelField::parse("serial").expect("field"),
                FieldCategory::State,
                StateCodec::Json,
                true,
            )],
            Vec::new(),
        )
        .expect("metadata");
        ComponentDescriptor::new(metadata)
    }

    fn replay_authority(
        registry: &ComponentRegistry,
        descriptor: &ComponentDescriptor,
        keys: &SnapshotKeyRing,
        route_start: u8,
        scope_start: u8,
        instance_start: u8,
    ) -> (TrustedLiveRequestContext, Vec<u8>) {
        let route = RouteIdentity::from_bytes(&test_bytes::<32>(route_start)).expect("route");
        let slot = IslandSlot::parse("root").expect("slot");
        let scope = ScopeFingerprint::from_bytes(&test_bytes::<32>(scope_start)).expect("scope");
        let contract = ComponentContract::new(
            descriptor.metadata().identity().clone(),
            descriptor.contract_digest().clone(),
            1,
            1,
            1,
        )
        .expect("component contract");
        let build = BuildId::parse("build-endpoint-replay").expect("build");
        let schemas = replay_schemas();
        let catalog = MountCatalogBuilder::new()
            .register(
                registry,
                MountCatalogEntry::new(
                    ExpectedSeedV1::new(
                        contract.clone(),
                        build.clone(),
                        route.clone(),
                        slot.clone(),
                        schemas.clone(),
                    ),
                    MountScopeRequirements::new(
                        ScopeRequirement::Required,
                        ScopeRequirement::Required,
                        ScopeRequirement::Required,
                    ),
                ),
            )
            .expect("mount catalog")
            .build();
        let facts = HostScopeFacts::new(
            scope.clone(),
            Some(
                SessionFingerprint::from_bytes(&test_bytes::<32>(scope_start.wrapping_add(1)))
                    .expect("session"),
            ),
            Some(
                PrincipalFingerprint::from_bytes(&test_bytes::<32>(scope_start.wrapping_add(2)))
                    .expect("principal"),
            ),
            Some(
                TenantFingerprint::from_bytes(&test_bytes::<32>(scope_start.wrapping_add(3)))
                    .expect("tenant"),
            ),
        );
        let mut checks = HostCheckFacts::new();
        for kind in CheckKind::ALL {
            checks
                .record(
                    kind,
                    CheckFact::new(CheckDisposition::Passed, UnixMillis::new(1_900)),
                )
                .expect("host check");
        }
        let context = LiveRequestContextValidator::new(300_000)
            .expect("context validator")
            .validate(
                &catalog,
                LiveRequestContextCandidate::new(
                    route.clone(),
                    slot.clone(),
                    MountSelection::new(
                        route.clone(),
                        slot.clone(),
                        descriptor.metadata().identity().clone(),
                        descriptor.contract_digest().clone(),
                        1,
                    ),
                    facts.clone(),
                    checks,
                    HostCapabilities::bound_to(facts),
                    UnixMillis::new(1_900),
                ),
                UnixMillis::new(1_000),
            )
            .expect("trusted context");
        let snapshot = InstanceBodyV1::new(
            InstanceFieldsV1 {
                component: contract,
                build_id: build,
                route,
                slot,
                key_id: keys.active_key_id().clone(),
                scope,
                instance_id: InstanceId::from_bytes(&test_bytes::<16>(instance_start))
                    .expect("instance"),
                revision: Revision::new(0),
                issued_at: UnixMillis::new(1_000),
                expires_at: UnixMillis::new(1_800),
                state: CanonicalValue::Object(BTreeMap::from([(
                    "serial".to_owned(),
                    CanonicalValue::String("1".to_owned()),
                )])),
                memo: CanonicalValue::Object(BTreeMap::new()),
                extensions: BTreeMap::new(),
            },
            &schemas,
            &replay_snapshot_limits(),
        )
        .expect("instance body")
        .sign(keys, UnixMillis::new(1_000), &replay_snapshot_limits())
        .expect("signed instance");
        (context, snapshot)
    }

    fn replay_request(
        context: TrustedLiveRequestContext,
        snapshot: Vec<u8>,
        correlation_start: u8,
    ) -> LiveEndpointRequest {
        replay_request_with_semantics(
            context,
            snapshot,
            correlation_start,
            correlation_start.wrapping_add(1),
            "execute",
            "default",
            "default",
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the hostile fixture names each independently varied semantic request field"
    )]
    fn replay_request_with_semantics(
        context: TrustedLiveRequestContext,
        snapshot: Vec<u8>,
        correlation_start: u8,
        idempotency_start: u8,
        action: &str,
        argument: &str,
        model_proposal: &str,
    ) -> LiveEndpointRequest {
        let snapshot: serde_json::Value = serde_json::from_slice(&snapshot).expect("snapshot JSON");
        let correlation =
            CorrelationId::from_bytes(&test_bytes::<16>(correlation_start)).expect("correlation");
        let idempotency =
            CorrelationId::from_bytes(&test_bytes::<16>(idempotency_start)).expect("idempotency");
        let body = serde_json::to_vec(&serde_json::json!({
            "base_revision": "0",
            "component": "tests.endpoint-replay",
            "correlation_id": correlation.to_base64url(),
            "extensions": {},
            "idempotency_key": idempotency.to_base64url(),
            "model_proposals": {"serial": model_proposal},
            "operations": [{"arguments": {"value": argument}, "kind": "invoke_action", "name": action}],
            "protocol_version": 1,
            "runtime_contract_version": 1,
            "snapshot": {"envelope": snapshot, "kind": "instance"},
            "snapshot_schema_version": 1,
        }))
        .expect("request JSON");
        LiveEndpointRequest::try_new(
            Method::POST,
            ParsedLiveMediaType::parse(LIVE_MEDIA_TYPE_V1).expect("media"),
            Bytes::from(body),
            Some(context),
            RequestCachePolicy::Bypass,
        )
        .expect("endpoint request")
    }

    fn replay_protocol_limits() -> ProtocolLimits {
        ProtocolLimits::new(ProtocolLimitConfig {
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
        .expect("protocol limits")
    }

    fn replay_snapshot_limits() -> SnapshotLimits {
        SnapshotLimits::new(
            InputLimits::new(4_096, 4, 64, 512).expect("snapshot input limits"),
            50,
            10_000,
            20_000,
            8,
            8,
        )
        .expect("snapshot limits")
    }

    fn replay_schemas() -> SnapshotSchemaSet {
        SnapshotSchemaSet::new(
            StateSchema::new(
                1,
                vec![
                    FieldSpec::new("serial", StateCodec::Json, FieldCategory::State, true)
                        .expect("state field"),
                ],
            )
            .expect("state schema"),
            StateSchema::new(1, Vec::new()).expect("memo schema"),
            StateSchema::new(1, Vec::new()).expect("mount schema"),
        )
        .expect("schemas")
    }

    fn replay_keys() -> SnapshotKeyRing {
        SnapshotKeyRing::new(
            KeyRecord::new(
                KeyId::parse("endpoint-replay-v1").expect("key id"),
                RootKey::new(vec![0x42; 32]).expect("root key"),
                UnixMillis::new(0),
                UnixMillis::new(10_000),
                UnixMillis::new(20_000),
            )
            .expect("key record"),
            Vec::new(),
        )
        .expect("key ring")
    }

    fn test_bytes<const LENGTH: usize>(start: u8) -> [u8; LENGTH] {
        std::array::from_fn(|index| start.wrapping_add(index as u8))
    }
}
