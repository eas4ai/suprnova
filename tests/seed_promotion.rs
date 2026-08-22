//! Public-seed verification, promotion, replay, and scope tests.

mod promotion_support;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;

use promotion_support::{
    browser_context, context, context_for_route, harness, nonce, promotion_limits, signed_seed,
    signed_seed_with_refresh, trusted_context_for_route,
};
use suprnova_live::action::{
    ActionEntry, ActionError, ActionFuture, ActionResult, ActionTable, ActionTarget,
    AuthorizedAction, PreparedActionArguments, RawActionArguments,
};
use suprnova_live::canonical::{CanonicalNumber, CanonicalValue};
use suprnova_live::clock::{Clock, ClockError};
use suprnova_live::component::{
    ComponentError, ComponentFactory, ComponentHooks, ComponentInstance, HydrationContext,
    LiveFuture, MountContext, RenderContext,
};
use suprnova_live::execution::{
    ActionExecutionRequest, ExecutionResult, ExecutionService, NoopExecutionTrace,
    PromotedActionRequest, RetryLegality,
};
use suprnova_live::identity::{
    ActionName, BrowserNonce, BuildId, IslandSlot, Revision, UnixMillis,
};
use suprnova_live::ledger::{LedgerLimits, LedgerPhase, MemoryInstanceLedger};
use suprnova_live::limits::InputLimits;
use suprnova_live::promotion::{PromotionErrorKind, PromotionService, RefreshBeforeAction};
use suprnova_live::registry::ComponentDescriptor;
use suprnova_live::snapshot::state::{FieldCategory, StateExposure};
use suprnova_live::snapshot::{ExpectedInstanceV1, verify_instance};
use suprnova_live::state::{
    ModelBindingSchema, ModelCodec, ModelFieldBinding, ModelPath, ProposalApplication,
    ProposalBatch, ProposalLimits, RawModelProposal,
};
use suprnova_live::validation::{
    BagPolicy, ValidationFuture, ValidationPort, ValidationPortError, ValidationRequest,
};
use suprnova_live::view::{AssetSet, IslandRender, RenderLimits, ViewRenderer};

#[tokio::test]
async fn verified_seed_promotes_to_a_server_identified_scoped_instance() {
    let harness = harness(promotion_limits(), 64);
    let seed = signed_seed(&harness.keys, "rust");
    let context = context(0x90);
    let browser_nonce = nonce(0x10);

    let promoted = harness
        .service
        .promote(&seed, browser_nonce.clone(), &context)
        .await
        .expect("valid seed promotes");

    assert_ne!(promoted.instance_id().as_bytes(), browser_nonce.as_bytes());
    assert_eq!(promoted.revision(), Revision::new(0));
    assert_eq!(
        promoted.refresh_before_action(),
        RefreshBeforeAction::Required
    );
    assert_eq!(promoted.advisory_generations().len(), 1);
    assert_eq!(format!("{promoted:?}"), "<PromotedInstance:redacted>");
}

#[tokio::test]
async fn integrity_and_trusted_bindings_are_checked_before_identity_or_ledger_creation() {
    let harness = harness(promotion_limits(), 64);
    let mut tampered = signed_seed(&harness.keys, "rust");
    let position = tampered
        .iter()
        .position(|byte| *byte == b'r')
        .expect("seed contains test state");
    tampered[position] = b'x';

    let error = harness
        .service
        .promote(&tampered, nonce(0x11), &context(0x91))
        .await
        .expect_err("tampered seed fails closed");
    assert_eq!(error.kind(), PromotionErrorKind::SnapshotRejected);
    assert_eq!(harness.generator.calls(), 0);
    assert!(
        harness
            .ledger
            .inspect(
                &promotion_support::scope(0x91),
                &promotion_support::instance(0xd0),
            )
            .expect("inspection succeeds")
            .is_none()
    );
}

#[tokio::test]
async fn valid_signature_with_the_wrong_current_route_fails_before_identity_generation() {
    let harness = harness(promotion_limits(), 64);
    let seed = signed_seed(&harness.keys, "rust");
    let wrong_context = context_for_route(0x95, 2);

    assert_eq!(
        harness
            .service
            .promote(&seed, nonce(0x15), &wrong_context)
            .await
            .expect_err("current route binding must match")
            .kind(),
        PromotionErrorKind::SnapshotRejected
    );
    assert_eq!(harness.generator.calls(), 0);
}

