//! Accepted-outcome coordination over verified instanced action input.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use bytes::Bytes;
use sha2::{Digest as _, Sha256};

use crate::action::{ActionOutcome, ActionResult, RawActionArguments};
use crate::canonical::CanonicalValue;
use crate::child::{
    ChildParameterLimits, EligibleChildParametersV2, PreparedChildParametersV2,
    VerifiedChildParametersV1, VerifiedChildParametersV2, authorize_child_parameters_v2,
};
use crate::clock::Clock;
use crate::component::{
    ActionExecutionOutput, ActionExecutionParts, ComponentExecutor, HydrationContext, MountContext,
    RenderContext,
};
use crate::crypto::SnapshotKeyRing;
use crate::endpoint::{
    AcceptedResponseAuthority, AcceptedResponseCandidate, AcceptedResponseRequestBinding,
    AcceptedResponseSealer, AcceptedResponseSnapshotAuthority, EndpointResponseIntents,
    SealedAcceptedResponse,
};
use crate::host::TrustedLiveRequestContext;
use crate::identity::{
    ActionName, BrowserNonce, BuildId, ContentDigest, IdempotencyKey, InstanceId, IslandSlot,
    Revision, RouteIdentity, ScopeFingerprint, UnixMillis,
};
use crate::ledger::{
    AcceptedOutcome, AcceptedOutcomeKind, AcceptedOutcomeMetadata, ClaimOutcome, ClaimRequest,
    ClaimToken, LiveInstanceLedger, RefreshReason,
};
use crate::limits::InputLimits;
use crate::promotion::{PromotedInstance, RefreshBeforeAction};
use crate::protocol::{BrowserRenderContext, ChildParameterDelivery};
use crate::registry::ComponentDescriptor;
use crate::snapshot::state::StateExposure;
use crate::snapshot::{
    COMPOSITION_LINEAGE_EXTENSION_V1, ComponentContract, CompositionChildLineageV1,
    CompositionLineageV1, CompositionOwnerLineageV1, InstanceBodyV1, InstanceFieldsV1, SeedBodyV1,
    SnapshotLimits, SnapshotSchemaSet, VerifiedInstanceV1, mounted_document_path,
};
use crate::state::ProposalBatch;
use crate::validation::{BagPolicy, ErrorBag, ValidationEngine, ValidationPort};
use crate::view::{
    ChildMountTransition, IslandRender, IslandRootInput, IslandSnapshotForm,
    MAX_SUCCESSOR_METADATA_BYTES, ViewRenderer, assemble_island_root,
};

use super::{
    ExecutionPhase, ExecutionTracePort, HostError, HostErrorKind, HostTransaction, RetryLegality,
    TransactionPort, record, run_host_future,
};

/// Explicit dependencies and bounded input for one registered action execution.
pub struct ActionExecutionRequest<'a> {
    pub(crate) action: &'a ActionName,
    pub(crate) raw_arguments: RawActionArguments,
    pub(crate) limits: &'a InputLimits,
    pub(crate) validation_engine: &'a ValidationEngine,
    pub(crate) validation_port: &'a dyn ValidationPort,
    pub(crate) bag_policy: BagPolicy,
    pub(crate) transaction_port: Option<&'a dyn TransactionPort>,
    pub(crate) trace: &'a dyn ExecutionTracePort,
    pub(crate) proposals: Option<&'a ProposalBatch>,
    pub(crate) response_intents: Option<&'a dyn ResponseIntentPreparationPort>,
    pub(crate) response_sealer: Option<AcceptedResponseSealer>,
    pub(crate) response_binding: Option<AcceptedResponseRequestBinding>,
}

impl<'a> ActionExecutionRequest<'a> {
    /// Creates one action request. `None` is valid only for no-transaction metadata.
    #[allow(
        clippy::too_many_arguments,
        reason = "the request keeps every authority and host boundary explicit"
    )]
    #[must_use]
    pub fn new(
        action: &'a ActionName,
        raw_arguments: RawActionArguments,
        limits: &'a InputLimits,
        validation_engine: &'a ValidationEngine,
        validation_port: &'a dyn ValidationPort,
        bag_policy: BagPolicy,
        transaction_port: Option<&'a dyn TransactionPort>,
        trace: &'a dyn ExecutionTracePort,
    ) -> Self {
        Self {
            action,
            raw_arguments,
            limits,
            validation_engine,
            validation_port,
            bag_policy,
            transaction_port,
            trace,
            proposals: None,
            response_intents: None,
            response_sealer: None,
            response_binding: None,
        }
    }

    /// Adds a separately prepared typed proposal batch to the bind phase.
    #[must_use]
    pub fn with_proposals(mut self, proposals: &'a ProposalBatch) -> Self {
        self.proposals = Some(proposals);
        self
    }

    /// Supplies fallible host semantic response preparation before durability.
    #[must_use]
    pub fn with_response_intent_preparation(
        mut self,
        response_intents: &'a dyn ResponseIntentPreparationPort,
    ) -> Self {
        self.response_intents = Some(response_intents);
        self
    }

    /// Supplies request-bound accepted-response sealing before durability.
    #[must_use]
    pub fn with_response_sealer(
        mut self,
        response_sealer: AcceptedResponseSealer,
        response_binding: AcceptedResponseRequestBinding,
    ) -> Self {
        self.response_sealer = Some(response_sealer);
        self.response_binding = Some(response_binding);
        self
    }
}

/// Narrow verified snapshot authority visible to response-intent preparation.
#[derive(Clone, Copy, Debug)]
pub struct VerifiedResponseIntentAuthority<'a> {
    mounted_document_path: Option<&'a str>,
    protocol_version: u16,
}

impl<'a> VerifiedResponseIntentAuthority<'a> {
    /// Returns the signed, normalized document path bound at mount time.
    #[must_use]
    pub const fn mounted_document_path(self) -> Option<&'a str> {
        self.mounted_document_path
    }

    /// Returns the registry-verified protocol selected for this request.
    #[must_use]
    pub const fn protocol_version(self) -> u16 {
        self.protocol_version
    }
}

/// Validated semantic result and least-privilege signed authority for preparation.
#[derive(Clone, Copy, Debug)]
pub struct ResponseIntentPreparationRequest<'a> {
    result: &'a ActionResult,
    authority: VerifiedResponseIntentAuthority<'a>,
}

impl<'a> ResponseIntentPreparationRequest<'a> {
    /// Returns the engine-validated action result.
    #[must_use]
    pub const fn result(self) -> &'a ActionResult {
        self.result
    }

    /// Returns the narrow signed snapshot authority for host resolution.
    #[must_use]
    pub const fn authority(self) -> VerifiedResponseIntentAuthority<'a> {
        self.authority
    }
}

