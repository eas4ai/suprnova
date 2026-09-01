//! Registered action results and the Suprnova HTTP update adapter.

pub use suprnova_live::action::{
    ActionOutcome, ActionResult, AuthorizedAction, FlashIntent, OutcomeError, OutcomeErrorKind,
    OutcomeMetadata, RouteIntent, UrlIntent,
};

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use bytes::Bytes;
use sha2::{Digest, Sha256};
use suprnova_live::action::RawActionArguments;
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::endpoint::{
    AcceptedResponseSealer, EndpointErrorKind, EndpointFuture, EndpointKernel, EndpointKernelError,
    LiveEndpointRequest, LiveEndpointResponse, ParsedLiveMediaType, RequestCachePolicy,
    VerifiedEndpointExecutionRequest, VerifiedEndpointRequest, VerifiedEndpointSnapshot,
    dispatch_execution_result,
};
use suprnova_live::execution::{
    ActionExecutionRequest, ExecutionService, ExecutionTracePort, InstancedActionRequest,
    InstancedFreshRenderRequest, InstancedLifecycleOperation, InstancedLifecycleRequest,
    PromotedActionRequest, TransactionPort,
};
use suprnova_live::identity::{
    ActionName, BrowserNonce, BrowserOperationName, ContentDigest, IdempotencyKey, InstanceId,
    ModelField,
};
use suprnova_live::limits::InputLimits;
use suprnova_live::promotion::PromotionService;
use suprnova_live::protocol::{
    BrowserRenderContext, Operation, OperationV2, SemanticIdempotencyInputV1, SnapshotInput,
    VersionedUpdateRequest, semantic_idempotency_digest_v1,
};
use suprnova_live::state::{
    ModelBindingSchema, ModelFieldBinding, ProposalBatch, ProposalLimits, RawModelProposal,
};
use suprnova_live::validation::{BagPolicy, ValidationEngine, ValidationPort};

use crate::{FrameworkError, Request, Response};

/// Builds a same-route URL reflection intent from bounded typed query state.
pub fn url_intent(query: CanonicalValue) -> Result<UrlIntent, ResponseIntentError> {
    UrlIntent::replace_same_route(query, &InputLimits::default())
        .map_err(|_| ResponseIntentError::new(ResponseIntentErrorKind::InvalidPayload))
}

/// Builds an ordinary session flash intent without exposing engine identities.
pub fn flash_intent(key: &str, value: CanonicalValue) -> Result<FlashIntent, ResponseIntentError> {
    let key = BrowserOperationName::parse(key)
        .map_err(|_| ResponseIntentError::new(ResponseIntentErrorKind::InvalidKey))?;
    FlashIntent::new(key, value, &InputLimits::default())
        .map_err(|_| ResponseIntentError::new(ResponseIntentErrorKind::InvalidPayload))
}

/// Validates authored outcome metadata against the generated component contract.
pub fn action_result<C: super::ComponentContract>(
    outcome: ActionOutcome,
    metadata: OutcomeMetadata,
) -> Result<ActionResult, ResponseIntentError> {
    let descriptor = C::__live_registration()
        .map_err(|_| ResponseIntentError::new(ResponseIntentErrorKind::InvalidComponent))?
        .into_engine();
    ActionResult::new(outcome, metadata, &descriptor)
        .map_err(|_| ResponseIntentError::new(ResponseIntentErrorKind::InvalidPayload))
}

/// Stable authored response-intent failure classes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseIntentErrorKind {
    /// A flash key was outside the registered browser-operation profile.
    InvalidKey,
    /// Typed response metadata was incompatible or outside configured bounds.
    InvalidPayload,
    /// Generated component metadata could not be reconstructed.
    InvalidComponent,
}

/// Redacted authored response-intent construction failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResponseIntentError {
    kind: ResponseIntentErrorKind,
}

impl ResponseIntentError {
    const fn new(kind: ResponseIntentErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the stable closed failure category.
    #[must_use]
    pub const fn kind(self) -> ResponseIntentErrorKind {
        self.kind
    }
}

impl fmt::Display for ResponseIntentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ResponseIntentErrorKind::InvalidKey => "invalid_live_response_intent_key",
            ResponseIntentErrorKind::InvalidPayload => "invalid_live_response_intent_payload",
            ResponseIntentErrorKind::InvalidComponent => "invalid_live_response_component",
        })
    }
}

impl Error for ResponseIntentError {}

