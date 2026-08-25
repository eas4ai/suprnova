//! Deterministic authorization and logical-source fixtures for async transports.

use std::collections::VecDeque;
use std::future::Future;
use std::num::NonZeroU8;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Waker;

use suprnova_live::async_updates::{
    AsyncEnvelope, AsyncEventSession, AsyncEventSource, AsyncMembershipRegistryPort,
    AsyncMembershipRequest, AsyncMembershipValidation, AsyncPayload, AsyncTransportAuthorityPort,
    AsyncTransportAuthorityRequest, AsyncTransportAuthorityValidation, AsyncTransportError,
    AsyncTransportErrorKind, AsyncTransportFuture, AuthoritativeStreamPosition,
    AuthorizedSubscription, AuthorizedTransportSubscription, BoundedEventContracts,
    BoundedEventNames, BoundedPresentationSignalContracts, BoundedTargets, BoundedTopics,
    BrowserPayloadSchema, CapabilityVersion, CloseDisposition, CurrentSubscriptionRegistration,
    EventCyclePolicy, EventOrder, EventSource, EventTarget, PollFallbackPolicy,
    PollInitialBehavior, PollVisibilityPolicy, ReconnectPolicy, StreamEpoch, StreamName,
    StreamPosition, StreamSequence, SubscriptionAuthorizationDecision,
    SubscriptionAuthorizationPort, SubscriptionAuthorizationRequest, SubscriptionBaselineRequest,
    SubscriptionContinuityPort, SubscriptionCredentialPort, SubscriptionCredentialRequest,
    SubscriptionCredentialRotationOutcome, SubscriptionCredentialRotationRequest,
    SubscriptionError, SubscriptionId, SubscriptionIssueRequest, SubscriptionMetadata,
    SubscriptionMode, SubscriptionModes, SubscriptionRegistryPort, SubscriptionRegistryRequest,
    SubscriptionService, TopicName, TransportCredential, TransportMembershipOperation,
    TrustedMountParameters, VerifiedOrigin,
};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::host::{
    CheckDisposition, CheckFact, CheckKind, HostCapabilities, HostCheckFacts, HostScopeFacts,
    LiveRequestContextCandidate, LiveRequestContextValidator, MountCatalogBuilder,
    MountCatalogEntry, MountScopeRequirements, MountSelection, PrincipalFingerprint,
    ScopeRequirement, SessionFingerprint, TenantFingerprint, TrustedLiveRequestContext,
};
use suprnova_live::identity::{
    BrowserOperationName, BuildId, IslandSlot, KeyId, ScopeFingerprint, UnixMillis,
};
use suprnova_live::metadata::{ComponentMetadata, EventMetadata, EventPayloadMetadata};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
use suprnova_live::snapshot::{ComponentContract, ExpectedSeedV1};

#[path = "../component_support.rs"]
mod component_support;

/// Controlled time used by every transport fixture.
pub const NOW: UnixMillis = UnixMillis::new(1_100);

struct OrdersUpdated;

impl EventPayloadMetadata for OrdersUpdated {
    const NAME: &'static str = "orders.updated";
    const VERSION: u16 = 1;
    const PAYLOAD_CONTRACT: &'static str = "orders.updated.payload";
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
}

type TestFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

struct SubscriptionFixturePorts {
    component: ComponentMetadata,
    parameters: TrustedMountParameters,
    baseline: StreamPosition,
}

impl SubscriptionRegistryPort for SubscriptionFixturePorts {
    fn resolve<'a>(
        &'a self,
        request: SubscriptionRegistryRequest<'a>,
    ) -> TestFuture<'a, Result<CurrentSubscriptionRegistration, SubscriptionError>> {
        Box::pin(async move {
            CurrentSubscriptionRegistration::from_registered(
                &self.component,
                request.stream(),
                &self.parameters,
            )
        })
    }
}

impl SubscriptionContinuityPort for SubscriptionFixturePorts {
    fn authoritative_baseline<'a>(
        &'a self,
        _request: SubscriptionBaselineRequest<'a>,
    ) -> TestFuture<'a, Result<AuthoritativeStreamPosition, SubscriptionError>> {
        Box::pin(async move {
            Ok(AuthoritativeStreamPosition::from_host_continuity(
                self.baseline,
            ))
        })
    }
}