/// Fallible host semantic preparation that runs before transaction commit.
pub trait ResponseIntentPreparationPort: Send + Sync {
    /// Resolves host-owned intents and stages request-scoped completion work.
    fn prepare<'a>(
        &'a self,
        request: ResponseIntentPreparationRequest<'a>,
    ) -> crate::component::LiveFuture<'a, Result<EndpointResponseIntents, HostError>>;
}

/// Fully verified authority and semantic request for one ordinary instanced action.
pub struct InstancedActionRequest<'a> {
    descriptor: &'a ComponentDescriptor,
    context: &'a TrustedLiveRequestContext,
    browser: BrowserRenderContext,
    snapshot: &'a VerifiedInstanceV1,
    idempotency_key: IdempotencyKey,
    request_digest: ContentDigest,
    action: ActionExecutionRequest<'a>,
}

/// Fully verified authority and semantic request for one instanced recovery render.
pub struct InstancedFreshRenderRequest<'a> {
    descriptor: &'a ComponentDescriptor,
    context: &'a TrustedLiveRequestContext,
    browser: BrowserRenderContext,
    snapshot: &'a VerifiedInstanceV1,
    idempotency_key: IdempotencyKey,
    request_digest: ContentDigest,
    trace: &'a dyn ExecutionTracePort,
    response_sealer: Option<AcceptedResponseSealer>,
    response_binding: Option<AcceptedResponseRequestBinding>,
}

/// One closed non-action lifecycle operation coordinated under island revision authority.
#[derive(Clone, Copy)]
pub enum InstancedLifecycleOperation<'a> {
    /// Apply a separately prepared model synchronization batch.
    SyncModels(&'a ProposalBatch),
    /// Apply a server-eligible exact-child v2 capability.
    ParamsChanged(&'a EligibleChildParametersV2),
    /// Historical v1 harness compatibility; never used by the production endpoint.
    ParamsChangedV1Historical(&'a VerifiedChildParametersV1),
    /// Complete one registered lazy lifecycle boundary.
    LazyComplete,
}

/// Verified authority and presentation context for one non-action lifecycle operation.
pub struct InstancedLifecycleRequest<'a> {
    descriptor: &'a ComponentDescriptor,
    context: &'a TrustedLiveRequestContext,
    browser: BrowserRenderContext,
    snapshot: &'a VerifiedInstanceV1,
    idempotency_key: IdempotencyKey,
    request_digest: ContentDigest,
    operation: InstancedLifecycleOperation<'a>,
    trace: &'a dyn ExecutionTracePort,
    response_sealer: Option<AcceptedResponseSealer>,
    response_binding: Option<AcceptedResponseRequestBinding>,
}

/// Promoted public-seed authority and first semantic operation.
pub struct PromotedActionRequest<'a> {
    descriptor: &'a ComponentDescriptor,
    context: &'a TrustedLiveRequestContext,
    browser: BrowserRenderContext,
    promoted: PromotedInstance,
    browser_nonce: BrowserNonce,
    idempotency_key: IdempotencyKey,
    request_digest: ContentDigest,
    action: ActionExecutionRequest<'a>,
}

impl<'a> PromotedActionRequest<'a> {
    /// Binds an internal promotion capability to the current trusted request.
    #[must_use]
    pub fn new(
        descriptor: &'a ComponentDescriptor,
        context: &'a TrustedLiveRequestContext,
        browser: BrowserRenderContext,
        promoted: PromotedInstance,
        browser_nonce: BrowserNonce,
        idempotency_key: IdempotencyKey,
        request_digest: ContentDigest,
        action: ActionExecutionRequest<'a>,
    ) -> Self {
        Self {
            descriptor,
            context,
            browser,
            promoted,
            browser_nonce,
            idempotency_key,
            request_digest,
            action,
        }
    }
}

impl<'a> InstancedActionRequest<'a> {
    /// Binds verified snapshot authority to current trusted host facts and action input.
    #[must_use]
    pub fn new(
        descriptor: &'a ComponentDescriptor,
        context: &'a TrustedLiveRequestContext,
        browser: BrowserRenderContext,
        snapshot: &'a VerifiedInstanceV1,
        idempotency_key: IdempotencyKey,
        request_digest: ContentDigest,
        action: ActionExecutionRequest<'a>,
    ) -> Self {
        Self {
            descriptor,
            context,
            browser,
            snapshot,
            idempotency_key,
            request_digest,
            action,
        }
    }
}

impl<'a> InstancedFreshRenderRequest<'a> {
    /// Binds verified snapshot authority to one recovery render without action replay.
    #[must_use]
    pub fn new(
        descriptor: &'a ComponentDescriptor,
        context: &'a TrustedLiveRequestContext,
        browser: BrowserRenderContext,
        snapshot: &'a VerifiedInstanceV1,
        idempotency_key: IdempotencyKey,
        request_digest: ContentDigest,
        trace: &'a dyn ExecutionTracePort,
    ) -> Self {
        Self {
            descriptor,
            context,
            browser,
            snapshot,
            idempotency_key,
            request_digest,
            trace,
            response_sealer: None,
            response_binding: None,
        }
    }

    /// Supplies request-bound accepted-response sealing before durability.
    #[must_use]
    pub fn with_response_sealer(
        mut self,
        response_sealer: AcceptedResponseSealer,
        response_binding: AcceptedResponseRequestBinding,
    ) -> Self {
        self.response_sealer = Some(response_sealer);
        self.response_binding = Some(response_binding);
        self
    }
}

impl<'a> InstancedLifecycleRequest<'a> {
    /// Binds one already verified lifecycle capability to revision coordination.
    #[allow(
        clippy::too_many_arguments,
        reason = "all lifecycle authority and host-observation inputs remain explicit"
    )]
    #[must_use]
    pub fn new(
        descriptor: &'a ComponentDescriptor,
        context: &'a TrustedLiveRequestContext,
        browser: BrowserRenderContext,
        snapshot: &'a VerifiedInstanceV1,
        idempotency_key: IdempotencyKey,
        request_digest: ContentDigest,
        operation: InstancedLifecycleOperation<'a>,
        trace: &'a dyn ExecutionTracePort,
    ) -> Self {
        Self {
            descriptor,
            context,
            browser,
            snapshot,
            idempotency_key,
            request_digest,
            operation,
            trace,
            response_sealer: None,
            response_binding: None,
        }
    }

    /// Supplies request-bound accepted-response sealing before durability.
    #[must_use]
    pub fn with_response_sealer(
        mut self,
        response_sealer: AcceptedResponseSealer,
        response_binding: AcceptedResponseRequestBinding,
    ) -> Self {
        self.response_sealer = Some(response_sealer);
        self.response_binding = Some(response_binding);
        self
    }
}

