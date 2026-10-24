//! Exact server action phase ordering and explicit transaction policy.

mod component_support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use suprnova_live::action::{
    ActionArgumentSchema, ActionEntry, ActionError, ActionFuture, ActionResult, ActionTable,
    ActionTarget, AuthorizationRequirement, AuthorizedAction, PreparedActionArguments,
    RawActionArguments, TransactionPolicy,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::component::{ComponentExecutor, HydrationContext, RenderContext};
use suprnova_live::execution::{
    ActionExecutionRequest, ExecutionPhase, ExecutionTracePort, HostError, HostTransaction,
    TransactionPort,
};
use suprnova_live::identity::{
    ActionName, ComponentName, InstanceId, ModelField, Revision, UnixMillis, ViewName,
};
use suprnova_live::limits::InputLimits;
use suprnova_live::metadata::{ActionMetadata, ComponentMetadata, ContractVersions, FieldMetadata};
use suprnova_live::registry::ComponentDescriptor;
use suprnova_live::snapshot::state::{FieldCategory, StateCodec};
use suprnova_live::validation::{
    BagPolicy, ValidationFuture, ValidationPort, ValidationPortError, ValidationRequest,
    ValidationSelection,
};

use component_support::{
    FailurePoint, FixtureControl, TraceFixture, bytes, install, metadata, trusted_context_for,
    trusted_context_with_authorization,
};

fn execute<'a>(
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
        Ok(ActionResult::render())
    })
}

struct AllowAuthorization;

impl suprnova_live::action::ActionAuthorizationPort for AllowAuthorization {
    fn authorize<'a>(
        &'a self,
        _request: suprnova_live::action::ActionAuthorizationRequest<'a>,
    ) -> ActionFuture<'a, Result<suprnova_live::action::AuthorizationDecision, ActionError>> {
        Box::pin(async { Ok(suprnova_live::action::AuthorizationDecision::Allow) })
    }
}

struct PassValidation;

impl ValidationPort for PassValidation {
    fn validate<'a>(
        &'a self,
        _request: ValidationRequest<'a>,
    ) -> ValidationFuture<
        'a,
        Result<Vec<suprnova_live::validation::ValidationIssue>, ValidationPortError>,
    > {
        Box::pin(async { Ok(Vec::new()) })
    }
}

#[derive(Default)]
struct Trace(Mutex<Vec<ExecutionPhase>>);

impl ExecutionTracePort for Trace {
    fn record(&self, phase: ExecutionPhase) {
        self.0.lock().expect("execution trace lock").push(phase);
    }
}

#[derive(Default)]
struct TransactionControl {
    begun: AtomicUsize,
    committed: AtomicUsize,
    rolled_back: AtomicUsize,
}

struct RecordingTransaction(Arc<TransactionControl>);

impl HostTransaction for RecordingTransaction {
    fn commit(
        self: Box<Self>,
    ) -> suprnova_live::component::LiveFuture<'static, Result<(), HostError>> {
        self.0.committed.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }

    fn rollback(
        self: Box<Self>,
    ) -> suprnova_live::component::LiveFuture<'static, Result<(), HostError>> {
        self.0.rolled_back.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }
}

struct SharedTransactionPort(Arc<TransactionControl>);

impl TransactionPort for SharedTransactionPort {
    fn begin(
        &self,
    ) -> suprnova_live::component::LiveFuture<'_, Result<Box<dyn HostTransaction>, HostError>> {
        self.0.begun.fetch_add(1, Ordering::SeqCst);
        let control = self.0.clone();
        Box::pin(
            async move { Ok(Box::new(RecordingTransaction(control)) as Box<dyn HostTransaction>) },
        )
    }
}

