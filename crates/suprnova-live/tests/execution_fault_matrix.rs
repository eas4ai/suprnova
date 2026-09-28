//! Deterministic faults at every accepted-outcome coordination boundary.

mod child_parameter_support;
mod component_support;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use suprnova_live::action::{
    ActionArgumentSchema, ActionAuthorizationPort, ActionAuthorizationRequest, ActionEntry,
    ActionError, ActionFuture, ActionResult, ActionTable, ActionTarget, AuthorizationDecision,
    AuthorizationRequirement, AuthorizedAction, PreparedActionArguments, RawActionArguments,
    TransactionPolicy,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::clock::{Clock, ClockError};
use suprnova_live::endpoint::{EndpointNavigationTarget, EndpointResponseIntents};
use suprnova_live::execution::{
    AcceptedExecutionReport, AcceptedOutcomeReporter, ActionExecutionRequest, ExecutionPhase,
    ExecutionRefreshReason, ExecutionResult, ExecutionService, ExecutionTracePort, HostError,
    HostErrorKind, HostTransaction, InstancedActionRequest, ResponseIntentPreparationPort,
    ResponseIntentPreparationRequest, RetryLegality, TransactionPort,
};
use suprnova_live::host::{
    MountCatalogBuilder, MountCatalogEntry, MountScopeRequirements, MountSelection,
    ScopeRequirement, TrustedLiveRequestContext,
};
use suprnova_live::identity::{
    ActionName, BuildId, ComponentName, InstanceId, IslandSlot, ModelField, Revision,
    RouteIdentity, ScopeFingerprint, UnixMillis, ViewName,
};
use suprnova_live::ledger::{
    AcceptedOutcome, ClaimOutcome, ClaimRequest, ClaimToken, InstanceAuthority, LedgerError,
    LedgerLimits, LedgerPhase, LiveInstanceLedger, MemoryInstanceLedger, MountInstanceRecord,
    PromotionOutcome, PromotionRecord, RefreshReason,
};
use suprnova_live::limits::InputLimits;
use suprnova_live::metadata::{ActionMetadata, ComponentMetadata, ContractVersions, FieldMetadata};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
use suprnova_live::snapshot::state::{FieldCategory, StateCodec};
use suprnova_live::snapshot::{
    ComponentContract, CompositionChildLineageV1, CompositionLineageV1, ExpectedInstanceV1,
    InstanceBodyV1, InstanceFieldsV1, VerifiedInstanceV1, verify_instance,
};
use suprnova_live::state::{ModelBindingSchema, ProposalBatch, ProposalLimits};
use suprnova_live::validation::{
    BagPolicy, ValidationFuture, ValidationPort, ValidationPortError, ValidationRequest,
    ValidationSelection,
};
use suprnova_live::view::{AssetSet, ChildMount, IslandRender, RenderLimits, ViewRenderer};
use suprnova_live_test_support::SyntheticLiveRequestContextBuilder;
use suprnova_live_test_support::VerifiedResponseSealing;
use tokio::sync::Notify;

use component_support::{
    FailurePoint, FixtureControl, ManualClock, TraceFixture, admitted_response_sealer,
    admitted_response_sealer_with_semantics, admitted_response_sealer_with_snapshot_limits,
    browser_context, bytes, digest, fixture_host_scope, idempotency, install, key_ring, ledger,
    schema_set, snapshot_limits, trusted_context_for,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Fault {
    Claim,
    Hydrate,
    Bind,
    Authorize,
    Validate,
    TransactionBegin,
    TransactionBeginPanic,
    BeforeAction,
    RollbackPanic,
    Action,
    AfterAction,
    Render,
    Dehydrate,
    Sign,
    OutcomeValidation,
    ResponseIntent,
    ResponseIntentShape,
    ResponseSealing,
    HostCommit,
    HostCommitPanic,
    LedgerAcceptance,
    Reporting,
    ReportingPanic,
}

#[derive(Default)]
struct Trace(Mutex<Vec<ExecutionPhase>>);

impl ExecutionTracePort for Trace {
    fn record(&self, phase: ExecutionPhase) {
        self.0.lock().expect("execution trace lock").push(phase);
    }
}

struct Authorization {
    fail: bool,
}

impl ActionAuthorizationPort for Authorization {
    fn authorize<'a>(
        &'a self,
        _request: ActionAuthorizationRequest<'a>,
    ) -> ActionFuture<'a, Result<AuthorizationDecision, ActionError>> {
        Box::pin(async move {
            if self.fail {
                Err(ActionError::dispatcher_contract())
            } else {
                Ok(AuthorizationDecision::Allow)
            }
        })
    }
}

struct Validation {
    fail: bool,
}

impl ValidationPort for Validation {
    fn validate<'a>(
        &'a self,
        _request: ValidationRequest<'a>,
    ) -> ValidationFuture<
        'a,
        Result<Vec<suprnova_live::validation::ValidationIssue>, ValidationPortError>,
    > {
        Box::pin(async move {
            if self.fail {
                Err(ValidationPortError::unavailable())
            } else {
                Ok(Vec::new())
            }
        })
    }
}

struct TransactionControl {
    fault: Fault,
    domain_commits: AtomicUsize,
    rollbacks: AtomicUsize,
    ledger_clock: Arc<ManualClock>,
}

struct TestTransaction(Arc<TransactionControl>);

impl HostTransaction for TestTransaction {
    fn commit(
        self: Box<Self>,
    ) -> suprnova_live::component::LiveFuture<'static, Result<(), HostError>> {
        let control = self.0.clone();
        Box::pin(async move {
            assert_ne!(control.fault, Fault::HostCommitPanic, "host commit panic");
            if control.fault == Fault::HostCommit {
                return Err(HostError::new(HostErrorKind::Commit));
            }
            control.domain_commits.fetch_add(1, Ordering::SeqCst);
            if control.fault == Fault::LedgerAcceptance {
                control.ledger_clock.set(1_101);
            }
            Ok(())
        })
    }

    fn rollback(
        self: Box<Self>,
    ) -> suprnova_live::component::LiveFuture<'static, Result<(), HostError>> {
        let control = self.0.clone();
        Box::pin(async move {
            control.rollbacks.fetch_add(1, Ordering::SeqCst);
            assert_ne!(control.fault, Fault::RollbackPanic, "host rollback panic");
            Ok(())
        })
    }
}

