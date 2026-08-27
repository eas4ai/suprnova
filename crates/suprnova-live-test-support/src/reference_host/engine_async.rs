//! Production async-transport fixture owned by the deterministic reference host.

use std::future::Future;
use std::num::NonZeroU8;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll};

use axum::body::Bytes;
use suprnova_live::async_updates::{
    AsyncEnvelope, AsyncEventSession, AsyncEventSource, AsyncMembershipRegistryPort,
    AsyncMembershipRequest, AsyncMembershipValidation, AsyncTransportAuthorityPort,
    AsyncTransportAuthorityRequest, AsyncTransportAuthorityValidation, AsyncTransportError,
    AsyncTransportFuture, AuthoritativeStreamPosition, AuthorizedSubscription,
    AuthorizedTransportSubscription, BoundedEventNames, BoundedPresentationSignalContracts,
    BoundedTargets, BoundedTopics, BrowserPayloadSchema, CapabilityVersion, CloseDisposition,
    CurrentSubscriptionRegistration, DocumentAuthorizationScope, DocumentTransportHandle,
    DocumentTransportKind, DocumentTransportLimits, DocumentTransportSession, EventCyclePolicy,
    EventOrder, EventSource, EventTarget, Heartbeat, PollFallbackPolicy, PollInitialBehavior,
    PollVisibilityPolicy, ReconnectPolicy, ResolvedEventFanout, StreamEpoch, StreamName,
    StreamPosition, StreamSequence, SubscriptionAuthorizationDecision,
    SubscriptionAuthorizationPort, SubscriptionAuthorizationRequest, SubscriptionBaselineRequest,
    SubscriptionContinuityPort, SubscriptionCredentialPort, SubscriptionCredentialRequest,
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
use suprnova_live::host::{
    CheckDisposition, CheckFact, CheckKind, HostCapabilities, HostCheckFacts, HostScopeFacts,
    LiveRequestContextCandidate, LiveRequestContextValidator, MountCatalogBuilder,
    MountCatalogEntry, MountScopeRequirements, MountSelection, PrincipalFingerprint,
    ScopeRequirement, SessionFingerprint, TenantFingerprint, TrustedLiveRequestContext,
};
use suprnova_live::identity::{
    BuildId, ComponentName, ContentDigest, IslandSlot, KeyId, RouteIdentity, ScopeFingerprint,
    UnixMillis, ViewName,
};
use suprnova_live::limits::InputLimits;
use suprnova_live::metadata::{
    ComponentMetadata, ContractVersions, EventMetadata, EventPayloadMetadata,
};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
use suprnova_live::snapshot::state::{SnapshotSchemaSet, StateSchema};
use suprnova_live::snapshot::{
    ComponentContract, ExpectedInstanceV1, ExpectedSeedV1, SnapshotLimits,
};
use suprnova_live::view::{AssetSet, IslandRender};

use crate::{ComponentHarness, ComponentHarnessConfig, HarnessServices};

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
    generation: u64,
}

impl ComponentFactory for FreshRenderFactory {
    fn mount<'a>(
        &'a self,
        _context: &'a MountContext<'a>,
    ) -> LiveFuture<'a, Result<Box<dyn ComponentInstance>, ComponentError>> {
        Box::pin(async move {
            Ok(Box::new(FreshRenderComponent {
                generation: self.generation,
            }) as Box<dyn ComponentInstance>)
        })
    }

    fn hydrate<'a>(
        &'a self,
        _context: &'a HydrationContext<'a>,
    ) -> LiveFuture<'a, Result<Box<dyn ComponentInstance>, ComponentError>> {
        Box::pin(async move {
            Ok(Box::new(FreshRenderComponent {
                generation: self.generation,
            }) as Box<dyn ComponentInstance>)
        })
    }
}

struct FreshRenderComponent {
    generation: u64,
}