impl SubscriptionAuthorizationPort for SubscriptionFixturePorts {
    fn authorize<'a>(
        &'a self,
        _request: SubscriptionAuthorizationRequest<'a>,
    ) -> TestFuture<'a, Result<SubscriptionAuthorizationDecision, SubscriptionError>> {
        Box::pin(async { Ok(SubscriptionAuthorizationDecision::Allow) })
    }
}

impl SubscriptionCredentialPort for SubscriptionFixturePorts {
    fn issue<'a>(
        &'a self,
        _request: SubscriptionCredentialRequest<'a>,
    ) -> TestFuture<'a, Result<TransportCredential, SubscriptionError>> {
        Box::pin(async { TransportCredential::from_host_authority_bearer(vec![0x71; 32]) })
    }

    fn consume_and_rotate<'a>(
        &'a self,
        _request: SubscriptionCredentialRotationRequest<'a>,
    ) -> TestFuture<'a, SubscriptionCredentialRotationOutcome> {
        Box::pin(async {
            match TransportCredential::from_host_authority_bearer(vec![0x72; 32]) {
                Ok(credential) => SubscriptionCredentialRotationOutcome::Rotated(credential),
                Err(_) => SubscriptionCredentialRotationOutcome::Failed,
            }
        })
    }
}

/// Current host membership registry used by transport admission and sequence tests.
pub struct MembershipRegistry {
    active: AtomicBool,
    now: AtomicU64,
    allow_subscribe: AtomicBool,
    allow_unsubscribe: AtomicBool,
    accept_twice: AtomicBool,
    subscriptions: Mutex<Vec<SubscriptionId>>,
    stream: StreamName,
    topics: BoundedTopics,
    events: BoundedEventContracts,
    signals: BoundedPresentationSignalContracts,
    modes: Mutex<SubscriptionModes>,
    authorization_memo: Mutex<suprnova_live::async_updates::AuthorizationMemo>,
}

impl MembershipRegistry {
    /// Activates one bounded logical membership for a test document.
    pub fn activate(&self, subscription: SubscriptionId) {
        self.subscriptions
            .lock()
            .expect("membership registry lock")
            .push(subscription);
    }

    /// Revokes every logical membership without changing descriptor bytes.
    pub fn revoke(&self) {
        self.active.store(false, Ordering::Release);
    }

    /// Advances deterministic transport time without sleeping.
    pub fn set_now(&self, now: UnixMillis) {
        self.now.store(now.get(), Ordering::Release);
    }

    /// Replaces the same-name current registration's canonical transport modes.
    pub fn set_modes(&self, modes: Vec<SubscriptionMode>) {
        *self.modes.lock().expect("membership mode lock") =
            SubscriptionModes::new(modes).expect("current modes");
    }

    /// Revokes browser-initiated removal while preserving internal retirement.
    pub fn deny_unsubscribe(&self) {
        self.allow_unsubscribe.store(false, Ordering::Release);
    }

    /// Makes the trusted-port fixture attempt two current-snapshot acceptances.
    pub fn accept_twice(&self) {
        self.accept_twice.store(true, Ordering::Release);
    }

    /// Simulates a principal, session, tenant, or component-contract scope change.
    pub fn change_authorization_scope(&self) {
        *self
            .authorization_memo
            .lock()
            .expect("authorization memo lock") =
            suprnova_live::async_updates::AuthorizationMemo::parse("current-scope-revision")
                .expect("authorization memo");
    }

    fn modes(&self) -> SubscriptionModes {
        self.modes.lock().expect("membership mode lock").clone()
    }
}

impl AsyncMembershipRegistryPort for MembershipRegistry {
    fn validate_current(
        &self,
        request: AsyncMembershipRequest<'_>,
        validation: &mut AsyncMembershipValidation<'_>,
    ) {
        let active = self.active.load(Ordering::Acquire)
            && self
                .subscriptions
                .lock()
                .expect("membership registry lock")
                .iter()
                .any(|subscription| subscription == request.subscription());
        if active {
            validation.accept_current(&self.stream, &self.events, &self.signals);
        }
    }
}

impl AsyncTransportAuthorityPort for MembershipRegistry {
    fn now(&self) -> UnixMillis {
        UnixMillis::new(self.now.load(Ordering::Acquire))
    }