struct TestTransactionPort(Arc<TransactionControl>);

impl TransactionPort for TestTransactionPort {
    fn begin(
        &self,
    ) -> suprnova_live::component::LiveFuture<'_, Result<Box<dyn HostTransaction>, HostError>> {
        assert_ne!(
            self.0.fault,
            Fault::TransactionBeginPanic,
            "host transaction begin panic"
        );
        let control = self.0.clone();
        Box::pin(async move {
            if control.fault == Fault::TransactionBegin {
                Err(HostError::new(HostErrorKind::Begin))
            } else {
                Ok(Box::new(TestTransaction(control)) as Box<dyn HostTransaction>)
            }
        })
    }
}

struct TestResponseIntentPort {
    fault: Fault,
}

impl ResponseIntentPreparationPort for TestResponseIntentPort {
    fn prepare<'a>(
        &'a self,
        _request: ResponseIntentPreparationRequest<'a>,
    ) -> suprnova_live::component::LiveFuture<'a, Result<EndpointResponseIntents, HostError>> {
        Box::pin(async move {
            if self.fault == Fault::ResponseIntent {
                Err(HostError::new(HostErrorKind::ResponseIntent))
            } else if self.fault == Fault::ResponseIntentShape {
                Ok(EndpointResponseIntents::default().with_redirect(
                    EndpointNavigationTarget::parse("/invalid").expect("safe test target"),
                ))
            } else {
                Ok(EndpointResponseIntents::default())
            }
        })
    }
}

#[derive(Debug)]
struct LifecycleClock {
    now: AtomicU64,
    fail: AtomicBool,
}

impl LifecycleClock {
    fn new(now: u64) -> Self {
        Self {
            now: AtomicU64::new(now),
            fail: AtomicBool::new(false),
        }
    }
}

impl Clock for LifecycleClock {
    fn now(&self) -> Result<UnixMillis, ClockError> {
        if self.fail.load(Ordering::SeqCst) {
            Err(ClockError::timestamp_overflow())
        } else {
            Ok(UnixMillis::new(self.now.load(Ordering::SeqCst)))
        }
    }
}

#[derive(Default)]
struct AcceptanceControl {
    pause_next: AtomicBool,
    fail_next: AtomicBool,
    entered: Notify,
    release: Notify,
}

struct ControlledLedger {
    inner: Arc<MemoryInstanceLedger>,
    clock: Arc<LifecycleClock>,
    control: Arc<AcceptanceControl>,
}

#[async_trait::async_trait]
impl LiveInstanceLedger for ControlledLedger {
    async fn mount_instance(
        &self,
        record: MountInstanceRecord,
    ) -> Result<InstanceAuthority, LedgerError> {
        self.inner.mount_instance(record).await
    }

    async fn promote(&self, request: PromotionRecord) -> Result<PromotionOutcome, LedgerError> {
        self.inner.promote(request).await
    }

    async fn claim(&self, request: ClaimRequest) -> Result<ClaimOutcome, LedgerError> {
        self.inner.claim(request).await
    }

    async fn current_accepted_revision(
        &self,
        scope: &ScopeFingerprint,
        instance_id: &InstanceId,
    ) -> Result<Option<Revision>, LedgerError> {
        self.inner
            .current_accepted_revision(scope, instance_id)
            .await
    }

    async fn commit(
        &self,
        claim: &ClaimToken,
        outcome: AcceptedOutcome,
    ) -> Result<(), LedgerError> {
        if self.control.pause_next.swap(false, Ordering::SeqCst) {
            self.control.entered.notify_one();
            self.control.release.notified().await;
        }
        if self.control.fail_next.swap(false, Ordering::SeqCst) {
            self.clock.fail.store(true, Ordering::SeqCst);
            return self.inner.commit(claim, outcome).await;
        }
        self.inner.commit(claim, outcome).await
    }

    async fn abandon(&self, claim: &ClaimToken) -> Result<(), LedgerError> {
        self.inner.abandon(claim).await
    }

    fn abandon_on_drop(&self, claim: ClaimToken) {
        self.inner.abandon_on_drop(claim);
    }

    fn fence_on_drop(&self, claim: ClaimToken) {
        self.inner.fence_on_drop(claim);
    }
}

#[derive(Default)]
struct LifecycleTransactionControl {
    pause_begin_next: AtomicBool,
    pause_after_domain_commit_next: AtomicBool,
    domain_commits: AtomicUsize,
    rollbacks: AtomicUsize,
    begin_entered: Notify,
    begin_release: Notify,
    commit_effect_entered: Notify,
    commit_effect_release: Notify,
}

struct LifecycleTransaction(Arc<LifecycleTransactionControl>);

impl HostTransaction for LifecycleTransaction {
    fn commit(
        self: Box<Self>,
    ) -> suprnova_live::component::LiveFuture<'static, Result<(), HostError>> {
        let control = Arc::clone(&self.0);
        Box::pin(async move {
            control.domain_commits.fetch_add(1, Ordering::SeqCst);
            if control
                .pause_after_domain_commit_next
                .swap(false, Ordering::SeqCst)
            {
                control.commit_effect_entered.notify_one();
                control.commit_effect_release.notified().await;
            }
            Ok(())
        })
    }

    fn rollback(
        self: Box<Self>,
    ) -> suprnova_live::component::LiveFuture<'static, Result<(), HostError>> {
        let control = Arc::clone(&self.0);
        Box::pin(async move {
            control.rollbacks.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

struct LifecycleTransactionPort(Arc<LifecycleTransactionControl>);

impl TransactionPort for LifecycleTransactionPort {
    fn begin(
        &self,
    ) -> suprnova_live::component::LiveFuture<'_, Result<Box<dyn HostTransaction>, HostError>> {
        let control = Arc::clone(&self.0);
        Box::pin(async move {
            if control.pause_begin_next.swap(false, Ordering::SeqCst) {
                control.begin_entered.notify_one();
                control.begin_release.notified().await;
            }
            Ok(Box::new(LifecycleTransaction(control)) as Box<dyn HostTransaction>)
        })
    }
}

struct Reporter {
    calls: AtomicUsize,
    panic: bool,
}

impl AcceptedOutcomeReporter for Reporter {
    fn report(
        &self,
        _report: AcceptedExecutionReport,
    ) -> suprnova_live::component::LiveFuture<'_, Result<(), HostError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(!self.panic, "reporting panic");
        Box::pin(async { Err(HostError::new(HostErrorKind::Reporting)) })
    }
}

