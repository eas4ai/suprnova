//! Production async-transport fixture owned by the deterministic reference host.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::num::NonZeroU8;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll, Waker};

use axum::body::Bytes;
use axum::http::Method;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use suprnova_live::action::{
    ActionArgumentSchema, ActionAuthorizationPort, ActionAuthorizationRequest, ActionEntry,
    ActionError, ActionFuture, ActionResult, ActionTable, ActionTarget, AuthorizationDecision,
    AuthorizationRequirement, AuthorizedAction, PreparedActionArguments, RawActionArguments,
    TransactionPolicy,
};
use suprnova_live::async_updates::{
    AsyncEnvelope, AsyncEventSession, AsyncEventSource, AsyncMembershipRegistryPort,
    AsyncMembershipRequest, AsyncMembershipValidation, AsyncTransportAuthorityPort,
    AsyncTransportAuthorityRequest, AsyncTransportAuthorityValidation, AsyncTransportError,
    AsyncTransportFuture, AuthoritativeStreamPosition, AuthorizedSubscription,
    AuthorizedTransportSubscription, BoundedEventNames, BoundedPresentationSignalContracts,
    BoundedTargets, BoundedTopics, BrowserPayloadSchema, CapabilityVersion, CloseDisposition,
    CurrentSubscriptionRegistration, DocumentAuthorizationScope, DocumentTransportHandle,
    DocumentTransportKind, DocumentTransportLimits, DocumentTransportSession, EventCyclePolicy,
    EventOrder, EventSource, EventTarget, Heartbeat, MAX_ASYNC_BUFFER_EVENTS, PollFallbackPolicy,
    PollInitialBehavior, PollVisibilityPolicy, ReconnectPolicy, RegisteredBrowserEvent,
    ResolvedEventFanout, StreamEpoch, StreamName, StreamPosition, StreamSequence,
    SubscriptionAuthorizationDecision, SubscriptionAuthorizationPort,
    SubscriptionAuthorizationRequest, SubscriptionBaselineRequest, SubscriptionContinuityPort,
    SubscriptionCredentialPort, SubscriptionCredentialRequest,
    SubscriptionCredentialRotationOutcome, SubscriptionCredentialRotationRequest,
    SubscriptionError, SubscriptionId, SubscriptionIssueRequest, SubscriptionMetadata,
    SubscriptionMode, SubscriptionModes, SubscriptionRegistryPort, SubscriptionRegistryRequest,
    SubscriptionService, TransportCredential, TransportMembershipOperation, TrustedMountParameters,
    VerifiedOrigin,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::component::{
    ComponentError, ComponentFactory, ComponentHooks, ComponentInstance, HydrationContext,
    LiveFuture, MountContext, RenderContext,
};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::endpoint::{
    EndpointDispatch, EndpointFuture, EndpointKernel, EndpointKernelError, EndpointOutcomeKind,
    LIVE_MEDIA_TYPE_V2, LiveEndpointConfig, LiveEndpointRequest, LiveEndpointResponse,
    LiveEndpointService, ParsedLiveMediaType, RequestCachePolicy, VerifiedEndpointRequest,
    VerifiedEndpointSnapshot,
};
use suprnova_live::execution::ExecutionResult;
use suprnova_live::host::{
    CheckDisposition, CheckFact, CheckKind, HostCapabilities, HostCheckFacts, HostScopeFacts,
    LiveRequestContextCandidate, LiveRequestContextValidator, MountCatalogBuilder,
    MountCatalogEntry, MountScopeRequirements, MountSelection, PrincipalFingerprint,
    ScopeRequirement, SessionFingerprint, TenantFingerprint, TrustedLiveRequestContext,
};
use suprnova_live::identity::{
    ActionName, BrowserOperationName, BuildId, ComponentName, ContentDigest, IslandSlot, KeyId,
    ModelField, RouteIdentity, ScopeFingerprint, UnixMillis, ViewName,
};
use suprnova_live::ledger::{
    AcceptedOutcome, ClaimOutcome, ClaimRequest, ClaimToken, InstanceAuthority, LedgerError,
    LedgerLimits, LiveInstanceLedger, MemoryInstanceLedger, MountInstanceRecord, PromotionOutcome,
    PromotionRecord,
};
use suprnova_live::limits::InputLimits;
use suprnova_live::metadata::{
    ActionMetadata, ComponentMetadata, ContractVersions, EventMetadata, EventPayloadMetadata,
    FieldMetadata,
};
use suprnova_live::protocol::{
    OperationV2, ProtocolLimitConfig, ProtocolLimits, SemanticIdempotencyInputV1, SnapshotInput,
    VersionedUpdateRequest, semantic_idempotency_digest_v1,
};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
use suprnova_live::snapshot::state::{
    FieldCategory, FieldSpec, SnapshotSchemaSet, StateCodec, StateSchema,
};
use suprnova_live::snapshot::{
    ComponentContract, ExpectedInstanceV1, ExpectedSeedV1, SnapshotLimits,
};
use suprnova_live::validation::ValidationSelection;
use suprnova_live::view::{AssetSet, IslandRender};
use tokio::sync::{Mutex as AsyncMutex, Notify};

use crate::{ComponentHarness, ComponentHarnessConfig, HarnessRequestIdentity, HarnessServices};

type EngineFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

const ENGINE_NOW: UnixMillis = UnixMillis::new(1_000);

struct OrdersUpdated;

impl EventPayloadMetadata for OrdersUpdated {
    const NAME: &'static str = "orders.updated";
    const VERSION: u16 = 1;
    const PAYLOAD_CONTRACT: &'static str = "orders.updated.payload";
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
}

struct FreshRenderFactory {
    replace_upload_on_successor: bool,
}

#[derive(Default)]
struct FreshRenderPause {
    enabled: AtomicBool,
    entered_commit: AtomicBool,
    entered: Notify,
    release: Notify,
}

impl ComponentFactory for FreshRenderFactory {
    fn mount<'a>(
        &'a self,
        _context: &'a MountContext<'a>,
    ) -> LiveFuture<'a, Result<Box<dyn ComponentInstance>, ComponentError>> {
        Box::pin(async move {
            Ok(Box::new(FreshRenderComponent {
                domain_count: 0,
                replace_upload_on_successor: self.replace_upload_on_successor,
            }) as Box<dyn ComponentInstance>)
        })
    }

    fn hydrate<'a>(
        &'a self,
        context: &'a HydrationContext<'a>,
    ) -> LiveFuture<'a, Result<Box<dyn ComponentInstance>, ComponentError>> {
        Box::pin(async move {
            let CanonicalValue::Object(fields) = context.state() else {
                return Err(ComponentError::contract_failure());
            };
            let Some(CanonicalValue::String(domain_count)) = fields.get("domain_count") else {
                return Err(ComponentError::contract_failure());
            };
            let domain_count = domain_count
                .parse::<u64>()
                .map_err(|_| ComponentError::contract_failure())?;
            Ok(Box::new(FreshRenderComponent {
                domain_count,
                replace_upload_on_successor: self.replace_upload_on_successor,
            }) as Box<dyn ComponentInstance>)
        })
    }
}