    fn validate_current<'a>(
        &'a self,
        request: AsyncTransportAuthorityRequest<'a>,
        validation: &'a mut AsyncTransportAuthorityValidation,
    ) -> AsyncTransportFuture<'a, ()> {
        Box::pin(async move {
            let operation_allowed = match request.operation() {
                TransportMembershipOperation::Subscribe => {
                    self.allow_subscribe.load(Ordering::Acquire)
                }
                TransportMembershipOperation::Unsubscribe => {
                    self.allow_unsubscribe.load(Ordering::Acquire)
                }
            };
            let active = operation_allowed
                && self.active.load(Ordering::Acquire)
                && self
                    .subscriptions
                    .lock()
                    .expect("membership registry lock")
                    .iter()
                    .any(|subscription| subscription == request.subscription());
            if active {
                validation.accept_current(
                    &self
                        .authorization_memo
                        .lock()
                        .expect("authorization memo lock"),
                    &self.stream,
                    &self.topics,
                    &self.events,
                    &self.modes(),
                );
                if self.accept_twice.load(Ordering::Acquire) {
                    validation.accept_current(
                        &self
                            .authorization_memo
                            .lock()
                            .expect("authorization memo lock"),
                        &self.stream,
                        &self.topics,
                        &self.events,
                        &self.modes(),
                    );
                }
            }
        })
    }
}

/// Complete Task 2 authorization fixture used by transport conformance.
pub struct TransportFixture {
    /// Connect-authorized descriptor and renewal credential.
    pub authorized: AuthorizedSubscription,
    /// Current membership and registry authority.
    pub registry: Arc<MembershipRegistry>,
}

impl TransportFixture {
    /// Builds a descriptor whose signed authoritative baseline is controlled.
    pub async fn new(baseline: StreamPosition) -> Self {
        Self::new_with_scope(
            baseline,
            component_support::fixture_host_scope(),
            SubscriptionModes::new(vec![
                SubscriptionMode::ServerSentEvents,
                SubscriptionMode::WebSocket,
            ])
            .expect("modes"),
        )
        .await
    }

    /// Builds a descriptor from a registration with exact current transport modes.
    pub async fn new_with_modes(baseline: StreamPosition, modes: Vec<SubscriptionMode>) -> Self {
        Self::new_with_scope(
            baseline,
            component_support::fixture_host_scope(),
            SubscriptionModes::new(modes).expect("modes"),
        )
        .await
    }

    /// Builds a descriptor under a distinct deterministic authorization scope.
    pub async fn new_in_scope(baseline: StreamPosition, marker: u8) -> Self {
        let fingerprint = [marker; 32];
        let scope = HostScopeFacts::new(
            ScopeFingerprint::from_bytes(&fingerprint).expect("scope"),
            Some(SessionFingerprint::from_bytes(&fingerprint).expect("session")),
            Some(PrincipalFingerprint::from_bytes(&fingerprint).expect("principal")),
            Some(TenantFingerprint::from_bytes(&fingerprint).expect("tenant")),
        );
        Self::new_with_scope(
            baseline,
            scope,
            SubscriptionModes::new(vec![
                SubscriptionMode::ServerSentEvents,
                SubscriptionMode::WebSocket,
            ])
            .expect("modes"),
        )
        .await
    }

    async fn new_with_scope(
        baseline: StreamPosition,
        scope: HostScopeFacts,
        modes: SubscriptionModes,
    ) -> Self {
        let ports = Arc::new(SubscriptionFixturePorts {
            component: subscription_component_metadata(modes.clone()),
            parameters: TrustedMountParameters::new(Vec::new()).expect("mount parameters"),
            baseline,
        });
        let context = trusted_context(ports.clone(), scope);
        let service = SubscriptionService::new(subscription_key_ring());
        let issued = service
            .issue(
                &context,
                SubscriptionIssueRequest::new(
                    stream(),
                    CapabilityVersion::new(1).expect("capability"),
                    UnixMillis::new(5_000),
                    PollFallbackPolicy::new(
                        10_000,
                        0,
                        PollInitialBehavior::AfterInterval,
                        PollVisibilityPolicy::PauseWhenHidden,
                    )
                    .expect("poll fallback"),
                ),
                UnixMillis::new(1_000),
            )
            .await
            .expect("issued subscription");
        let authorized = service
            .connect(
                &context,
                issued.descriptor(),
                issued.transport_credential(),
                NOW,
            )
            .await
            .expect("authorized subscription");
        let registry = Arc::new(MembershipRegistry {
            active: AtomicBool::new(true),
            now: AtomicU64::new(NOW.get()),
            allow_subscribe: AtomicBool::new(true),
            allow_unsubscribe: AtomicBool::new(true),
            accept_twice: AtomicBool::new(false),
            subscriptions: Mutex::new(Vec::new()),
            stream: stream(),
            topics: authorized.verified().claims().topics().clone(),
            events: authorized.verified().claims().events().clone(),
            signals: BoundedPresentationSignalContracts::new(Vec::new()).expect("empty signals"),
            modes: Mutex::new(modes),
            authorization_memo: Mutex::new(
                authorized.verified().claims().authorization_memo().clone(),
            ),
        });
        Self {
            authorized,
            registry,
        }
    }