fn action<'a>(
    target: &'a mut dyn ActionTarget,
    _authorization: &'a AuthorizedAction,
    _arguments: &'a PreparedActionArguments,
) -> ActionFuture<'a, Result<ActionResult, ActionError>> {
    Box::pin(async move {
        let target = target
            .as_any_mut()
            .downcast_mut::<TraceFixture>()
            .ok_or_else(ActionError::dispatcher_contract)?;
        target.record("action");
        if target.failure == FailurePoint::Action {
            Err(ActionError::dispatcher_contract())
        } else {
            Ok(ActionResult::render())
        }
    })
}

fn metadata() -> &'static ComponentMetadata {
    static METADATA: OnceLock<ComponentMetadata> = OnceLock::new();
    METADATA.get_or_init(|| {
        ComponentMetadata::new(
            ComponentName::parse("tests.execution-faults").expect("component identity"),
            ViewName::parse("tests/execution-faults.html").expect("view identity"),
            ContractVersions::new(1, 1, 1, 1, 1).expect("versions"),
            vec![FieldMetadata::new(
                ModelField::parse("serial").expect("field identity"),
                FieldCategory::State,
                StateCodec::Json,
                true,
            )],
            vec![
                ActionMetadata::new_with_contract(
                    ActionName::parse("execute").expect("action identity"),
                    1,
                    ActionArgumentSchema::empty(),
                    AuthorizationRequirement::Current,
                    ValidationSelection::ComponentAndArguments,
                    TransactionPolicy::Required,
                )
                .expect("action metadata"),
            ],
        )
        .expect("component metadata")
    })
}

fn fixture_failure(fault: Fault) -> FailurePoint {
    match fault {
        Fault::Hydrate => FailurePoint::Hydrate,
        Fault::Bind => FailurePoint::Bind,
        Fault::BeforeAction => FailurePoint::BeforeAction,
        Fault::RollbackPanic => FailurePoint::BeforeAction,
        Fault::Action => FailurePoint::Action,
        Fault::AfterAction => FailurePoint::AfterAction,
        Fault::Render => FailurePoint::Render,
        Fault::Dehydrate => FailurePoint::Dehydrate,
        Fault::Sign => FailurePoint::InvalidSnapshotState,
        Fault::OutcomeValidation => FailurePoint::ExecutableRender,
        _ => FailurePoint::None,
    }
}

fn trusted_v2_context(
    component_metadata: &'static ComponentMetadata,
    authorization: Option<Arc<dyn ActionAuthorizationPort>>,
) -> TrustedLiveRequestContext {
    let descriptor = ComponentDescriptor::new(component_metadata.clone());
    let contract = ComponentContract::new(
        component_metadata.identity().clone(),
        descriptor.contract_digest().clone(),
        1,
        1,
        1,
    )
    .expect("component contract");
    let registry = ComponentRegistryBuilder::new()
        .register(descriptor)
        .expect("component registers")
        .build();
    let route = RouteIdentity::from_bytes(&bytes::<32>(0x30)).expect("route identity");
    let slot = IslandSlot::parse("trace").expect("slot identity");
    let catalog = MountCatalogBuilder::new()
        .register(
            &registry,
            MountCatalogEntry::new(
                suprnova_live::snapshot::ExpectedSeedV1::new(
                    contract,
                    BuildId::parse("build-lifecycle-tests").expect("build identity"),
                    route.clone(),
                    slot.clone(),
                    schema_set(),
                ),
                MountScopeRequirements::new(
                    ScopeRequirement::Required,
                    ScopeRequirement::Required,
                    ScopeRequirement::Required,
                ),
            ),
        )
        .expect("mount catalog entry")
        .build();
    let mut builder = SyntheticLiveRequestContextBuilder::new(
        catalog,
        MountSelection::new(
            route,
            slot,
            component_metadata.identity().clone(),
            component_metadata.contract_digest().clone(),
            2,
        ),
        fixture_host_scope(),
        UnixMillis::new(1_000),
        UnixMillis::new(2_000),
    );
    if let Some(authorization) = authorization {
        builder = builder.with_action_authorization(authorization);
    }
    builder.build().expect("trusted v2 context")
}

struct ClaimLifecycleFixture {
    service: ExecutionService,
    descriptor: ComponentDescriptor,
    context: TrustedLiveRequestContext,
    snapshot: VerifiedInstanceV1,
    encoded_snapshot: Vec<u8>,
    instance_id: InstanceId,
    ledger: Arc<MemoryInstanceLedger>,
    ledger_clock: Arc<LifecycleClock>,
    acceptance: Arc<AcceptanceControl>,
    transaction: Arc<LifecycleTransactionControl>,
    transaction_port: LifecycleTransactionPort,
    snapshot_limits: suprnova_live::snapshot::SnapshotLimits,
    protocol: u16,
}

impl ClaimLifecycleFixture {
    async fn new() -> Self {
        Self::build(None, 1).await
    }

    async fn new_with_pending_child(
        pending: Option<suprnova_live::component::composition::PendingChildParameters>,
    ) -> Self {
        Self::build(pending, 2).await
    }

