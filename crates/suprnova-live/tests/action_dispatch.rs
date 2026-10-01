//! Closed registered action dispatch, authorization, and argument contracts.

mod component_support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use suprnova_live::action::{
    ActionArgumentField, ActionArgumentSchema, ActionAuthorizationPort, ActionAuthorizationRequest,
    ActionDispatchFn, ActionEntry, ActionError, ActionErrorKind, ActionFuture, ActionResult,
    ActionTable, ActionTarget, AuthorizationDecision, AuthorizationRequirement, AuthorizedAction,
    PreparedActionArguments, RawActionArguments, TransactionPolicy,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::component::{ComponentExecutor, HydrationContext, RenderContext};
use suprnova_live::host::{HostCapabilities, HostScopeFacts};
use suprnova_live::identity::{
    ActionName, ComponentName, InstanceId, ModelField, Revision, ScopeFingerprint, UnixMillis,
};
use suprnova_live::limits::InputLimits;
use suprnova_live::metadata::ActionMetadata;
use suprnova_live::registry::ComponentDescriptor;
use suprnova_live::state::ModelCodec;
use suprnova_live::validation::{
    BagPolicy, ValidationFuture, ValidationIssue, ValidationMessageId, ValidationPort,
    ValidationPortError, ValidationRequest, ValidationSelection,
};

use component_support::{
    FailurePoint, FixtureControl, TraceFixture, install, metadata,
    trusted_context_with_authorization,
};

#[derive(Default)]
struct Counter {
    value: u64,
}

fn increment<'a>(
    target: &'a mut dyn ActionTarget,
    _authorization: &'a AuthorizedAction,
    arguments: &'a PreparedActionArguments,
) -> ActionFuture<'a, Result<ActionResult, ActionError>> {
    Box::pin(async move {
        let target = target
            .as_any_mut()
            .downcast_mut::<Counter>()
            .ok_or_else(ActionError::dispatcher_contract)?;
        target.value += arguments.decode::<u64>("amount")?;
        Ok(ActionResult::render())
    })
}

fn reset_sync<'a>(
    target: &'a mut dyn ActionTarget,
    _authorization: &'a AuthorizedAction,
    _arguments: &'a PreparedActionArguments,
) -> ActionFuture<'a, Result<ActionResult, ActionError>> {
    let result = target
        .as_any_mut()
        .downcast_mut::<Counter>()
        .ok_or_else(ActionError::dispatcher_contract)
        .map(|target| {
            target.value = 0;
            ActionResult::no_render()
        });
    Box::pin(async move { result })
}

fn panic_action<'a>(
    _target: &'a mut dyn ActionTarget,
    _authorization: &'a AuthorizedAction,
    _arguments: &'a PreparedActionArguments,
) -> ActionFuture<'a, Result<ActionResult, ActionError>> {
    Box::pin(async move { panic!("browser-supplied-secret") })
}

struct RecordingAuthorization {
    trace: Mutex<Vec<String>>,
    decision: AuthorizationDecision,
}

impl ActionAuthorizationPort for RecordingAuthorization {
    fn authorize<'a>(
        &'a self,
        request: ActionAuthorizationRequest<'a>,
    ) -> ActionFuture<'a, Result<AuthorizationDecision, ActionError>> {
        Box::pin(async move {
            self.trace
                .lock()
                .expect("authorization trace lock")
                .push(request.action().as_str().to_owned());
            Ok(self.decision)
        })
    }
}

fn action(
    name: &str,
    schema: ActionArgumentSchema,
    authorization: AuthorizationRequirement,
    dispatcher: ActionDispatchFn,
) -> ActionEntry {
    let metadata = ActionMetadata::new_with_contract(
        ActionName::parse(name).expect("action name"),
        1,
        schema,
        authorization,
        ValidationSelection::None,
        TransactionPolicy::None,
    )
    .expect("action metadata");
    ActionEntry::new(metadata, dispatcher)
}

fn capabilities(port: Arc<dyn ActionAuthorizationPort>) -> HostCapabilities {
    let scope = HostScopeFacts::new(
        ScopeFingerprint::from_bytes(&[0x11; 32]).expect("scope fingerprint"),
        None,
        None,
        None,
    );
    HostCapabilities::bound_to(scope).with_action_authorization(port)
}

fn raw_amount(value: CanonicalValue) -> RawActionArguments {
    RawActionArguments::new(CanonicalValue::Object(BTreeMap::from([(
        "amount".to_owned(),
        value,
    )])))
}

