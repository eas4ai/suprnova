//! Accepted-outcome coordination over verified instanced action input.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use bytes::Bytes;
use sha2::{Digest as _, Sha256};

use crate::action::{ActionOutcome, ActionResult, RawActionArguments};
use crate::canonical::CanonicalValue;
use crate::clock::Clock;
use crate::component::{
    ActionExecutionOutput, ActionExecutionParts, ComponentExecutor, HydrationContext, MountContext,
    RenderContext,
};
use crate::crypto::SnapshotKeyRing;
use crate::host::TrustedLiveRequestContext;
use crate::identity::{
    ActionName, BuildId, ContentDigest, IdempotencyKey, InstanceId, IslandSlot, Revision,
    RouteIdentity, ScopeFingerprint, UnixMillis,
};
use crate::ledger::{
    AcceptedOutcome, AcceptedOutcomeKind, AcceptedOutcomeMetadata, ClaimOutcome, ClaimRequest,
    ClaimToken, LiveInstanceLedger, RefreshReason,
};
use crate::limits::InputLimits;
use crate::promotion::{PromotedInstance, RefreshBeforeAction};
use crate::protocol::BrowserRenderContext;
use crate::registry::ComponentDescriptor;
use crate::snapshot::state::StateExposure;
use crate::snapshot::{
    ComponentContract, InstanceBodyV1, InstanceFieldsV1, SeedBodyV1, SnapshotLimits,
    SnapshotSchemaSet, VerifiedInstanceV1,
};
use crate::state::ProposalBatch;
use crate::validation::{BagPolicy, ErrorBag, ValidationEngine, ValidationPort};
use crate::view::{
    IslandRender, IslandRootInput, IslandSnapshotForm, MAX_SUCCESSOR_METADATA_BYTES, ViewRenderer,
    assemble_island_root,
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
        }
    }

    /// Adds a separately prepared typed proposal batch to the bind phase.
    #[must_use]
    pub fn with_proposals(mut self, proposals: &'a ProposalBatch) -> Self {
        self.proposals = Some(proposals);
        self
    }
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

/// Promoted public-seed authority and first semantic operation.
pub struct PromotedActionRequest<'a> {
    descriptor: &'a ComponentDescriptor,
    context: &'a TrustedLiveRequestContext,
    browser: BrowserRenderContext,
    promoted: PromotedInstance,
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
        idempotency_key: IdempotencyKey,
        request_digest: ContentDigest,
        action: ActionExecutionRequest<'a>,
    ) -> Self {
        Self {
            descriptor,
            context,
            browser,
            promoted,
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
    validation: ErrorBag,
    action_executed: bool,
    reporting_failed: bool,
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
    Accepted(AcceptedExecution),
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
    token: ClaimToken,
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
}

struct SuccessorPresentation {
    document_key: String,
    protocol_minimum: u16,
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

    /// Executes one verified instanced action under exact Tier 0 ordering.
    pub async fn execute_instanced(&self, request: InstancedActionRequest<'_>) -> ExecutionResult {
        let body = request.snapshot.body();
        let trace = request.action.trace;
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
                self.consume_failed_claim(claimed.token).await;
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
            },
            SuccessorPresentation {
                document_key: request.browser.document_key().as_str().to_owned(),
                protocol_minimum: request.context.mount().minimum_protocol(),
            },
            request.context.mount().expected_seed().schemas(),
            output,
            None,
        )
        .await
    }

    /// Executes the first operation after public-seed promotion without publishing partial state.
    pub async fn execute_promoted(&self, request: PromotedActionRequest<'_>) -> ExecutionResult {
        let trace = request.action.trace;
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
                self.consume_failed_claim(claimed.token).await;
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
            },
            SuccessorPresentation {
                document_key: request.browser.document_key().as_str().to_owned(),
                protocol_minimum: request.context.mount().minimum_protocol(),
            },
            schemas,
            output,
            kind_override,
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
                token: grant.into_token(),
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
    ) -> ExecutionResult {
        let successor_revision = claimed.successor_revision;
        let claim = claimed.token;
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
                extensions: authority.extensions.clone(),
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
        if let Some(transaction) = transaction.take() {
            record(trace, ExecutionPhase::HostCommit);
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
            .commit(claim, AcceptedOutcome::new(kind, digest))
            .await
            .is_err()
        {
            return refresh(ExecutionRefreshReason::LedgerAcceptanceFailed);
        }

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
        ExecutionResult::Accepted(AcceptedExecution {
            revision: successor_revision,
            signed_snapshot,
            render,
            result,
            validation,
            action_executed,
            reporting_failed,
        })
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

    async fn consume_failed_claim(&self, claim: ClaimToken) {
        let _ = self.ledger.abandon(claim).await;
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