    async fn build(
        pending: Option<suprnova_live::component::composition::PendingChildParameters>,
        protocol: u16,
    ) -> Self {
        let ledger_clock = Arc::new(LifecycleClock::new(1_000));
        let ledger = Arc::new(MemoryInstanceLedger::new(
            ledger_clock.clone(),
            LedgerLimits::new(100, 10_000, 8, 64).expect("ledger limits"),
        ));
        let acceptance = Arc::new(AcceptanceControl::default());
        let controlled_ledger = Arc::new(ControlledLedger {
            inner: Arc::clone(&ledger),
            clock: Arc::clone(&ledger_clock),
            control: Arc::clone(&acceptance),
        });
        let component_metadata = metadata();
        let component_control =
            FixtureControl::new_with_metadata(FailurePoint::None, component_metadata);
        if let Some(pending) = pending.as_ref() {
            component_control.set_render(IslandRender {
                body: bytes::Bytes::from_static(
                    b"<section data-suprnova-live-root=\"results\"></section>",
                ),
                assets: AssetSet::empty(),
                children: vec![ChildMount::pending_parameters(
                    IslandSlot::parse("results").expect("child slot"),
                    pending.clone(),
                )],
            });
        }
        let table = ActionTable::new(vec![ActionEntry::new(
            component_metadata.actions()[0].clone(),
            action,
        )])
        .expect("action table");
        let descriptor =
            ComponentDescriptor::with_hooks(component_metadata.clone(), install(component_control))
                .with_actions(table)
                .expect("matching action table");
        let authorization: Option<Arc<dyn ActionAuthorizationPort>> =
            Some(Arc::new(Authorization { fail: false }));
        let context = if protocol == 2 {
            trusted_v2_context(component_metadata, authorization)
        } else {
            trusted_context_for(component_metadata, authorization)
        };
        let instance_id = InstanceId::from_bytes(&bytes::<16>(0x70)).expect("instance identity");
        ledger
            .mount_instance(MountInstanceRecord::new(
                context.scope().clone(),
                instance_id.clone(),
                descriptor.contract_digest().clone(),
                Revision::new(0),
                UnixMillis::new(1_900),
            ))
            .await
            .expect("ledger authority");

        let keys = Arc::new(key_ring());
        let limits = suprnova_live::snapshot::SnapshotLimits::new(
            InputLimits::new(64 * 1024, 8, 1_024, 512).expect("composition input limits"),
            50,
            10_000,
            20_000,
            8,
            8,
        )
        .expect("composition snapshot limits");
        let contract = ComponentContract::new(
            component_metadata.identity().clone(),
            descriptor.contract_digest().clone(),
            1,
            1,
            1,
        )
        .expect("component contract");
        let build = BuildId::parse("build-lifecycle-tests").expect("build identity");
        let route = RouteIdentity::from_bytes(&bytes::<32>(0x30)).expect("route identity");
        let slot = IslandSlot::parse("trace").expect("slot identity");
        let mut fields = InstanceFieldsV1 {
            component: contract.clone(),
            build_id: build.clone(),
            route: route.clone(),
            slot: slot.clone(),
            key_id: keys.active_key_id().clone(),
            scope: context.scope().clone(),
            instance_id: instance_id.clone(),
            revision: Revision::new(0),
            issued_at: UnixMillis::new(1_000),
            expires_at: UnixMillis::new(1_900),
            state: CanonicalValue::Object(BTreeMap::from([(
                "serial".to_owned(),
                CanonicalValue::String("7".to_owned()),
            )])),
            memo: CanonicalValue::Object(BTreeMap::new()),
            extensions: BTreeMap::new(),
        };
        if let Some(pending) = pending.as_ref() {
            fields
                .set_composition_lineage(
                    CompositionLineageV1::new(
                        None,
                        vec![
                            CompositionChildLineageV1::new(
                                instance_id.clone(),
                                Revision::new(0),
                                pending.child().key().clone(),
                                pending.child().component_contract().clone(),
                                pending.child().instance_id().clone(),
                                1,
                            )
                            .expect("child lineage"),
                        ],
                    )
                    .expect("composition lineage"),
                )
                .expect("composition extension");
        }
        let body = InstanceBodyV1::new(fields, &schema_set(), &limits).expect("instance body");
        let encoded = body
            .sign(&keys, UnixMillis::new(1_000), &limits)
            .expect("signed instance");
        let expected = ExpectedInstanceV1::new(
            contract,
            build,
            route,
            slot,
            context.scope().clone(),
            schema_set(),
        );
        let snapshot = verify_instance(&encoded, &expected, &keys, UnixMillis::new(1_000), &limits)
            .expect("verified instance");
        let transaction = Arc::new(LifecycleTransactionControl::default());
        let transaction_port = LifecycleTransactionPort(Arc::clone(&transaction));
        let service = ExecutionService::new(
            controlled_ledger,
            Arc::new(ManualClock::new(1_000)),
            keys,
            limits.clone(),
            ViewRenderer::new(RenderLimits::standard()).expect("renderer"),
        );
        Self {
            service,
            descriptor,
            context,
            snapshot,
            encoded_snapshot: encoded,
            instance_id,
            ledger,
            ledger_clock,
            acceptance,
            transaction,
            transaction_port,
            snapshot_limits: limits,
            protocol,
        }
    }

    async fn execute(&self) -> ExecutionResult {
        self.execute_with_max_response_bytes(None).await
    }

    async fn execute_with_max_response_bytes(
        &self,
        max_response_bytes: Option<usize>,
    ) -> ExecutionResult {
        let response_context = if self.protocol == 2 {
            trusted_v2_context(metadata(), None)
        } else {
            trusted_context_for(metadata(), None)
        };
        let response_sealer = admitted_response_sealer_with_snapshot_limits(
            self.descriptor.clone(),
            response_context,
            &self.encoded_snapshot,
            Revision::new(0),
            0x45,
            max_response_bytes,
            self.snapshot_limits.clone(),
        )
        .await;
        self.execute_with_response_sealer(response_sealer).await
    }

    async fn execute_with_response_sealer(
        &self,
        response_sealing: VerifiedResponseSealing,
    ) -> ExecutionResult {
        let (response_sealer, response_binding) = response_sealing.into_parts();
        self.execute_with_response_parts(response_sealer, response_binding)
            .await
    }