struct FreshRenderComponent {
    domain_count: u64,
    replace_upload_on_successor: bool,
}

fn upload_controls(replacement: bool) -> String {
    let suffix = if replacement { "-replacement" } else { "" };
    let key_suffix = if replacement { "replacement" } else { "stable" };
    format!(
        "<label for=\"attachment-input{suffix}\">Attachment</label><input id=\"attachment-input{suffix}\" type=\"file\" live:upload=\"attachment\" data-suprnova-live-key=\"attachment-input-{key_suffix}\"><div id=\"attachment-progress{suffix}\" live:progress=\"attachment\" data-suprnova-live-key=\"attachment-progress-{key_suffix}\" role=\"progressbar\" aria-label=\"Attachment upload progress\" aria-valuemin=\"0\" aria-valuemax=\"100\" aria-valuenow=\"0\"></div><button id=\"attachment-cancel{suffix}\" type=\"button\" live:upload.cancel=\"attachment\" data-suprnova-live-key=\"attachment-cancel-{key_suffix}\">Cancel upload</button><button id=\"attachment-retry{suffix}\" type=\"button\" live:upload.retry=\"attachment\" data-suprnova-live-key=\"attachment-retry-{key_suffix}\">Retry upload</button><button id=\"attachment-remove{suffix}\" type=\"button\" live:upload.remove=\"attachment\" data-suprnova-live-key=\"attachment-remove-{key_suffix}\">Remove upload</button>"
    )
}

struct EngineActionAuthorization;

impl ActionAuthorizationPort for EngineActionAuthorization {
    fn authorize<'a>(
        &'a self,
        _request: ActionAuthorizationRequest<'a>,
    ) -> ActionFuture<'a, Result<AuthorizationDecision, ActionError>> {
        Box::pin(async { Ok(AuthorizationDecision::Allow) })
    }
}

fn increment_domain<'a>(
    target: &'a mut dyn ActionTarget,
    _authorization: &'a AuthorizedAction,
    _arguments: &'a PreparedActionArguments,
) -> ActionFuture<'a, Result<ActionResult, ActionError>> {
    Box::pin(async move {
        let target = target
            .as_any_mut()
            .downcast_mut::<FreshRenderComponent>()
            .ok_or_else(ActionError::dispatcher_contract)?;
        target.domain_count = target
            .domain_count
            .checked_add(1)
            .ok_or_else(ActionError::dispatcher_contract)?;
        Ok(ActionResult::render())
    })
}

impl ComponentInstance for FreshRenderComponent {
    fn metadata(&self) -> &'static ComponentMetadata {
        engine_metadata()
    }

    fn render<'a>(
        &'a self,
        context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<IslandRender, ComponentError>> {
        Box::pin(async move {
            let (replacement_id, replacement_tag) = if context.revision().get() == 0 {
                ("fresh-render-replacement-old", "section")
            } else {
                ("fresh-render-replacement", "article")
            };
            let replace_upload = self.replace_upload_on_successor && context.revision().get() > 0;
            let upload_controls = upload_controls(replace_upload);
            Ok(IslandRender {
                body: Bytes::from(format!(
                    "<button id=\"fresh-render-preserved\" type=\"button\" data-suprnova-live-key=\"fresh-render-preserved\">Preserved focus target</button><output data-live-domain-count=\"{}\">{}</output>{upload_controls}<{replacement_tag} id=\"{replacement_id}\" data-live-poll-generation=\"{}\" data-live-render-source=\"component-harness\"></{replacement_tag}>",
                    self.domain_count,
                    self.domain_count,
                    context.revision().get(),
                )),
                assets: AssetSet::empty(),
                children: Vec::new(),
            })
        })
    }

    fn dehydrate(
        &self,
        _exposure: suprnova_live::snapshot::state::StateExposure,
    ) -> Result<CanonicalValue, ComponentError> {
        Ok(CanonicalValue::Object(BTreeMap::from([(
            "domain_count".to_owned(),
            CanonicalValue::String(self.domain_count.to_string()),
        )])))
    }

    fn dehydrate_memo(&self) -> Result<CanonicalValue, ComponentError> {
        Ok(CanonicalValue::Object(Default::default()))
    }
}

struct RollbackableFreshRenderLedger {
    inner: MemoryInstanceLedger,
    pause: Arc<FreshRenderPause>,
}

