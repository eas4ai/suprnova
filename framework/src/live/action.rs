//! Registered action results and the Suprnova HTTP update adapter.

pub use suprnova_live::action::{
    ActionOutcome, ActionResult, AuthorizedAction, FlashIntent, OutcomeError, OutcomeErrorKind,
    OutcomeMetadata, RouteIntent, UrlIntent,
};

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use sha2::{Digest, Sha256};
use suprnova_live::action::RawActionArguments;
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::child::ChildParameterEligibilityErrorKind;
use suprnova_live::clock::Clock;
use suprnova_live::endpoint::{
    AcceptedResponseSealer, EndpointError, EndpointErrorKind, EndpointFuture, EndpointKernel,
    EndpointKernelError, LiveEndpointRequest, LiveEndpointResponse, ParsedLiveMediaType,
    RequestCachePolicy, VerifiedEndpointExecutionRequest, VerifiedEndpointRequest,
    VerifiedEndpointSnapshot, dispatch_execution_result,
};
use suprnova_live::execution::{
    ActionExecutionRequest, ExecutionResult, ExecutionService, ExecutionTracePort,
    InstancedActionRequest, InstancedFreshRenderRequest, InstancedLifecycleOperation,
    InstancedLifecycleRequest, PromotedActionRequest, PromotedModelSyncRequest,
    PromotedRequestIdentity, TransactionPort,
};
use suprnova_live::identity::{
    ActionName, BrowserNonce, BrowserOperationName, ContentDigest, IdempotencyKey, InstanceId,
    ModelField,
};
use suprnova_live::ledger::AcceptedOutcomeKind;
use suprnova_live::limits::InputLimits;
use suprnova_live::promotion::PromotionService;
use suprnova_live::protocol::{
    BrowserRenderContext, Operation, OperationV2, SemanticIdempotencyInputV1, SnapshotInput,
    VersionedUpdateRequest, semantic_idempotency_digest_v1,
};
use suprnova_live::state::{
    ModelBindingSchema, ModelFieldBinding, ProposalBatch, ProposalLimits, RawModelProposal,
};
use suprnova_live::upload::{
    FinalizeUploadRequest, ReadyUploadProposal, UploadErrorKind, UploadFieldPolicy,
    UploadFinalizationService, UploadHandle, UploadIdempotencyKey,
};
use suprnova_live::validation::{BagPolicy, ValidationEngine, ValidationPort};

use super::LiveLimitExceeded;
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
    /// The named route was absent, or two different route patterns shared
    /// its opaque identity. Several names on one pattern are one route.
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
        Err(error) => return failure_response(EndpointErrorKind::KernelUnavailable, &error),
    };
    let max_request_bytes = runtime.config().max_request_bytes();
    let declared_bytes = request
        .header("content-length")
        .and_then(|value| value.trim().parse::<u64>().ok());
    let mut request = match request.buffer_body(max_request_bytes).await {
        Ok(request) => request,
        Err(error) if error.status_code() == 413 => {
            // A declared length is the real size; a streamed body stops at the
            // limit, so all that is known is that it went past it.
            let breach = match declared_bytes {
                Some(bytes) if bytes > max_request_bytes as u64 => {
                    LiveLimitExceeded::request_bytes(bytes, max_request_bytes as u64, false)
                }
                _ => LiveLimitExceeded::request_bytes(
                    max_request_bytes as u64 + 1,
                    max_request_bytes as u64,
                    true,
                ),
            };
            tracing::warn!(limit = %breach, "Live request was refused");
            return failure_response(EndpointErrorKind::RequestTooLarge, &breach);
        }
        Err(error) => return failure_response(EndpointErrorKind::KernelUnavailable, &error),
    };
    // A second handle on the buffered body, not a copy: the request is
    // mutated below to close the identity absences its mount permits, after
    // the body has named that mount, and that touches only its headers. A
    // copy held a second 16 MiB per request at the default limit.
    let body = request
        .cached_body()
        .cloned()
        .expect("the Live handler just buffered the complete request body");
    let selection = match runtime.inspect_mount(&body, media) {
        Ok(selection) => selection,
        Err(error) => return endpoint_failure(&error),
    };
    let upload_context = match runtime.validate_upload_action_context(&request, &selection) {
        Ok(context) => context,
        Err(_) => return error_response(EndpointErrorKind::ContextInconsistent),
    };
    if runtime
        .close_mount_scope_absences(&mut request, &selection)
        .is_err()
    {
        return error_response(EndpointErrorKind::ContextInconsistent);
    }
    let context = match runtime.validate_request_context(&request, selection) {
        Ok(context) => context,
        Err(_) => return error_response(EndpointErrorKind::ContextInconsistent),
    };
    let endpoint_request = match LiveEndpointRequest::try_new(
        request.method().clone(),
        media,
        body,
        Some(context),
        RequestCachePolicy::Bypass,
    ) {
        Ok(request) => request,
        Err(error) => return failure_response(error.kind(), &error),
    };
    let (service, completion) = runtime.endpoint_service(upload_context);
    // Session-only fields the request's components staged reach the
    // session only once the outcome is accepted, below.
    let ((response, failure), staged_session) =
        super::session_state::scope(service.handle_reported(endpoint_request)).await;
    let completed = response.status.is_success();
    let projected = project_response(response);
    if let Some(breach) = failure.and_then(|error| size_breach(&error)) {
        tracing::warn!(limit = %breach, "Live response was refused");
        return match projected {
            Ok(response) => Ok(response.with_error_report_from(&breach)),
            Err(response) => Err(response.with_error_report_from(&breach)),
        };
    }
    if !completed || projected.is_err() {
        return projected;
    }
    if let Err(error) = completion.commit() {
        return failure_response(EndpointErrorKind::KernelUnavailable, &error);
    }
    super::session_state::commit(staged_session);
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