/// Safe reason the browser must obtain a fresh authorized island.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionRefreshReason {
    /// Ledger authority was absent, expired, consumed, or exhausted.
    Ledger(RefreshReason),
    /// The supplied base revision was no longer current.
    Stale,
    /// Accepted metadata existed but full prior response bytes were not retained.
    DuplicateResponseUnavailable,
    /// A pre-commit execution, render, dehydration, signing, or validation phase failed.
    ExecutionFailed,
    /// The host commit failed or had an indeterminate durable result.
    HostCommitFailed,
    /// Durable host work committed but ledger acceptance could not be recorded.
    LedgerAcceptanceFailed,
    /// The ledger provider could not conclusively arbitrate the request.
    LedgerUnavailable,
    /// A refresh-on-promote component requires protocol v2 fresh-render semantics.
    ProtocolUpgradeRequired,
}

/// Refresh response that never represents the original action as accepted.
pub struct RefreshRequiredExecution {
    reason: ExecutionRefreshReason,
    retry: RetryLegality,
    accepted: Option<AcceptedOutcomeMetadata>,
}

impl RefreshRequiredExecution {
    /// Returns the stable fresh-render reason.
    #[must_use]
    pub const fn reason(&self) -> ExecutionRefreshReason {
        self.reason
    }

    /// Returns whether automatic replay of the action is legal.
    #[must_use]
    pub const fn retry_legality(&self) -> RetryLegality {
        self.retry
    }

    /// Returns bounded prior acceptance metadata for an exact duplicate.
    #[must_use]
    pub const fn accepted_metadata(&self) -> Option<&AcceptedOutcomeMetadata> {
        self.accepted.as_ref()
    }
}

impl fmt::Debug for RefreshRequiredExecution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RefreshRequiredExecution")
            .field("reason", &self.reason)
            .field("retry", &self.retry)
            .field("has_accepted_metadata", &self.accepted.is_some())
            .finish()
    }
}

/// Complete accepted result whose bytes were built before durable acceptance.
pub struct AcceptedExecution {
    revision: Revision,
    signed_snapshot: Vec<u8>,
    render: Option<IslandRender>,
    result: ActionResult,
    response_intents: EndpointResponseIntents,
    validation: ErrorBag,
    action_executed: bool,
    reporting_failed: bool,
    child_deliveries: Vec<ChildParameterDelivery>,
    sealed_response: SealedAcceptedResponse,
}

impl AcceptedExecution {
    /// Returns the committed successor revision.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the complete signed successor snapshot.
    #[must_use]
    pub fn signed_snapshot(&self) -> &[u8] {
        &self.signed_snapshot
    }

    /// Returns fresh bounded island HTML when rendering was requested.
    #[must_use]
    pub const fn render(&self) -> Option<&IslandRender> {
        self.render.as_ref()
    }

    /// Returns the closed semantic action result.
    #[must_use]
    pub const fn result(&self) -> &ActionResult {
        &self.result
    }

    /// Returns the host-prepared semantic intents sealed before acceptance.
    #[must_use]
    pub const fn response_intents(&self) -> &EndpointResponseIntents {
        &self.response_intents
    }

    /// Returns bounded validation state accepted with the outcome.
    #[must_use]
    pub const fn validation(&self) -> &ErrorBag {
        &self.validation
    }

    /// Returns whether the registered Rust action body ran.
    #[must_use]
    pub const fn action_executed(&self) -> bool {
        self.action_executed
    }

    /// Returns whether post-acceptance reporting failed observably.
    #[must_use]
    pub const fn reporting_failed(&self) -> bool {
        self.reporting_failed
    }

    /// Returns changed surviving child deliveries sealed with this accepted parent outcome.
    #[must_use]
    pub fn child_deliveries(&self) -> &[ChildParameterDelivery] {
        &self.child_deliveries
    }

    pub(crate) fn into_sealed_response(self) -> SealedAcceptedResponse {
        self.sealed_response
    }
}

impl fmt::Debug for AcceptedExecution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcceptedExecution")
            .field("revision", &self.revision)
            .field("rendered", &self.render.is_some())
            .field("action_executed", &self.action_executed)
            .field("reporting_failed", &self.reporting_failed)
            .finish()
    }
}

/// Closed result of expected-revision arbitration and execution.
#[derive(Debug)]
pub enum ExecutionResult {
    /// One complete outcome committed and was accepted.
    Accepted(Box<AcceptedExecution>),
    /// The exact request is already running and was not invoked again.
    InProgress {
        /// Already claimed successor revision.
        successor_revision: Revision,
    },
    /// Browser state must fresh-render without replaying the action.
    RefreshRequired(RefreshRequiredExecution),
    /// A retry identity was reused for different semantic input.
    IdempotencyConflict,
}

/// Fixed metadata supplied only after ledger acceptance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedExecutionReport {
    revision: Revision,
    outcome: AcceptedOutcomeKind,
}

impl AcceptedExecutionReport {
    /// Returns the accepted successor revision.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the accepted bounded outcome category.
    #[must_use]
    pub const fn outcome(&self) -> AcceptedOutcomeKind {
        self.outcome
    }
}

/// Fallible observability only; durable delivery belongs to a transaction-owned outbox.
pub trait AcceptedOutcomeReporter: Send + Sync {
    /// Reports an already accepted result without authority to rewrite it.
    fn report(
        &self,
        report: AcceptedExecutionReport,
    ) -> crate::component::LiveFuture<'_, Result<(), HostError>>;
}

/// Coordinates Tier 0 claim, action, publication, host commit, and ledger acceptance order.
pub struct ExecutionService {
    ledger: Arc<dyn LiveInstanceLedger>,
    clock: Arc<dyn Clock>,
    keys: Arc<SnapshotKeyRing>,
    snapshot_limits: SnapshotLimits,
    renderer: ViewRenderer,
    reporter: Option<Arc<dyn AcceptedOutcomeReporter>>,
}

struct ClaimedOutcome {
    successor_revision: Revision,
    claim: ClaimGuard,
}

struct ClaimGuard {
    ledger: Arc<dyn LiveInstanceLedger>,
    token: Option<ClaimToken>,
    phase: ClaimGuardPhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClaimGuardPhase {
    Rollbackable,
    Finalizing,
}

impl ClaimGuard {
    fn new(ledger: Arc<dyn LiveInstanceLedger>, token: ClaimToken) -> Self {
        Self {
            ledger,
            token: Some(token),
            phase: ClaimGuardPhase::Rollbackable,
        }
    }

    fn token(&self) -> &ClaimToken {
        self.token.as_ref().expect("armed execution claim")
    }

    fn disarm(&mut self) {
        self.token.take();
    }

    fn begin_finalizing(&mut self) {
        self.phase = ClaimGuardPhase::Finalizing;
    }
}

impl Drop for ClaimGuard {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            match self.phase {
                ClaimGuardPhase::Rollbackable => self.ledger.abandon_on_drop(token),
                ClaimGuardPhase::Finalizing => self.ledger.fence_on_drop(token),
            }
        }
    }
}