#[tokio::test]
async fn only_registered_actions_dispatch_and_sync_async_share_the_outcome_contract() {
    let amount = ActionArgumentSchema::new(vec![
        ActionArgumentField::new(
            ModelField::parse("amount").expect("argument name"),
            ModelCodec::U64,
            true,
        )
        .expect("argument field"),
    ])
    .expect("argument schema");
    let table = ActionTable::new(vec![
        action(
            "increment",
            amount,
            AuthorizationRequirement::Current,
            increment,
        ),
        action(
            "reset",
            ActionArgumentSchema::empty(),
            AuthorizationRequirement::Public,
            reset_sync,
        ),
    ])
    .expect("closed action table");
    let authorization = Arc::new(RecordingAuthorization {
        trace: Mutex::new(Vec::new()),
        decision: AuthorizationDecision::Allow,
    });
    let host = capabilities(authorization.clone());
    let component = ComponentName::parse("counter").expect("component name");
    let mut counter = Counter::default();

    let result = table
        .invoke(
            &component,
            &host,
            &ActionName::parse("increment").expect("action name"),
            &mut counter,
            raw_amount(CanonicalValue::String("3".to_owned())),
            &InputLimits::default(),
        )
        .await
        .expect("async action result");
    assert!(result.outcome().requires_render());
    assert_eq!(counter.value, 3);

    let result = table
        .invoke(
            &component,
            &host,
            &ActionName::parse("reset").expect("action name"),
            &mut counter,
            RawActionArguments::empty(),
            &InputLimits::default(),
        )
        .await
        .expect("sync action result");
    assert!(!result.outcome().requires_render());
    assert_eq!(counter.value, 0);
    assert_eq!(
        authorization
            .trace
            .lock()
            .expect("authorization trace lock")
            .as_slice(),
        ["increment"]
    );

    let error = table
        .invoke(
            &component,
            &host,
            &ActionName::parse("private_helper").expect("action name"),
            &mut counter,
            RawActionArguments::empty(),
            &InputLimits::default(),
        )
        .await
        .expect_err("unknown private method must not dispatch");
    assert_eq!(error.kind(), ActionErrorKind::UnknownAction);
}

#[tokio::test]
async fn malformed_oversized_denied_and_panicking_actions_are_closed_and_redacted() {
    let amount = ActionArgumentSchema::new(vec![
        ActionArgumentField::new(
            ModelField::parse("amount").expect("argument name"),
            ModelCodec::U64,
            true,
        )
        .expect("argument field"),
    ])
    .expect("argument schema");
    let table = ActionTable::new(vec![
        action(
            "increment",
            amount,
            AuthorizationRequirement::Current,
            increment,
        ),
        action(
            "panic",
            ActionArgumentSchema::empty(),
            AuthorizationRequirement::Public,
            panic_action,
        ),
    ])
    .expect("closed action table");
    let denied = Arc::new(RecordingAuthorization {
        trace: Mutex::new(Vec::new()),
        decision: AuthorizationDecision::Deny,
    });
    let host = capabilities(denied);
    let component = ComponentName::parse("counter").expect("component name");
    let mut counter = Counter::default();

    let denied = table
        .invoke(
            &component,
            &host,
            &ActionName::parse("increment").expect("action name"),
            &mut counter,
            raw_amount(CanonicalValue::String("1".to_owned())),
            &InputLimits::default(),
        )
        .await
        .expect_err("current authorization denial");
    assert_eq!(denied.kind(), ActionErrorKind::AuthorizationDenied);
    assert_eq!(counter.value, 0);

    let invalid = table
        .prepare(
            &ActionName::parse("increment").expect("action name"),
            raw_amount(CanonicalValue::String("not-an-integer".to_owned())),
            &InputLimits::default(),
        )
        .expect_err("malformed typed argument");
    assert_eq!(invalid.kind(), ActionErrorKind::InvalidArguments);

    let tiny = InputLimits::new(8, 4, 4, 4).expect("tiny limits");
    let oversized = table
        .prepare(
            &ActionName::parse("increment").expect("action name"),
            raw_amount(CanonicalValue::String("123456789".to_owned())),
            &tiny,
        )
        .expect_err("oversized typed argument");
    assert_eq!(oversized.kind(), ActionErrorKind::InvalidArguments);

    let panicked = table
        .invoke(
            &component,
            &host,
            &ActionName::parse("panic").expect("action name"),
            &mut counter,
            RawActionArguments::empty(),
            &InputLimits::default(),
        )
        .await
        .expect_err("panic must not become success");
    assert_eq!(panicked.kind(), ActionErrorKind::Panicked);
    assert!(!format!("{panicked:?}").contains("browser-supplied-secret"));
}

struct OrderedAuthorization {
    trace: Arc<Mutex<Vec<&'static str>>>,
}

impl ActionAuthorizationPort for OrderedAuthorization {
    fn authorize<'a>(
        &'a self,
        _request: ActionAuthorizationRequest<'a>,
    ) -> ActionFuture<'a, Result<AuthorizationDecision, ActionError>> {
        Box::pin(async move {
            self.trace
                .lock()
                .expect("ordered trace lock")
                .push("authorize");
            Ok(AuthorizationDecision::Allow)
        })
    }
}

struct OrderedValidation {
    trace: Arc<Mutex<Vec<&'static str>>>,
    reject: bool,
}