    /// Creates one current descriptor-bound transport membership request.
    pub fn request(
        &self,
        subscription: SubscriptionId,
        origin: VerifiedOrigin,
    ) -> AuthorizedTransportSubscription {
        self.request_at(subscription, origin, NOW)
            .expect("authorized transport subscription")
    }

    /// Attempts to bind a membership at controlled current time.
    pub fn request_at(
        &self,
        subscription: SubscriptionId,
        origin: VerifiedOrigin,
        now: UnixMillis,
    ) -> Result<AuthorizedTransportSubscription, AsyncTransportError> {
        self.registry.activate(subscription.clone());
        AuthorizedTransportSubscription::new(
            &self.authorized,
            subscription,
            self.registry.as_ref(),
            origin,
            self.registry.modes(),
            self.registry.clone(),
            now,
        )
    }
}

/// One deterministic logical source item.
#[derive(Clone)]
pub enum ScriptItem {
    /// A closed registered envelope at the supplied position.
    Envelope(StreamPosition, AsyncPayload),
    /// A prebuilt envelope used to prove cross-subscription routing rejection.
    RawEnvelope(AsyncEnvelope),
    /// A typed transport failure.
    Error(AsyncTransportErrorKind),
    /// A cancellation-safe pending read used to prove bounded fan-in fairness.
    Pending,
    /// Graceful logical completion without another envelope.
    End,
}

/// Deterministic source producing one script per subscription call.
pub struct ScriptedSource {
    scripts: Mutex<VecDeque<Vec<ScriptItem>>>,
    baseline_override: Option<StreamPosition>,
    pending_first_close: bool,
    close_count: Arc<AtomicUsize>,
}

impl ScriptedSource {
    /// Creates scripts consumed in subscription order.
    pub fn new(scripts: Vec<Vec<ScriptItem>>) -> Self {
        Self {
            scripts: Mutex::new(scripts.into()),
            baseline_override: None,
            pending_first_close: false,
            close_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Deliberately returns a source baseline that differs from the descriptor.
    pub fn with_baseline_override(mut self, baseline: StreamPosition) -> Self {
        self.baseline_override = Some(baseline);
        self
    }

    /// Makes each logical session's first close poll cancellation-safe pending.
    pub fn with_pending_first_close(mut self) -> Self {
        self.pending_first_close = true;
        self
    }

    /// Returns how many logical sessions performed their first close transition.
    pub fn close_count(&self) -> usize {
        self.close_count.load(Ordering::Acquire)
    }
}

impl AsyncEventSource for ScriptedSource {
    fn subscribe<'a>(
        &'a self,
        request: &'a AuthorizedTransportSubscription,
    ) -> AsyncTransportFuture<'a, Result<Box<dyn AsyncEventSession>, AsyncTransportError>> {
        Box::pin(async move {
            let script = self
                .scripts
                .lock()
                .expect("script source lock")
                .pop_front()
                .ok_or_else(|| AsyncTransportError::new(AsyncTransportErrorKind::SourceFailed))?;
            let mut events = VecDeque::new();
            for item in script {
                match item {
                    ScriptItem::Envelope(position, payload) => {
                        events.push_back(SessionStep::Ready(Ok(Some(
                            AsyncEnvelope::new(request.context(), position, payload).map_err(
                                |_| {
                                    AsyncTransportError::new(
                                        AsyncTransportErrorKind::InvalidEnvelope,
                                    )
                                },
                            )?,
                        ))));
                    }
                    ScriptItem::RawEnvelope(envelope) => {
                        events.push_back(SessionStep::Ready(Ok(Some(envelope))))
                    }
                    ScriptItem::Error(kind) => {
                        events.push_back(SessionStep::Ready(Err(AsyncTransportError::new(kind))))
                    }
                    ScriptItem::Pending => events.push_back(SessionStep::Pending),
                    ScriptItem::End => events.push_back(SessionStep::Ready(Ok(None))),
                }
            }
            Ok(Box::new(ScriptedSession {
                baseline: self.baseline_override.unwrap_or_else(|| request.baseline()),
                events,
                closed: false,
                pending_first_close: self.pending_first_close,
                close_was_pending: false,
                close_count: self.close_count.clone(),
            }) as Box<dyn AsyncEventSession>)
        })
    }
}

