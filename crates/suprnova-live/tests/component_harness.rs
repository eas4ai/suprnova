//! Host-neutral component harness acceptance.

mod component_support;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::OnceLock;

use serde::Serialize;
use suprnova_live::action::{
    ActionArgumentSchema, ActionEntry, ActionError, ActionFuture, ActionOutcome, ActionResult,
    ActionTable, ActionTarget, AuthorizationDecision, AuthorizationRequirement, AuthorizedAction,
    LiveEffectPayload, LiveEventPayload, OutcomeMetadata, PreparedActionArguments,
    RawActionArguments, RegisteredEmission, RouteIntent,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::clock::Clock as _;
use suprnova_live::execution::{
    ExecutionPhase, ExecutionRefreshReason, ExecutionResult, RetryLegality,
};
use suprnova_live::identity::{
    ActionName, BuildId, ComponentName, IslandSlot, ModelField, Revision, RouteIdentity,
    UnixMillis, ViewName,
};
use suprnova_live::limits::InputLimits;
use suprnova_live::metadata::{
    ActionMetadata, ComponentMetadata, ContractVersions, EffectMetadata, EffectPayloadMetadata,
    EventMetadata, EventPayloadMetadata, FieldMetadata,
};
use suprnova_live::registry::ComponentDescriptor;
use suprnova_live::snapshot::state::{FieldCategory, FieldSpec, StateCodec, StateSchema};
use suprnova_live::snapshot::{ComponentContract, ExpectedInstanceV1, SnapshotSchemaSet};
use suprnova_live::state::{
    ModelBindingSchema, ModelCodec, ModelFieldBinding, ProposalBatch, ProposalLimits,
    RawModelProposal, SessionField, SessionIntent, SessionPort,
};
use suprnova_live::validation::{ValidationIssue, ValidationMessageId, ValidationSelection};
use suprnova_live_test_support::{
    ComponentHarness, ComponentHarnessConfig, HarnessAssertions, HarnessRequestIdentity,
    HarnessServices, HarnessTraceEvent, TransactionFault,
};

use component_support::{
    FailurePoint, FixtureControl, TraceFixture, install, key_ring, metadata, schema_set,
    snapshot_limits, trusted_context_for, trusted_context_for_with_schemas,
};

#[derive(Serialize)]
struct SavedEvent {
    private_note: String,
}

impl EventPayloadMetadata for SavedEvent {
    const NAME: &'static str = "profile.saved";
    const VERSION: u16 = 1;
}

impl LiveEventPayload for SavedEvent {}

#[derive(Serialize)]
struct FocusEffect {
    target: String,
}

impl EffectPayloadMetadata for FocusEffect {
    const NAME: &'static str = "focus";
    const VERSION: u16 = 1;
}

impl LiveEffectPayload for FocusEffect {}

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

fn emit<'a>(
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
        let descriptor = ComponentDescriptor::new(rich_metadata().clone());
        let limits = InputLimits::default();
        let event = RegisteredEmission::event(
            &descriptor,
            &SavedEvent {
                private_note: "browser-secret".to_owned(),
            },
            &limits,
        )
        .map_err(|_| ActionError::dispatcher_contract())?;
        let effect = RegisteredEmission::effect(
            &descriptor,
            &FocusEffect {
                target: "name".to_owned(),
            },
            &limits,
        )
        .map_err(|_| ActionError::dispatcher_contract())?;
        let metadata = OutcomeMetadata::new(vec![], vec![event], vec![effect], None)
            .map_err(|_| ActionError::dispatcher_contract())?;
        ActionResult::new(ActionOutcome::Render, metadata, &descriptor)
            .map_err(|_| ActionError::dispatcher_contract())
    })
}