    async fn execute_with_response_parts(
        &self,
        response_sealer: suprnova_live::endpoint::AcceptedResponseSealer,
        response_binding: suprnova_live::endpoint::AcceptedResponseRequestBinding,
    ) -> ExecutionResult {
        let validation_engine =
            suprnova_live::validation::ValidationEngine::new(16).expect("validation engine");
        let validation_port = Validation { fail: false };
        let action_name = ActionName::parse("execute").expect("action name");
        let input_limits = InputLimits::default();
        let proposal_schema = ModelBindingSchema::new(Vec::new()).expect("proposal schema");
        let proposals =
            ProposalBatch::prepare(&proposal_schema, Vec::new(), &ProposalLimits::default())
                .expect("proposal batch");
        let trace = Trace::default();
        self.service
            .execute_instanced(InstancedActionRequest::new(
                &self.descriptor,
                &self.context,
                browser_context(),
                &self.snapshot,
                idempotency(0x50),
                digest(0x60),
                ActionExecutionRequest::new(
                    &action_name,
                    RawActionArguments::empty(),
                    &input_limits,
                    &validation_engine,
                    &validation_port,
                    BagPolicy::Replace,
                    Some(&self.transaction_port),
                    &trace,
                )
                .with_response_sealer(response_sealer, response_binding)
                .with_proposals(&proposals),
            ))
            .await
    }

    fn inspection(&self) -> suprnova_live::ledger::LedgerInspection {
        self.ledger
            .inspect(self.context.scope(), &self.instance_id)
            .expect("ledger inspection")
            .expect("instance authority")
    }
}

#[tokio::test]
async fn accepted_parent_builds_one_changed_child_delivery_before_commit() {
    let (pending, _) = child_parameter_support::pending_parameters("signed-update");
    let child_instance = pending.child().instance_id().clone();
    let parameter_hash =
        suprnova_live::identity::ContentDigest::from_bytes(pending.parameter_value().as_bytes())
            .expect("parameter hash");
    let fixture = ClaimLifecycleFixture::new_with_pending_child(Some(pending)).await;

    let ExecutionResult::Accepted(accepted) = fixture.execute().await else {
        panic!("consistent changed child must be accepted");
    };

    let [delivery] = accepted.child_deliveries() else {
        panic!("exactly one changed child delivery is produced");
    };
    assert_eq!(delivery.child_instance(), &child_instance);
    assert_eq!(delivery.parameter_hash(), &parameter_hash);
    assert!(!delivery.envelope().is_empty());
    assert_eq!(fixture.transaction.domain_commits.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.inspection().accepted_outcome_count(), 1);
}

#[tokio::test]
async fn child_delivery_response_sealing_failure_rolls_back_before_acceptance() {
    let (pending, _) = child_parameter_support::pending_parameters("bounded-update");
    let fixture = ClaimLifecycleFixture::new_with_pending_child(Some(pending)).await;

    let ExecutionResult::RefreshRequired(refresh) =
        fixture.execute_with_max_response_bytes(Some(128)).await
    else {
        panic!("an unsealable complete response must not expose accepted bytes");
    };

    assert_eq!(refresh.reason(), ExecutionRefreshReason::ExecutionFailed);
    assert_eq!(fixture.transaction.domain_commits.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.transaction.rollbacks.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.inspection().accepted_outcome_count(), 0);
}

#[tokio::test]
async fn foreign_request_sealer_fails_before_host_or_ledger_acceptance() {
    let fixture = ClaimLifecycleFixture::new().await;
    let admitted_request = admitted_response_sealer(
        fixture.descriptor.clone(),
        trusted_context_for(metadata(), None),
        &fixture.encoded_snapshot,
        Revision::new(0),
        0x45,
        None,
    )
    .await;
    let foreign_request = admitted_response_sealer(
        fixture.descriptor.clone(),
        trusted_context_for(metadata(), None),
        &fixture.encoded_snapshot,
        Revision::new(0),
        0x46,
        None,
    )
    .await;
    let (foreign_request_sealer, _) = foreign_request.into_parts();
    let (_, admitted_request_binding) = admitted_request.into_parts();

    let ExecutionResult::RefreshRequired(refresh) = fixture
        .execute_with_response_parts(foreign_request_sealer, admitted_request_binding)
        .await
    else {
        panic!("a sealer from another verified request must fail before acceptance");
    };

    assert_eq!(refresh.reason(), ExecutionRefreshReason::ExecutionFailed);
    assert_eq!(fixture.transaction.domain_commits.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.inspection().accepted_outcome_count(), 0);
}

#[tokio::test]
async fn semantic_request_sealer_swap_fails_before_host_or_ledger_acceptance() {
    let fixture = ClaimLifecycleFixture::new().await;
    let request_a = admitted_response_sealer_with_semantics(
        fixture.descriptor.clone(),
        trusted_context_for(metadata(), None),
        &fixture.encoded_snapshot,
        Revision::new(0),
        0x45,
        0x65,
        "execute",
        serde_json::json!({"value": "request-a"}),
        serde_json::json!({"serial": "model-a"}),
    )
    .await;
    let request_b = admitted_response_sealer_with_semantics(
        fixture.descriptor.clone(),
        trusted_context_for(metadata(), None),
        &fixture.encoded_snapshot,
        Revision::new(0),
        0x45,
        0x66,
        "execute",
        serde_json::json!({"value": "request-b"}),
        serde_json::json!({"serial": "model-b"}),
    )
    .await;
    let (request_a_sealer, _) = request_a.into_parts();
    let (_, request_b_binding) = request_b.into_parts();

    let ExecutionResult::RefreshRequired(refresh) = fixture
        .execute_with_response_parts(request_a_sealer, request_b_binding)
        .await
    else {
        panic!("a sealer from another semantic request must fail before acceptance");
    };

    assert_eq!(refresh.reason(), ExecutionRefreshReason::ExecutionFailed);
    assert_eq!(fixture.transaction.domain_commits.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.inspection().accepted_outcome_count(), 0);
}

fn assert_consumed_refresh(result: ExecutionResult) {
    let ExecutionResult::RefreshRequired(refresh) = result else {
        panic!("fenced base revision must require refresh");
    };
    assert_eq!(
        refresh.reason(),
        ExecutionRefreshReason::Ledger(RefreshReason::Consumed)
    );
    assert_eq!(refresh.retry_legality(), RetryLegality::Prohibited);
}