#[async_trait::async_trait]
impl LiveInstanceLedger for RollbackableFreshRenderLedger {
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
        if self.pause.enabled.load(Ordering::Acquire) {
            self.pause.entered_commit.store(true, Ordering::Release);
            self.pause.entered.notify_one();
            self.pause.release.notified().await;
            self.pause.entered_commit.store(false, Ordering::Release);
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

struct EngineSubscriptionPorts {
    component: ComponentMetadata,
    parameters: TrustedMountParameters,
}

impl SubscriptionRegistryPort for EngineSubscriptionPorts {
    fn resolve<'a>(
        &'a self,
        request: SubscriptionRegistryRequest<'a>,
    ) -> EngineFuture<'a, Result<CurrentSubscriptionRegistration, SubscriptionError>> {
        Box::pin(async move {
            CurrentSubscriptionRegistration::from_registered(
                &self.component,
                request.stream(),
                &self.parameters,
            )
        })
    }
}

impl SubscriptionContinuityPort for EngineSubscriptionPorts {
    fn authoritative_baseline<'a>(
        &'a self,
        _request: SubscriptionBaselineRequest<'a>,
    ) -> EngineFuture<'a, Result<AuthoritativeStreamPosition, SubscriptionError>> {
        Box::pin(async {
            Ok(AuthoritativeStreamPosition::from_host_continuity(
                StreamPosition::new(StreamEpoch::new(1), StreamSequence::new(0)),
            ))
        })
    }
}

impl SubscriptionAuthorizationPort for EngineSubscriptionPorts {
    fn authorize<'a>(
        &'a self,
        _request: SubscriptionAuthorizationRequest<'a>,
    ) -> EngineFuture<'a, Result<SubscriptionAuthorizationDecision, SubscriptionError>> {
        Box::pin(async { Ok(SubscriptionAuthorizationDecision::Allow) })
    }
}

impl SubscriptionCredentialPort for EngineSubscriptionPorts {
    fn issue<'a>(
        &'a self,
        _request: SubscriptionCredentialRequest<'a>,
    ) -> EngineFuture<'a, Result<TransportCredential, SubscriptionError>> {
        Box::pin(async { TransportCredential::from_host_authority_bearer(vec![0x71; 32]) })
    }

    fn consume_and_rotate<'a>(
        &'a self,
        _request: SubscriptionCredentialRotationRequest<'a>,
    ) -> EngineFuture<'a, SubscriptionCredentialRotationOutcome> {
        Box::pin(async {
            match TransportCredential::from_host_authority_bearer(vec![0x72; 32]) {
                Ok(credential) => SubscriptionCredentialRotationOutcome::Rotated(credential),
                Err(_) => SubscriptionCredentialRotationOutcome::Failed,
            }
        })
    }
}

struct EngineMembershipRegistry {
    subscriptions: Mutex<Vec<SubscriptionId>>,
    stream: StreamName,
    topics: BoundedTopics,
    events: suprnova_live::async_updates::BoundedEventContracts,
    signals: BoundedPresentationSignalContracts,
    modes: SubscriptionModes,
    authorization_memo: suprnova_live::async_updates::AuthorizationMemo,
    document_scope: DocumentAuthorizationScope,
    resolved_event_fanout: AtomicU64,
}

struct FreshRenderKernel {
    harness: Arc<AsyncMutex<ComponentHarness>>,
}

impl EndpointKernel for FreshRenderKernel {
    fn dispatch<'request>(
        &'request self,
        request: VerifiedEndpointRequest<'request>,
    ) -> EndpointFuture<'request> {
        Box::pin(async move {
            let VersionedUpdateRequest::V2(parsed) = request.request() else {
                return Ok(EndpointDispatch::new(
                    EndpointOutcomeKind::Concealed,
                    Bytes::new(),
                ));
            };
            if parsed.operations() != [OperationV2::FreshRender] {
                return Ok(EndpointDispatch::new(
                    EndpointOutcomeKind::Concealed,
                    Bytes::new(),
                ));
            }
            let (
                VerifiedEndpointSnapshot::Instance(verified),
                SnapshotInput::Instance { envelope },
            ) = (request.snapshot(), parsed.snapshot())
            else {
                return Ok(EndpointDispatch::new(
                    EndpointOutcomeKind::Concealed,
                    Bytes::new(),
                ));
            };
            let mut harness = self.harness.lock().await;
            if harness.current_encoded_snapshot() != Some(envelope.as_slice()) {
                return Ok(EndpointDispatch::new(
                    EndpointOutcomeKind::Concealed,
                    Bytes::new(),
                ));
            }
            let authority = ContentDigest::from_bytes(&Sha256::digest(envelope))
                .map_err(|_| EndpointKernelError::unavailable())?;
            let digest = semantic_idempotency_digest_v1(&SemanticIdempotencyInputV1::new(
                request.context().scope().clone(),
                verified.body().instance_id().clone(),
                request.context().mount().contract_digest().clone(),
                authority,
                request.request(),
            ))
            .map_err(|_| EndpointKernelError::unavailable())?;
            let result = harness
                .execute_fresh_render(parsed.idempotency_key().clone(), digest)
                .await
                .map_err(|_| EndpointKernelError::unavailable())?;
            let ExecutionResult::Accepted(accepted) = result else {
                return Ok(EndpointDispatch::new(
                    EndpointOutcomeKind::Concealed,
                    Bytes::new(),
                ));
            };
            let snapshot: Value = serde_json::from_slice(accepted.signed_snapshot())
                .map_err(|_| EndpointKernelError::unavailable())?;
            let render = accepted
                .render()
                .ok_or_else(EndpointKernelError::unavailable)?;
            let html = String::from_utf8(render.body.to_vec())
                .map_err(|_| EndpointKernelError::unavailable())?;
            let response = serde_json::to_vec(&json!({
                "accepted_revision": accepted.revision().get().to_string(),
                "child_deliveries": [],
                "correlation_id": parsed.correlation_id().to_base64url(),
                "effects": [],
                "events": [],
                "extensions": {},
                "outcome": "accepted",
                "protocol_version": 2,
                "render": { "html": html, "kind": "html" },
                "snapshot": snapshot,
                "url_intent": null,
                "validation": {},
            }))
            .map_err(|_| EndpointKernelError::unavailable())?;
            Ok(EndpointDispatch::new(
                EndpointOutcomeKind::Accepted,
                Bytes::from(response),
            ))
        })
    }
}