#[tokio::test]
async fn request_authority_expiring_after_validation_blocks_promotion() {
    let harness = harness(promotion_limits(), 64);
    let seed = signed_seed(&harness.keys, "rust");
    let context = context(0x98);
    harness.clock.set(2_000);

    let error = harness
        .service
        .promote(&seed, nonce(0x18), &context)
        .await
        .expect_err("expired request authority");

    assert_eq!(error.kind(), PromotionErrorKind::ContextRejected);
    assert_eq!(harness.generator.calls(), 0);
}

#[tokio::test]
async fn exact_retry_recovers_one_instance_while_new_nonce_and_scope_are_independent() {
    let harness = harness(promotion_limits(), 64);
    let seed = signed_seed(&harness.keys, "rust");
    let first_context = context(0x92);
    let first_nonce = nonce(0x12);
    let first = harness
        .service
        .promote(&seed, first_nonce.clone(), &first_context)
        .await
        .expect("first promotion succeeds");
    let retry = harness
        .service
        .promote(&seed, first_nonce.clone(), &first_context)
        .await
        .expect("exact retry recovers");
    assert_eq!(retry.instance_id(), first.instance_id());

    let independent = harness
        .service
        .promote(&seed, nonce(0x13), &first_context)
        .await
        .expect("new nonce creates independent instance");
    assert_ne!(independent.instance_id(), first.instance_id());

    let other_scope = harness
        .service
        .promote(&seed, first_nonce, &context(0x93))
        .await
        .expect("same public seed and nonce in another scope stays independent");
    assert_ne!(other_scope.instance_id(), first.instance_id());
}

#[tokio::test]
async fn nonce_reuse_with_changed_signed_input_is_rejected() {
    let harness = harness(promotion_limits(), 64);
    let context = context(0x94);
    let nonce = nonce(0x14);
    harness
        .service
        .promote(&signed_seed(&harness.keys, "rust"), nonce.clone(), &context)
        .await
        .expect("first promotion succeeds");

    let error = harness
        .service
        .promote(&signed_seed(&harness.keys, "other"), nonce, &context)
        .await
        .expect_err("same nonce cannot identify changed signed input");
    assert_eq!(error.kind(), PromotionErrorKind::NonceConflict);
}

#[test]
fn browser_nonce_type_rejects_less_than_128_bits() {
    assert!(BrowserNonce::from_bytes(&[0_u8; 15]).is_err());
}

#[tokio::test]
async fn refresh_on_promote_is_a_typed_component_choice_not_a_coherence_gate() {
    let harness = harness(promotion_limits(), 64);
    let promoted = harness
        .service
        .promote(
            &signed_seed_with_refresh(&harness.keys, "rust", false),
            nonce(0x16),
            &context(0x96),
        )
        .await
        .expect("advisory generations do not reject promotion");
    assert_eq!(
        promoted.refresh_before_action(),
        RefreshBeforeAction::NotRequired
    );
    assert_eq!(promoted.advisory_generations().len(), 1);
}

#[derive(Debug)]
struct CompletionClock {
    calls: AtomicUsize,
}

impl Clock for CompletionClock {
    fn now(&self) -> Result<UnixMillis, ClockError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(UnixMillis::new(if call < 2 { 1_000 } else { 1_101 }))
    }
}

#[tokio::test]
async fn promotion_completion_after_the_policy_lease_fails_closed() {
    let clock = Arc::new(CompletionClock {
        calls: AtomicUsize::new(0),
    });
    let ledger = Arc::new(MemoryInstanceLedger::new(
        clock.clone(),
        LedgerLimits::new(100, 10_000, 4, 64).expect("ledger limits are valid"),
    ));
    let generator = Arc::new(promotion_support::SequenceGenerator::new(0xd0));
    let keys = Arc::new(promotion_support::snapshot_support::key_ring());
    let snapshot_limits = promotion_support::snapshot_support::snapshot_limits();
    let service = PromotionService::new(
        ledger,
        clock,
        generator,
        keys.clone(),
        snapshot_limits,
        promotion_limits(),
    )
    .expect("promotion service config is valid");

    let error = service
        .promote(&signed_seed(&keys, "rust"), nonce(0x17), &context(0x97))
        .await
        .expect_err("completion after the promotion lease must fail closed");
    assert_eq!(error.kind(), PromotionErrorKind::ProviderInvariant);
}