fn redirect<'a>(
    target: &'a mut dyn ActionTarget,
    _authorization: &'a AuthorizedAction,
    _arguments: &'a PreparedActionArguments,
) -> ActionFuture<'a, Result<ActionResult, ActionError>> {
    Box::pin(async move {
        let target = target
            .as_any_mut()
            .downcast_mut::<TraceFixture>()
            .ok_or_else(ActionError::dispatcher_contract)?;
        target.record("redirect");
        let descriptor = ComponentDescriptor::new(rich_metadata().clone());
        let route = RouteIntent::new(
            RouteIdentity::from_bytes(&component_support::bytes::<32>(0x75))
                .map_err(|_| ActionError::dispatcher_contract())?,
            CanonicalValue::Object(BTreeMap::new()),
            &InputLimits::default(),
        )
        .map_err(|_| ActionError::dispatcher_contract())?;
        let metadata = OutcomeMetadata::new(vec![], vec![], vec![], None)
            .map_err(|_| ActionError::dispatcher_contract())?;
        ActionResult::new(ActionOutcome::Redirect(route), metadata, &descriptor)
            .map_err(|_| ActionError::dispatcher_contract())
    })
}

fn expected_instance(
    component_metadata: &ComponentMetadata,
    descriptor: &ComponentDescriptor,
    context: &suprnova_live::host::TrustedLiveRequestContext,
    schemas: SnapshotSchemaSet,
) -> ExpectedInstanceV1 {
    ExpectedInstanceV1::new(
        ComponentContract::new(
            component_metadata.identity().clone(),
            descriptor.contract_digest().clone(),
            1,
            1,
            1,
        )
        .expect("component contract"),
        BuildId::parse("build-lifecycle-tests").expect("build identity"),
        component_support::snapshot_support::route(0x30),
        IslandSlot::parse("trace").expect("slot identity"),
        context.scope().clone(),
        schemas,
    )
}

fn harness_schema_set() -> SnapshotSchemaSet {
    SnapshotSchemaSet::new(
        schema_set().state().clone(),
        StateSchema::new(1, vec![]).expect("memo schema"),
        StateSchema::new(
            1,
            ["query", "child", "session"]
                .into_iter()
                .map(|name| {
                    FieldSpec::new(name, StateCodec::Json, FieldCategory::Public, true)
                        .expect("mount field")
                })
                .collect(),
        )
        .expect("mount schema"),
    )
    .expect("harness schema set")
}

fn required_metadata() -> &'static ComponentMetadata {
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
                    suprnova_live::action::TransactionPolicy::Required,
                )
                .expect("action metadata"),
            ],
        )
        .expect("component metadata")
    })
}

fn rich_metadata() -> &'static ComponentMetadata {
    static METADATA: OnceLock<ComponentMetadata> = OnceLock::new();
    METADATA.get_or_init(|| {
        let actions = ["emit", "redirect"]
            .into_iter()
            .map(|name| {
                ActionMetadata::new_with_contract(
                    ActionName::parse(name).expect("action identity"),
                    1,
                    ActionArgumentSchema::empty(),
                    AuthorizationRequirement::Current,
                    ValidationSelection::ComponentAndArguments,
                    suprnova_live::action::TransactionPolicy::None,
                )
                .expect("action metadata")
            })
            .collect();
        ComponentMetadata::new_with_browser_contracts(
            ComponentName::parse("tests.rich-outcomes").expect("component identity"),
            ViewName::parse("tests/rich-outcomes.html").expect("view identity"),
            ContractVersions::new(1, 1, 1, 1, 1).expect("versions"),
            vec![FieldMetadata::new(
                ModelField::parse("serial").expect("field identity"),
                FieldCategory::State,
                StateCodec::Json,
                true,
            )],
            actions,
            vec![EventMetadata::from_payload::<SavedEvent>().expect("event metadata")],
            vec![EffectMetadata::from_payload::<FocusEffect>().expect("effect metadata")],
            false,
        )
        .expect("component metadata")
    })
}

