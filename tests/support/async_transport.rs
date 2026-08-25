//! Deterministic authorization and logical-source fixtures for async transports.

use std::collections::VecDeque;
use std::future::Future;
use std::num::NonZeroU8;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::task::{Context, Poll, Waker};

use suprnova_live::async_updates::{
    AsyncEnvelope, AsyncEventSession, AsyncEventSource, AsyncMembershipRegistryPort,
    AsyncMembershipRequest, AsyncMembershipValidation, AsyncPayload, AsyncTransportAuthorityPort,
    AsyncTransportAuthorityRequest, AsyncTransportAuthorityValidation, AsyncTransportError,
    AsyncTransportErrorKind, AsyncTransportFuture, AuthoritativeStreamPosition, AuthorizationMemo,
    AuthorizedSubscription, AuthorizedTransportSubscription, BoundedEventContracts,
    BoundedEventNames, BoundedPresentationSignalContracts, BoundedTargets, BoundedTopics,
    BrowserPayloadSchema, CapabilityVersion, CloseDisposition, CurrentSubscriptionRegistration,
    DocumentAuthorizationScope, DocumentTransportHandle, DocumentTransportKind,
    DocumentTransportLimits, DocumentTransportSession, EventCyclePolicy, EventOrder, EventSource,
    EventTarget, PollFallbackPolicy, PollInitialBehavior, PollVisibilityPolicy, ReconnectPolicy,
    StreamEpoch, StreamName, StreamPosition, StreamSequence, SubscriptionAuthorizationDecision,
    SubscriptionAuthorizationPort, SubscriptionAuthorizationRequest, SubscriptionBaselineRequest,
    SubscriptionBinding, SubscriptionContinuityPort, SubscriptionCredentialPort,
    SubscriptionCredentialRequest, SubscriptionCredentialRotationOutcome,
    SubscriptionCredentialRotationRequest, SubscriptionError, SubscriptionId,
    SubscriptionIssueRequest, SubscriptionMetadata, SubscriptionMode, SubscriptionModes,
    SubscriptionRegistryPort, SubscriptionRegistryRequest, SubscriptionService, TopicName,
    TransportCredential, TransportMembershipOperation, TrustedMountParameters, VerifiedOrigin,
};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::host::{
    CheckDisposition, CheckFact, CheckKind, HostCapabilities, HostCheckFacts, HostScopeFacts,
    LiveRequestContextCandidate, LiveRequestContextValidator, MountCatalogBuilder,
    MountCatalogEntry, MountScopeRequirements, MountSelection, PrincipalFingerprint,
    ScopeRequirement, SessionFingerprint, TenantFingerprint, TrustedLiveRequestContext,
};
use suprnova_live::identity::{
    BrowserOperationName, BuildId, ComponentName, ContentDigest, IslandSlot, KeyId,
    ScopeFingerprint, UnixMillis,
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

/// Deterministic one-way gate that records and wakes its current waiter.
pub struct WakeGate {
    state: Mutex<WakeGateState>,
    observed: AtomicBool,
}

struct WakeGateState {
    released: bool,
    waiter: Option<Waker>,
    interleaving: Option<Arc<RegistrationInterleaving>>,
}

/// Deterministic coordination for the exact release-versus-registration boundary.
pub struct RegistrationInterleaving {
    observed_unreleased: Barrier,
    release_attempt_started: Barrier,
}

impl RegistrationInterleaving {
    /// Creates one two-party poll/release choreography.
    pub fn new() -> Self {
        Self {
            observed_unreleased: Barrier::new(2),
            release_attempt_started: Barrier::new(2),
        }
    }

    fn before_waiter_registration(&self) {
        self.observed_unreleased.wait();
        self.release_attempt_started.wait();
    }

    /// Waits until poll observes unreleased while holding the state mutex.
    pub fn wait_until_observed_unreleased(&self) {
        self.observed_unreleased.wait();
    }

    /// Announces that release is about to attempt the state mutex.
    pub fn signal_release_attempt_started(&self) {
        self.release_attempt_started.wait();
    }
}

impl WakeGate {
    /// Creates one unreleased gate.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(WakeGateState {
                released: false,
                waiter: None,
                interleaving: None,
            }),
            observed: AtomicBool::new(false),
        }
    }

    /// Installs one deterministic release-versus-registration choreography.
    pub fn with_registration_interleaving(
        self,
        interleaving: Arc<RegistrationInterleaving>,
    ) -> Self {
        self.state
            .lock()
            .expect("wake gate state lock")
            .interleaving = Some(interleaving);
        self
    }

    /// Polls the controlled gate directly for deterministic wake tests.
    pub fn poll(&self, task: &mut Context<'_>) -> Poll<()> {
        self.observed.store(true, Ordering::Release);
        let mut state = self.state.lock().expect("wake gate state lock");
        if state.released {
            return Poll::Ready(());
        }
        let interleaving = state.interleaving.take();
        if let Some(interleaving) = &interleaving {
            interleaving.before_waiter_registration();
        }
        register_current_waker(&mut state.waiter, task.waker());
        Poll::Pending
    }

    /// Returns whether the controlled operation reached this gate.
    pub fn observed(&self) -> bool {
        self.observed.load(Ordering::Acquire)
    }

    /// Returns whether a pending poll registered a waker.
    pub fn waiter_registered(&self) -> bool {
        self.state
            .lock()
            .expect("wake gate state lock")
            .waiter
            .is_some()
    }

    /// Releases the gate and wakes the exact waiter, if present.
    pub fn release(&self) {
        let waiter = {
            let mut state = self.state.lock().expect("wake gate state lock");
            if state.released {
                None
            } else {
                state.released = true;
                state.waiter.take()
            }
        };
        if let Some(waiter) = waiter {
            waiter.wake();
        }
    }
}