#[test]
fn production_instance_generator_returns_128_bits_of_server_identity() {
    use suprnova_live::promotion::{InstanceIdGenerator, SystemInstanceIdGenerator};

    let instance = SystemInstanceIdGenerator
        .generate()
        .expect("operating-system randomness is available");
    assert_eq!(instance.as_bytes().len(), 16);
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize)]
struct SearchState {
    query: String,
    selected: String,
    server: String,
    count: u64,
}

struct SearchControl {
    mount_failure: bool,
    mounts: AtomicUsize,
    hydrations: AtomicUsize,
    binds: AtomicUsize,
    actions: AtomicUsize,
    action_observations: Mutex<Vec<SearchState>>,
}

impl SearchControl {
    fn new(mount_failure: bool) -> Arc<Self> {
        Arc::new(Self {
            mount_failure,
            mounts: AtomicUsize::new(0),
            hydrations: AtomicUsize::new(0),
            binds: AtomicUsize::new(0),
            actions: AtomicUsize::new(0),
            action_observations: Mutex::new(Vec::new()),
        })
    }
}

struct SearchFactory {
    control: Arc<SearchControl>,
}

impl ComponentFactory for SearchFactory {
    fn mount<'a>(
        &'a self,
        _context: &'a MountContext<'a>,
    ) -> LiveFuture<'a, Result<Box<dyn ComponentInstance>, ComponentError>> {
        Box::pin(async move {
            self.control.mounts.fetch_add(1, Ordering::SeqCst);
            if self.control.mount_failure {
                return Err(ComponentError::application_failure());
            }
            Ok(Box::new(SearchComponent {
                control: self.control.clone(),
                state: SearchState {
                    query: "fresh".to_owned(),
                    selected: "fresh".to_owned(),
                    server: "authoritative".to_owned(),
                    count: 1,
                },
                page: 9,
            }) as Box<dyn ComponentInstance>)
        })
    }

    fn hydrate<'a>(
        &'a self,
        context: &'a HydrationContext<'a>,
    ) -> LiveFuture<'a, Result<Box<dyn ComponentInstance>, ComponentError>> {
        Box::pin(async move {
            let state = decode_search_state(context.state())?;
            let page = decode_page(
                context
                    .memo()
                    .ok_or_else(ComponentError::contract_failure)?,
            )?;
            self.control.hydrations.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(SearchComponent {
                control: self.control.clone(),
                state,
                page,
            }) as Box<dyn ComponentInstance>)
        })
    }
}

struct SearchComponent {
    control: Arc<SearchControl>,
    state: SearchState,
    page: u64,
}

impl ComponentInstance for SearchComponent {
    fn metadata(&self) -> &'static suprnova_live::metadata::ComponentMetadata {
        promotion_support::promotion_metadata()
    }

    fn bind_models(&mut self, proposals: &ProposalBatch) -> Result<(), ComponentError> {
        self.control.binds.fetch_add(1, Ordering::SeqCst);
        let path = ModelPath::parse("count").map_err(|_| ComponentError::contract_failure())?;
        match proposals.apply_required(&path, &mut self.state, |state, count: u64| {
            state.count = count;
        }) {
            ProposalApplication::Applied => Ok(()),
            _ => Err(ComponentError::contract_failure()),
        }
    }

    fn render<'a>(
        &'a self,
        _context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<IslandRender, ComponentError>> {
        Box::pin(async move {
            Ok(IslandRender {
                body: Bytes::from(format!("<p>{}</p>", self.state.count)),
                assets: AssetSet::empty(),
                children: Vec::new(),
            })
        })
    }

    fn dehydrate(&self, _exposure: StateExposure) -> Result<CanonicalValue, ComponentError> {
        Ok(encode_search_state(&self.state))
    }

    fn dehydrate_memo(&self) -> Result<CanonicalValue, ComponentError> {
        Ok(CanonicalValue::Object(BTreeMap::from([(
            "page".to_owned(),
            canonical_u64(self.page),
        )])))
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

fn search_action<'a>(
    target: &'a mut dyn ActionTarget,
    _authorization: &'a AuthorizedAction,
    _arguments: &'a PreparedActionArguments,
) -> ActionFuture<'a, Result<ActionResult, ActionError>> {
    Box::pin(async move {
        let target = target
            .as_any_mut()
            .downcast_mut::<SearchComponent>()
            .ok_or_else(ActionError::dispatcher_contract)?;
        target.control.actions.fetch_add(1, Ordering::SeqCst);
        target
            .control
            .action_observations
            .lock()
            .expect("observation lock")
            .push(target.state.clone());
        target.state.count += 1;
        Ok(ActionResult::render())
    })
}