#[test]
fn harness_services_control_every_host_dependency_without_raw_authority() {
    let services = HarnessServices::new(UnixMillis::new(1_000));

    assert_eq!(
        services.clock().now().expect("controlled clock"),
        UnixMillis::new(1_000)
    );
    services.clock().set(UnixMillis::new(1_250));
    services
        .authorization()
        .set_decision(AuthorizationDecision::Deny);
    services.transactions().set_fault(TransactionFault::Commit);

    assert_eq!(
        services.clock().now().expect("advanced clock"),
        UnixMillis::new(1_250)
    );
    assert_eq!(
        services.authorization().decision(),
        AuthorizationDecision::Deny
    );
    assert_eq!(services.transactions().fault(), TransactionFault::Commit);
    assert!(services.trace().events().is_empty());

    services.trace().record(HarnessTraceEvent::ClockAdvanced);
    assert_eq!(
        services.trace().events(),
        vec![HarnessTraceEvent::ClockAdvanced]
    );
    assert!(!format!("{services:?}").contains("cookie"));
}

#[tokio::test]
async fn harness_mounts_and_advances_a_real_component_snapshot_without_a_framework_adapter() {
    let services = HarnessServices::new(UnixMillis::new(1_000));
    let control = FixtureControl::new(FailurePoint::None);
    let actions = ActionTable::new(vec![ActionEntry::new(
        metadata().actions()[0].clone(),
        execute,
    )])
    .expect("action table");
    let descriptor = ComponentDescriptor::with_hooks(metadata().clone(), install(control.clone()))
        .with_actions(actions)
        .expect("matching action table");
    let schemas = harness_schema_set();
    let context = trusted_context_for_with_schemas(
        metadata(),
        Some(Arc::clone(services.authorization())
            as Arc<dyn suprnova_live::action::ActionAuthorizationPort>),
        schemas.clone(),
    );
    let expected_instance = expected_instance(metadata(), &descriptor, &context, schemas);
    let config = ComponentHarnessConfig::new(
        descriptor,
        context,
        expected_instance,
        key_ring(),
        snapshot_limits(),
        services.clone(),
    );
    let mut harness = ComponentHarness::new(config).expect("component harness");

    let session_metadata = FieldMetadata::new(
        ModelField::parse("theme").expect("session field"),
        FieldCategory::Session,
        StateCodec::Json,
        false,
    )
    .with_session_binding(ModelCodec::String)
    .expect("session binding");
    let session_field = SessionField::from_metadata(&session_metadata).expect("session field");
    let session_intent = SessionIntent::set(
        session_field.clone(),
        &"midnight".to_owned(),
        &InputLimits::default(),
    )
    .expect("session intent");
    services
        .session()
        .apply(&session_intent)
        .await
        .expect("session write");
    let session_value = services
        .session()
        .read(&session_field)
        .await
        .expect("session read")
        .expect("stored session value");
    assert_eq!(
        session_value
            .decode::<String>(&session_field, &InputLimits::default())
            .expect("typed session value"),
        "midnight"
    );

    let mounted = harness
        .mount(CanonicalValue::Object(BTreeMap::from([
            (
                "query".to_owned(),
                CanonicalValue::String("rust".to_owned()),
            ),
            (
                "child".to_owned(),
                CanonicalValue::Object(BTreeMap::from([(
                    "key".to_owned(),
                    CanonicalValue::String("results".to_owned()),
                )])),
            ),
            (
                "session".to_owned(),
                CanonicalValue::String("theme".to_owned()),
            ),
        ])))
        .await
        .expect("initial mount");
    assert_eq!(mounted.revision(), Revision::new(0));
    assert!(String::from_utf8_lossy(mounted.body()).contains("<p>1</p>"));
    assert!(harness.current_encoded_snapshot().is_some());

    let action = ActionName::parse("execute").expect("action name");
    let proposal_schema = ModelBindingSchema::new(vec![
        ModelFieldBinding::new("query", FieldCategory::Model, ModelCodec::String)
            .expect("query binding"),
        ModelFieldBinding::new("page", FieldCategory::Model, ModelCodec::U64)
            .expect("page binding"),
    ])
    .expect("proposal schema");
    let proposals = ProposalBatch::prepare(
        &proposal_schema,
        vec![
            RawModelProposal::new("query", CanonicalValue::String("live".to_owned())),
            RawModelProposal::new("page", CanonicalValue::String("invalid".to_owned())),
        ],
        &ProposalLimits::default(),
    )
    .expect("valid and invalid values remain typed");
    assert_eq!(proposals.issues().len(), 1);
    services.validation().set_issues(vec![ValidationIssue::new(
        suprnova_live::state::ModelPath::parse("query").expect("validation path"),
        ValidationMessageId::parse("validation.query").expect("validation message"),
    )]);
    let rejected = harness
        .execute_action(
            &action,
            RawActionArguments::empty(),
            Some(&proposals),
            HarnessRequestIdentity::from_seed(0x4f),
        )
        .await
        .expect("validation outcome");
    let rejected = HarnessAssertions::accepted(&rejected);
    HarnessAssertions::revision(rejected, Revision::new(1));
    HarnessAssertions::validation_issue(rejected, "query", "validation.query");
    assert!(!control.values().contains(&"action"));

    services.validation().set_issues(Vec::new());
    let result = harness
        .execute_action(
            &action,
            RawActionArguments::empty(),
            Some(&proposals),
            HarnessRequestIdentity::from_seed(0x50),
        )
        .await
        .expect("harness action input");
    let accepted = HarnessAssertions::accepted(&result);
    HarnessAssertions::revision(accepted, Revision::new(2));
    HarnessAssertions::html_contains(accepted, "<p>3</p>");
    assert!(matches!(result, ExecutionResult::Accepted(_)));
    assert_eq!(
        harness
            .current_snapshot()
            .expect("successor snapshot")
            .body()
            .revision(),
        Revision::new(2)
    );
    assert!(
        services
            .trace()
            .events()
            .contains(&HarnessTraceEvent::Execution(
                ExecutionPhase::LedgerAcceptance
            ))
    );
    assert!(control.values().contains(&"action"));
    assert_eq!(
        services
            .trace()
            .events()
            .iter()
            .filter(|event| **event == HarnessTraceEvent::Authorization)
            .count(),
        2
    );
}