/// Builds a closed redirect intent from an ordinary registered route name.
///
/// Route lookup and placeholder validation remain owned by Suprnova routing;
/// application code never handles the opaque Live route identity.
pub fn route_intent(
    name: &str,
    parameters: CanonicalValue,
) -> Result<RouteIntent, RouteIntentError> {
    let route = crate::routing::prepare_live_route_identity(name, &parameters)
        .map_err(RouteIntentError::from_resolution)?;
    RouteIntent::new(route, parameters, &InputLimits::default())
        .map_err(|_| RouteIntentError::new(RouteIntentErrorKind::InvalidParameters))
}

/// Stable failure classes for authored registered-route intents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteIntentErrorKind {
    /// The named route was absent or its opaque identity was not unique.
    RouteUnavailable,
    /// Parameters were not a bounded scalar object satisfying route placeholders.
    InvalidParameters,
}

/// Redacted registered-route intent construction failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteIntentError {
    kind: RouteIntentErrorKind,
}

impl RouteIntentError {
    const fn new(kind: RouteIntentErrorKind) -> Self {
        Self { kind }
    }

    fn from_resolution(error: crate::routing::LiveRouteResolutionError) -> Self {
        let kind = match error {
            crate::routing::LiveRouteResolutionError::InvalidParameters => {
                RouteIntentErrorKind::InvalidParameters
            }
            crate::routing::LiveRouteResolutionError::UnknownRoute
            | crate::routing::LiveRouteResolutionError::AmbiguousIdentity
            | crate::routing::LiveRouteResolutionError::InvalidIdentity => {
                RouteIntentErrorKind::RouteUnavailable
            }
        };
        Self::new(kind)
    }

    /// Returns the stable closed failure category.
    #[must_use]
    pub const fn kind(self) -> RouteIntentErrorKind {
        self.kind
    }
}

impl fmt::Display for RouteIntentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            RouteIntentErrorKind::RouteUnavailable => "live_route_intent_unavailable",
            RouteIntentErrorKind::InvalidParameters => "invalid_live_route_parameters",
        })
    }
}

impl Error for RouteIntentError {}

pub(crate) async fn handle(request: Request) -> Response {
    let media = match normalize_media(&request) {
        Ok(media) => media,
        Err(kind) => return error_response(kind),
    };
    let runtime = match super::runtime::LiveRuntime::bind() {
        Ok(runtime) => runtime,
        Err(_) => return error_response(EndpointErrorKind::KernelUnavailable),
    };
    let request = match request
        .buffer_body(runtime.config().max_request_bytes())
        .await
    {
        Ok(request) => request,
        Err(error) if error.status_code() == 413 => {
            return error_response(EndpointErrorKind::RequestTooLarge);
        }
        Err(_) => return error_response(EndpointErrorKind::KernelUnavailable),
    };
    let body = request
        .cached_body()
        .expect("the Live handler just buffered the complete request body");
    let selection = match runtime.inspect_mount(body, media) {
        Ok(selection) => selection,
        Err(error) => return error_response(error.kind()),
    };
    let current_route = selection.route().clone();
    let current_slot = selection.slot().clone();
    let context =
        match runtime.validate_request_context(&request, current_route, current_slot, selection) {
            Ok(context) => context,
            Err(_) => return error_response(EndpointErrorKind::ContextInconsistent),
        };
    let endpoint_request = match LiveEndpointRequest::try_new(
        request.method().clone(),
        media,
        Bytes::copy_from_slice(body),
        Some(context),
        RequestCachePolicy::Bypass,
    ) {
        Ok(request) => request,
        Err(error) => return error_response(error.kind()),
    };
    let (service, completion) = runtime.endpoint_service();
    let response = service.handle(endpoint_request).await;
    let completed = response.status.is_success();
    let projected = project_response(response);
    if !completed || projected.is_err() {
        return projected;
    }
    if completion.commit().is_err() {
        return error_response(EndpointErrorKind::KernelUnavailable);
    }
    projected
}

fn normalize_media(request: &Request) -> Result<ParsedLiveMediaType, EndpointErrorKind> {
    if request.method() != hyper::Method::POST {
        return Err(EndpointErrorKind::MethodNotAllowed);
    }
    let content_type = request
        .header("content-type")
        .ok_or(EndpointErrorKind::UnsupportedMediaType)?;
    ParsedLiveMediaType::parse(content_type).map_err(|error| error.kind())
}

fn error_response(kind: EndpointErrorKind) -> Response {
    project_response(LiveEndpointResponse::from_error_kind(kind))
}

fn project_response(response: LiveEndpointResponse) -> Response {
    Ok(super::response::project(response).map_err(|error| {
        FrameworkError::internal(format!("failed to project Live endpoint response: {error}"))
    })?)
}