/// Deterministic source that pauses subscription establishment at an await boundary.
pub struct ControlledSubscribeSource {
    observed: AtomicBool,
    released: AtomicBool,
    waiter: Mutex<Option<Waker>>,
    close_count: Arc<AtomicUsize>,
}

impl ControlledSubscribeSource {
    /// Creates one unreleased subscription barrier.
    pub fn new() -> Self {
        Self {
            observed: AtomicBool::new(false),
            released: AtomicBool::new(false),
            waiter: Mutex::new(None),
            close_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Returns whether source establishment reached the controlled await.
    pub fn observed(&self) -> bool {
        self.observed.load(Ordering::Acquire)
    }

    /// Releases the source establishment future and wakes its exact waiter.
    pub fn release(&self) {
        self.released.store(true, Ordering::Release);
        if let Some(waiter) = self.waiter.lock().expect("subscribe waiter lock").take() {
            waiter.wake();
        }
    }

    /// Returns how many opened sessions completed their close transition.
    pub fn close_count(&self) -> usize {
        self.close_count.load(Ordering::Acquire)
    }
}

impl AsyncEventSource for ControlledSubscribeSource {
    fn subscribe<'a>(
        &'a self,
        request: &'a AuthorizedTransportSubscription,
    ) -> AsyncTransportFuture<'a, Result<Box<dyn AsyncEventSession>, AsyncTransportError>> {
        Box::pin(std::future::poll_fn(move |task| {
            self.observed.store(true, Ordering::Release);
            if !self.released.load(Ordering::Acquire) {
                *self.waiter.lock().expect("subscribe waiter lock") = Some(task.waker().clone());
                return std::task::Poll::Pending;
            }
            std::task::Poll::Ready(Ok(Box::new(ScriptedSession {
                baseline: request.baseline(),
                events: VecDeque::new(),
                closed: false,
                pending_first_close: false,
                close_was_pending: false,
                close_count: self.close_count.clone(),
            }) as Box<dyn AsyncEventSession>))
        }))
    }
}

struct ScriptedSession {
    baseline: StreamPosition,
    events: VecDeque<SessionStep>,
    closed: bool,
    pending_first_close: bool,
    close_was_pending: bool,
    close_count: Arc<AtomicUsize>,
}

enum SessionStep {
    Ready(Result<Option<AsyncEnvelope>, AsyncTransportError>),
    Pending,
}

impl AsyncEventSession for ScriptedSession {
    fn baseline(&self) -> StreamPosition {
        self.baseline
    }

    fn next<'a>(
        &'a mut self,
    ) -> AsyncTransportFuture<'a, Result<Option<AsyncEnvelope>, AsyncTransportError>> {
        Box::pin(std::future::poll_fn(move |_task| {
            if self.closed {
                return std::task::Poll::Ready(Err(AsyncTransportError::new(
                    AsyncTransportErrorKind::Closed,
                )));
            }
            match self.events.front() {
                Some(SessionStep::Pending) => std::task::Poll::Pending,
                Some(SessionStep::Ready(_)) => {
                    let Some(SessionStep::Ready(result)) = self.events.pop_front() else {
                        unreachable!("front variant checked before pop")
                    };
                    std::task::Poll::Ready(result)
                }
                None => std::task::Poll::Ready(Ok(None)),
            }
        }))
    }

    fn close<'a>(
        &'a mut self,
    ) -> AsyncTransportFuture<'a, Result<CloseDisposition, AsyncTransportError>> {
        Box::pin(std::future::poll_fn(move |_task| {
            if self.pending_first_close && !self.close_was_pending {
                self.close_was_pending = true;
                return std::task::Poll::Pending;
            }
            if self.closed {
                return std::task::Poll::Ready(Ok(CloseDisposition::AlreadyClosed));
            }
            self.closed = true;
            self.close_count.fetch_add(1, Ordering::AcqRel);
            std::task::Poll::Ready(Ok(CloseDisposition::Closed))
        }))
    }
}

