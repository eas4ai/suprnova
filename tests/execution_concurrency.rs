//! Deterministic expected-revision races and metadata-only duplicate recovery.

mod component_support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use suprnova_live::action::{
    ActionEntry, ActionError, ActionFuture, ActionResult, ActionTable, ActionTarget,
    AuthorizedAction, PreparedActionArguments, RawActionArguments,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::execution::{
    ActionExecutionRequest, ExecutionPhase, ExecutionRefreshReason, ExecutionResult,
    ExecutionService, ExecutionTracePort, InstancedActionRequest, RetryLegality,
};
use suprnova_live::identity::{
    ActionName, BuildId, IslandSlot, Revision, RouteIdentity, UnixMillis,
};
use suprnova_live::ledger::{LiveInstanceLedger, MountInstanceRecord};
use suprnova_live::limits::InputLimits;
use suprnova_live::registry::ComponentDescriptor;
use suprnova_live::snapshot::{
    ComponentContract, ExpectedInstanceV1, InstanceBodyV1, InstanceFieldsV1, verify_instance,
};
use suprnova_live::validation::{
    BagPolicy, ValidationFuture, ValidationPort, ValidationPortError, ValidationRequest,
};
use suprnova_live::view::{RenderLimits, ViewRenderer};

use component_support::{
    FailurePoint, FixtureControl, ManualClock, TraceFixture, browser_context, bytes, digest,
    idempotency, install, key_ring, ledger, metadata, schema_set, snapshot_limits,
    trusted_context_with_authorization,
};

fn yielding_action<'a>(
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
        tokio::task::yield_now().await;
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
        self.0.lock().expect("trace lock").push(phase);
    }
}

#[tokio::test]
async fn concurrent_duplicates_accept_one_outcome_and_never_reinvoke_without_bytes() {
    let control = FixtureControl::new(FailurePoint::None);
    let table = ActionTable::new(vec![ActionEntry::new(
        metadata().actions()[0].clone(),
        yielding_action,
    )])
    .expect("action table");
    let descriptor = ComponentDescriptor::with_hooks(metadata().clone(), install(control.clone()))
        .with_actions(table)
        .expect("matching action table");
    let context = trusted_context_with_authorization(Arc::new(AllowAuthorization));
    let component_contract = ComponentContract::new(
        metadata().identity().clone(),
        descriptor.contract_digest().clone(),
        1,
        1,
        1,
    )
    .expect("component contract");
    let build_id = BuildId::parse("build-lifecycle-tests").expect("build identity");
    let route = RouteIdentity::from_bytes(&bytes::<32>(0x30)).expect("route identity");
    let slot = IslandSlot::parse("trace").expect("slot identity");
    let instance_id = suprnova_live::identity::InstanceId::from_bytes(&bytes::<16>(0x70))
        .expect("instance identity");
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = Arc::new(ledger(clock.clone(), 8));
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
    let body = InstanceBodyV1::new(
        InstanceFieldsV1 {
            component: component_contract.clone(),
            build_id: build_id.clone(),
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
        component_contract,
        build_id,
        route,
        slot,
        context.scope().clone(),
        schema_set(),
    );
    let first_snapshot =
        verify_instance(&encoded, &expected, &keys, UnixMillis::new(1_000), &limits)
            .expect("first verified snapshot");
    let second_snapshot =
        verify_instance(&encoded, &expected, &keys, UnixMillis::new(1_000), &limits)
            .expect("second verified snapshot");
    let service = ExecutionService::new(
        ledger.clone(),
        clock,
        keys,
        limits,
        ViewRenderer::new(RenderLimits::standard()).expect("renderer"),
    );
    let action = ActionName::parse("execute").expect("action name");
    let input_limits = InputLimits::default();
    let validation =
        suprnova_live::validation::ValidationEngine::new(16).expect("validation engine");
    let first_trace = Trace::default();
    let second_trace = Trace::default();
    let retry_key = idempotency(0x50);
    let request_digest = digest(0x60);

    let first = service.execute_instanced(InstancedActionRequest::new(
        &descriptor,
        &context,
        browser_context(),
        &first_snapshot,
        retry_key.clone(),
        request_digest.clone(),
        ActionExecutionRequest::new(
            &action,
            RawActionArguments::empty(),
            &input_limits,
            &validation,
            &PassValidation,
            BagPolicy::Replace,
            None,
            &first_trace,
        ),
    ));
    let second = service.execute_instanced(InstancedActionRequest::new(
        &descriptor,
        &context,
        browser_context(),
        &second_snapshot,
        retry_key.clone(),
        request_digest.clone(),
        ActionExecutionRequest::new(
            &action,
            RawActionArguments::empty(),
            &input_limits,
            &validation,
            &PassValidation,
            BagPolicy::Replace,
            None,
            &second_trace,
        ),
    ));

    let (first, second) = tokio::join!(first, second);
    let ExecutionResult::Accepted(first) = first else {
        panic!("first request must be accepted");
    };
    let successor_html = std::str::from_utf8(&first.render().expect("rendered successor").body)
        .expect("successor HTML");
    assert!(successor_html.starts_with("<div data-suprnova-live-root=\"trace\""));
    assert!(successor_html.contains("data-suprnova-live-document-key=\"test-root\""));
    assert!(successor_html.contains("data-suprnova-live-snapshot-kind=\"instance\""));
    assert!(successor_html.contains("data-suprnova-live-revision=\"1\""));
    assert!(matches!(second, ExecutionResult::InProgress { .. }));
    assert_eq!(
        control
            .values()
            .iter()
            .filter(|phase| **phase == "action")
            .count(),
        1
    );
    let inspection = ledger
        .inspect(context.scope(), &instance_id)
        .expect("ledger inspection")
        .expect("instance remains inspectable");
    assert_eq!(inspection.current_revision(), Revision::new(1));
    assert_eq!(inspection.accepted_outcome_count(), 1);

    let duplicate_snapshot = verify_instance(
        &encoded,
        &expected,
        &Arc::new(key_ring()),
        UnixMillis::new(1_000),
        &snapshot_limits(),
    )
    .expect("duplicate verified snapshot");
    let duplicate_trace = Trace::default();
    let duplicate = service
        .execute_instanced(InstancedActionRequest::new(
            &descriptor,
            &context,
            browser_context(),
            &duplicate_snapshot,
            retry_key,
            request_digest,
            ActionExecutionRequest::new(
                &action,
                RawActionArguments::empty(),
                &input_limits,
                &validation,
                &PassValidation,
                BagPolicy::Replace,
                None,
                &duplicate_trace,
            ),
        ))
        .await;
    let ExecutionResult::RefreshRequired(duplicate) = duplicate else {
        panic!("metadata-only duplicate must fresh-render");
    };
    assert_eq!(
        duplicate.reason(),
        ExecutionRefreshReason::DuplicateResponseUnavailable
    );
    assert_eq!(duplicate.retry_legality(), RetryLegality::Prohibited);
    assert!(duplicate.accepted_metadata().is_some());
    assert_eq!(
        control
            .values()
            .iter()
            .filter(|phase| **phase == "action")
            .count(),
        1
    );
    assert_eq!(
        duplicate_trace.0.lock().expect("trace lock").as_slice(),
        [ExecutionPhase::Claim]
    );
}