/// The configured limit an endpoint failure went over, named by its setting.
fn size_breach(error: &EndpointError) -> Option<LiveLimitExceeded> {
    let size = error.size()?;
    let (measured, configured) = (size.measured as u64, size.configured as u64);
    match error.kind() {
        EndpointErrorKind::RequestTooLarge => Some(LiveLimitExceeded::request_bytes(
            measured,
            configured,
            size.at_least,
        )),
        EndpointErrorKind::ResponseTooLarge => Some(LiveLimitExceeded::response_bytes(
            measured,
            configured,
            size.at_least,
        )),
        _ => None,
    }
}

/// [`failure_response`] for an endpoint error, reporting a size breach by the
/// setting it went over instead of a bare kind.
fn endpoint_failure(error: &EndpointError) -> Response {
    match size_breach(error) {
        Some(breach) => {
            tracing::warn!(limit = %breach, "Live request was refused");
            failure_response(error.kind(), &breach)
        }
        None => failure_response(error.kind(), error),
    }
}

fn error_response(kind: EndpointErrorKind) -> Response {
    project_response(LiveEndpointResponse::from_error_kind(kind))
}

/// [`error_response`] for a failure with its error in hand: the client gets
/// the same closed answer, and the response carries `error` as its
/// in-process report.
fn failure_response(kind: EndpointErrorKind, error: &dyn Error) -> Response {
    match error_response(kind) {
        Ok(response) => Ok(response.with_error_report_from(error)),
        Err(response) => Err(response.with_error_report_from(error)),
    }
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
    clock: Arc<dyn Clock>,
    upload_finalization: Arc<UploadFinalizationService>,
    upload_operation_locks: Arc<super::upload::UploadOperationLocks>,
    upload_provider: Arc<super::ports::upload_provider::SuprnovaUploadProviderRouter>,
    upload_context: Option<suprnova_live::host::TrustedLiveRequestContext>,
}

struct PreparedProposalBatch {
    batch: ProposalBatch,
    uploads: Vec<PreparedUploadFinalization>,
    authorized_action: Option<AuthorizedAction>,
    _operation_guards: Vec<tokio::sync::OwnedMutexGuard<()>>,
}

struct PreparedUploadFinalization {
    field: ModelField,
    policy: UploadFieldPolicy,
    proposal: ReadyUploadProposal,
}

struct UploadProposalCandidate {
    field: ModelField,
    policy: UploadFieldPolicy,
    handles: Vec<UploadHandle>,
}