/// Creates a canonical logical subscription identifier from a deterministic byte.
pub fn subscription(byte: u8) -> SubscriptionId {
    SubscriptionId::from_bytes(&[byte; 16]).expect("subscription id")
}

/// Creates a stream position.
pub const fn position(epoch: u64, sequence: u64) -> StreamPosition {
    StreamPosition::new(StreamEpoch::new(epoch), StreamSequence::new(sequence))
}

/// Returns the registered stream identity used by this fixture.
pub fn stream() -> StreamName {
    StreamName::parse("orders").expect("stream")
}

fn subscription_component_metadata(modes: SubscriptionModes) -> ComponentMetadata {
    let base = component_support::metadata();
    let event = EventMetadata::from_payload_with_contract::<OrdersUpdated>(
        EventSource::Stream,
        BoundedTargets::new(vec![EventTarget::SelfIsland, EventTarget::Document])
            .expect("bounded targets"),
        EventOrder::PerSourceSequence,
        EventCyclePolicy::MaximumHops(NonZeroU8::new(4).expect("nonzero hops")),
        4,
    )
    .expect("registered event metadata");
    let subscription = SubscriptionMetadata::new(
        stream(),
        BoundedTopics::new(vec![TopicName::parse("orders").expect("topic")]).expect("topics"),
        BoundedEventNames::new(vec![
            BrowserOperationName::parse(OrdersUpdated::NAME).expect("event name"),
        ])
        .expect("event names"),
        modes,
        ReconnectPolicy::RefreshOnReconnect,
    );
    ComponentMetadata::new_with_async_contracts(
        base.identity().clone(),
        base.view().clone(),
        base.versions(),
        base.fields().to_vec(),
        base.actions().to_vec(),
        vec![event],
        base.effects().to_vec(),
        vec![subscription],
        base.refresh_on_promote(),
    )
    .expect("subscription component metadata")
}

fn subscription_key_ring() -> SnapshotKeyRing {
    let key = KeyRecord::new(
        KeyId::parse("async-transport-key").expect("key ID"),
        RootKey::new(vec![0x49; 32]).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(20_000),
        UnixMillis::new(40_000),
    )
    .expect("key record");
    SnapshotKeyRing::new(key, Vec::new()).expect("key ring")
}

fn trusted_context(
    ports: Arc<SubscriptionFixturePorts>,
    scope: HostScopeFacts,
) -> TrustedLiveRequestContext {
    let component = &ports.component;
    let descriptor = ComponentDescriptor::new(component.clone());
    let contract = ComponentContract::new(
        component.identity().clone(),
        descriptor.contract_digest().clone(),
        1,
        1,
        1,
    )
    .expect("component contract");
    let registry = ComponentRegistryBuilder::new()
        .register(descriptor)
        .expect("component registration")
        .build();
    let route = component_support::snapshot_support::route(0x74);
    let slot = IslandSlot::parse("async-transport").expect("slot");
    let catalog = MountCatalogBuilder::new()
        .register(
            &registry,
            MountCatalogEntry::new(
                ExpectedSeedV1::new(
                    contract,
                    BuildId::parse("build-async-transport").expect("build"),
                    route.clone(),
                    slot.clone(),
                    component_support::schema_set(),
                ),
                MountScopeRequirements::new(
                    ScopeRequirement::Required,
                    ScopeRequirement::Required,
                    ScopeRequirement::Required,
                ),
            ),
        )
        .expect("mount registration")
        .build();
    let capabilities = HostCapabilities::bound_to(scope.clone())
        .with_subscription_registry(ports.clone())
        .with_subscription_continuity(ports.clone())
        .with_subscription_authorization(ports.clone())
        .with_subscription_credentials(ports.clone());
    let expires_at = UnixMillis::new(10_000);
    let mut checks = HostCheckFacts::new();
    for kind in CheckKind::ALL {
        checks
            .record(kind, CheckFact::new(CheckDisposition::Passed, expires_at))
            .expect("host check");
    }
    let selection = MountSelection::new(
        route.clone(),
        slot.clone(),
        component.identity().clone(),
        component.contract_digest().clone(),
        1,
    );
    LiveRequestContextValidator::new(300_000)
        .expect("validator")
        .validate(
            &catalog,
            LiveRequestContextCandidate::new(
                route,
                slot,
                selection,
                scope,
                checks,
                capabilities,
                expires_at,
            ),
            UnixMillis::new(1_000),
        )
        .expect("trusted context")
}