#[tokio::test]
async fn cancellation_before_host_commit_attempt_releases_rollbackable_claim_for_retry() {
    let fixture = ClaimLifecycleFixture::new().await;
    fixture
        .transaction
        .pause_begin_next
        .store(true, Ordering::SeqCst);

    {
        let execution = fixture.execute();
        tokio::pin!(execution);
        tokio::select! {
            () = fixture.transaction.begin_entered.notified() => {}
            result = &mut execution => panic!("host transaction begin unexpectedly completed: {result:?}"),
        }
        assert_eq!(fixture.transaction.domain_commits.load(Ordering::SeqCst), 0);
    }

    let ExecutionResult::Accepted(accepted) = fixture.execute().await else {
        panic!("rollbackable pre-commit cancellation must allow exact retry");
    };
    assert_eq!(accepted.revision(), Revision::new(1));
    assert_eq!(fixture.transaction.domain_commits.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.inspection().phase(), LedgerPhase::Ready);
    assert_eq!(fixture.inspection().accepted_outcome_count(), 1);
}

#[tokio::test]
async fn cancellation_after_physical_host_commit_before_ack_fences_base_revision() {
    let fixture = ClaimLifecycleFixture::new().await;
    fixture
        .transaction
        .pause_after_domain_commit_next
        .store(true, Ordering::SeqCst);

    {
        let execution = fixture.execute();
        tokio::pin!(execution);
        tokio::select! {
            () = fixture.transaction.commit_effect_entered.notified() => {}
            result = &mut execution => panic!("indeterminate host commit unexpectedly completed: {result:?}"),
        }
        assert_eq!(fixture.transaction.domain_commits.load(Ordering::SeqCst), 1);
    }

    assert_consumed_refresh(fixture.execute().await);
    assert_eq!(fixture.transaction.domain_commits.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.inspection().phase(), LedgerPhase::Consumed);
    assert_eq!(fixture.inspection().accepted_outcome_count(), 0);
}

#[tokio::test]
async fn cancellation_after_host_commit_fences_base_revision_from_replay() {
    let fixture = ClaimLifecycleFixture::new().await;
    fixture.acceptance.pause_next.store(true, Ordering::SeqCst);

    {
        let execution = fixture.execute();
        tokio::pin!(execution);
        tokio::select! {
            () = fixture.acceptance.entered.notified() => {}
            result = &mut execution => panic!("ledger acceptance unexpectedly completed: {result:?}"),
        }
        assert_eq!(fixture.transaction.domain_commits.load(Ordering::SeqCst), 1);
    }

    assert_consumed_refresh(fixture.execute().await);
    assert_eq!(fixture.transaction.domain_commits.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.inspection().phase(), LedgerPhase::Consumed);
    assert_eq!(fixture.inspection().accepted_outcome_count(), 0);
}

#[tokio::test]
async fn ledger_failure_after_host_commit_fences_base_revision_from_replay() {
    let fixture = ClaimLifecycleFixture::new().await;
    fixture.acceptance.fail_next.store(true, Ordering::SeqCst);

    let ExecutionResult::RefreshRequired(first) = fixture.execute().await else {
        panic!("failed ledger acceptance must require refresh");
    };
    assert_eq!(
        first.reason(),
        ExecutionRefreshReason::LedgerAcceptanceFailed
    );
    assert_eq!(fixture.transaction.domain_commits.load(Ordering::SeqCst), 1);

    fixture.ledger_clock.fail.store(false, Ordering::SeqCst);
    assert_consumed_refresh(fixture.execute().await);
    assert_eq!(fixture.transaction.domain_commits.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.inspection().phase(), LedgerPhase::Consumed);
    assert_eq!(fixture.inspection().accepted_outcome_count(), 0);
}