fn search_descriptor(control: Arc<SearchControl>) -> ComponentDescriptor {
    let metadata = promotion_support::promotion_metadata();
    ComponentDescriptor::with_hooks(
        metadata.clone(),
        ComponentHooks::new(Arc::new(SearchFactory { control })),
    )
    .with_actions(
        ActionTable::new(vec![ActionEntry::new(
            metadata.actions()[0].clone(),
            search_action,
        )])
        .expect("action table"),
    )
    .expect("matching action table")
}

fn proposal_batch(count: u64) -> ProposalBatch {
    let schema = ModelBindingSchema::new(vec![
        ModelFieldBinding::new("count", FieldCategory::Model, ModelCodec::U64)
            .expect("model binding"),
    ])
    .expect("model schema");
    ProposalBatch::prepare(
        &schema,
        vec![RawModelProposal::new("count", canonical_u64(count))],
        &ProposalLimits::default(),
    )
    .expect("proposal batch")
}

fn action_request<'a>(
    action: &'a ActionName,
    limits: &'a InputLimits,
    validation: &'a suprnova_live::validation::ValidationEngine,
    proposals: &'a ProposalBatch,
) -> ActionExecutionRequest<'a> {
    ActionExecutionRequest::new(
        action,
        RawActionArguments::empty(),
        limits,
        validation,
        &PassValidation,
        BagPolicy::Replace,
        None,
        &NoopExecutionTrace,
    )
    .with_proposals(proposals)
}

fn expected_instance(
    context: &suprnova_live::host::TrustedLiveRequestContext,
) -> ExpectedInstanceV1 {
    ExpectedInstanceV1::new(
        promotion_support::promotion_component_contract(),
        BuildId::parse("build-2026-08-21").expect("build identity"),
        promotion_support::snapshot_support::route(1),
        IslandSlot::parse("search-results").expect("slot identity"),
        context.scope().clone(),
        promotion_support::promotion_schema_set(),
    )
}