struct SnapshotAuthority {
    component: ComponentContract,
    build_id: BuildId,
    route: RouteIdentity,
    slot: IslandSlot,
    scope: ScopeFingerprint,
    instance_id: InstanceId,
    expires_at: UnixMillis,
    extensions: BTreeMap<String, CanonicalValue>,
    composition_lineage: Option<CompositionLineageV1>,
    owner_parent_revision: Option<Revision>,
    request_snapshot: RequestSnapshotAuthority,
}

struct SuccessorComposition {
    extensions: BTreeMap<String, CanonicalValue>,
    pending: Vec<crate::component::composition::PendingChildParameters>,
}

enum RequestSnapshotAuthority {
    Instance(InstanceId),
    SeedPromotion(BrowserNonce),
}

struct SuccessorPresentation {
    document_key: String,
    protocol_minimum: u16,
    protocol_version: u16,
    context_expires_at: UnixMillis,
}

impl ExecutionService {
    /// Creates a coordinator with no fallible post-acceptance reporter.
    #[must_use]
    pub fn new(
        ledger: Arc<dyn LiveInstanceLedger>,
        clock: Arc<dyn Clock>,
        keys: Arc<SnapshotKeyRing>,
        snapshot_limits: SnapshotLimits,
        renderer: ViewRenderer,
    ) -> Self {
        Self {
            ledger,
            clock,
            keys,
            snapshot_limits,
            renderer,
            reporter: None,
        }
    }

    /// Installs non-authoritative post-acceptance reporting.
    #[must_use]
    pub fn with_reporter(mut self, reporter: Arc<dyn AcceptedOutcomeReporter>) -> Self {
        self.reporter = Some(reporter);
        self
    }

    /// Authorizes a verified child-v2 envelope against signed lineage and the current ledger.
    pub async fn authorize_child_parameters_v2(
        &self,
        verified: &VerifiedChildParametersV2,
        parent: &VerifiedInstanceV1,
    ) -> Result<EligibleChildParametersV2, crate::child::ChildParameterEligibilityError> {
        authorize_child_parameters_v2(verified, parent, self.ledger.as_ref()).await
    }

    /// Executes one verified instanced action under exact Tier 0 ordering.
    pub async fn execute_instanced(
        &self,
        mut request: InstancedActionRequest<'_>,
    ) -> ExecutionResult {
        let body = request.snapshot.body();
        let trace = request.action.trace;
        let response_intents = request.action.response_intents;
        let response_sealer = request.action.response_sealer.take();
        let response_binding = request.action.response_binding;
        let claimed = match self
            .claim(
                body.scope(),
                body.instance_id(),
                body.revision(),
                request.idempotency_key,
                request.request_digest,
                trace,
            )
            .await
        {
            Ok(claimed) => claimed,
            Err(result) => return result,
        };
        let render_context = RenderContext::new(
            request.context,
            body.instance_id(),
            claimed.successor_revision,
            body.expires_at(),
        )
        .with_browser_context(&request.browser);
        let hydration = HydrationContext::new(render_context, body.state()).with_memo(body.memo());
        let output = ComponentExecutor::new()
            .coordinated_action(request.descriptor, &hydration, request.action)
            .await;
        let output = match output {
            Ok(output) => output,
            Err(_) => {
                self.consume_failed_claim(claimed.claim).await;
                return refresh(ExecutionRefreshReason::ExecutionFailed);
            }
        };
        self.accept_output(
            request.descriptor,
            trace,
            claimed,
            SnapshotAuthority {
                component: body.component().clone(),
                build_id: body.build_id().clone(),
                route: body.route().clone(),
                slot: body.slot().clone(),
                scope: body.scope().clone(),
                instance_id: body.instance_id().clone(),
                expires_at: body.expires_at(),
                extensions: body.extensions().clone(),
                composition_lineage: body.composition_lineage().cloned(),
                owner_parent_revision: None,
                request_snapshot: RequestSnapshotAuthority::Instance(body.instance_id().clone()),
            },
            SuccessorPresentation {
                document_key: request.browser.document_key().as_str().to_owned(),
                protocol_minimum: request.context.mount().minimum_protocol(),
                protocol_version: request.context.mount().protocol(),
                context_expires_at: request.context.expires_at(),
            },
            request.context.mount().expected_seed().schemas(),
            output,
            None,
            response_intents,
            response_sealer,
            response_binding,
        )
        .await
    }

    /// Reconstructs and renders one existing island under exact Tier 0 revision ordering.
    pub async fn execute_fresh_render(
        &self,
        request: InstancedFreshRenderRequest<'_>,
    ) -> ExecutionResult {
        let body = request.snapshot.body();
        let successor_revision = match body.revision().checked_next() {
            Ok(revision) => revision,
            Err(_) => return refresh(ExecutionRefreshReason::ExecutionFailed),
        };
        let render_context = RenderContext::new(
            request.context,
            body.instance_id(),
            successor_revision,
            body.expires_at(),
        )
        .with_browser_context(&request.browser);
        let hydration = HydrationContext::new(render_context, body.state()).with_memo(body.memo());
        let output = match ComponentExecutor::new()
            .reconstruct(request.descriptor, &hydration)
            .await
        {
            Ok(output) => ActionExecutionOutput::fresh_render(output),
            Err(_) => return refresh(ExecutionRefreshReason::ExecutionFailed),
        };
        let claimed = match self
            .claim(
                body.scope(),
                body.instance_id(),
                body.revision(),
                request.idempotency_key,
                request.request_digest,
                request.trace,
            )
            .await
        {
            Ok(claimed) if claimed.successor_revision == successor_revision => claimed,
            Ok(claimed) => {
                self.consume_failed_claim(claimed.claim).await;
                return refresh(ExecutionRefreshReason::ExecutionFailed);
            }
            Err(result) => return result,
        };
        self.accept_output(
            request.descriptor,
            request.trace,
            claimed,
            SnapshotAuthority {
                component: body.component().clone(),
                build_id: body.build_id().clone(),
                route: body.route().clone(),
                slot: body.slot().clone(),
                scope: body.scope().clone(),
                instance_id: body.instance_id().clone(),
                expires_at: body.expires_at(),
                extensions: body.extensions().clone(),
                composition_lineage: body.composition_lineage().cloned(),
                owner_parent_revision: None,
                request_snapshot: RequestSnapshotAuthority::Instance(body.instance_id().clone()),
            },
            SuccessorPresentation {
                document_key: request.browser.document_key().as_str().to_owned(),
                protocol_minimum: request.context.mount().minimum_protocol(),
                protocol_version: request.context.mount().protocol(),
                context_expires_at: request.context.expires_at(),
            },
            request.context.mount().expected_seed().schemas(),
            output,
            Some(AcceptedOutcomeKind::Recovery),
            None,
            request.response_sealer,
            request.response_binding,
        )
        .await
    }