impl Default for WakeGate {
    fn default() -> Self {
        Self::new()
    }
}

/// Redaction-safe exact authority request observed by the trusted test port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorityObservation {
    /// Membership operation that crossed the authority boundary.
    pub operation: TransportMembershipOperation,
    /// Exact physical origin.
    pub origin: VerifiedOrigin,
    /// Exact physical transport kind.
    pub kind: DocumentTransportKind,
    /// Correlation-only document handle.
    pub handle: DocumentTransportHandle,
    /// Trusted document sharing scope.
    pub document_scope: DocumentAuthorizationScope,
    /// Exact component-specific authorization memo.
    pub component_memo: AuthorizationMemo,
    /// Binding of the exact signed descriptor wire.
    pub binding: SubscriptionBinding,
    /// Exact logical subscription routing identity.
    pub subscription: SubscriptionId,
}

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
    document_scope: Mutex<DocumentAuthorizationScope>,
    authority_calls: AtomicUsize,
    authority_pause_call: AtomicUsize,
    authority_gate: Mutex<Option<Arc<WakeGate>>>,
    authority_observations: Mutex<Vec<AuthorityObservation>>,
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

    /// Pauses one exact future transport-authority call at a controlled boundary.
    pub fn pause_authority_on_call(&self, call: usize) -> Arc<WakeGate> {
        assert!(call > 0, "authority call is one-based");
        let gate = Arc::new(WakeGate::new());
        self.authority_pause_call.store(call, Ordering::Release);
        *self.authority_gate.lock().expect("authority gate lock") = Some(gate.clone());
        gate
    }

    /// Returns how many fresh authority checks were entered.
    pub fn authority_call_count(&self) -> usize {
        self.authority_calls.load(Ordering::Acquire)
    }

    /// Returns the exact redaction-safe authority requests observed so far.
    pub fn authority_observations(&self) -> Vec<AuthorityObservation> {
        self.authority_observations
            .lock()
            .expect("authority observations lock")
            .clone()
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

    /// Simulates connection-level identity or aggregate transport-policy drift.
    pub fn change_document_scope(&self) {
        let fingerprint = [0xe1; 32];
        let facts = HostScopeFacts::new(
            ScopeFingerprint::from_bytes(&fingerprint).expect("scope"),
            Some(SessionFingerprint::from_bytes(&[0xe2; 32]).expect("session")),
            Some(PrincipalFingerprint::from_bytes(&[0xe3; 32]).expect("principal")),
            Some(TenantFingerprint::from_bytes(&[0xe4; 32]).expect("tenant")),
        );
        let policy = ContentDigest::from_bytes(&[0xe5; 32]).expect("transport policy");
        *self
            .document_scope
            .lock()
            .expect("document authorization scope lock") =
            DocumentAuthorizationScope::derive(&facts, &policy).expect("revised document scope");
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
            let call = self.authority_calls.fetch_add(1, Ordering::AcqRel) + 1;
            if self.authority_pause_call.load(Ordering::Acquire) == call {
                let gate = self
                    .authority_gate
                    .lock()
                    .expect("authority gate lock")
                    .clone()
                    .expect("configured authority gate");
                std::future::poll_fn(|task| gate.poll(task)).await;
            }
            self.authority_observations
                .lock()
                .expect("authority observations lock")
                .push(AuthorityObservation {
                    operation: request.operation(),
                    origin: request.document_origin().clone(),
                    kind: request.document_kind(),
                    handle: request.document_handle().clone(),
                    document_scope: request.document_scope().clone(),
                    component_memo: request.descriptor().claims().authorization_memo().clone(),
                    binding: request.binding().clone(),
                    subscription: request.subscription().clone(),
                });
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
                        .document_scope
                        .lock()
                        .expect("document authorization scope lock"),
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
                            .document_scope
                            .lock()
                            .expect("document authorization scope lock"),
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
    /// Connection-level scope shared independently from component contracts.
    pub document_scope: DocumentAuthorizationScope,
    /// Exact registered component identity for heterogeneous sharing assertions.
    pub component_name: ComponentName,
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

    /// Builds an otherwise identical descriptor under another signing key.
    pub async fn new_with_signing_key(
        baseline: StreamPosition,
        key_id: &str,
        key_marker: u8,
    ) -> Self {
        Self::new_with_configuration(
            baseline,
            component_support::fixture_host_scope(),
            SubscriptionModes::new(vec![
                SubscriptionMode::ServerSentEvents,
                SubscriptionMode::WebSocket,
            ])
            .expect("modes"),
            subscription_component_metadata_with_hops(4),
            subscription_key_ring_with(key_id, key_marker),
        )
        .await
    }

    /// Builds the same physical scope with a different component contract.
    pub async fn new_with_contract_revision(baseline: StreamPosition) -> Self {
        Self::new_with_configuration(
            baseline,
            component_support::fixture_host_scope(),
            SubscriptionModes::new(vec![
                SubscriptionMode::ServerSentEvents,
                SubscriptionMode::WebSocket,
            ])
            .expect("modes"),
            subscription_component_metadata_with_hops(3),
            subscription_key_ring(),
        )
        .await
    }

    /// Builds the same async contract under a genuinely distinct component identity.
    pub async fn new_with_component_name(baseline: StreamPosition, name: &str) -> Self {
        Self::new_with_configuration(
            baseline,
            component_support::fixture_host_scope(),
            SubscriptionModes::new(vec![
                SubscriptionMode::ServerSentEvents,
                SubscriptionMode::WebSocket,
            ])
            .expect("modes"),
            subscription_component_metadata_with_name(name),
            subscription_key_ring(),
        )
        .await
    }

    async fn new_with_scope(
        baseline: StreamPosition,
        scope: HostScopeFacts,
        modes: SubscriptionModes,
    ) -> Self {
        Self::new_with_configuration(
            baseline,
            scope,
            modes.clone(),
            subscription_component_metadata(modes),
            subscription_key_ring(),
        )
        .await
    }

    async fn new_with_configuration(
        baseline: StreamPosition,
        scope: HostScopeFacts,
        modes: SubscriptionModes,
        component: ComponentMetadata,
        keys: SnapshotKeyRing,
    ) -> Self {
        let transport_policy =
            ContentDigest::from_bytes(&[0xd4; 32]).expect("transport policy identity");
        let document_scope = DocumentAuthorizationScope::derive(&scope, &transport_policy)
            .expect("document authorization scope");
        let component_name = component.identity().clone();
        let ports = Arc::new(SubscriptionFixturePorts {
            component,
            parameters: TrustedMountParameters::new(Vec::new()).expect("mount parameters"),
            baseline,
        });
        let context = trusted_context(ports.clone(), scope);
        let service = SubscriptionService::new(keys);
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
            document_scope: Mutex::new(document_scope.clone()),
            authority_calls: AtomicUsize::new(0),
            authority_pause_call: AtomicUsize::new(usize::MAX),
            authority_gate: Mutex::new(None),
            authority_observations: Mutex::new(Vec::new()),
        });
        Self {
            authorized,
            registry,
            document_scope,
            component_name,
        }
    }

    /// Creates a physical document transport using this trusted sharing scope.
    pub fn document(
        &self,
        origin: VerifiedOrigin,
        kind: DocumentTransportKind,
        handle_marker: u8,
        max_memberships: usize,
    ) -> DocumentTransportSession {
        DocumentTransportSession::new(
            origin,
            kind,
            DocumentTransportHandle::from_bytes(&[handle_marker; 16]).expect("handle"),
            DocumentTransportLimits::new(max_memberships).expect("limits"),
            self.document_scope.clone(),
        )
    }

    /// Deliberately combines one component descriptor with another component's authority.
    pub fn cross_component_request(
        &self,
        authority: &Self,
        subscription: SubscriptionId,
        origin: VerifiedOrigin,
    ) -> Result<AuthorizedTransportSubscription, AsyncTransportError> {
        authority.registry.activate(subscription.clone());
        AuthorizedTransportSubscription::new(
            &self.authorized,
            subscription,
            authority.registry.as_ref(),
            origin,
            self.document_scope.clone(),
            authority.registry.modes(),
            authority.registry.clone(),
            NOW,
        )
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
            self.document_scope.clone(),
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
    /// A controlled pending read resumed by its registered waker.
    Wait(Arc<WakeGate>),
    /// Graceful logical completion without another envelope.
    End,
}