#[tokio::test]
async fn first_promoted_action_mounts_overlays_then_binds_before_observation() {
    let harness = harness(promotion_limits(), 64);
    let context = trusted_context_for_route(0xa0, 1);
    let promoted = harness
        .service
        .promote(
            &signed_seed_with_refresh(&harness.keys, "cached", false),
            nonce(0x20),
            &context.for_promotion(),
        )
        .await
        .expect("seed promotes");
    let control = SearchControl::new(false);
    let descriptor = search_descriptor(control.clone());
    let service = ExecutionService::new(
        harness.ledger.clone(),
        harness.clock.clone(),
        harness.keys.clone(),
        harness.snapshot_limits.clone(),
        ViewRenderer::new(RenderLimits::standard()).expect("renderer"),
    );
    let action = ActionName::parse("search").expect("action identity");
    let input_limits = InputLimits::default();
    let validation =
        suprnova_live::validation::ValidationEngine::new(16).expect("validation engine");
    let proposals = proposal_batch(7);

    let result = service
        .execute_promoted(PromotedActionRequest::new(
            &descriptor,
            &context,
            browser_context(),
            promoted,
            promotion_support::idempotency(0x40),
            promotion_support::digest(0x50),
            action_request(&action, &input_limits, &validation, &proposals),
        ))
        .await;
    let ExecutionResult::Accepted(accepted) = result else {
        panic!("first action must be accepted");
    };
    assert!(accepted.action_executed());
    let successor_html =
        std::str::from_utf8(&accepted.render().expect("promoted successor render").body)
            .expect("successor HTML");
    assert!(successor_html.contains("data-suprnova-live-snapshot-kind=\"instance\""));
    assert!(successor_html.contains("data-suprnova-live-instance="));
    assert!(!successor_html.contains("data-suprnova-live-snapshot-kind=\"seed\""));
    let verified = verify_instance(
        accepted.signed_snapshot(),
        &expected_instance(&context),
        &harness.keys,
        UnixMillis::new(1_000),
        &harness.snapshot_limits,
    )
    .expect("first publishable instance verifies");
    assert_eq!(
        verified
            .hydrate_state::<SearchState>(promotion_support::promotion_schema_set().state())
            .expect("complete state"),
        SearchState {
            query: "cached".to_owned(),
            selected: "1".to_owned(),
            server: "authoritative".to_owned(),
            count: 8,
        }
    );
    assert_eq!(
        verified
            .hydrate_memo::<BTreeMap<String, u64>>(
                promotion_support::promotion_schema_set().memo(),
            )
            .expect("memo")["page"],
        1
    );
    assert_eq!(control.mounts.load(Ordering::SeqCst), 1);
    assert_eq!(control.hydrations.load(Ordering::SeqCst), 1);
    assert_eq!(control.binds.load(Ordering::SeqCst), 1);
    assert_eq!(control.actions.load(Ordering::SeqCst), 1);
    assert_eq!(
        control
            .action_observations
            .lock()
            .expect("observation lock")
            .as_slice(),
        [SearchState {
            query: "cached".to_owned(),
            selected: "1".to_owned(),
            server: "authoritative".to_owned(),
            count: 7,
        }]
    );
}

#[tokio::test]
async fn promotion_mount_failure_consumes_authority_without_action_or_partial_snapshot() {
    let harness = harness(promotion_limits(), 64);
    let context = trusted_context_for_route(0xa1, 1);
    let promoted = harness
        .service
        .promote(
            &signed_seed_with_refresh(&harness.keys, "cached", false),
            nonce(0x21),
            &context.for_promotion(),
        )
        .await
        .expect("seed promotes");
    let instance_id = promoted.instance_id().clone();
    let control = SearchControl::new(true);
    let descriptor = search_descriptor(control.clone());
    let service = ExecutionService::new(
        harness.ledger.clone(),
        harness.clock.clone(),
        harness.keys.clone(),
        harness.snapshot_limits.clone(),
        ViewRenderer::new(RenderLimits::standard()).expect("renderer"),
    );
    let action = ActionName::parse("search").expect("action identity");
    let input_limits = InputLimits::default();
    let validation =
        suprnova_live::validation::ValidationEngine::new(16).expect("validation engine");
    let proposals = proposal_batch(7);

    let result = service
        .execute_promoted(PromotedActionRequest::new(
            &descriptor,
            &context,
            browser_context(),
            promoted,
            promotion_support::idempotency(0x41),
            promotion_support::digest(0x51),
            action_request(&action, &input_limits, &validation, &proposals),
        ))
        .await;
    let ExecutionResult::RefreshRequired(refresh) = result else {
        panic!("mount failure must refresh");
    };
    assert_eq!(
        refresh.reason(),
        suprnova_live::execution::ExecutionRefreshReason::ExecutionFailed
    );
    assert_eq!(refresh.retry_legality(), RetryLegality::Prohibited);
    assert_eq!(control.actions.load(Ordering::SeqCst), 0);
    let inspection = harness
        .ledger
        .inspect(context.scope(), &instance_id)
        .expect("ledger inspection")
        .expect("promoted instance remains inspectable");
    assert_eq!(inspection.phase(), LedgerPhase::Consumed);
    assert_eq!(inspection.accepted_outcome_count(), 0);
}