impl ComponentInstance for FreshRenderComponent {
    fn metadata(&self) -> &'static ComponentMetadata {
        engine_metadata()
    }

    fn render<'a>(
        &'a self,
        _context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<IslandRender, ComponentError>> {
        Box::pin(async move {
            Ok(IslandRender {
                body: Bytes::from(format!(
                    "<section data-live-poll-generation=\"{}\" data-live-render-source=\"component-harness\"></section>",
                    self.generation
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
        Ok(CanonicalValue::Object(Default::default()))
    }

    fn dehydrate_memo(&self) -> Result<CanonicalValue, ComponentError> {
        Ok(CanonicalValue::Object(Default::default()))
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
}

impl EngineMembershipRegistry {
    fn activate(&self, subscription: SubscriptionId) {
        self.subscriptions
            .lock()
            .expect("engine membership lock")
            .push(subscription);
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
            validation.accept_delivery_current(
                &self.stream,
                &self.events,
                &self.signals,
                &self.authorization_memo,
                &self.document_scope,
                None::<ResolvedEventFanout>,
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
    document_scope: DocumentAuthorizationScope,
    descriptor: String,
    descriptor_binding: String,
    credential: String,
    expires_at: UnixMillis,
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
        });
        Ok(Self {
            descriptor,
            descriptor_binding: authorized.binding().to_base64url(),
            credential,
            expires_at,
            authorized,
            registry,
            document_scope,
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

    pub(super) async fn fresh_render(generation: u64) -> Result<(String, Vec<u8>), &'static str> {
        let metadata = engine_metadata().clone();
        let ports = Arc::new(EngineSubscriptionPorts {
            component: metadata.clone(),
            parameters: TrustedMountParameters::new(Vec::new())
                .map_err(|_| "fresh render parameters")?,
        });
        let (context, _) =
            engine_context(metadata.clone(), ports).map_err(|_| "fresh render context")?;
        let descriptor = ComponentDescriptor::with_hooks(
            metadata.clone(),
            ComponentHooks::new(Arc::new(FreshRenderFactory { generation })),
        );
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
        let limits = SnapshotLimits::new(InputLimits::default(), 500, 60_000, 60_000, 8, 8)
            .map_err(|_| "fresh render snapshot limits")?;
        let mut harness = ComponentHarness::new(ComponentHarnessConfig::new(
            descriptor,
            context,
            expected,
            reference_key_ring(),
            limits,
            HarnessServices::new(ENGINE_NOW),
        ))
        .map_err(|_| "fresh render harness")?;
        let mounted = harness
            .mount(CanonicalValue::Object(Default::default()))
            .await
            .map_err(|_| "fresh render mount")?;
        let html =
            String::from_utf8(mounted.body().to_vec()).map_err(|_| "fresh render encoding")?;
        let snapshot = harness
            .current_encoded_snapshot()
            .ok_or("fresh render snapshot")?
            .to_vec();
        Ok((html, snapshot))
    }
}

pub(super) struct EngineSource;

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
            }) as Pin<Box<dyn AsyncEventSession>>)
        })
    }
}

struct EngineSession {
    baseline: StreamPosition,
    closed: bool,
}

impl AsyncEventSession for EngineSession {
    fn baseline(&self) -> StreamPosition {
        self.baseline
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _task: &mut Context<'_>,
    ) -> Poll<Result<Option<AsyncEnvelope>, AsyncTransportError>> {
        Poll::Pending
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        _task: &mut Context<'_>,
    ) -> Poll<Result<CloseDisposition, AsyncTransportError>> {
        if self.closed {
            Poll::Ready(Ok(CloseDisposition::AlreadyClosed))
        } else {
            self.closed = true;
            Poll::Ready(Ok(CloseDisposition::Closed))
        }
    }
}

fn engine_metadata() -> &'static ComponentMetadata {
    static METADATA: OnceLock<ComponentMetadata> = OnceLock::new();
    METADATA.get_or_init(|| {
        let event = EventMetadata::from_payload_with_contract::<OrdersUpdated>(
            EventSource::Stream,
            BoundedTargets::new(vec![EventTarget::SelfIsland]).expect("engine event targets"),
            EventOrder::PerSourceSequence,
            EventCyclePolicy::ForbidRepeatedIsland,
            1,
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
            vec![],
            vec![],
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

fn engine_schemas() -> Result<SnapshotSchemaSet, &'static str> {
    SnapshotSchemaSet::new(
        StateSchema::new(1, vec![]).map_err(|_| "engine state schema")?,
        StateSchema::new(1, vec![]).map_err(|_| "engine memo schema")?,
        StateSchema::new(1, vec![]).map_err(|_| "engine mount schema")?,
    )
    .map_err(|_| "engine schema set")
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
        1,
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