fn expected_trace(fault: Fault) -> &'static [ExecutionPhase] {
    use ExecutionPhase as Phase;
    match fault {
        Fault::Claim => &[Phase::Claim],
        Fault::Hydrate => &[Phase::Claim, Phase::Hydrate],
        Fault::Bind => &[Phase::Claim, Phase::Hydrate, Phase::Bind],
        Fault::Authorize => &[Phase::Claim, Phase::Hydrate, Phase::Bind, Phase::Authorize],
        Fault::Validate => &[
            Phase::Claim,
            Phase::Hydrate,
            Phase::Bind,
            Phase::Authorize,
            Phase::Validate,
        ],
        Fault::TransactionBegin | Fault::TransactionBeginPanic => &[
            Phase::Claim,
            Phase::Hydrate,
            Phase::Bind,
            Phase::Authorize,
            Phase::Validate,
            Phase::TransactionBegin,
        ],
        Fault::BeforeAction | Fault::RollbackPanic => &[
            Phase::Claim,
            Phase::Hydrate,
            Phase::Bind,
            Phase::Authorize,
            Phase::Validate,
            Phase::TransactionBegin,
            Phase::BeforeAction,
        ],
        Fault::Action => &[
            Phase::Claim,
            Phase::Hydrate,
            Phase::Bind,
            Phase::Authorize,
            Phase::Validate,
            Phase::TransactionBegin,
            Phase::BeforeAction,
            Phase::Action,
        ],
        Fault::AfterAction => &[
            Phase::Claim,
            Phase::Hydrate,
            Phase::Bind,
            Phase::Authorize,
            Phase::Validate,
            Phase::TransactionBegin,
            Phase::BeforeAction,
            Phase::Action,
            Phase::AfterAction,
        ],
        Fault::Render => &[
            Phase::Claim,
            Phase::Hydrate,
            Phase::Bind,
            Phase::Authorize,
            Phase::Validate,
            Phase::TransactionBegin,
            Phase::BeforeAction,
            Phase::Action,
            Phase::AfterAction,
            Phase::Render,
        ],
        Fault::Dehydrate => &[
            Phase::Claim,
            Phase::Hydrate,
            Phase::Bind,
            Phase::Authorize,
            Phase::Validate,
            Phase::TransactionBegin,
            Phase::BeforeAction,
            Phase::Action,
            Phase::AfterAction,
            Phase::Render,
            Phase::Dehydrate,
        ],
        Fault::Sign => &[
            Phase::Claim,
            Phase::Hydrate,
            Phase::Bind,
            Phase::Authorize,
            Phase::Validate,
            Phase::TransactionBegin,
            Phase::BeforeAction,
            Phase::Action,
            Phase::AfterAction,
            Phase::Render,
            Phase::Dehydrate,
            Phase::Sign,
        ],
        Fault::OutcomeValidation => &[
            Phase::Claim,
            Phase::Hydrate,
            Phase::Bind,
            Phase::Authorize,
            Phase::Validate,
            Phase::TransactionBegin,
            Phase::BeforeAction,
            Phase::Action,
            Phase::AfterAction,
            Phase::Render,
            Phase::Dehydrate,
            Phase::Sign,
            Phase::OutcomeValidation,
        ],
        Fault::ResponseIntent | Fault::ResponseIntentShape => &[
            Phase::Claim,
            Phase::Hydrate,
            Phase::Bind,
            Phase::Authorize,
            Phase::Validate,
            Phase::TransactionBegin,
            Phase::BeforeAction,
            Phase::Action,
            Phase::AfterAction,
            Phase::Render,
            Phase::Dehydrate,
            Phase::Sign,
            Phase::OutcomeValidation,
            Phase::ResponseIntentPreparation,
        ],
        Fault::ResponseSealing => &[
            Phase::Claim,
            Phase::Hydrate,
            Phase::Bind,
            Phase::Authorize,
            Phase::Validate,
            Phase::TransactionBegin,
            Phase::BeforeAction,
            Phase::Action,
            Phase::AfterAction,
            Phase::Render,
            Phase::Dehydrate,
            Phase::Sign,
            Phase::OutcomeValidation,
            Phase::ResponseIntentPreparation,
            Phase::ResponseSealing,
        ],
        Fault::HostCommit | Fault::HostCommitPanic => &[
            Phase::Claim,
            Phase::Hydrate,
            Phase::Bind,
            Phase::Authorize,
            Phase::Validate,
            Phase::TransactionBegin,
            Phase::BeforeAction,
            Phase::Action,
            Phase::AfterAction,
            Phase::Render,
            Phase::Dehydrate,
            Phase::Sign,
            Phase::OutcomeValidation,
            Phase::ResponseIntentPreparation,
            Phase::ResponseSealing,
            Phase::HostCommit,
        ],
        Fault::LedgerAcceptance => &[
            Phase::Claim,
            Phase::Hydrate,
            Phase::Bind,
            Phase::Authorize,
            Phase::Validate,
            Phase::TransactionBegin,
            Phase::BeforeAction,
            Phase::Action,
            Phase::AfterAction,
            Phase::Render,
            Phase::Dehydrate,
            Phase::Sign,
            Phase::OutcomeValidation,
            Phase::ResponseIntentPreparation,
            Phase::ResponseSealing,
            Phase::HostCommit,
            Phase::LedgerAcceptance,
        ],
        Fault::Reporting | Fault::ReportingPanic => &[
            Phase::Claim,
            Phase::Hydrate,
            Phase::Bind,
            Phase::Authorize,
            Phase::Validate,
            Phase::TransactionBegin,
            Phase::BeforeAction,
            Phase::Action,
            Phase::AfterAction,
            Phase::Render,
            Phase::Dehydrate,
            Phase::Sign,
            Phase::OutcomeValidation,
            Phase::ResponseIntentPreparation,
            Phase::ResponseSealing,
            Phase::HostCommit,
            Phase::LedgerAcceptance,
            Phase::Reporting,
        ],
    }
}