pub(super) struct ReferenceFreshRender {
    endpoint: LiveEndpointService,
    harness: Arc<AsyncMutex<ComponentHarness>>,
    initial_html: String,
    ports: Arc<EngineSubscriptionPorts>,
    pause: Arc<FreshRenderPause>,
    action_sequence: AtomicU64,
}

impl ReferenceFreshRender {
    async fn new(
        ports: Arc<EngineSubscriptionPorts>,
        replace_upload_on_successor: bool,
    ) -> Result<Self, String> {
        let metadata = engine_metadata().clone();
        let (context, _) = engine_context(metadata.clone(), ports.clone())?;
        let pause = Arc::new(FreshRenderPause::default());
        let actions = ActionTable::new(vec![ActionEntry::new(
            increment_action_metadata(),
            increment_domain,
        )])
        .map_err(|_| "fresh render actions")?;
        let descriptor = ComponentDescriptor::with_hooks(
            metadata.clone(),
            ComponentHooks::new(Arc::new(FreshRenderFactory {
                replace_upload_on_successor,
            })),
        )
        .with_actions(actions)
        .map_err(|_| "fresh render actions")?;
        let expected = ExpectedInstanceV1::new(
            ComponentContract::new(
                metadata.identity().clone(),
                descriptor.contract_digest().clone(),
                1,
                1,
                1,
            )
            .map_err(|_| "fresh render contract")?,
            BuildId::parse("build-reference-host").map_err(|_| "fresh render build")?,
            RouteIdentity::from_bytes(&[0x74; 32]).map_err(|_| "fresh render route")?,
            IslandSlot::parse("reference-uploads").map_err(|_| "fresh render slot")?,
            context.scope().clone(),
            engine_schemas()?,
        );
        let snapshot_limits = reference_snapshot_limits()?;
        let services = HarnessServices::new(ENGINE_NOW);
        let clock = Arc::clone(services.clock());
        let ledger_limits =
            LedgerLimits::new(1_000, 60_000, 16, 256).map_err(|_| "fresh render ledger limits")?;
        let ledger = Arc::new(RollbackableFreshRenderLedger {
            inner: MemoryInstanceLedger::new(
                Arc::clone(&clock) as Arc<dyn suprnova_live::clock::Clock>,
                ledger_limits,
            ),
            pause: Arc::clone(&pause),
        });
        let mut harness = ComponentHarness::new(
            ComponentHarnessConfig::new(
                descriptor.clone(),
                context,
                expected,
                reference_key_ring(),
                snapshot_limits.clone(),
                services,
            )
            .with_instance_ledger(ledger),
        )
        .map_err(|_| "fresh render harness")?;
        let mounted = harness
            .mount(CanonicalValue::Object(Default::default()))
            .await
            .map_err(|_| "fresh render mount")?;
        let initial_html = String::from_utf8(mounted.body().to_vec())
            .map_err(|_| "fresh render initial encoding")?
            .replacen("<div ", "<div live:poll.immediate=\"\" ", 1);
        let harness = Arc::new(AsyncMutex::new(harness));
        let registry = Arc::new(
            ComponentRegistryBuilder::new()
                .register(descriptor)
                .map_err(|_| "fresh render registry")?
                .build(),
        );
        let endpoint = LiveEndpointService::new(
            LiveEndpointConfig::new(reference_protocol_limits(), snapshot_limits)
                .map_err(|_| "fresh render endpoint config")?,
            registry,
            clock,
            Arc::new(reference_key_ring()),
            Arc::new(FreshRenderKernel {
                harness: Arc::clone(&harness),
            }),
        );
        Ok(Self {
            endpoint,
            harness,
            initial_html,
            ports,
            pause,
            action_sequence: AtomicU64::new(0),
        })
    }