pub(crate) struct SuprnovaEndpointKernel {
    promotion: Arc<PromotionService>,
    execution: Arc<ExecutionService>,
    input_limits: InputLimits,
    proposal_limits: ProposalLimits,
    validation_engine: ValidationEngine,
    transaction: Arc<dyn TransactionPort>,
    validation: Arc<dyn ValidationPort>,
    trace: Arc<dyn ExecutionTracePort>,
    response_intents: super::ports::response::RequestResponseIntentPort,
}

impl SuprnovaEndpointKernel {
    pub(crate) fn new(
        promotion: Arc<PromotionService>,
        execution: Arc<ExecutionService>,
        input_limits: InputLimits,
        proposal_limits: ProposalLimits,
        validation_engine: ValidationEngine,
        ports: &super::ports::HostPorts,
        completion: Arc<super::ports::response::PreparedResponseCompletion>,
    ) -> Self {
        Self {
            promotion,
            execution,
            input_limits,
            proposal_limits,
            validation_engine,
            transaction: Arc::clone(&ports.transaction),
            validation: Arc::clone(&ports.validation),
            trace: Arc::clone(&ports.trace),
            response_intents: ports.response.bind(completion),
        }
    }

    async fn dispatch_verified(
        &self,
        request: VerifiedEndpointRequest<'_>,
    ) -> Result<suprnova_live::endpoint::EndpointDispatch, EndpointKernelError> {
        let (request, response_sealer) = request.into_execution_parts();
        let envelope = snapshot_envelope(request.request())?;
        let authority = ContentDigest::from_bytes(&Sha256::digest(envelope))
            .map_err(|_| EndpointKernelError::unavailable())?;
        let expected_key = request.context().mount().document_key().clone();
        let browser = BrowserRenderContext::from_request(request.request(), &expected_key)
            .map_err(|_| EndpointKernelError::context_inconsistent())?;

        let operation = requested_operation(request.request())?;
        let result = match request.snapshot() {
            VerifiedEndpointSnapshot::Instance(snapshot) => {
                let digest =
                    request_digest(&request, snapshot.body().instance_id().clone(), authority)?;
                self.dispatch_instanced(
                    &request,
                    snapshot,
                    browser,
                    operation,
                    digest,
                    response_sealer,
                )
                .await?
            }
            VerifiedEndpointSnapshot::Seed(_) => {
                let (encoded_seed, browser_nonce) = seed_input(request.request())?;
                let promoted = self
                    .promotion
                    .promote(
                        encoded_seed,
                        browser_nonce.clone(),
                        &request.context().for_promotion(),
                    )
                    .await
                    .map_err(|_| EndpointKernelError::unavailable())?;
                let digest = request_digest(&request, promoted.instance_id().clone(), authority)?;
                self.dispatch_promoted(
                    &request,
                    browser,
                    operation,
                    promoted,
                    browser_nonce.clone(),
                    digest,
                    response_sealer,
                )
                .await?
            }
        };
        dispatch_execution_result(request.request(), result)
    }