#[tokio::test]
async fn every_locked_boundary_has_exact_recovery_and_durability_semantics() {
    let cases = [
        Fault::Claim,
        Fault::Hydrate,
        Fault::Bind,
        Fault::Authorize,
        Fault::Validate,
        Fault::TransactionBegin,
        Fault::TransactionBeginPanic,
        Fault::BeforeAction,
        Fault::RollbackPanic,
        Fault::Action,
        Fault::AfterAction,
        Fault::Render,
        Fault::Dehydrate,
        Fault::Sign,
        Fault::OutcomeValidation,
        Fault::ResponseIntent,
        Fault::ResponseIntentShape,
        Fault::ResponseSealing,
        Fault::HostCommit,
        Fault::HostCommitPanic,
        Fault::LedgerAcceptance,
        Fault::Reporting,
        Fault::ReportingPanic,
    ];

    for fault in cases {
        let ledger_clock = Arc::new(ManualClock::new(1_000));
        let ledger = Arc::new(ledger(ledger_clock.clone(), 8));
        let service_clock = Arc::new(ManualClock::new(1_000));
        let component_metadata = metadata();
        let control = FixtureControl::new_with_metadata(fixture_failure(fault), component_metadata);
        let table = ActionTable::new(vec![ActionEntry::new(
            component_metadata.actions()[0].clone(),
            action,
        )])
        .expect("action table");
        let descriptor =
            ComponentDescriptor::with_hooks(component_metadata.clone(), install(control.clone()))
                .with_actions(table)
                .expect("matching action table");
        let context = trusted_context_for(
            component_metadata,
            Some(Arc::new(Authorization {
                fail: fault == Fault::Authorize,
            })),
        );
        let instance_id = suprnova_live::identity::InstanceId::from_bytes(&bytes::<16>(0x70))
            .expect("instance identity");
        if fault != Fault::Claim {
            ledger
                .mount_instance(MountInstanceRecord::new(
                    context.scope().clone(),
                    instance_id.clone(),
                    descriptor.contract_digest().clone(),
                    Revision::new(0),
                    UnixMillis::new(1_900),
                ))
                .await
                .expect("ledger authority");
        }
        let keys = Arc::new(key_ring());
        let limits = snapshot_limits();
        let contract = ComponentContract::new(
            component_metadata.identity().clone(),
            descriptor.contract_digest().clone(),
            1,
            1,
            1,
        )
        .expect("component contract");
        let build = BuildId::parse("build-lifecycle-tests").expect("build identity");
        let route = RouteIdentity::from_bytes(&bytes::<32>(0x30)).expect("route identity");
        let slot = IslandSlot::parse("trace").expect("slot identity");
        let body = InstanceBodyV1::new(
            InstanceFieldsV1 {
                component: contract.clone(),
                build_id: build.clone(),
                route: route.clone(),
                slot: slot.clone(),
                key_id: keys.active_key_id().clone(),
                scope: context.scope().clone(),
                instance_id: instance_id.clone(),
                revision: Revision::new(0),
                issued_at: UnixMillis::new(1_000),
                expires_at: UnixMillis::new(1_900),
                state: CanonicalValue::Object(BTreeMap::from([(
                    "serial".to_owned(),
                    CanonicalValue::String("7".to_owned()),
                )])),
                memo: CanonicalValue::Object(BTreeMap::new()),
                extensions: BTreeMap::new(),
            },
            &schema_set(),
            &limits,
        )
        .expect("instance body");
        let encoded = body
            .sign(&keys, UnixMillis::new(1_000), &limits)
            .expect("signed instance");
        let expected = ExpectedInstanceV1::new(
            contract,
            build,
            route,
            slot,
            context.scope().clone(),
            schema_set(),
        );
        let snapshot = verify_instance(&encoded, &expected, &keys, UnixMillis::new(1_000), &limits)
            .expect("verified instance");
        let reporter = Arc::new(Reporter {
            calls: AtomicUsize::new(0),
            panic: fault == Fault::ReportingPanic,
        });
        let mut service = ExecutionService::new(
            ledger.clone(),
            service_clock,
            keys,
            limits,
            ViewRenderer::new(RenderLimits::standard()).expect("renderer"),
        );
        if matches!(fault, Fault::Reporting | Fault::ReportingPanic) {
            service = service.with_reporter(reporter.clone());
        }
        let transaction_control = Arc::new(TransactionControl {
            fault,
            domain_commits: AtomicUsize::new(0),
            rollbacks: AtomicUsize::new(0),
            ledger_clock,
        });
        let transaction_port = TestTransactionPort(transaction_control.clone());
        let response_intent_port = TestResponseIntentPort { fault };
        let response_sealer = admitted_response_sealer(
            descriptor.clone(),
            trusted_context_for(component_metadata, None),
            &encoded,
            Revision::new(0),
            0x45,
            (fault == Fault::ResponseSealing).then_some(64),
        )
        .await;
        let (response_sealer, response_binding) = response_sealer.into_parts();
        let trace = Trace::default();
        let validation_engine =
            suprnova_live::validation::ValidationEngine::new(16).expect("validation engine");
        let validation_port = Validation {
            fail: fault == Fault::Validate,
        };
        let action_name = ActionName::parse("execute").expect("action name");
        let input_limits = InputLimits::default();
        let proposal_schema = ModelBindingSchema::new(Vec::new()).expect("empty proposal schema");
        let proposals =
            ProposalBatch::prepare(&proposal_schema, Vec::new(), &ProposalLimits::default())
                .expect("empty proposal batch");
        let result = service
            .execute_instanced(InstancedActionRequest::new(
                &descriptor,
                &context,
                browser_context(),
                &snapshot,
                idempotency(0x50),
                digest(0x60),
                ActionExecutionRequest::new(
                    &action_name,
                    RawActionArguments::empty(),
                    &input_limits,
                    &validation_engine,
                    &validation_port,
                    BagPolicy::Replace,
                    Some(&transaction_port),
                    &trace,
                )
                .with_response_intent_preparation(&response_intent_port)
                .with_response_sealer(response_sealer, response_binding)
                .with_proposals(&proposals),
            ))
            .await;

        assert_eq!(
            trace.0.lock().expect("execution trace lock").as_slice(),
            expected_trace(fault),
            "fault {fault:?}"
        );
        assert_eq!(
            transaction_control.domain_commits.load(Ordering::SeqCst),
            usize::from(matches!(
                fault,
                Fault::LedgerAcceptance | Fault::Reporting | Fault::ReportingPanic
            )),
            "fault {fault:?}"
        );
        assert_eq!(
            transaction_control.rollbacks.load(Ordering::SeqCst),
            usize::from(matches!(
                fault,
                Fault::BeforeAction
                    | Fault::RollbackPanic
                    | Fault::Action
                    | Fault::AfterAction
                    | Fault::Render
                    | Fault::Dehydrate
                    | Fault::Sign
                    | Fault::OutcomeValidation
                    | Fault::ResponseIntent
                    | Fault::ResponseIntentShape
                    | Fault::ResponseSealing
            )),
            "fault {fault:?}"
        );
        if matches!(fault, Fault::Reporting | Fault::ReportingPanic) {
            let ExecutionResult::Accepted(accepted) = result else {
                panic!("reporting failure cannot rewrite acceptance");
            };
            assert!(accepted.reporting_failed());
            assert!(accepted.action_executed());
            assert_eq!(reporter.calls.load(Ordering::SeqCst), 1);
        } else {
            let ExecutionResult::RefreshRequired(refresh) = result else {
                panic!("fault {fault:?} must require refresh");
            };
            assert_eq!(refresh.retry_legality(), RetryLegality::Prohibited);
            let expected_reason = match fault {
                Fault::Claim => {
                    ExecutionRefreshReason::Ledger(suprnova_live::ledger::RefreshReason::Missing)
                }
                Fault::HostCommit | Fault::HostCommitPanic => {
                    ExecutionRefreshReason::HostCommitFailed
                }
                Fault::LedgerAcceptance => ExecutionRefreshReason::LedgerAcceptanceFailed,
                _ => ExecutionRefreshReason::ExecutionFailed,
            };
            assert_eq!(refresh.reason(), expected_reason, "fault {fault:?}");
        }

        if fault != Fault::Claim {
            let inspection = ledger
                .inspect(context.scope(), &instance_id)
                .expect("ledger inspection")
                .expect("instance remains inspectable");
            if matches!(fault, Fault::Reporting | Fault::ReportingPanic) {
                assert_eq!(inspection.phase(), LedgerPhase::Ready);
                assert_eq!(inspection.accepted_outcome_count(), 1);
            } else {
                assert_eq!(inspection.phase(), LedgerPhase::Consumed, "fault {fault:?}");
                assert_eq!(inspection.accepted_outcome_count(), 0, "fault {fault:?}");
            }
        }
    }
}