    pub(super) async fn execute_ordinary_action(&self) -> Result<Value, &'static str> {
        let sequence = self.action_sequence.fetch_add(1, Ordering::SeqCst);
        let mut harness = self.harness.lock().await;
        let result = harness
            .execute_action(
                &ActionName::parse("increment").map_err(|_| "ordinary action identity")?,
                RawActionArguments::empty(),
                None,
                HarnessRequestIdentity::from_counter(sequence),
            )
            .await
            .map_err(|_| "ordinary action execution")?;
        let ExecutionResult::Accepted(accepted) = result else {
            return Err("ordinary action rejected");
        };
        if !accepted.action_executed() {
            return Err("ordinary action not executed");
        }
        let html = accepted
            .render()
            .and_then(|render| String::from_utf8(render.body.to_vec()).ok())
            .ok_or("ordinary action render")?;
        let revision = accepted.revision().get();
        let marker = "data-live-domain-count=\"";
        let domain_count = html
            .split_once(marker)
            .and_then(|(_, tail)| tail.split_once('\"'))
            .and_then(|(value, _)| value.parse::<u64>().ok())
            .ok_or("ordinary action domain state")?;
        Ok(json!({
            "action": "increment",
            "domain_count": domain_count,
            "html": html,
            "revision": revision,
        }))
    }

    pub(super) async fn request(
        &self,
        correlation: &str,
        seed: u8,
    ) -> Result<String, &'static str> {
        let harness = self.harness.lock().await;
        let snapshot = harness
            .current_encoded_snapshot()
            .ok_or("fresh render snapshot")?;
        let snapshot: Value =
            serde_json::from_slice(snapshot).map_err(|_| "fresh render snapshot")?;
        let current = harness.current_snapshot().ok_or("fresh render snapshot")?;
        let mut idempotency = [seed; 16];
        idempotency[15] = seed.wrapping_add(1).max(1);
        let idempotency = suprnova_live::identity::IdempotencyKey::from_bytes(&idempotency)
            .map_err(|_| "fresh render idempotency")?;
        serde_json::to_string(&json!({
            "base_revision": current.body().revision().get().to_string(),
            "child_parameters": null,
            "component": engine_metadata().identity().as_str(),
            "correlation_id": correlation,
            "extensions": { "x_suprnova_live_document_key_v1": "harness-root" },
            "idempotency_key": idempotency.to_base64url(),
            "model_proposals": {},
            "operations": [{ "kind": "fresh_render" }],
            "protocol_version": 2,
            "runtime_contract_version": 2,
            "snapshot": { "envelope": snapshot, "kind": "instance" },
            "snapshot_schema_version": 1,
        }))
        .map_err(|_| "fresh render request")
    }

    pub(super) async fn handle(&self, body: Bytes) -> Result<LiveEndpointResponse, &'static str> {
        let (context, _) = engine_context(engine_metadata().clone(), self.ports.clone())
            .map_err(|_| "fresh render context")?;
        let request = LiveEndpointRequest::try_new(
            Method::POST,
            ParsedLiveMediaType::parse(LIVE_MEDIA_TYPE_V2).map_err(|_| "fresh render media")?,
            body,
            Some(context),
            RequestCachePolicy::Bypass,
        )
        .map_err(|_| "fresh render request")?;
        Ok(self.endpoint.handle(request).await)
    }

    pub(super) fn initial_html(&self) -> &str {
        &self.initial_html
    }

    pub(super) fn pause_render(&self) {
        self.pause.enabled.store(true, Ordering::Release);
    }

    pub(super) async fn wait_until_render_paused(&self) {
        self.pause.entered.notified().await;
    }

    pub(super) fn resume_render(&self) {
        self.pause.enabled.store(false, Ordering::Release);
        self.pause.release.notify_waiters();
    }

    pub(super) fn render_paused(&self) -> bool {
        self.pause.entered_commit.load(Ordering::Acquire)
    }
}

impl EngineMembershipRegistry {
    fn activate(&self, subscription: SubscriptionId) {
        let mut subscriptions = self.subscriptions.lock().expect("engine membership lock");
        if !subscriptions
            .iter()
            .any(|candidate| candidate == &subscription)
        {
            subscriptions.push(subscription);
        }
    }

    fn remove(&self, subscription: &SubscriptionId) {
        self.subscriptions
            .lock()
            .expect("engine membership lock")
            .retain(|candidate| candidate != subscription);
    }

    fn contains(&self, subscription: &SubscriptionId) -> bool {
        self.subscriptions
            .lock()
            .expect("engine membership lock")
            .iter()
            .any(|candidate| candidate == subscription)
    }
}

impl AsyncMembershipRegistryPort for EngineMembershipRegistry {
    fn validate_current(
        &self,
        request: AsyncMembershipRequest<'_>,
        validation: &mut AsyncMembershipValidation<'_>,
    ) {
        if !self.contains(request.subscription()) {
            return;
        }
        if request.envelope().is_some() {
            let resolved = match request.envelope().map(AsyncEnvelope::payload) {
                Some(suprnova_live::async_updates::AsyncPayload::BrowserEvent(_)) => {
                    std::num::NonZeroU16::new(
                        u16::try_from(self.resolved_event_fanout.load(Ordering::Acquire))
                            .unwrap_or(u16::MAX),
                    )
                    .map(|recipients| {
                        ResolvedEventFanout::from_host(
                            recipients,
                            ContentDigest::from_bytes(&[0xe4; 32])
                                .expect("static resolved target scope"),
                        )
                    })
                }
                _ => None,
            };
            validation.accept_delivery_current(
                &self.stream,
                &self.events,
                &self.signals,
                &self.authorization_memo,
                &self.document_scope,
                resolved,
            );
        } else if request.binding().is_some() {
            validation.accept_scope_current(
                &self.stream,
                &self.events,
                &self.signals,
                &self.authorization_memo,
                &self.document_scope,
            );
        } else {
            validation.accept_current(&self.stream, &self.events, &self.signals);
        }
    }

    fn validate_replay_current(
        &self,
        _request: suprnova_live::async_updates::AsyncReplayMembershipRequest<'_>,
        _validation: &mut suprnova_live::async_updates::AsyncReplayMembershipValidation<'_>,
    ) {
    }
}

impl AsyncTransportAuthorityPort for EngineMembershipRegistry {
    fn now(&self) -> UnixMillis {
        ENGINE_NOW
    }

    fn validate_current<'a>(
        &'a self,
        request: AsyncTransportAuthorityRequest<'a>,
        validation: &'a mut AsyncTransportAuthorityValidation,
    ) -> AsyncTransportFuture<'a, ()> {
        Box::pin(async move {
            let allowed = match request.operation() {
                TransportMembershipOperation::Subscribe => self.contains(request.subscription()),
                TransportMembershipOperation::Unsubscribe => self.contains(request.subscription()),
            };
            if allowed {
                validation.accept_current(
                    &self.document_scope,
                    &self.authorization_memo,
                    &self.stream,
                    &self.topics,
                    &self.events,
                    &self.modes,
                );
            }
        })
    }
}

pub(super) struct EngineAsyncFixture {
    authorized: AuthorizedSubscription,
    registry: Arc<EngineMembershipRegistry>,
    source: Arc<EngineSource>,
    document_scope: DocumentAuthorizationScope,
    descriptor: String,
    descriptor_binding: String,
    credential: String,
    expires_at: UnixMillis,
    fresh_render: Mutex<Arc<ReferenceFreshRender>>,
}