impl ValidationPort for OrderedValidation {
    fn validate<'a>(
        &'a self,
        _request: ValidationRequest<'a>,
    ) -> ValidationFuture<'a, Result<Vec<ValidationIssue>, ValidationPortError>> {
        Box::pin(async move {
            self.trace
                .lock()
                .expect("ordered trace lock")
                .push("validate");
            if self.reject {
                Ok(vec![ValidationIssue::new(
                    suprnova_live::state::ModelPath::parse("serial").expect("validation path"),
                    ValidationMessageId::parse("validation.rejected").expect("message identity"),
                )])
            } else {
                Ok(Vec::new())
            }
        })
    }
}

fn execute_fixture<'a>(
    target: &'a mut dyn ActionTarget,
    authorization: &'a AuthorizedAction,
    _arguments: &'a PreparedActionArguments,
) -> ActionFuture<'a, Result<ActionResult, ActionError>> {
    Box::pin(async move {
        assert!(authorization.is_current());
        let fixture = target
            .as_any_mut()
            .downcast_mut::<TraceFixture>()
            .ok_or_else(ActionError::dispatcher_contract)?;
        fixture.record("action");
        fixture.serial += 10;
        Ok(ActionResult::render())
    })
}

fn bytes<const LENGTH: usize>(start: u8) -> [u8; LENGTH] {
    std::array::from_fn(|index| start.wrapping_add(index as u8))
}

#[tokio::test]
async fn executor_hydrates_then_authorizes_validates_and_dispatches_before_rendering() {
    let control = FixtureControl::new(FailurePoint::None);
    let authorization = Arc::new(OrderedAuthorization {
        trace: control.trace.clone(),
    });
    let request = Box::leak(Box::new(trusted_context_with_authorization(authorization)));
    let instance = Box::leak(Box::new(
        InstanceId::from_bytes(&bytes::<16>(0x51)).expect("instance identity"),
    ));
    let render = RenderContext::new(request, instance, Revision::new(4), UnixMillis::new(5_000));
    let state = CanonicalValue::Object(BTreeMap::from([(
        "serial".to_owned(),
        CanonicalValue::String("0".to_owned()),
    )]));
    let hydration = HydrationContext::new(render, &state);
    let table = ActionTable::new(vec![ActionEntry::new(
        metadata().actions()[0].clone(),
        execute_fixture,
    )])
    .expect("generated action table");
    let descriptor = ComponentDescriptor::with_hooks(metadata().clone(), install(control.clone()))
        .with_actions(table)
        .expect("action metadata equivalence");
    let validation = OrderedValidation {
        trace: control.trace.clone(),
        reject: false,
    };

    let output = ComponentExecutor::new()
        .action(
            &descriptor,
            &hydration,
            &ActionName::parse("execute").expect("action name"),
            RawActionArguments::empty(),
            &InputLimits::default(),
            &suprnova_live::validation::ValidationEngine::default(),
            &validation,
            BagPolicy::Replace,
        )
        .await
        .expect("ordered action execution");

    assert!(output.action_executed());
    assert_eq!(output.render().expect("render outcome").body, "<p>11</p>");
    assert_eq!(
        control.values(),
        [
            "reconstruct",
            "hydrated",
            "authorize",
            "validate",
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
async fn validation_failure_renders_issues_without_running_the_action_body() {
    let control = FixtureControl::new(FailurePoint::None);
    let authorization = Arc::new(OrderedAuthorization {
        trace: control.trace.clone(),
    });
    let request = Box::leak(Box::new(trusted_context_with_authorization(authorization)));
    let instance = Box::leak(Box::new(
        InstanceId::from_bytes(&bytes::<16>(0x61)).expect("instance identity"),
    ));
    let render = RenderContext::new(request, instance, Revision::new(8), UnixMillis::new(5_000));
    let state = CanonicalValue::Object(BTreeMap::new());
    let hydration = HydrationContext::new(render, &state);
    let table = ActionTable::new(vec![ActionEntry::new(
        metadata().actions()[0].clone(),
        execute_fixture,
    )])
    .expect("generated action table");
    let descriptor = ComponentDescriptor::with_hooks(metadata().clone(), install(control.clone()))
        .with_actions(table)
        .expect("action metadata equivalence");
    let validation = OrderedValidation {
        trace: control.trace.clone(),
        reject: true,
    };

    let output = ComponentExecutor::new()
        .action(
            &descriptor,
            &hydration,
            &ActionName::parse("execute").expect("action name"),
            RawActionArguments::empty(),
            &InputLimits::default(),
            &suprnova_live::validation::ValidationEngine::default(),
            &validation,
            BagPolicy::Replace,
        )
        .await
        .expect("validation result is renderable");

    assert!(!output.action_executed());
    assert_eq!(output.validation().len(), 1);
    assert!(!control.values().contains(&"action"));
}