/// Deterministic source producing one script per subscription call.
pub struct ScriptedSource {
    scripts: Mutex<VecDeque<Vec<ScriptItem>>>,
    baseline_override: Option<StreamPosition>,
    pending_first_close: bool,
    permanently_pending_close: bool,
    close_error_attempts: usize,
    close_error_kind: AsyncTransportErrorKind,
    close_count: Arc<AtomicUsize>,
    close_poll_count: Arc<AtomicUsize>,
    drop_count: Arc<AtomicUsize>,
    close_gate: Option<Arc<WakeGate>>,
}

impl ScriptedSource {
    /// Creates scripts consumed in subscription order.
    pub fn new(scripts: Vec<Vec<ScriptItem>>) -> Self {
        Self {
            scripts: Mutex::new(scripts.into()),
            baseline_override: None,
            pending_first_close: false,
            permanently_pending_close: false,
            close_error_attempts: 0,
            close_error_kind: AsyncTransportErrorKind::SourceFailed,
            close_count: Arc::new(AtomicUsize::new(0)),
            close_poll_count: Arc::new(AtomicUsize::new(0)),
            drop_count: Arc::new(AtomicUsize::new(0)),
            close_gate: None,
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

    /// Makes every logical close remain cancellation-safe pending forever.
    pub fn with_permanently_pending_close(mut self) -> Self {
        self.permanently_pending_close = true;
        self.close_gate = Some(Arc::new(WakeGate::new()));
        self
    }

    /// Holds close at a controlled waker-aware boundary until explicitly released.
    pub fn with_controlled_close(mut self, gate: Arc<WakeGate>) -> Self {
        self.close_gate = Some(gate);
        self
    }

    /// Makes every logical close fail the requested number of polls before succeeding.
    pub fn with_close_error_attempts(
        mut self,
        attempts: usize,
        kind: AsyncTransportErrorKind,
    ) -> Self {
        self.close_error_attempts = attempts;
        self.close_error_kind = kind;
        self
    }

    /// Returns how many logical sessions performed their first close transition.
    pub fn close_count(&self) -> usize {
        self.close_count.load(Ordering::Acquire)
    }

    /// Returns the total number of persistent close polls across logical sessions.
    pub fn close_poll_count(&self) -> usize {
        self.close_poll_count.load(Ordering::Acquire)
    }

    /// Returns how many logical session owners released provider resources by drop.
    pub fn drop_count(&self) -> usize {
        self.drop_count.load(Ordering::Acquire)
    }
}

impl AsyncEventSource for ScriptedSource {
    fn subscribe<'a>(
        &'a self,
        request: &'a AuthorizedTransportSubscription,
    ) -> AsyncTransportFuture<'a, Result<Pin<Box<dyn AsyncEventSession>>, AsyncTransportError>>
    {
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
                    ScriptItem::Wait(gate) => events.push_back(SessionStep::Wait(gate)),
                    ScriptItem::End => events.push_back(SessionStep::Ready(Ok(None))),
                }
            }
            Ok(Box::pin(ScriptedSession {
                baseline: self.baseline_override.unwrap_or_else(|| request.baseline()),
                events,
                closed: false,
                pending_first_close: self.pending_first_close,
                permanently_pending_close: self.permanently_pending_close,
                close_was_pending: false,
                close_error_attempts: self.close_error_attempts,
                close_error_kind: self.close_error_kind,
                close_count: self.close_count.clone(),
                close_poll_count: self.close_poll_count.clone(),
                drop_count: self.drop_count.clone(),
                close_gate: self.close_gate.clone(),
                pending_read_waker: None,
                pending_close_waker: None,
            }) as Pin<Box<dyn AsyncEventSession>>)
        })
    }
}