impl EngineAsyncFixture {
    pub(super) async fn new() -> Result<Self, String> {
        let metadata = engine_metadata().clone();
        let ports = Arc::new(EngineSubscriptionPorts {
            component: metadata.clone(),
            parameters: TrustedMountParameters::new(Vec::new())
                .map_err(|_| "engine mount parameters")?,
        });
        let (context, scope) = engine_context(metadata, ports.clone())?;
        let policy =
            ContentDigest::from_bytes(&[0xd4; 32]).map_err(|_| "engine transport policy")?;
        let document_scope = DocumentAuthorizationScope::derive(&scope, &policy)
            .map_err(|_| "engine document scope")?;
        let service = SubscriptionService::new(reference_key_ring());
        let issued = service
            .issue(
                &context,
                SubscriptionIssueRequest::new(
                    StreamName::parse("orders").map_err(|_| "engine stream")?,
                    CapabilityVersion::new(1).map_err(|_| "engine capability")?,
                    UnixMillis::new(60_000),
                    PollFallbackPolicy::new(
                        30_000,
                        0,
                        PollInitialBehavior::AfterInterval,
                        PollVisibilityPolicy::PauseWhenHidden,
                    )
                    .map_err(|_| "engine fallback")?,
                ),
                ENGINE_NOW,
            )
            .await
            .map_err(|_| "engine subscription issue")?;
        let descriptor = issued.descriptor().as_str().to_owned();
        let credential = String::from_utf8(
            issued
                .transport_credential()
                .expose_authorization_bearer()
                .to_vec(),
        )
        .map_err(|_| "engine credential encoding")?;
        let expires_at = issued.expires_at();
        let authorized = service
            .connect(
                &context,
                issued.descriptor(),
                issued.transport_credential(),
                ENGINE_NOW,
            )
            .await
            .map_err(|_| "engine subscription connect")?;
        let fresh_render = Arc::new(ReferenceFreshRender::new(ports, false).await?);
        let claims = authorized.verified().claims();
        let registry = Arc::new(EngineMembershipRegistry {
            subscriptions: Mutex::new(Vec::new()),
            stream: claims.stream().clone(),
            topics: claims.topics().clone(),
            events: claims.events().clone(),
            signals: BoundedPresentationSignalContracts::new(Vec::new())
                .map_err(|_| "engine signals")?,
            modes: engine_modes(),
            authorization_memo: claims.authorization_memo().clone(),
            document_scope: document_scope.clone(),
            resolved_event_fanout: AtomicU64::new(1),
        });
        let source = Arc::new(EngineSource::default());
        Ok(Self {
            descriptor,
            descriptor_binding: authorized.binding().to_base64url(),
            credential,
            expires_at,
            authorized,
            registry,
            source,
            document_scope,
            fresh_render: Mutex::new(fresh_render),
        })
    }

    pub(super) fn descriptor(&self) -> &str {
        &self.descriptor
    }

    pub(super) fn descriptor_binding(&self) -> &str {
        &self.descriptor_binding
    }

    pub(super) fn credential(&self) -> &str {
        &self.credential
    }

    pub(super) const fn expires_at(&self) -> UnixMillis {
        self.expires_at
    }

    pub(super) fn authorization(
        &self,
        subscription: &str,
        origin: &str,
    ) -> Result<AuthorizedTransportSubscription, &'static str> {
        let subscription =
            SubscriptionId::parse(subscription).map_err(|_| "engine subscription identity")?;
        self.registry.activate(subscription.clone());
        AuthorizedTransportSubscription::new(
            &self.authorized,
            subscription,
            self.registry.as_ref(),
            VerifiedOrigin::parse(origin).map_err(|_| "engine origin")?,
            self.document_scope.clone(),
            engine_modes(),
            self.registry.clone(),
            ENGINE_NOW,
        )
        .map_err(|_| "engine membership authorization")
    }

    pub(super) fn document(
        &self,
        origin: &str,
        kind: DocumentTransportKind,
        marker: u8,
    ) -> Result<DocumentTransportSession, &'static str> {
        Ok(DocumentTransportSession::new(
            VerifiedOrigin::parse(origin).map_err(|_| "engine origin")?,
            kind,
            DocumentTransportHandle::from_bytes(&[marker; 16])
                .map_err(|_| "engine document handle")?,
            DocumentTransportLimits::new(8).map_err(|_| "engine document limits")?,
            self.document_scope.clone(),
        ))
    }

    pub(super) fn source(&self) -> &EngineSource {
        self.source.as_ref()
    }

    pub(super) fn queue(&self, envelope: AsyncEnvelope) -> Result<(), &'static str> {
        self.source.push(envelope)
    }

    pub(super) fn remove(&self, authorization: &AuthorizedTransportSubscription) {
        self.registry.remove(authorization.subscription());
    }

    pub(super) fn envelope(
        &self,
        authorization: &AuthorizedTransportSubscription,
        sequence: u64,
    ) -> Result<AsyncEnvelope, &'static str> {
        AsyncEnvelope::new(
            authorization.context(),
            StreamPosition::new(StreamEpoch::new(1), StreamSequence::new(sequence)),
            suprnova_live::async_updates::AsyncPayload::Heartbeat(Heartbeat),
        )
        .map_err(|_| "engine envelope")
    }

    pub(super) fn authorization_lost_envelope(
        &self,
        authorization: &AuthorizedTransportSubscription,
        sequence: u64,
    ) -> Result<AsyncEnvelope, &'static str> {
        AsyncEnvelope::new(
            authorization.context(),
            StreamPosition::new(StreamEpoch::new(1), StreamSequence::new(sequence)),
            suprnova_live::async_updates::AsyncPayload::Error(
                suprnova_live::async_updates::StreamErrorCode::AuthorizationLost,
            ),
        )
        .map_err(|_| "engine envelope")
    }

    pub(super) fn reauthorize(&self, authorization: &AuthorizedTransportSubscription) {
        self.registry.activate(authorization.subscription().clone());
    }

    pub(super) fn browser_event_envelope(
        &self,
        authorization: &AuthorizedTransportSubscription,
        sequence: u64,
    ) -> Result<AsyncEnvelope, &'static str> {
        let event = RegisteredBrowserEvent::new(
            authorization.context(),
            BrowserOperationName::parse("orders.updated").map_err(|_| "engine event")?,
            1,
            EventTarget::Document,
            CanonicalValue::Null,
        )
        .map_err(|_| "engine event")?;
        AsyncEnvelope::new(
            authorization.context(),
            StreamPosition::new(StreamEpoch::new(1), StreamSequence::new(sequence)),
            suprnova_live::async_updates::AsyncPayload::BrowserEvent(event),
        )
        .map_err(|_| "engine envelope")
    }

    pub(super) fn revoke(&self, authorization: &AuthorizedTransportSubscription) {
        self.registry.remove(authorization.subscription());
    }

    pub(super) fn set_resolved_event_fanout(&self, fanout: u16) {
        self.registry
            .resolved_event_fanout
            .store(u64::from(fanout), Ordering::Release);
    }

    pub(super) fn registry(&self) -> &dyn AsyncMembershipRegistryPort {
        self.registry.as_ref()
    }

    pub(super) fn fresh_render_endpoint(&self) -> Arc<ReferenceFreshRender> {
        Arc::clone(&self.fresh_render.lock().expect("fresh render fixture lock"))
    }

    pub(super) async fn reset_fresh_render(
        &self,
        replace_upload_on_successor: bool,
    ) -> Result<(), String> {
        let ports = Arc::clone(&self.fresh_render_endpoint().ports);
        let replacement =
            Arc::new(ReferenceFreshRender::new(ports, replace_upload_on_successor).await?);
        *self.fresh_render.lock().expect("fresh render fixture lock") = replacement;
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct EngineSource {
    state: Arc<Mutex<BTreeMap<String, EngineSourceQueue>>>,
}

#[derive(Default)]
struct EngineSourceQueue {
    envelopes: VecDeque<AsyncEnvelope>,
    waker: Option<Waker>,
}

impl EngineSource {
    pub(super) fn push(&self, envelope: AsyncEnvelope) -> Result<(), &'static str> {
        let mut state = self.state.lock().expect("engine source lock");
        let queue = state
            .entry(envelope.subscription().to_base64url())
            .or_default();
        if queue.envelopes.len() >= MAX_ASYNC_BUFFER_EVENTS {
            return Err("engine_source_exhausted");
        }
        queue.envelopes.push_back(envelope);
        if let Some(waker) = queue.waker.take() {
            waker.wake();
        }
        Ok(())
    }
}