impl SuprnovaEndpointKernel {
    #[allow(
        clippy::too_many_arguments,
        reason = "the endpoint kernel receives each immutable authority and request-bound response capability explicitly"
    )]
    pub(crate) fn new(
        promotion: Arc<PromotionService>,
        execution: Arc<ExecutionService>,
        input_limits: InputLimits,
        proposal_limits: ProposalLimits,
        validation_engine: ValidationEngine,
        ports: &super::ports::HostPorts,
        clock: Arc<dyn Clock>,
        upload_finalization: Arc<UploadFinalizationService>,
        upload_context: Option<suprnova_live::host::TrustedLiveRequestContext>,
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
            clock,
            upload_finalization,
            upload_operation_locks: Arc::clone(&ports.uploads.operation_locks),
            upload_provider: Arc::clone(&ports.uploads.provider_adapter),
            upload_context,
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
        // The browser is only told to refresh the island, so the closed cause of a failed
        // operation is recorded here for an operator. A refresh without a cause, such as a
        // stale revision or a duplicate, is an ordinary outcome and is not a fault.
        if let ExecutionResult::RefreshRequired(refresh) = &result
            && let Some(cause) = refresh.cause()
        {
            // A render over the island HTML limit also names the setting to
            // raise, with both sizes; every other cause stays closed.
            let limit = match cause {
                suprnova_live::execution::ExecutionFailure::ViewTooLarge(size) => {
                    Some(LiveLimitExceeded::html_bytes(
                        size.measured as u64,
                        size.configured as u64,
                        size.at_least,
                    ))
                }
                _ => None,
            };
            match limit {
                Some(limit) => tracing::warn!(
                    component = request.component().as_str(),
                    reason = ?refresh.reason(),
                    cause = ?cause,
                    limit = %limit,
                    "Live operation failed and the browser must refresh the island"
                ),
                None => tracing::warn!(
                    component = request.component().as_str(),
                    reason = ?refresh.reason(),
                    cause = ?cause,
                    "Live operation failed and the browser must refresh the island"
                ),
            }
        }
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
                let proposal_batch = self
                    .prepare_proposals(request.descriptor(), Some(name), &synchronized, proposals)
                    .await?;
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
                    Some(proposals) => action.with_proposals(&proposals.batch),
                    None => action,
                };
                let result = self
                    .execution
                    .execute_instanced(InstancedActionRequest::new(
                        request.descriptor(),
                        request.context(),
                        browser,
                        snapshot,
                        idempotency_key(request.request()).clone(),
                        digest,
                        action,
                    ))
                    .await;
                if let Some(proposals) = proposal_batch.as_ref() {
                    self.finalize_upload_proposals(
                        &result,
                        proposals,
                        idempotency_key(request.request()),
                    )
                    .await?;
                }
                result
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
                    .prepare_proposals(request.descriptor(), None, &synchronized, proposals)
                    .await?
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
                            InstancedLifecycleOperation::SyncModels(&proposal_batch.batch),
                            self.trace.as_ref(),
                        )
                        .with_response_sealer(response_sealer, request.response_binding()),
                    )
                    .await
            }
            RequestedOperation::ParamsChanged => {
                let admission = request
                    .child_admission()
                    .ok_or_else(EndpointKernelError::unavailable)?;
                let eligible = self
                    .execution
                    .authorize_child_parameters_v2(
                        admission.parameters(),
                        admission.parent_snapshot(),
                    )
                    .await
                    .map_err(|error| child_eligibility_kernel_error(error.kind()))?;
                self.execution
                    .execute_lifecycle(
                        InstancedLifecycleRequest::new(
                            request.descriptor(),
                            request.context(),
                            browser,
                            snapshot,
                            idempotency_key(request.request()).clone(),
                            digest,
                            InstancedLifecycleOperation::ParamsChanged(&eligible),
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
        let identity = PromotedRequestIdentity::new(
            browser_nonce,
            idempotency_key(request.request()).clone(),
            digest,
        );
        let (name, arguments, synchronized, proposals) = match operation {
            RequestedOperation::Action {
                name,
                arguments,
                synchronized,
                proposals,
            } => (name, arguments, synchronized, proposals),
            // The browser runtime sends an immediate `live:model` edit on a
            // public seed as a model sync, so it promotes the seed too. It needs
            // no upload finalization: `prepare_proposals` refuses an upload
            // handle unless the request names the field's finalize action.
            RequestedOperation::ModelSync {
                synchronized,
                proposals,
            } => {
                let proposal_batch = self
                    .prepare_proposals(request.descriptor(), None, &synchronized, proposals)
                    .await?
                    .ok_or_else(EndpointKernelError::unavailable)?;
                return Ok(self
                    .execution
                    .execute_promoted_model_sync(
                        PromotedModelSyncRequest::new(
                            request.descriptor(),
                            request.context(),
                            browser,
                            promoted,
                            identity,
                            &proposal_batch.batch,
                            self.trace.as_ref(),
                        )
                        .with_response_sealer(response_sealer, request.response_binding()),
                    )
                    .await);
            }
            RequestedOperation::ParamsChanged
            | RequestedOperation::FreshRender
            | RequestedOperation::LazyComplete => return Err(EndpointKernelError::unavailable()),
        };
        let proposal_batch = self
            .prepare_proposals(request.descriptor(), Some(name), &synchronized, proposals)
            .await?;
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
            Some(proposals) => action.with_proposals(&proposals.batch),
            None => action,
        };
        let result = self
            .execution
            .execute_promoted(PromotedActionRequest::new(
                request.descriptor(),
                request.context(),
                browser,
                promoted,
                identity,
                action,
            ))
            .await;
        if let Some(proposals) = proposal_batch.as_ref() {
            self.finalize_upload_proposals(&result, proposals, idempotency_key(request.request()))
                .await?;
        }
        Ok(result)
    }

    async fn prepare_proposals(
        &self,
        descriptor: &suprnova_live::registry::ComponentDescriptor,
        action: Option<&ActionName>,
        synchronized: &[&ModelField],
        proposals: &std::collections::BTreeMap<ModelField, CanonicalValue>,
    ) -> Result<Option<PreparedProposalBatch>, EndpointKernelError> {
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
        let mut raw = Vec::with_capacity(synchronized.len());
        let mut upload_candidates = Vec::new();
        for field in synchronized {
            let value = proposals
                .get(*field)
                .cloned()
                .ok_or_else(EndpointKernelError::unavailable)?;
            if let Some(policy) = descriptor
                .metadata()
                .fields()
                .iter()
                .find(|candidate| candidate.name() == *field)
                .and_then(|candidate| candidate.upload_policy())
            {
                let handles = upload_proposal_handles(&value, policy.maximum_files())?;
                if !handles.is_empty() {
                    if action != Some(policy.finalize_action()) {
                        return Err(EndpointKernelError::context_inconsistent());
                    }
                    upload_candidates.push(UploadProposalCandidate {
                        field: (*field).clone(),
                        policy: policy.clone(),
                        handles,
                    });
                }
            }
            raw.push(RawModelProposal::new(field.as_str(), value));
        }
        let batch = ProposalBatch::prepare(&schema, raw, &self.proposal_limits)
            .map_err(|_| EndpointKernelError::unavailable())?;
        if upload_candidates.is_empty() {
            return Ok(Some(PreparedProposalBatch {
                batch,
                uploads: Vec::new(),
                authorized_action: None,
                _operation_guards: Vec::new(),
            }));
        }
        let context = self
            .upload_context
            .as_ref()
            .ok_or_else(EndpointKernelError::context_inconsistent)?;
        let action = action.expect("upload candidates require the declared finalize action");
        let mut ordered_handles = upload_candidates
            .iter()
            .flat_map(|candidate| candidate.handles.iter().cloned())
            .collect::<Vec<_>>();
        ordered_handles.sort_unstable_by_key(ToString::to_string);
        if ordered_handles.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(EndpointKernelError::context_inconsistent());
        }
        let mut operation_guards = Vec::with_capacity(ordered_handles.len());
        for handle in &ordered_handles {
            operation_guards.push(self.upload_operation_locks.acquire(handle).await);
        }
        let authorized_action = descriptor
            .actions()
            .authorize(
                descriptor.metadata().identity(),
                context.capabilities(),
                action,
            )
            .await
            .map_err(|_| EndpointKernelError::context_inconsistent())?;
        let now = self
            .clock
            .now()
            .map_err(|_| EndpointKernelError::unavailable())?;
        let mut uploads = Vec::with_capacity(ordered_handles.len());
        for candidate in upload_candidates {
            for handle in candidate.handles {
                let proposal = self
                    .upload_finalization
                    .authorize_ready_proposal(
                        context,
                        candidate.field.clone(),
                        handle,
                        action,
                        &candidate.policy,
                        now,
                    )
                    .await
                    .map_err(|_| EndpointKernelError::context_inconsistent())?;
                uploads.push(PreparedUploadFinalization {
                    field: candidate.field.clone(),
                    policy: candidate.policy.clone(),
                    proposal,
                });
            }
        }
        Ok(Some(PreparedProposalBatch {
            batch,
            uploads,
            authorized_action: Some(authorized_action),
            _operation_guards: operation_guards,
        }))
    }

    async fn finalize_upload_proposals(
        &self,
        result: &ExecutionResult,
        proposals: &PreparedProposalBatch,
        request_idempotency: &IdempotencyKey,
    ) -> Result<(), EndpointKernelError> {
        if proposals.uploads.is_empty() {
            return Ok(());
        }
        let action_committed = match result {
            ExecutionResult::Accepted(accepted) => accepted.validation().is_empty(),
            ExecutionResult::RefreshRequired(refresh) => {
                refresh.accepted_metadata().is_some_and(|accepted| {
                    accepted.outcome().kind() != AcceptedOutcomeKind::Validation
                })
            }
            ExecutionResult::InProgress { .. } | ExecutionResult::IdempotencyConflict => false,
        };
        if !action_committed {
            return Ok(());
        }
        let context = self
            .upload_context
            .as_ref()
            .ok_or_else(EndpointKernelError::context_inconsistent)?;
        let action = proposals
            .authorized_action
            .as_ref()
            .ok_or_else(EndpointKernelError::context_inconsistent)?;
        let now = self
            .clock
            .now()
            .map_err(|_| EndpointKernelError::unavailable())?;
        for upload in &proposals.uploads {
            let idempotency =
                upload_finalize_idempotency(request_idempotency, upload.proposal.handle())?;
            for attempt in 0..2 {
                let result = self
                    .upload_finalization
                    .finalize(
                        context,
                        FinalizeUploadRequest::new(
                            upload.proposal.handle().clone(),
                            upload.field.clone(),
                            upload.proposal.ready_revision(),
                            idempotency.clone(),
                            action.clone(),
                            upload.policy.clone(),
                        ),
                        now,
                    )
                    .await;
                match result {
                    Ok(_) => {
                        // The upload is durable, so the slots it held go back
                        // now rather than when cleanup reclaims the record
                        // (ROOT-16). The action already committed; a failure
                        // here leaves the slots to that cleanup.
                        if let Err(error) = self
                            .upload_provider
                            .release_after_finalization(upload.proposal.handle())
                            .await
                        {
                            tracing::warn!(
                                kind = error.kind().as_str(),
                                "a finalized Live upload kept its slots until cleanup"
                            );
                        }
                        break;
                    }
                    Err(error)
                        if attempt == 0
                            && matches!(
                                error.kind(),
                                UploadErrorKind::ProviderUnavailable
                                    | UploadErrorKind::LedgerUnavailable
                                    | UploadErrorKind::ReconciliationRequired
                            ) => {}
                    Err(_) => return Err(EndpointKernelError::unavailable()),
                }
            }
        }
        Ok(())
    }
}