#[tokio::test]
async fn transaction_commit_failure_consumes_the_claim_and_retry_never_reinvokes() {
    let services = HarnessServices::new(UnixMillis::new(1_000));
    let component_metadata = required_metadata();
    let control = FixtureControl::new_with_metadata(FailurePoint::None, component_metadata);
    let actions = ActionTable::new(vec![ActionEntry::new(
        component_metadata.actions()[0].clone(),
        execute,
    )])
    .expect("action table");
    let descriptor =
        ComponentDescriptor::with_hooks(component_metadata.clone(), install(control.clone()))
            .with_actions(actions)
            .expect("matching action table");
    let context = trusted_context_for(
        component_metadata,
        Some(Arc::clone(services.authorization())
            as Arc<dyn suprnova_live::action::ActionAuthorizationPort>),
    );
    let expected = expected_instance(component_metadata, &descriptor, &context, schema_set());
    let config = ComponentHarnessConfig::new(
        descriptor.clone(),
        context,
        expected,
        key_ring(),
        snapshot_limits(),
        services.clone(),
    );
    let mut harness = ComponentHarness::new(config).expect("component harness");
    harness
        .mount(CanonicalValue::Object(BTreeMap::new()))
        .await
        .expect("initial mount");

    services.transactions().set_fault(TransactionFault::Commit);
    let action = ActionName::parse("execute").expect("action name");
    let first = harness
        .execute_action(
            &action,
            RawActionArguments::empty(),
            None,
            HarnessRequestIdentity::from_seed(0x60),
        )
        .await
        .expect("commit failure outcome");
    let refresh = HarnessAssertions::refresh_required(&first);
    assert_eq!(refresh.reason(), ExecutionRefreshReason::HostCommitFailed);
    assert_eq!(refresh.retry_legality(), RetryLegality::Prohibited);
    assert_eq!(services.transactions().begun(), 1);
    assert_eq!(services.transactions().committed(), 0);
    assert_eq!(
        control
            .values()
            .iter()
            .filter(|value| **value == "action")
            .count(),
        1
    );

    services.transactions().set_fault(TransactionFault::None);
    let retry = harness
        .execute_action(
            &action,
            RawActionArguments::empty(),
            None,
            HarnessRequestIdentity::from_seed(0x60),
        )
        .await
        .expect("retry outcome");
    let _refresh = HarnessAssertions::refresh_required(&retry);
    assert_eq!(
        control
            .values()
            .iter()
            .filter(|value| **value == "action")
            .count(),
        1
    );
}