    /// Executes model-only and registered lifecycle requests without faking an action.
    pub async fn execute_lifecycle(
        &self,
        request: InstancedLifecycleRequest<'_>,
    ) -> ExecutionResult {
        let body = request.snapshot.body();
        let claimed = match self
            .claim(
                body.scope(),
                body.instance_id(),
                body.revision(),
                request.idempotency_key,
                request.request_digest,
                request.trace,
            )
            .await
        {
            Ok(claimed) => claimed,
            Err(result) => return result,
        };
        let render_context = RenderContext::new(
            request.context,
            body.instance_id(),
            claimed.successor_revision,
            body.expires_at(),
        )
        .with_browser_context(&request.browser);
        let hydration = HydrationContext::new(render_context, body.state()).with_memo(body.memo());
        let output = match request.operation {
            InstancedLifecycleOperation::SyncModels(proposals) => {
                ComponentExecutor::new()
                    .synchronize(request.descriptor, &hydration, proposals, request.trace)
                    .await
            }
            InstancedLifecycleOperation::ParamsChanged(parameters) => {
                ComponentExecutor::new()
                    .params_changed_v2(request.descriptor, &hydration, parameters)
                    .await
            }
            InstancedLifecycleOperation::ParamsChangedV1Historical(parameters) => {
                ComponentExecutor::new()
                    .params_changed(request.descriptor, &hydration, parameters)
                    .await
            }
            InstancedLifecycleOperation::LazyComplete => {
                ComponentExecutor::new()
                    .lazy_complete(request.descriptor, &hydration)
                    .await
            }
        };
        let output = match output {
            Ok(output) => ActionExecutionOutput::fresh_render(output),
            Err(_) => {
                self.consume_failed_claim(claimed.claim).await;
                return refresh(ExecutionRefreshReason::ExecutionFailed);
            }
        };
        self.accept_output(
            request.descriptor,
            request.trace,
            claimed,
            SnapshotAuthority {
                component: body.component().clone(),
                build_id: body.build_id().clone(),
                route: body.route().clone(),
                slot: body.slot().clone(),
                scope: body.scope().clone(),
                instance_id: body.instance_id().clone(),
                expires_at: body.expires_at(),
                extensions: body.extensions().clone(),
                composition_lineage: body.composition_lineage().cloned(),
                owner_parent_revision: match request.operation {
                    InstancedLifecycleOperation::ParamsChanged(parameters) => {
                        Some(parameters.parent_revision())
                    }
                    InstancedLifecycleOperation::SyncModels(_)
                    | InstancedLifecycleOperation::ParamsChangedV1Historical(_)
                    | InstancedLifecycleOperation::LazyComplete => None,
                },
                request_snapshot: RequestSnapshotAuthority::Instance(body.instance_id().clone()),
            },
            SuccessorPresentation {
                document_key: request.browser.document_key().as_str().to_owned(),
                protocol_minimum: request.context.mount().minimum_protocol(),
                protocol_version: request.context.mount().protocol(),
                context_expires_at: request.context.expires_at(),
            },
            request.context.mount().expected_seed().schemas(),
            output,
            None,
            None,
            request.response_sealer,
            request.response_binding,
        )
        .await
    }

    /// Executes the first operation after public-seed promotion without publishing partial state.
    pub async fn execute_promoted(
        &self,
        mut request: PromotedActionRequest<'_>,
    ) -> ExecutionResult {
        let trace = request.action.trace;
        let response_intents = request.action.response_intents;
        let response_sealer = request.action.response_sealer.take();
        let response_binding = request.action.response_binding;
        let (authority, verified_seed, refresh_before_action) = request.promoted.into_parts();
        let seed = verified_seed.body();
        let claimed = match self
            .claim(
                request.context.scope(),
                authority.instance_id(),
                authority.revision(),
                request.idempotency_key,
                request.request_digest,
                trace,
            )
            .await
        {
            Ok(claimed) => claimed,
            Err(result) => return result,
        };
        let render_context = RenderContext::new(
            request.context,
            authority.instance_id(),
            claimed.successor_revision,
            authority.expires_at(),
        )
        .with_browser_context(&request.browser);
        let (output, kind_override) = match self
            .prepare_promoted_output(
                request.descriptor,
                request.context,
                seed,
                refresh_before_action,
                render_context,
                request.action,
            )
            .await
        {
            Ok(output) => output,
            Err(reason) => {
                self.consume_failed_claim(claimed.claim).await;
                return refresh(reason);
            }
        };

        let schemas = request.context.mount().expected_seed().schemas();
        self.accept_output(
            request.descriptor,
            trace,
            claimed,
            SnapshotAuthority {
                component: seed.component().clone(),
                build_id: seed.build_id().clone(),
                route: seed.route().clone(),
                slot: seed.slot().clone(),
                scope: request.context.scope().clone(),
                instance_id: authority.instance_id().clone(),
                expires_at: authority.expires_at(),
                extensions: seed.extensions().clone(),
                composition_lineage: None,
                owner_parent_revision: None,
                request_snapshot: RequestSnapshotAuthority::SeedPromotion(request.browser_nonce),
            },
            SuccessorPresentation {
                document_key: request.browser.document_key().as_str().to_owned(),
                protocol_minimum: request.context.mount().minimum_protocol(),
                protocol_version: request.context.mount().protocol(),
                context_expires_at: request.context.expires_at(),
            },
            schemas,
            output,
            kind_override,
            response_intents,
            response_sealer,
            response_binding,
        )
        .await
    }

