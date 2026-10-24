//! Accepted HTML is a complete engine-owned successor island before durable acceptance.

mod component_support;

use std::collections::BTreeMap;
use std::sync::Arc;

use base64::Engine as _;
use suprnova_live::action::{
    ActionEntry, ActionError, ActionFuture, ActionResult, ActionTable, ActionTarget,
    AuthorizedAction, PreparedActionArguments, RawActionArguments,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::execution::{
    ActionExecutionRequest, ExecutionResult, ExecutionService, ExecutionTracePort,
    InstancedActionRequest,
};
use suprnova_live::identity::{
    ActionName, BuildId, InstanceId, IslandSlot, Revision, RouteIdentity, UnixMillis,
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
    FailurePoint, FixtureControl, ManualClock, TraceFixture, admitted_response_sealer,
    browser_context, bytes, digest, idempotency, install, key_ring, ledger, metadata, schema_set,
    snapshot_limits, trusted_context_with_authorization,
};

fn render_action<'a>(
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

struct NoopTrace;

impl ExecutionTracePort for NoopTrace {
    fn record(&self, _phase: suprnova_live::execution::ExecutionPhase) {}
}

#[tokio::test]
async fn accepted_html_contains_the_signed_successor_and_authoritative_root_identity() {
    let control = FixtureControl::new(FailurePoint::None);
    let actions = ActionTable::new(vec![ActionEntry::new(
        metadata().actions()[0].clone(),
        render_action,
    )])
    .expect("action table");
    let descriptor = ComponentDescriptor::with_hooks(metadata().clone(), install(control))
        .with_actions(actions)
        .expect("descriptor");
    let context = trusted_context_with_authorization(Arc::new(AllowAuthorization));
    let contract = ComponentContract::new(
        metadata().identity().clone(),
        descriptor.contract_digest().clone(),
        1,
        1,
        1,
    )
    .expect("component contract");
    let build = BuildId::parse("build-lifecycle-tests").expect("build");
    let route = RouteIdentity::from_bytes(&bytes::<32>(0x30)).expect("route");
    let slot = IslandSlot::parse("trace").expect("slot");
    let instance = InstanceId::from_bytes(&bytes::<16>(0x70)).expect("instance");
    let clock = Arc::new(ManualClock::new(1_000));
    let instance_ledger = Arc::new(ledger(clock.clone(), 8));
    instance_ledger
        .mount_instance(MountInstanceRecord::new(
            context.scope().clone(),
            instance.clone(),
            descriptor.contract_digest().clone(),
            Revision::new(0),
            UnixMillis::new(1_900),
        ))
        .await
        .expect("ledger mount");
    let keys = Arc::new(key_ring());
    let limits = snapshot_limits();
    let encoded = InstanceBodyV1::new(
        InstanceFieldsV1 {
            component: contract.clone(),
            build_id: build.clone(),
            route: route.clone(),
            slot: slot.clone(),
            key_id: keys.active_key_id().clone(),
            scope: context.scope().clone(),
            instance_id: instance.clone(),
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
    .expect("instance body")
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
    let verified = verify_instance(&encoded, &expected, &keys, UnixMillis::new(1_000), &limits)
        .expect("verified instance");
    let service = ExecutionService::new(
        instance_ledger,
        clock,
        keys,
        limits,
        ViewRenderer::new(RenderLimits::standard()).expect("renderer"),
    );
    let action = ActionName::parse("execute").expect("action");
    let input_limits = InputLimits::default();
    let validation = suprnova_live::validation::ValidationEngine::new(16).expect("validation");
    let response_sealer = admitted_response_sealer(
        descriptor.clone(),
        trusted_context_with_authorization(Arc::new(AllowAuthorization)),
        &encoded,
        Revision::new(0),
        0x45,
        None,
    )
    .await;
    let (response_sealer, response_binding) = response_sealer.into_parts();
    let outcome = service
        .execute_instanced(InstancedActionRequest::new(
            &descriptor,
            &context,
            browser_context(),
            &verified,
            idempotency(0x50),
            digest(0x60),
            ActionExecutionRequest::new(
                &action,
                RawActionArguments::empty(),
                &input_limits,
                &validation,
                &PassValidation,
                BagPolicy::Replace,
                None,
                &NoopTrace,
            )
            .with_response_sealer(response_sealer, response_binding),
        ))
        .await;
    let ExecutionResult::Accepted(accepted) = outcome else {
        panic!("execution must be accepted");
    };
    let render = accepted.render().expect("successor render");
    let html = std::str::from_utf8(&render.body).expect("successor UTF-8");
    let signed =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(accepted.signed_snapshot());
    assert!(html.starts_with("<div data-suprnova-live-root=\"trace\""));
    assert!(html.contains("data-suprnova-live-component=\"tests.trace\""));
    assert!(html.contains("data-suprnova-live-document-key=\"test-root\""));
    assert!(html.contains("data-suprnova-live-snapshot-kind=\"instance\""));
    assert!(html.contains("data-suprnova-live-revision=\"1\""));
    assert!(html.contains(&format!("data-suprnova-live-snapshot=\"{signed}\"")));
    assert!(html.contains("<p>"));
    assert!(html.ends_with("</p></div>"));
}
