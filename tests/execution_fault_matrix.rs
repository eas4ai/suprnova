//! Deterministic faults at every accepted-outcome coordination boundary.

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
use suprnova_live::execution::{
    AcceptedExecutionReport, AcceptedOutcomeReporter, ActionExecutionRequest, ExecutionPhase,
    ExecutionRefreshReason, ExecutionResult, ExecutionService, ExecutionTracePort, HostError,
    HostErrorKind, HostTransaction, InstancedActionRequest, RetryLegality, TransactionPort,
};
use suprnova_live::host::TrustedLiveRequestContext;
use suprnova_live::identity::{
    ActionName, BuildId, ComponentName, InstanceId, IslandSlot, ModelField, Revision,
    RouteIdentity, UnixMillis, ViewName,
};
use suprnova_live::ledger::{
    AcceptedOutcome, ClaimOutcome, ClaimRequest, ClaimToken, InstanceAuthority, LedgerError,
    LedgerLimits, LedgerPhase, LiveInstanceLedger, MemoryInstanceLedger, MountInstanceRecord,
    PromotionOutcome, PromotionRecord, RefreshReason,
};
use suprnova_live::limits::InputLimits;
use suprnova_live::metadata::{ActionMetadata, ComponentMetadata, ContractVersions, FieldMetadata};
use suprnova_live::registry::ComponentDescriptor;
use suprnova_live::snapshot::state::{FieldCategory, StateCodec};
use suprnova_live::snapshot::{
    ComponentContract, ExpectedInstanceV1, InstanceBodyV1, InstanceFieldsV1, VerifiedInstanceV1,
    verify_instance,
};
use suprnova_live::state::{ModelBindingSchema, ProposalBatch, ProposalLimits};
use suprnova_live::validation::{
    BagPolicy, ValidationFuture, ValidationPort, ValidationPortError, ValidationRequest,
    ValidationSelection,
};
use suprnova_live::view::{RenderLimits, ViewRenderer};
use tokio::sync::Notify;

use component_support::{
    FailurePoint, FixtureControl, ManualClock, TraceFixture, browser_context, bytes, digest,
    idempotency, install, key_ring, ledger, schema_set, snapshot_limits, trusted_context_for,
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
    pause_next: AtomicBool,
    domain_commits: AtomicUsize,
    rollbacks: AtomicUsize,
    entered: Notify,
    release: Notify,
}

struct LifecycleTransaction(Arc<LifecycleTransactionControl>);

impl HostTransaction for LifecycleTransaction {
    fn commit(
        self: Box<Self>,
    ) -> suprnova_live::component::LiveFuture<'static, Result<(), HostError>> {
        let control = Arc::clone(&self.0);
        Box::pin(async move {
            if control.pause_next.swap(false, Ordering::SeqCst) {
                control.entered.notify_one();
                control.release.notified().await;
            }
            control.domain_commits.fetch_add(1, Ordering::SeqCst);
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
        Box::pin(
            async move { Ok(Box::new(LifecycleTransaction(control)) as Box<dyn HostTransaction>) },
        )
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

struct ClaimLifecycleFixture {
    service: ExecutionService,
    descriptor: ComponentDescriptor,
    context: TrustedLiveRequestContext,
    snapshot: VerifiedInstanceV1,
    instance_id: InstanceId,
    ledger: Arc<MemoryInstanceLedger>,
    ledger_clock: Arc<LifecycleClock>,
    acceptance: Arc<AcceptanceControl>,
    transaction: Arc<LifecycleTransactionControl>,
    transaction_port: LifecycleTransactionPort,
}

impl ClaimLifecycleFixture {
    async fn new() -> Self {
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
        let table = ActionTable::new(vec![ActionEntry::new(
            component_metadata.actions()[0].clone(),
            action,
        )])
        .expect("action table");
        let descriptor =
            ComponentDescriptor::with_hooks(component_metadata.clone(), install(component_control))
                .with_actions(table)
                .expect("matching action table");
        let context = trusted_context_for(
            component_metadata,
            Some(Arc::new(Authorization { fail: false })),
        );
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
        let transaction = Arc::new(LifecycleTransactionControl::default());
        let transaction_port = LifecycleTransactionPort(Arc::clone(&transaction));
        let service = ExecutionService::new(
            controlled_ledger,
            Arc::new(ManualClock::new(1_000)),
            keys,
            limits,
            ViewRenderer::new(RenderLimits::standard()).expect("renderer"),
        );
        Self {
            service,
            descriptor,
            context,
            snapshot,
            instance_id,
            ledger,
            ledger_clock,
            acceptance,
            transaction,
            transaction_port,
        }
    }

    async fn execute(&self) -> ExecutionResult {
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
async fn cancellation_before_host_commit_releases_rollbackable_claim_for_retry() {
    let fixture = ClaimLifecycleFixture::new().await;
    fixture.transaction.pause_next.store(true, Ordering::SeqCst);

    {
        let execution = fixture.execute();
        tokio::pin!(execution);
        tokio::select! {
            () = fixture.transaction.entered.notified() => {}
            result = &mut execution => panic!("host commit unexpectedly completed: {result:?}"),
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