#[tokio::test]
async fn registered_events_effects_redirects_and_current_authorization_are_typed() {
    let services = HarnessServices::new(UnixMillis::new(1_000));
    let component_metadata = rich_metadata();
    let control = FixtureControl::new_with_metadata(FailurePoint::None, component_metadata);
    let actions = ActionTable::new(vec![
        ActionEntry::new(component_metadata.actions()[0].clone(), emit),
        ActionEntry::new(component_metadata.actions()[1].clone(), redirect),
    ])
    .expect("action table");
    let descriptor =
        ComponentDescriptor::with_hooks(component_metadata.clone(), install(control.clone()))
            .with_actions(actions)
            .expect("matching action table");
    let context = trusted_context_for(
        component_metadata,
        Some(Arc::clone(services.authorization())
            as Arc<dyn suprnova_live::action::ActionAuthorizationPort>),
    );
    let expected = expected_instance(component_metadata, &descriptor, &context, schema_set());
    let config = ComponentHarnessConfig::new(
        descriptor.clone(),
        context,
        expected,
        key_ring(),
        snapshot_limits(),
        services.clone(),
    );
    let mut harness = ComponentHarness::new(config).expect("component harness");
    harness
        .mount(CanonicalValue::Object(BTreeMap::new()))
        .await
        .expect("initial mount");

    let emit_action = ActionName::parse("emit").expect("action name");
    let emitted = harness
        .execute_action(
            &emit_action,
            RawActionArguments::empty(),
            None,
            HarnessRequestIdentity::from_seed(0x70),
        )
        .await
        .expect("emission outcome");
    let emitted = HarnessAssertions::accepted(&emitted);
    HarnessAssertions::event(emitted, "profile.saved", 1);
    HarnessAssertions::effect(emitted, "focus", 1);
    assert!(!format!("{emitted:?}").contains("browser-secret"));

    let redirect_action = ActionName::parse("redirect").expect("action name");
    let redirected = harness
        .execute_action(
            &redirect_action,
            RawActionArguments::empty(),
            None,
            HarnessRequestIdentity::from_seed(0x71),
        )
        .await
        .expect("redirect outcome");
    HarnessAssertions::redirect(
        HarnessAssertions::accepted(&redirected),
        &RouteIdentity::from_bytes(&component_support::bytes::<32>(0x75)).expect("route identity"),
    );

    services
        .authorization()
        .set_decision(AuthorizationDecision::Deny);
    let denied = harness
        .execute_action(
            &emit_action,
            RawActionArguments::empty(),
            None,
            HarnessRequestIdentity::from_seed(0x72),
        )
        .await
        .expect("authorization outcome");
    let _refresh = HarnessAssertions::refresh_required(&denied);
    assert_eq!(
        control
            .values()
            .iter()
            .filter(|value| **value == "action")
            .count(),
        1
    );
}