    async fn prepare_promoted_output(
        &self,
        descriptor: &ComponentDescriptor,
        context: &TrustedLiveRequestContext,
        seed: &SeedBodyV1,
        refresh_before_action: RefreshBeforeAction,
        render_context: RenderContext<'_>,
        action: ActionExecutionRequest<'_>,
    ) -> Result<(ActionExecutionOutput, Option<AcceptedOutcomeKind>), ExecutionRefreshReason> {
        let trace = action.trace;
        let mount = MountContext::new(render_context, seed.mount());
        record(trace, ExecutionPhase::PromotionMount);
        if refresh_before_action == RefreshBeforeAction::Required {
            if context.mount().protocol() < 2 {
                return Err(ExecutionRefreshReason::ProtocolUpgradeRequired);
            }
            let mounted = ComponentExecutor::new()
                .initial_mount(descriptor, &mount)
                .await
                .map_err(|_| ExecutionRefreshReason::ExecutionFailed)?;
            return Ok((
                ActionExecutionOutput::fresh_render(mounted),
                Some(AcceptedOutcomeKind::Recovery),
            ));
        }

        let mounted = ComponentExecutor::new()
            .promotion_mount_state(descriptor, &mount)
            .await
            .map_err(|_| ExecutionRefreshReason::ExecutionFailed)?;
        let mut state = mounted.state;
        let mut memo = mounted.memo;
        if !overlay_verified_public(&mut state, seed.state())
            || !overlay_verified_public(&mut memo, seed.memo())
        {
            return Err(ExecutionRefreshReason::ExecutionFailed);
        }
        let schemas = context.mount().expected_seed().schemas();
        if schemas
            .state()
            .validate(&state, StateExposure::Instanced)
            .is_err()
            || schemas
                .memo()
                .validate(&memo, StateExposure::Instanced)
                .is_err()
        {
            return Err(ExecutionRefreshReason::ExecutionFailed);
        }
        let hydration = HydrationContext::new(render_context, &state).with_memo(&memo);
        let output = ComponentExecutor::new()
            .coordinated_action(descriptor, &hydration, action)
            .await
            .map_err(|_| ExecutionRefreshReason::ExecutionFailed)?;
        Ok((output, None))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "claim authority stays explicit at the only revision-arbitration boundary"
    )]
    async fn claim(
        &self,
        scope: &ScopeFingerprint,
        instance_id: &InstanceId,
        base_revision: Revision,
        idempotency_key: IdempotencyKey,
        request_digest: ContentDigest,
        trace: &dyn ExecutionTracePort,
    ) -> Result<ClaimedOutcome, ExecutionResult> {
        record(trace, ExecutionPhase::Claim);
        match self
            .ledger
            .claim(ClaimRequest::new(
                scope.clone(),
                instance_id.clone(),
                base_revision,
                idempotency_key,
                request_digest,
            ))
            .await
        {
            Ok(ClaimOutcome::Granted(grant)) => Ok(ClaimedOutcome {
                successor_revision: grant.successor_revision(),
                claim: ClaimGuard::new(Arc::clone(&self.ledger), grant.into_token()),
            }),
            Ok(ClaimOutcome::InProgress { successor_revision }) => {
                Err(ExecutionResult::InProgress { successor_revision })
            }
            Ok(ClaimOutcome::Accepted(metadata)) => {
                Err(ExecutionResult::RefreshRequired(RefreshRequiredExecution {
                    reason: ExecutionRefreshReason::DuplicateResponseUnavailable,
                    retry: RetryLegality::Prohibited,
                    accepted: Some(metadata),
                }))
            }
            Ok(ClaimOutcome::Stale { .. }) => Err(refresh(ExecutionRefreshReason::Stale)),
            Ok(ClaimOutcome::IdempotencyConflict) => Err(ExecutionResult::IdempotencyConflict),
            Ok(ClaimOutcome::RefreshRequired(reason)) => {
                Err(refresh(ExecutionRefreshReason::Ledger(reason)))
            }
            Err(_) => Err(refresh(ExecutionRefreshReason::LedgerUnavailable)),
        }
    }

    fn prepare_successor_composition(
        &self,
        authority: &SnapshotAuthority,
        successor_revision: Revision,
        render: Option<&IslandRender>,
    ) -> Result<SuccessorComposition, ()> {
        let mut extensions = authority.extensions.clone();
        extensions.remove(COMPOSITION_LINEAGE_EXTENSION_V1);
        let mut owner = authority
            .composition_lineage
            .as_ref()
            .and_then(CompositionLineageV1::owner)
            .cloned();
        if let Some(parent_revision) = authority.owner_parent_revision {
            let current = owner.as_ref().ok_or(())?;
            owner = Some(
                CompositionOwnerLineageV1::new(
                    current.parent_instance().clone(),
                    parent_revision,
                    current.child_key().clone(),
                    current.child_contract().clone(),
                    current.child_instance().clone(),
                    current.depth(),
                )
                .map_err(|_| ())?,
            );
        }
        let previous = authority
            .composition_lineage
            .as_ref()
            .map(CompositionLineageV1::children)
            .unwrap_or_default();
        let mut children = Vec::new();
        let mut pending = Vec::new();

        if let Some(render) = render {
            for mount in &render.children {
                let Some(transition) = mount.transition() else {
                    if !previous.is_empty() {
                        return Err(());
                    }
                    continue;
                };
                let child = match transition {
                    ChildMountTransition::Surviving(child) => child,
                    ChildMountTransition::Pending(parameters) => {
                        pending.push(parameters.clone());
                        parameters.child()
                    }
                };
                let prior = previous
                    .iter()
                    .find(|entry| entry.child_key() == child.key())
                    .filter(|entry| {
                        entry.child_contract() == child.component_contract()
                            && entry.child_instance() == child.instance_id()
                    })
                    .ok_or(())?;
                children.push(
                    CompositionChildLineageV1::new(
                        authority.instance_id.clone(),
                        successor_revision,
                        child.key().clone(),
                        child.component_contract().clone(),
                        child.instance_id().clone(),
                        prior.depth(),
                    )
                    .map_err(|_| ())?,
                );
            }
        } else {
            children = previous
                .iter()
                .map(|child| {
                    CompositionChildLineageV1::new(
                        authority.instance_id.clone(),
                        successor_revision,
                        child.child_key().clone(),
                        child.child_contract().clone(),
                        child.child_instance().clone(),
                        child.depth(),
                    )
                    .map_err(|_| ())
                })
                .collect::<Result<Vec<_>, _>>()?;
        }

        if owner.is_some() || !children.is_empty() {
            let lineage = CompositionLineageV1::new(owner, children).map_err(|_| ())?;
            extensions.insert(
                COMPOSITION_LINEAGE_EXTENSION_V1.to_owned(),
                lineage.to_canonical(),
            );
        }

        Ok(SuccessorComposition {
            extensions,
            pending,
        })
    }

    fn seal_child_deliveries(
        &self,
        authority: &SnapshotAuthority,
        successor_revision: Revision,
        pending: Vec<crate::component::composition::PendingChildParameters>,
        now: UnixMillis,
    ) -> Result<Vec<ChildParameterDelivery>, ()> {
        const MAX_CHILD_PARAMETER_LIFETIME_MS: u64 = 300_000;
        let child_limits = ChildParameterLimits::new(
            *self.snapshot_limits.input(),
            0,
            MAX_CHILD_PARAMETER_LIFETIME_MS,
        )
        .map_err(|_| ())?;
        let expires_at = UnixMillis::new(
            now.get()
                .checked_add(MAX_CHILD_PARAMETER_LIFETIME_MS)
                .ok_or(())?
                .min(authority.expires_at.get()),
        );
        pending
            .into_iter()
            .map(|pending| {
                let child_instance = pending.child().instance_id().clone();
                let parameter_hash =
                    ContentDigest::from_bytes(pending.parameter_value().as_bytes())
                        .map_err(|_| ())?;
                let envelope = PreparedChildParametersV2::new(
                    authority.scope.clone(),
                    authority.instance_id.clone(),
                    successor_revision,
                    child_instance.clone(),
                    pending,
                    now,
                    expires_at,
                    self.keys.active_key_id().clone(),
                    &child_limits,
                )
                .and_then(|prepared| {
                    prepared.seal_claimed_successor(
                        &authority.scope,
                        &authority.instance_id,
                        successor_revision,
                        &self.keys,
                        now,
                        &child_limits,
                    )
                })
                .map_err(|_| ())?;
                Ok(ChildParameterDelivery::sealed(
                    child_instance,
                    parameter_hash,
                    envelope,
                ))
            })
            .collect()
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "publication keeps every authority and rollback input visible in one stage"
    )]
    async fn accept_output(
        &self,
        descriptor: &ComponentDescriptor,
        trace: &dyn ExecutionTracePort,
        claimed: ClaimedOutcome,
        authority: SnapshotAuthority,
        presentation: SuccessorPresentation,
        schemas: &SnapshotSchemaSet,
        output: ActionExecutionOutput,
        kind_override: Option<AcceptedOutcomeKind>,
        response_intent_port: Option<&dyn ResponseIntentPreparationPort>,
        response_sealer: Option<AcceptedResponseSealer>,
        response_binding: Option<AcceptedResponseRequestBinding>,
    ) -> ExecutionResult {
        let successor_revision = claimed.successor_revision;
        let mut claim = claimed.claim;
        let ActionExecutionParts {
            result,
            render,
            state,
            memo,
            validation,
            action_executed,
            mut transaction,
        } = output.into_parts();

        record(trace, ExecutionPhase::Sign);
        let now = match self.clock.now() {
            Ok(now) => now,
            Err(_) => {
                rollback(&mut transaction).await;
                self.consume_failed_claim(claim).await;
                return refresh(ExecutionRefreshReason::ExecutionFailed);
            }
        };
        if now >= presentation.context_expires_at {
            rollback(&mut transaction).await;
            self.consume_failed_claim(claim).await;
            return refresh(ExecutionRefreshReason::ExecutionFailed);
        }
        let successor_composition = match self.prepare_successor_composition(
            &authority,
            successor_revision,
            render.as_ref(),
        ) {
            Ok(composition) => composition,
            Err(()) => {
                rollback(&mut transaction).await;
                self.consume_failed_claim(claim).await;
                return refresh(ExecutionRefreshReason::ExecutionFailed);
            }
        };
        let signed_snapshot = InstanceBodyV1::new(
            InstanceFieldsV1 {
                component: authority.component.clone(),
                build_id: authority.build_id.clone(),
                route: authority.route.clone(),
                slot: authority.slot.clone(),
                key_id: self.keys.active_key_id().clone(),
                scope: authority.scope.clone(),
                instance_id: authority.instance_id.clone(),
                revision: successor_revision,
                issued_at: now,
                expires_at: authority.expires_at,
                state,
                memo,
                extensions: successor_composition.extensions,
            },
            schemas,
            &self.snapshot_limits,
        )
        .and_then(|body| body.sign(&self.keys, now, &self.snapshot_limits));
        let signed_snapshot = match signed_snapshot {
            Ok(snapshot) => snapshot,
            Err(_) => {
                rollback(&mut transaction).await;
                self.consume_failed_claim(claim).await;
                return refresh(ExecutionRefreshReason::ExecutionFailed);
            }
        };

        record(trace, ExecutionPhase::OutcomeValidation);
        if !self.validate_outcome(descriptor, &result, render.as_ref(), &signed_snapshot) {
            rollback(&mut transaction).await;
            self.consume_failed_claim(claim).await;
            return refresh(ExecutionRefreshReason::ExecutionFailed);
        }
        let render = match render {
            Some(fragment) => {
                let assembled = assemble_island_root(
                    fragment,
                    IslandRootInput {
                        component: authority.component.name().clone(),
                        slot: authority.slot.clone(),
                        document_key: presentation.document_key,
                        protocol_minimum: presentation.protocol_minimum,
                        runtime_contract: 1,
                        snapshot: Bytes::from(signed_snapshot.clone()),
                        snapshot_form: IslandSnapshotForm::Instance,
                        instance_id: Some(authority.instance_id.clone()),
                        revision: successor_revision,
                        lazy_complete: false,
                        flags: Vec::new(),
                    },
                    MAX_SUCCESSOR_METADATA_BYTES,
                )
                .and_then(|assembled| {
                    self.renderer
                        .validate_island_output(descriptor.metadata().view().clone(), assembled)
                });
                match assembled {
                    Ok(render) => Some(render),
                    Err(_) => {
                        rollback(&mut transaction).await;
                        self.consume_failed_claim(claim).await;
                        return refresh(ExecutionRefreshReason::ExecutionFailed);
                    }
                }
            }
            None => None,
        };
        let child_deliveries = if presentation.protocol_version == 2
            && !matches!(result.outcome(), ActionOutcome::Redirect(_))
        {
            match self.seal_child_deliveries(
                &authority,
                successor_revision,
                successor_composition.pending,
                now,
            ) {
                Ok(deliveries) => deliveries,
                Err(()) => {
                    rollback(&mut transaction).await;
                    self.consume_failed_claim(claim).await;
                    return refresh(ExecutionRefreshReason::ExecutionFailed);
                }
            }
        } else {
            Vec::new()
        };
        let response_intents =
            if response_intent_port.is_some() || response_intent_preparation_required(&result) {
                record(trace, ExecutionPhase::ResponseIntentPreparation);
                let document_path = match mounted_document_path(&authority.extensions) {
                    Ok(path) => path,
                    Err(_) => {
                        rollback(&mut transaction).await;
                        self.consume_failed_claim(claim).await;
                        return refresh(ExecutionRefreshReason::ExecutionFailed);
                    }
                };
                let Some(port) = response_intent_port else {
                    rollback(&mut transaction).await;
                    self.consume_failed_claim(claim).await;
                    return refresh(ExecutionRefreshReason::ExecutionFailed);
                };
                match run_host_future(
                    || {
                        port.prepare(ResponseIntentPreparationRequest {
                            result: &result,
                            authority: VerifiedResponseIntentAuthority {
                                mounted_document_path: document_path,
                                protocol_version: presentation.protocol_version,
                            },
                        })
                    },
                    HostErrorKind::ResponseIntent,
                )
                .await
                {
                    Ok(intents) => intents,
                    Err(_) => {
                        rollback(&mut transaction).await;
                        self.consume_failed_claim(claim).await;
                        return refresh(ExecutionRefreshReason::ExecutionFailed);
                    }
                }
            } else {
                EndpointResponseIntents::default()
            };
        if !response_intents.is_valid_for(&result, presentation.protocol_version) {
            rollback(&mut transaction).await;
            self.consume_failed_claim(claim).await;
            return refresh(ExecutionRefreshReason::ExecutionFailed);
        }
        record(trace, ExecutionPhase::ResponseSealing);
        let Some(response_sealer) = response_sealer else {
            rollback(&mut transaction).await;
            self.consume_failed_claim(claim).await;
            return refresh(ExecutionRefreshReason::ExecutionFailed);
        };
        let Some(response_binding) = response_binding else {
            rollback(&mut transaction).await;
            self.consume_failed_claim(claim).await;
            return refresh(ExecutionRefreshReason::ExecutionFailed);
        };
        if response_sealer.protocol_version() != presentation.protocol_version {
            rollback(&mut transaction).await;
            self.consume_failed_claim(claim).await;
            return refresh(ExecutionRefreshReason::ExecutionFailed);
        }
        let snapshot_authority = match &authority.request_snapshot {
            RequestSnapshotAuthority::Instance(instance_id) => {
                AcceptedResponseSnapshotAuthority::Instance(instance_id)
            }
            RequestSnapshotAuthority::SeedPromotion(browser_nonce) => {
                AcceptedResponseSnapshotAuthority::SeedPromotion(browser_nonce)
            }
        };
        let sealed_response = match response_sealer.seal(AcceptedResponseCandidate {
            request_binding: response_binding,
            revision: successor_revision,
            signed_snapshot: &signed_snapshot,
            render: render.as_ref(),
            result: &result,
            intents: &response_intents,
            validation: &validation,
            child_deliveries: &child_deliveries,
            authority: AcceptedResponseAuthority {
                component: &authority.component,
                route: &authority.route,
                slot: &authority.slot,
                scope: &authority.scope,
                snapshot: snapshot_authority,
            },
        }) {
            Ok(response) => response,
            Err(_) => {
                rollback(&mut transaction).await;
                self.consume_failed_claim(claim).await;
                return refresh(ExecutionRefreshReason::ExecutionFailed);
            }
        };
        if let Some(transaction) = transaction.take() {
            record(trace, ExecutionPhase::HostCommit);
            claim.begin_finalizing();
            if run_host_future(|| transaction.commit(), HostErrorKind::Commit)
                .await
                .is_err()
            {
                self.consume_failed_claim(claim).await;
                return refresh(ExecutionRefreshReason::HostCommitFailed);
            }
        }

        let kind = kind_override.unwrap_or_else(|| outcome_kind(&result, &validation));
        let digest = outcome_digest(successor_revision, &signed_snapshot, render.as_ref(), kind);
        record(trace, ExecutionPhase::LedgerAcceptance);
        if self
            .ledger
            .commit(claim.token(), AcceptedOutcome::new(kind, digest))
            .await
            .is_err()
        {
            return refresh(ExecutionRefreshReason::LedgerAcceptanceFailed);
        }
        claim.disarm();

        record(trace, ExecutionPhase::Reporting);
        let reporting_failed = if let Some(reporter) = &self.reporter {
            run_host_future(
                || {
                    reporter.report(AcceptedExecutionReport {
                        revision: successor_revision,
                        outcome: kind,
                    })
                },
                HostErrorKind::Reporting,
            )
            .await
            .is_err()
        } else {
            false
        };
        ExecutionResult::Accepted(Box::new(AcceptedExecution {
            revision: successor_revision,
            signed_snapshot,
            render,
            result,
            response_intents,
            validation,
            action_executed,
            reporting_failed,
            child_deliveries,
            sealed_response,
        }))
    }

    fn validate_outcome(
        &self,
        descriptor: &ComponentDescriptor,
        result: &ActionResult,
        render: Option<&IslandRender>,
        signed_snapshot: &[u8],
    ) -> bool {
        if signed_snapshot.is_empty() {
            return false;
        }
        let shape_matches = match result.outcome() {
            ActionOutcome::Render => render.is_some(),
            ActionOutcome::NoRender | ActionOutcome::Redirect(_) => render.is_none(),
        };
        shape_matches
            && render.is_none_or(|render| {
                self.renderer
                    .validate_island_fragment(descriptor.metadata().view().clone(), render)
                    .is_ok()
            })
    }

    async fn consume_failed_claim(&self, mut claim: ClaimGuard) {
        claim.begin_finalizing();
        if self.ledger.abandon(claim.token()).await.is_ok() {
            claim.disarm();
        }
    }
}