    async fn dispatch_instanced(
        &self,
        request: &VerifiedEndpointExecutionRequest<'_>,
        snapshot: &suprnova_live::snapshot::VerifiedInstanceV1,
        browser: BrowserRenderContext,
        operation: RequestedOperation<'_>,
        digest: ContentDigest,
        response_sealer: AcceptedResponseSealer,
    ) -> Result<suprnova_live::execution::ExecutionResult, EndpointKernelError> {
        Ok(match operation {
            RequestedOperation::Action {
                name,
                arguments,
                synchronized,
                proposals,
            } => {
                let proposal_batch =
                    self.prepare_proposals(request.descriptor(), &synchronized, proposals)?;
                let action = ActionExecutionRequest::new(
                    name,
                    RawActionArguments::new(CanonicalValue::Object(arguments.clone())),
                    &self.input_limits,
                    &self.validation_engine,
                    self.validation.as_ref(),
                    BagPolicy::Replace,
                    Some(self.transaction.as_ref()),
                    self.trace.as_ref(),
                )
                .with_response_intent_preparation(&self.response_intents)
                .with_response_sealer(response_sealer, request.response_binding());
                let action = match proposal_batch.as_ref() {
                    Some(proposals) => action.with_proposals(proposals),
                    None => action,
                };
                self.execution
                    .execute_instanced(InstancedActionRequest::new(
                        request.descriptor(),
                        request.context(),
                        browser,
                        snapshot,
                        idempotency_key(request.request()).clone(),
                        digest,
                        action,
                    ))
                    .await
            }
            RequestedOperation::FreshRender => {
                self.execution
                    .execute_fresh_render(
                        InstancedFreshRenderRequest::new(
                            request.descriptor(),
                            request.context(),
                            browser,
                            snapshot,
                            idempotency_key(request.request()).clone(),
                            digest,
                            self.trace.as_ref(),
                        )
                        .with_response_sealer(response_sealer, request.response_binding()),
                    )
                    .await
            }
            RequestedOperation::ModelSync {
                synchronized,
                proposals,
            } => {
                let proposal_batch = self
                    .prepare_proposals(request.descriptor(), &synchronized, proposals)?
                    .ok_or_else(EndpointKernelError::unavailable)?;
                self.execution
                    .execute_lifecycle(
                        InstancedLifecycleRequest::new(
                            request.descriptor(),
                            request.context(),
                            browser,
                            snapshot,
                            idempotency_key(request.request()).clone(),
                            digest,
                            InstancedLifecycleOperation::SyncModels(&proposal_batch),
                            self.trace.as_ref(),
                        )
                        .with_response_sealer(response_sealer, request.response_binding()),
                    )
                    .await
            }
            RequestedOperation::LazyComplete => {
                self.execution
                    .execute_lifecycle(
                        InstancedLifecycleRequest::new(
                            request.descriptor(),
                            request.context(),
                            browser,
                            snapshot,
                            idempotency_key(request.request()).clone(),
                            digest,
                            InstancedLifecycleOperation::LazyComplete,
                            self.trace.as_ref(),
                        )
                        .with_response_sealer(response_sealer, request.response_binding()),
                    )
                    .await
            }
        })
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the helper keeps every independently verified promotion and response-sealing authority explicit"
    )]
    async fn dispatch_promoted(
        &self,
        request: &VerifiedEndpointExecutionRequest<'_>,
        browser: BrowserRenderContext,
        operation: RequestedOperation<'_>,
        promoted: suprnova_live::promotion::PromotedInstance,
        browser_nonce: BrowserNonce,
        digest: ContentDigest,
        response_sealer: AcceptedResponseSealer,
    ) -> Result<suprnova_live::execution::ExecutionResult, EndpointKernelError> {
        let RequestedOperation::Action {
            name,
            arguments,
            synchronized,
            proposals,
        } = operation
        else {
            return Err(EndpointKernelError::unavailable());
        };
        let proposal_batch =
            self.prepare_proposals(request.descriptor(), &synchronized, proposals)?;
        let action = ActionExecutionRequest::new(
            name,
            RawActionArguments::new(CanonicalValue::Object(arguments.clone())),
            &self.input_limits,
            &self.validation_engine,
            self.validation.as_ref(),
            BagPolicy::Replace,
            Some(self.transaction.as_ref()),
            self.trace.as_ref(),
        )
        .with_response_intent_preparation(&self.response_intents)
        .with_response_sealer(response_sealer, request.response_binding());
        let action = match proposal_batch.as_ref() {
            Some(proposals) => action.with_proposals(proposals),
            None => action,
        };
        Ok(self
            .execution
            .execute_promoted(PromotedActionRequest::new(
                request.descriptor(),
                request.context(),
                browser,
                promoted,
                browser_nonce,
                idempotency_key(request.request()).clone(),
                digest,
                action,
            ))
            .await)
    }

    fn prepare_proposals(
        &self,
        descriptor: &suprnova_live::registry::ComponentDescriptor,
        synchronized: &[&ModelField],
        proposals: &std::collections::BTreeMap<ModelField, CanonicalValue>,
    ) -> Result<Option<ProposalBatch>, EndpointKernelError> {
        if synchronized.is_empty() {
            return Ok(None);
        }
        let schema = ModelBindingSchema::new(
            descriptor
                .metadata()
                .fields()
                .iter()
                .filter_map(|field| {
                    field.model_codec().map(|codec| {
                        ModelFieldBinding::new(
                            field.name().as_str(),
                            field.category(),
                            codec.clone(),
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| EndpointKernelError::unavailable())?,
        )
        .map_err(|_| EndpointKernelError::unavailable())?;
        let raw = synchronized
            .iter()
            .map(|field| {
                proposals
                    .get(*field)
                    .cloned()
                    .map(|value| RawModelProposal::new(field.as_str(), value))
                    .ok_or_else(EndpointKernelError::unavailable)
            })
            .collect::<Result<Vec<_>, _>>()?;
        ProposalBatch::prepare(&schema, raw, &self.proposal_limits)
            .map(Some)
            .map_err(|_| EndpointKernelError::unavailable())
    }
}

impl EndpointKernel for SuprnovaEndpointKernel {
    fn dispatch<'request>(
        &'request self,
        request: VerifiedEndpointRequest<'request>,
    ) -> EndpointFuture<'request> {
        Box::pin(async move { self.dispatch_verified(request).await })
    }
}

enum RequestedOperation<'request> {
    Action {
        name: &'request ActionName,
        arguments: &'request std::collections::BTreeMap<String, CanonicalValue>,
        synchronized: Vec<&'request ModelField>,
        proposals: &'request std::collections::BTreeMap<ModelField, CanonicalValue>,
    },
    ModelSync {
        synchronized: Vec<&'request ModelField>,
        proposals: &'request std::collections::BTreeMap<ModelField, CanonicalValue>,
    },
    FreshRender,
    LazyComplete,
}