impl AsyncEventSource for EngineSource {
    fn subscribe<'a>(
        &'a self,
        request: &'a AuthorizedTransportSubscription,
    ) -> AsyncTransportFuture<'a, Result<Pin<Box<dyn AsyncEventSession>>, AsyncTransportError>>
    {
        Box::pin(async move {
            Ok(Box::pin(EngineSession {
                baseline: request.baseline(),
                closed: false,
                source: Arc::clone(&self.state),
                subscription: request.subscription().to_base64url(),
            }) as Pin<Box<dyn AsyncEventSession>>)
        })
    }
}

struct EngineSession {
    baseline: StreamPosition,
    closed: bool,
    source: Arc<Mutex<BTreeMap<String, EngineSourceQueue>>>,
    subscription: String,
}

impl AsyncEventSession for EngineSession {
    fn baseline(&self) -> StreamPosition {
        self.baseline
    }

    fn poll_next(
        self: Pin<&mut Self>,
        task: &mut Context<'_>,
    ) -> Poll<Result<Option<AsyncEnvelope>, AsyncTransportError>> {
        let this = self.get_mut();
        if this.closed {
            return Poll::Ready(Ok(None));
        }
        let mut source = this.source.lock().expect("engine source lock");
        let queue = source.entry(this.subscription.clone()).or_default();
        if let Some(envelope) = queue.envelopes.pop_front() {
            Poll::Ready(Ok(Some(envelope)))
        } else {
            queue.waker = Some(task.waker().clone());
            Poll::Pending
        }
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        _task: &mut Context<'_>,
    ) -> Poll<Result<CloseDisposition, AsyncTransportError>> {
        if self.closed {
            Poll::Ready(Ok(CloseDisposition::AlreadyClosed))
        } else {
            self.closed = true;
            self.source
                .lock()
                .expect("engine source lock")
                .remove(&self.subscription);
            Poll::Ready(Ok(CloseDisposition::Closed))
        }
    }
}

fn engine_metadata() -> &'static ComponentMetadata {
    static METADATA: OnceLock<ComponentMetadata> = OnceLock::new();
    METADATA.get_or_init(|| {
        let event = EventMetadata::from_payload_with_contract::<OrdersUpdated>(
            EventSource::Stream,
            BoundedTargets::new(vec![EventTarget::SelfIsland, EventTarget::Document])
                .expect("engine event targets"),
            EventOrder::PerSourceSequence,
            EventCyclePolicy::ForbidRepeatedIsland,
            4,
        )
        .expect("engine event metadata");
        let subscription = SubscriptionMetadata::new(
            StreamName::parse("orders").expect("engine stream"),
            BoundedTopics::new(vec![
                suprnova_live::async_updates::TopicName::parse("orders").expect("engine topic"),
            ])
            .expect("engine topics"),
            BoundedEventNames::new(vec![
                suprnova_live::identity::BrowserOperationName::parse("orders.updated")
                    .expect("engine event name"),
            ])
            .expect("engine event names"),
            engine_modes(),
            ReconnectPolicy::ResumeOrRefresh {
                maximum_attempts: NonZeroU8::new(4).expect("four attempts"),
            },
        );
        ComponentMetadata::new_with_async_contracts(
            ComponentName::parse("reference.uploads").expect("engine component"),
            ViewName::parse("reference/uploads.html").expect("engine view"),
            ContractVersions::new(1, 1, 1, 1, 1).expect("engine versions"),
            vec![FieldMetadata::new(
                ModelField::parse("domain_count").expect("engine domain field"),
                FieldCategory::State,
                StateCodec::Json,
                true,
            )],
            vec![increment_action_metadata()],
            vec![event],
            vec![],
            vec![subscription],
            false,
        )
        .expect("engine component metadata")
    })
}