fn upload_finalize_idempotency(
    request: &IdempotencyKey,
    handle: &UploadHandle,
) -> Result<UploadIdempotencyKey, EndpointKernelError> {
    let mut digest = Sha256::new();
    digest.update(b"suprnova-live/framework-upload-finalize/v1\0");
    digest.update(request.as_bytes());
    digest.update(handle.to_string().as_bytes());
    UploadIdempotencyKey::parse(&format!("live-action:{}", hex::encode(digest.finalize())))
        .map_err(|_| EndpointKernelError::unavailable())
}

fn upload_proposal_handles(
    value: &CanonicalValue,
    maximum_files: usize,
) -> Result<Vec<UploadHandle>, EndpointKernelError> {
    let values = match value {
        CanonicalValue::Null => return Ok(Vec::new()),
        CanonicalValue::String(value) if maximum_files == 1 => vec![value.as_str()],
        CanonicalValue::Array(values)
            if maximum_files > 1 && !values.is_empty() && values.len() <= maximum_files =>
        {
            values
                .iter()
                .map(|value| match value {
                    CanonicalValue::String(value) => Ok(value.as_str()),
                    _ => Err(EndpointKernelError::context_inconsistent()),
                })
                .collect::<Result<Vec<_>, _>>()?
        }
        _ => return Err(EndpointKernelError::context_inconsistent()),
    };
    let mut handles = Vec::with_capacity(values.len());
    for value in values {
        let handle =
            UploadHandle::parse(value).map_err(|_| EndpointKernelError::context_inconsistent())?;
        if handles.contains(&handle) {
            return Err(EndpointKernelError::context_inconsistent());
        }
        handles.push(handle);
    }
    Ok(handles)
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
    ParamsChanged,
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
            if request.operations() == [OperationV2::ParamsChanged] {
                return Ok(RequestedOperation::ParamsChanged);
            }
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

const fn child_eligibility_kernel_error(
    kind: ChildParameterEligibilityErrorKind,
) -> EndpointKernelError {
    match kind {
        ChildParameterEligibilityErrorKind::ProviderUnavailable => {
            EndpointKernelError::unavailable()
        }
        _ => EndpointKernelError::context_inconsistent(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_oversized_request_reports_the_limit_its_size_and_the_key() {
        use suprnova_live::endpoint::LiveEndpointConfig;
        use suprnova_live::protocol::{ProtocolLimitConfig, ProtocolLimits};
        use suprnova_live::snapshot::SnapshotLimits;

        let input = InputLimits::new(64, 8, 64, 64).expect("small input limits");
        let protocol = ProtocolLimits::new(ProtocolLimitConfig {
            input,
            max_snapshot_bytes: 64,
            max_html_bytes: 64,
            max_model_proposals: 1,
            max_operations: 1,
            max_arguments: 1,
            max_validation_entries: 1,
            max_events: 1,
            max_effects: 1,
            max_extensions: 1,
        })
        .expect("small protocol limits");
        let snapshot = SnapshotLimits::new(input, 0, 1, 1, 1, 1).expect("small snapshot limits");
        let config = LiveEndpointConfig::new(protocol, snapshot).expect("endpoint config");
        let media = ParsedLiveMediaType::parse(
            "application/vnd.suprnova.live+json; charset=utf-8; version=1",
        )
        .expect("the Live media type");
        let error = config
            .inspect_mount(&[b'x'; 100], media)
            .expect_err("a body over the configured limit");
        let response = endpoint_failure(&error);
        let (Ok(response) | Err(response)) = response;
        assert_eq!(response.status_code(), 413);
        let report = response
            .error_report()
            .expect("the refusal carries a report");
        let message = report.chain().first().expect("one message");
        assert_eq!(
            message,
            "Suprnova Live request size limit exceeded: measured 100 bytes, configured 64 bytes. \
             Raise LIVE_MAX_REQUEST_BYTES in the application's .env file to allow it."
        );
    }

    #[test]
    fn child_eligibility_errors_conceal_authority_rejections_but_preserve_provider_failure() {
        for kind in [
            ChildParameterEligibilityErrorKind::BindingMismatch,
            ChildParameterEligibilityErrorKind::CompositionLineageMismatch,
            ChildParameterEligibilityErrorKind::ParentAuthorityMissing,
            ChildParameterEligibilityErrorKind::ParentRevisionMismatch,
        ] {
            assert_eq!(
                child_eligibility_kernel_error(kind),
                EndpointKernelError::context_inconsistent(),
            );
        }
        assert_eq!(
            child_eligibility_kernel_error(ChildParameterEligibilityErrorKind::ProviderUnavailable,),
            EndpointKernelError::unavailable(),
        );
    }
}