#[tokio::test]
async fn refresh_on_promote_publishes_fresh_mount_and_discards_original_operation() {
    let harness = harness(promotion_limits(), 64);
    let context = trusted_context_for_route(0xa2, 1);
    let promoted = harness
        .service
        .promote(
            &signed_seed_with_refresh(&harness.keys, "cached", true),
            nonce(0x22),
            &context.for_promotion(),
        )
        .await
        .expect("seed promotes");
    let control = SearchControl::new(false);
    let descriptor = search_descriptor(control.clone());
    let service = ExecutionService::new(
        harness.ledger.clone(),
        harness.clock.clone(),
        harness.keys.clone(),
        harness.snapshot_limits.clone(),
        ViewRenderer::new(RenderLimits::standard()).expect("renderer"),
    );
    let action = ActionName::parse("search").expect("action identity");
    let input_limits = InputLimits::default();
    let validation =
        suprnova_live::validation::ValidationEngine::new(16).expect("validation engine");
    let proposals = proposal_batch(99);

    let result = service
        .execute_promoted(PromotedActionRequest::new(
            &descriptor,
            &context,
            browser_context(),
            promoted,
            promotion_support::idempotency(0x42),
            promotion_support::digest(0x52),
            action_request(&action, &input_limits, &validation, &proposals),
        ))
        .await;
    let ExecutionResult::Accepted(accepted) = result else {
        panic!("fresh recovery must be accepted");
    };
    assert!(!accepted.action_executed());
    let verified = verify_instance(
        accepted.signed_snapshot(),
        &expected_instance(&context),
        &harness.keys,
        UnixMillis::new(1_000),
        &harness.snapshot_limits,
    )
    .expect("fresh instance verifies");
    assert_eq!(
        verified
            .hydrate_state::<SearchState>(promotion_support::promotion_schema_set().state())
            .expect("complete state"),
        SearchState {
            query: "fresh".to_owned(),
            selected: "fresh".to_owned(),
            server: "authoritative".to_owned(),
            count: 1,
        }
    );
    assert_eq!(
        verified
            .hydrate_memo::<BTreeMap<String, u64>>(
                promotion_support::promotion_schema_set().memo(),
            )
            .expect("memo")["page"],
        9
    );
    assert_eq!(control.mounts.load(Ordering::SeqCst), 1);
    assert_eq!(control.hydrations.load(Ordering::SeqCst), 0);
    assert_eq!(control.binds.load(Ordering::SeqCst), 0);
    assert_eq!(control.actions.load(Ordering::SeqCst), 0);
}

fn encode_search_state(state: &SearchState) -> CanonicalValue {
    CanonicalValue::Object(BTreeMap::from([
        ("count".to_owned(), canonical_u64(state.count)),
        (
            "query".to_owned(),
            CanonicalValue::String(state.query.clone()),
        ),
        (
            "selected".to_owned(),
            CanonicalValue::String(state.selected.clone()),
        ),
        (
            "server".to_owned(),
            CanonicalValue::String(state.server.clone()),
        ),
    ]))
}

fn decode_search_state(value: &CanonicalValue) -> Result<SearchState, ComponentError> {
    let CanonicalValue::Object(fields) = value else {
        return Err(ComponentError::contract_failure());
    };
    Ok(SearchState {
        query: string_field(fields, "query")?,
        selected: string_field(fields, "selected")?,
        server: string_field(fields, "server")?,
        count: number_field(fields, "count")?,
    })
}

fn decode_page(value: &CanonicalValue) -> Result<u64, ComponentError> {
    let CanonicalValue::Object(fields) = value else {
        return Err(ComponentError::contract_failure());
    };
    number_field(fields, "page")
}

fn string_field(
    fields: &BTreeMap<String, CanonicalValue>,
    name: &str,
) -> Result<String, ComponentError> {
    let Some(CanonicalValue::String(value)) = fields.get(name) else {
        return Err(ComponentError::contract_failure());
    };
    Ok(value.clone())
}

fn number_field(
    fields: &BTreeMap<String, CanonicalValue>,
    name: &str,
) -> Result<u64, ComponentError> {
    let Some(CanonicalValue::Number(value)) = fields.get(name) else {
        return Err(ComponentError::contract_failure());
    };
    let value = value.get();
    if value < 0.0 || value.fract() != 0.0 || value > u64::MAX as f64 {
        return Err(ComponentError::contract_failure());
    }
    Ok(value as u64)
}

fn canonical_u64(value: u64) -> CanonicalValue {
    CanonicalValue::Number(
        CanonicalNumber::new(value as f64).expect("small test integer is canonical"),
    )
}