fn required_transaction_metadata() -> &'static ComponentMetadata {
    static METADATA: OnceLock<ComponentMetadata> = OnceLock::new();
    METADATA.get_or_init(|| {
        ComponentMetadata::new(
            ComponentName::parse("tests.transaction").expect("component identity"),
            ViewName::parse("tests/transaction.html").expect("view identity"),
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

#[tokio::test]
async fn action_phases_are_exact_and_no_transaction_policy_never_fabricates_one() {
    let control = FixtureControl::new(FailurePoint::None);
    let table = ActionTable::new(vec![ActionEntry::new(
        metadata().actions()[0].clone(),
        execute,
    )])
    .expect("action table");
    let descriptor = ComponentDescriptor::with_hooks(metadata().clone(), install(control.clone()))
        .with_actions(table)
        .expect("matching action table");
    let request_context = trusted_context_with_authorization(Arc::new(AllowAuthorization));
    let instance = InstanceId::from_bytes(&bytes::<16>(0x70)).expect("instance identity");
    let render = RenderContext::new(
        &request_context,
        &instance,
        Revision::new(1),
        UnixMillis::new(1_900),
    );
    let state = CanonicalValue::Object(std::collections::BTreeMap::from([(
        "serial".to_owned(),
        CanonicalValue::String("7".to_owned()),
    )]));
    let hydration = HydrationContext::new(render, &state);
    let trace = Trace::default();
    let validation =
        suprnova_live::validation::ValidationEngine::new(16).expect("validation engine");
    let action = ActionName::parse("execute").expect("action name");
    let input_limits = InputLimits::default();
    let request = ActionExecutionRequest::new(
        &action,
        RawActionArguments::empty(),
        &input_limits,
        &validation,
        &PassValidation,
        BagPolicy::Replace,
        None,
        &trace,
    );

    let output = ComponentExecutor::new()
        .coordinated_action(&descriptor, &hydration, request)
        .await
        .expect("coordinated action");

    assert!(output.transaction().is_none());
    assert_eq!(
        trace.0.lock().expect("execution trace lock").as_slice(),
        [
            ExecutionPhase::Hydrate,
            ExecutionPhase::Bind,
            ExecutionPhase::Authorize,
            ExecutionPhase::Validate,
            ExecutionPhase::BeforeAction,
            ExecutionPhase::Action,
            ExecutionPhase::AfterAction,
            ExecutionPhase::Render,
            ExecutionPhase::Dehydrate,
        ]
    );
    assert_eq!(
        control.values(),
        [
            "reconstruct",
            "hydrated",
            "before_action",
            "action",
            "after_action",
            "rendering",
            "render",
            "rendered",
            "dehydrating",
            "dehydrate",
            "memo",
            "teardown",
        ]
    );
}

#[tokio::test]
async fn required_transaction_begins_after_validation_and_remains_uncommitted() {
    let component_metadata = required_transaction_metadata();
    let control = FixtureControl::new_with_metadata(FailurePoint::None, component_metadata);
    let table = ActionTable::new(vec![ActionEntry::new(
        component_metadata.actions()[0].clone(),
        execute,
    )])
    .expect("action table");
    let descriptor = ComponentDescriptor::with_hooks(component_metadata.clone(), install(control))
        .with_actions(table)
        .expect("matching action table");
    let request_context =
        trusted_context_for(component_metadata, Some(Arc::new(AllowAuthorization)));
    let instance = InstanceId::from_bytes(&bytes::<16>(0x71)).expect("instance identity");
    let render = RenderContext::new(
        &request_context,
        &instance,
        Revision::new(1),
        UnixMillis::new(1_900),
    );
    let state = CanonicalValue::Object(std::collections::BTreeMap::from([(
        "serial".to_owned(),
        CanonicalValue::String("7".to_owned()),
    )]));
    let hydration = HydrationContext::new(render, &state);
    let trace = Trace::default();
    let validation =
        suprnova_live::validation::ValidationEngine::new(16).expect("validation engine");
    let action = ActionName::parse("execute").expect("action name");
    let input_limits = InputLimits::default();
    let transactions = Arc::new(TransactionControl::default());
    let port = SharedTransactionPort(transactions.clone());
    let request = ActionExecutionRequest::new(
        &action,
        RawActionArguments::empty(),
        &input_limits,
        &validation,
        &PassValidation,
        BagPolicy::Replace,
        Some(&port),
        &trace,
    );

    let mut output = ComponentExecutor::new()
        .coordinated_action(&descriptor, &hydration, request)
        .await
        .expect("coordinated action");

    assert_eq!(transactions.begun.load(Ordering::SeqCst), 1);
    assert_eq!(transactions.committed.load(Ordering::SeqCst), 0);
    assert_eq!(transactions.rolled_back.load(Ordering::SeqCst), 0);
    assert!(output.transaction().is_some());
    assert_eq!(
        trace.0.lock().expect("execution trace lock").as_slice(),
        [
            ExecutionPhase::Hydrate,
            ExecutionPhase::Bind,
            ExecutionPhase::Authorize,
            ExecutionPhase::Validate,
            ExecutionPhase::TransactionBegin,
            ExecutionPhase::BeforeAction,
            ExecutionPhase::Action,
            ExecutionPhase::AfterAction,
            ExecutionPhase::Render,
            ExecutionPhase::Dehydrate,
        ]
    );

    output
        .take_transaction()
        .expect("required transaction")
        .commit()
        .await
        .expect("test commit");
    assert_eq!(transactions.committed.load(Ordering::SeqCst), 1);
}