/// Deterministic source that pauses subscription establishment at an await boundary.
pub struct ControlledSubscribeSource {
    observed: AtomicBool,
    state: Mutex<ControlledSubscribeState>,
    close_count: Arc<AtomicUsize>,
    drop_count: Arc<AtomicUsize>,
}

struct ControlledSubscribeState {
    released: bool,
    waiter: Option<Waker>,
    interleaving: Option<Arc<RegistrationInterleaving>>,
}

impl ControlledSubscribeSource {
    /// Creates one unreleased subscription barrier.
    pub fn new() -> Self {
        Self {
            observed: AtomicBool::new(false),
            state: Mutex::new(ControlledSubscribeState {
                released: false,
                waiter: None,
                interleaving: None,
            }),
            close_count: Arc::new(AtomicUsize::new(0)),
            drop_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Installs one deterministic release-versus-registration choreography.
    pub fn with_registration_interleaving(
        self,
        interleaving: Arc<RegistrationInterleaving>,
    ) -> Self {
        self.state
            .lock()
            .expect("subscribe state lock")
            .interleaving = Some(interleaving);
        self
    }

    /// Returns whether source establishment reached the controlled await.
    pub fn observed(&self) -> bool {
        self.observed.load(Ordering::Acquire)
    }

    /// Releases the source establishment future and wakes its exact waiter.
    pub fn release(&self) {
        let waiter = {
            let mut state = self.state.lock().expect("subscribe state lock");
            if state.released {
                None
            } else {
                state.released = true;
                state.waiter.take()
            }
        };
        if let Some(waiter) = waiter {
            waiter.wake();
        }
    }

    /// Returns how many opened sessions completed their close transition.
    pub fn close_count(&self) -> usize {
        self.close_count.load(Ordering::Acquire)
    }

    /// Returns how many opened sessions released provider resources by drop.
    pub fn drop_count(&self) -> usize {
        self.drop_count.load(Ordering::Acquire)
    }
}

impl AsyncEventSource for ControlledSubscribeSource {
    fn subscribe<'a>(
        &'a self,
        request: &'a AuthorizedTransportSubscription,
    ) -> AsyncTransportFuture<'a, Result<Pin<Box<dyn AsyncEventSession>>, AsyncTransportError>>
    {
        Box::pin(std::future::poll_fn(move |task| {
            self.observed.store(true, Ordering::Release);
            let mut state = self.state.lock().expect("subscribe state lock");
            if state.released {
                return ready_controlled_session(self, request);
            }
            let interleaving = state.interleaving.take();
            if let Some(interleaving) = &interleaving {
                interleaving.before_waiter_registration();
            }
            register_current_waker(&mut state.waiter, task.waker());
            Poll::Pending
        }))
    }
}

fn ready_controlled_session(
    source: &ControlledSubscribeSource,
    request: &AuthorizedTransportSubscription,
) -> Poll<Result<Pin<Box<dyn AsyncEventSession>>, AsyncTransportError>> {
    Poll::Ready(Ok(Box::pin(ScriptedSession {
        baseline: request.baseline(),
        events: VecDeque::new(),
        closed: false,
        pending_first_close: false,
        permanently_pending_close: false,
        close_was_pending: false,
        close_error_attempts: 0,
        close_error_kind: AsyncTransportErrorKind::SourceFailed,
        close_count: source.close_count.clone(),
        close_poll_count: Arc::new(AtomicUsize::new(0)),
        drop_count: source.drop_count.clone(),
        close_gate: None,
        pending_read_waker: None,
        pending_close_waker: None,
    }) as Pin<Box<dyn AsyncEventSession>>))
}

fn register_current_waker(slot: &mut Option<Waker>, candidate: &Waker) {
    if slot
        .as_ref()
        .is_none_or(|current| !current.will_wake(candidate))
    {
        *slot = Some(candidate.clone());
    }
}

struct ScriptedSession {
    baseline: StreamPosition,
    events: VecDeque<SessionStep>,
    closed: bool,
    pending_first_close: bool,
    permanently_pending_close: bool,
    close_was_pending: bool,
    close_error_attempts: usize,
    close_error_kind: AsyncTransportErrorKind,
    close_count: Arc<AtomicUsize>,
    close_poll_count: Arc<AtomicUsize>,
    drop_count: Arc<AtomicUsize>,
    close_gate: Option<Arc<WakeGate>>,
    pending_read_waker: Option<Waker>,
    pending_close_waker: Option<Waker>,
}

enum SessionStep {
    Ready(Result<Option<AsyncEnvelope>, AsyncTransportError>),
    Pending,
    Wait(Arc<WakeGate>),
}

impl AsyncEventSession for ScriptedSession {
    fn baseline(&self) -> StreamPosition {
        self.baseline
    }