async fn rollback(transaction: &mut Option<Box<dyn HostTransaction>>) {
    if let Some(transaction) = transaction.take() {
        let _ = run_host_future(|| transaction.rollback(), HostErrorKind::Rollback).await;
    }
}

fn refresh(reason: ExecutionRefreshReason) -> ExecutionResult {
    ExecutionResult::RefreshRequired(RefreshRequiredExecution {
        reason,
        retry: RetryLegality::Prohibited,
        accepted: None,
    })
}

fn outcome_kind(result: &ActionResult, validation: &ErrorBag) -> AcceptedOutcomeKind {
    if !validation.is_empty() {
        return AcceptedOutcomeKind::Validation;
    }
    match result.outcome() {
        ActionOutcome::Render => AcceptedOutcomeKind::Rendered,
        ActionOutcome::NoRender => AcceptedOutcomeKind::NoRender,
        ActionOutcome::Redirect(_) => AcceptedOutcomeKind::Redirect,
    }
}

fn response_intent_preparation_required(result: &ActionResult) -> bool {
    matches!(result.outcome(), ActionOutcome::Redirect(_))
        || result.metadata().url().is_some()
        || !result.metadata().flash().is_empty()
}

fn overlay_verified_public(
    mounted: &mut crate::canonical::CanonicalValue,
    public: &crate::canonical::CanonicalValue,
) -> bool {
    let crate::canonical::CanonicalValue::Object(mounted) = mounted else {
        return false;
    };
    let crate::canonical::CanonicalValue::Object(public) = public else {
        return false;
    };
    mounted.extend(
        public
            .iter()
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    true
}

fn outcome_digest(
    revision: Revision,
    snapshot: &[u8],
    render: Option<&IslandRender>,
    kind: AcceptedOutcomeKind,
) -> ContentDigest {
    let mut digest = Sha256::new();
    digest.update(b"suprnova-live/accepted-outcome/v1");
    digest.update(revision.get().to_be_bytes());
    digest.update([outcome_kind_tag(kind)]);
    digest.update((snapshot.len() as u64).to_be_bytes());
    digest.update(snapshot);
    if let Some(render) = render {
        digest.update((render.body.len() as u64).to_be_bytes());
        digest.update(&render.body);
    }
    ContentDigest::from_bytes(&digest.finalize()).expect("SHA-256 is always 32 bytes")
}

const fn outcome_kind_tag(kind: AcceptedOutcomeKind) -> u8 {
    match kind {
        AcceptedOutcomeKind::Rendered => 1,
        AcceptedOutcomeKind::Validation => 2,
        AcceptedOutcomeKind::NoRender => 3,
        AcceptedOutcomeKind::Redirect => 4,
        AcceptedOutcomeKind::Recovery => 5,
    }
}