fn requested_operation(
    request: &VersionedUpdateRequest,
) -> Result<RequestedOperation<'_>, EndpointKernelError> {
    match request {
        VersionedUpdateRequest::V1(request) => {
            let mut synchronized = Vec::new();
            let mut action = None;
            for operation in request.operations() {
                match operation {
                    Operation::SyncModel { field } => synchronized.push(field),
                    Operation::InvokeAction { name, arguments } => action = Some((name, arguments)),
                }
            }
            match action {
                Some((name, arguments)) => Ok(RequestedOperation::Action {
                    name,
                    arguments,
                    synchronized,
                    proposals: request.model_proposals(),
                }),
                None if !synchronized.is_empty() => Ok(RequestedOperation::ModelSync {
                    synchronized,
                    proposals: request.model_proposals(),
                }),
                None => Err(EndpointKernelError::unavailable()),
            }
        }
        VersionedUpdateRequest::V2(request) => {
            if request.operations() == [OperationV2::FreshRender] {
                return Ok(RequestedOperation::FreshRender);
            }
            if request.operations() == [OperationV2::LazyComplete] {
                return Ok(RequestedOperation::LazyComplete);
            }
            let mut synchronized = Vec::new();
            let mut action = None;
            for operation in request.operations() {
                match operation {
                    OperationV2::SyncModel { field } => synchronized.push(field),
                    OperationV2::InvokeAction { name, arguments } => {
                        action = Some((name, arguments));
                    }
                    OperationV2::ParamsChanged
                    | OperationV2::LazyComplete
                    | OperationV2::FreshRender => {
                        return Err(EndpointKernelError::unavailable());
                    }
                }
            }
            match action {
                Some((name, arguments)) => Ok(RequestedOperation::Action {
                    name,
                    arguments,
                    synchronized,
                    proposals: request.model_proposals(),
                }),
                None if !synchronized.is_empty() => Ok(RequestedOperation::ModelSync {
                    synchronized,
                    proposals: request.model_proposals(),
                }),
                None => Err(EndpointKernelError::unavailable()),
            }
        }
    }
}

fn snapshot_envelope(request: &VersionedUpdateRequest) -> Result<&[u8], EndpointKernelError> {
    let snapshot = match request {
        VersionedUpdateRequest::V1(request) => request.snapshot(),
        VersionedUpdateRequest::V2(request) => request.snapshot(),
    };
    match snapshot {
        SnapshotInput::Instance { envelope } | SnapshotInput::SeedPromotion { envelope, .. } => {
            Ok(envelope)
        }
    }
}

fn seed_input(
    request: &VersionedUpdateRequest,
) -> Result<(&[u8], &BrowserNonce), EndpointKernelError> {
    let snapshot = match request {
        VersionedUpdateRequest::V1(request) => request.snapshot(),
        VersionedUpdateRequest::V2(request) => request.snapshot(),
    };
    match snapshot {
        SnapshotInput::SeedPromotion {
            envelope,
            browser_nonce,
        } => Ok((envelope, browser_nonce)),
        SnapshotInput::Instance { .. } => Err(EndpointKernelError::unavailable()),
    }
}

fn request_digest(
    request: &VerifiedEndpointExecutionRequest<'_>,
    instance_id: InstanceId,
    authority: ContentDigest,
) -> Result<ContentDigest, EndpointKernelError> {
    semantic_idempotency_digest_v1(&SemanticIdempotencyInputV1::new(
        request.context().scope().clone(),
        instance_id,
        request.descriptor().contract_digest().clone(),
        authority,
        request.request(),
    ))
    .map_err(|_| EndpointKernelError::unavailable())
}

fn idempotency_key(request: &VersionedUpdateRequest) -> &IdempotencyKey {
    match request {
        VersionedUpdateRequest::V1(request) => request.idempotency_key(),
        VersionedUpdateRequest::V2(request) => request.idempotency_key(),
    }
}