    fn poll_next(
        self: Pin<&mut Self>,
        task: &mut Context<'_>,
    ) -> Poll<Result<Option<AsyncEnvelope>, AsyncTransportError>> {
        let this = self.get_mut();
        if this.closed {
            return Poll::Ready(Err(AsyncTransportError::new(
                AsyncTransportErrorKind::Closed,
            )));
        }
        loop {
            match this.events.front() {
                Some(SessionStep::Pending) => {
                    this.pending_read_waker = Some(task.waker().clone());
                    return Poll::Pending;
                }
                Some(SessionStep::Wait(gate)) => match gate.poll(task) {
                    Poll::Ready(()) => {
                        this.events.pop_front();
                    }
                    Poll::Pending => return Poll::Pending,
                },
                Some(SessionStep::Ready(_)) => {
                    let Some(SessionStep::Ready(result)) = this.events.pop_front() else {
                        unreachable!("front variant checked before pop")
                    };
                    return Poll::Ready(result);
                }
                None => return Poll::Ready(Ok(None)),
            }
        }
    }

    fn poll_close(
        self: Pin<&mut Self>,
        task: &mut Context<'_>,
    ) -> Poll<Result<CloseDisposition, AsyncTransportError>> {
        let this = self.get_mut();
        this.close_poll_count.fetch_add(1, Ordering::AcqRel);
        if let Some(gate) = &this.close_gate
            && gate.poll(task).is_pending()
        {
            return Poll::Pending;
        }
        if this.permanently_pending_close {
            return Poll::Pending;
        }
        if this.pending_first_close && !this.close_was_pending {
            this.close_was_pending = true;
            this.pending_close_waker = Some(task.waker().clone());
            return Poll::Pending;
        }
        this.pending_close_waker.take();
        if this.close_error_attempts > 0 {
            this.close_error_attempts -= 1;
            return Poll::Ready(Err(AsyncTransportError::new(this.close_error_kind)));
        }
        if this.closed {
            return Poll::Ready(Ok(CloseDisposition::AlreadyClosed));
        }
        this.closed = true;
        this.close_count.fetch_add(1, Ordering::AcqRel);
        Poll::Ready(Ok(CloseDisposition::Closed))
    }
}

impl Drop for ScriptedSession {
    fn drop(&mut self) {
        self.drop_count.fetch_add(1, Ordering::AcqRel);
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
    subscription_component_metadata_with_modes_and_hops(modes, 4)
}

fn subscription_component_metadata_with_hops(maximum_hops: u8) -> ComponentMetadata {
    subscription_component_metadata_with_modes_and_hops(
        SubscriptionModes::new(vec![
            SubscriptionMode::ServerSentEvents,
            SubscriptionMode::WebSocket,
        ])
        .expect("modes"),
        maximum_hops,
    )
}

fn subscription_component_metadata_with_name(name: &str) -> ComponentMetadata {
    let base = component_support::metadata();
    let metadata = subscription_component_metadata_with_hops(4);
    ComponentMetadata::new_with_async_contracts(
        ComponentName::parse(name).expect("component identity"),
        base.view().clone(),
        metadata.versions(),
        metadata.fields().to_vec(),
        metadata.actions().to_vec(),
        metadata.events().to_vec(),
        metadata.effects().to_vec(),
        metadata.subscriptions().to_vec(),
        metadata.refresh_on_promote(),
    )
    .expect("subscription component metadata")
}

fn subscription_component_metadata_with_modes_and_hops(
    modes: SubscriptionModes,
    maximum_hops: u8,
) -> ComponentMetadata {
    let base = component_support::metadata();
    let event = EventMetadata::from_payload_with_contract::<OrdersUpdated>(
        EventSource::Stream,
        BoundedTargets::new(vec![EventTarget::SelfIsland, EventTarget::Document])
            .expect("bounded targets"),
        EventOrder::PerSourceSequence,
        EventCyclePolicy::MaximumHops(NonZeroU8::new(maximum_hops).expect("nonzero maximum hops")),
        u16::from(maximum_hops),
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
    subscription_key_ring_with("async-transport-key", 0x49)
}

fn subscription_key_ring_with(key_id: &str, key_marker: u8) -> SnapshotKeyRing {
    let key = KeyRecord::new(
        KeyId::parse(key_id).expect("key ID"),
        RootKey::new(vec![key_marker; 32]).expect("root key"),
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