fn engine_modes() -> SubscriptionModes {
    SubscriptionModes::new(vec![
        SubscriptionMode::ServerSentEvents,
        SubscriptionMode::WebSocket,
    ])
    .expect("engine modes")
}

fn increment_action_metadata() -> ActionMetadata {
    ActionMetadata::new_with_contract(
        ActionName::parse("increment").expect("engine action"),
        1,
        ActionArgumentSchema::empty(),
        AuthorizationRequirement::Current,
        ValidationSelection::ComponentAndArguments,
        TransactionPolicy::None,
    )
    .expect("engine action metadata")
}

fn engine_schemas() -> Result<SnapshotSchemaSet, &'static str> {
    SnapshotSchemaSet::new(
        StateSchema::new(
            1,
            vec![
                FieldSpec::new("domain_count", StateCodec::Json, FieldCategory::State, true)
                    .map_err(|_| "engine domain field")?,
            ],
        )
        .map_err(|_| "engine state schema")?,
        StateSchema::new(1, vec![]).map_err(|_| "engine memo schema")?,
        StateSchema::new(1, vec![]).map_err(|_| "engine mount schema")?,
    )
    .map_err(|_| "engine schema set")
}

fn reference_snapshot_limits() -> Result<SnapshotLimits, &'static str> {
    SnapshotLimits::new(InputLimits::default(), 500, 60_000, 60_000, 8, 8)
        .map_err(|_| "fresh render snapshot limits")
}

fn reference_protocol_limits() -> ProtocolLimits {
    ProtocolLimits::new(ProtocolLimitConfig {
        input: InputLimits::new(64 * 1024, 12, 512, 40 * 1024)
            .expect("reference protocol input limits"),
        max_snapshot_bytes: 32 * 1024,
        max_html_bytes: 32 * 1024,
        max_model_proposals: 8,
        max_operations: 8,
        max_arguments: 16,
        max_validation_entries: 16,
        max_events: 8,
        max_effects: 8,
        max_extensions: 8,
    })
    .expect("reference protocol limits")
}

fn reference_key_ring() -> SnapshotKeyRing {
    let active = KeyRecord::new(
        KeyId::parse("task9-async-key").expect("engine key id"),
        RootKey::new(vec![0x91; 32]).expect("engine root key"),
        UnixMillis::new(0),
        UnixMillis::new(120_000),
        UnixMillis::new(240_000),
    )
    .expect("engine key window");
    SnapshotKeyRing::new(active, Vec::new()).expect("engine key ring")
}

fn engine_context(
    component: ComponentMetadata,
    ports: Arc<EngineSubscriptionPorts>,
) -> Result<(TrustedLiveRequestContext, HostScopeFacts), String> {
    let descriptor = ComponentDescriptor::new(component.clone());
    let contract = ComponentContract::new(
        component.identity().clone(),
        descriptor.contract_digest().clone(),
        1,
        1,
        1,
    )
    .map_err(|_| "engine component contract")?;
    let schemas = engine_schemas().map_err(str::to_owned)?;
    let route = RouteIdentity::from_bytes(&[0x74; 32]).map_err(|_| "engine route")?;
    let slot = IslandSlot::parse("reference-uploads").map_err(|_| "engine slot")?;
    let registry = ComponentRegistryBuilder::new()
        .register(descriptor)
        .map_err(|_| "engine registry")?
        .build();
    let catalog = MountCatalogBuilder::new()
        .register(
            &registry,
            MountCatalogEntry::new(
                ExpectedSeedV1::new(
                    contract,
                    BuildId::parse("build-reference-host").map_err(|_| "engine build")?,
                    route.clone(),
                    slot.clone(),
                    schemas,
                ),
                MountScopeRequirements::new(
                    ScopeRequirement::Required,
                    ScopeRequirement::Required,
                    ScopeRequirement::Required,
                ),
            ),
        )
        .map_err(|_| "engine mount catalog")?
        .build();
    let scope = HostScopeFacts::new(
        ScopeFingerprint::from_bytes(&[0x31; 32]).map_err(|_| "engine scope")?,
        Some(SessionFingerprint::from_bytes(&[0x32; 32]).map_err(|_| "engine session")?),
        Some(PrincipalFingerprint::from_bytes(&[0x33; 32]).map_err(|_| "engine principal")?),
        Some(TenantFingerprint::from_bytes(&[0x34; 32]).map_err(|_| "engine tenant")?),
    );
    let capabilities = HostCapabilities::bound_to(scope.clone())
        .with_action_authorization(Arc::new(EngineActionAuthorization))
        .with_subscription_registry(ports.clone())
        .with_subscription_continuity(ports.clone())
        .with_subscription_authorization(ports.clone())
        .with_subscription_credentials(ports);
    let expires_at = UnixMillis::new(60_000);
    let mut checks = HostCheckFacts::new();
    for kind in CheckKind::ALL {
        checks
            .record(kind, CheckFact::new(CheckDisposition::Passed, expires_at))
            .map_err(|_| "engine checks")?;
    }
    let selection = MountSelection::new(
        route.clone(),
        slot.clone(),
        component.identity().clone(),
        component.contract_digest().clone(),
        2,
    );
    let context = LiveRequestContextValidator::new(300_000)
        .map_err(|_| "engine context validator")?
        .validate(
            &catalog,
            LiveRequestContextCandidate::new(
                route,
                slot,
                selection,
                scope.clone(),
                checks,
                capabilities,
                expires_at,
            ),
            ENGINE_NOW,
        )
        .map_err(|error| format!("engine trusted context: {error:?}"))?;
    Ok((context, scope))
}
