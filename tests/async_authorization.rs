//! Current authorization and credential-renewal tests for subscriptions.

use std::collections::BTreeMap;
use std::future::Future;
use std::num::NonZeroU8;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use suprnova_live::async_updates::{
    AuthoritativeStreamPosition, BoundedEventContracts, BoundedEventNames, BoundedTargets,
    BoundedTopics, BrowserPayloadSchema, CapabilityVersion, CurrentSubscriptionRegistration,
    EventCyclePolicy, EventOrder, EventSource, EventTarget,
    MAX_CANONICAL_SUBSCRIPTION_CLAIMS_BYTES, PollFallbackPolicy, PollInitialBehavior,
    PollVisibilityPolicy, ReconnectPolicy, StreamEpoch, StreamName, StreamPosition, StreamSequence,
    SubscriptionAuthorizationDecision, SubscriptionAuthorizationOperation,
    SubscriptionAuthorizationPort, SubscriptionAuthorizationRequest, SubscriptionBaselineRequest,
    SubscriptionClaims, SubscriptionContinuityPort, SubscriptionCredentialDecision,
    SubscriptionCredentialPort, SubscriptionCredentialRequest, SubscriptionCredentialScope,
    SubscriptionDescriptorCodec, SubscriptionError, SubscriptionErrorKind,
    SubscriptionEventContract, SubscriptionIssueRequest, SubscriptionMetadata,
    SubscriptionRegistryPort, SubscriptionRegistryRequest, SubscriptionService, TopicName,
    TransportCredential, TrustedMountParameters,
};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::host::{
    CheckDisposition, CheckFact, CheckKind, HostCapabilities, HostCheckFacts, HostScopeFacts,
    LiveRequestContextCandidate, LiveRequestContextValidator, MountCatalogBuilder,
    MountCatalogEntry, MountScopeRequirements, MountSelection, PrincipalFingerprint,
    ScopeRequirement, SessionFingerprint, TenantFingerprint, TrustedLiveRequestContext,
};
use suprnova_live::identity::{
    BrowserOperationName, BuildId, ComponentName, IslandSlot, KeyId, UnixMillis, ViewName,
};
use suprnova_live::metadata::{ComponentMetadata, EventMetadata, EventPayloadMetadata};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
use suprnova_live::snapshot::{ComponentContract, ExpectedSeedV1};

#[path = "component_support.rs"]
mod component_support;

const CREDENTIAL_SENTINEL: &[u8] = b"scoped-async-credential-sentinel";

type TestFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

struct ControlledSubscriptionPorts {
    allowed: AtomicBool,
    operations: Mutex<Vec<SubscriptionAuthorizationOperation>>,
    current_component: Mutex<ComponentMetadata>,
    mount_parameters: Mutex<TrustedMountParameters>,
    credentials: Mutex<BTreeMap<Vec<u8>, CredentialRecord>>,
    next_credential: AtomicU64,
    baseline: Mutex<StreamPosition>,
    fixed_credentials: AtomicBool,
}

#[derive(Clone, Eq, PartialEq)]
struct CredentialRecord {
    binding: String,
    operation: SubscriptionAuthorizationOperation,
    scope: SubscriptionCredentialScope,
    expires_at: UnixMillis,
}

impl ControlledSubscriptionPorts {
    fn allowing() -> Arc<Self> {
        Arc::new(Self {
            allowed: AtomicBool::new(true),
            operations: Mutex::new(Vec::new()),
            current_component: Mutex::new(component_metadata().clone()),
            mount_parameters: Mutex::new(
                TrustedMountParameters::new(vec![("tenant".to_owned(), "7".to_owned())])
                    .expect("trusted mount parameters"),
            ),
            credentials: Mutex::new(BTreeMap::new()),
            next_credential: AtomicU64::new(1),
            baseline: Mutex::new(StreamPosition::new(
                StreamEpoch::new(4),
                StreamSequence::new(19),
            )),
            fixed_credentials: AtomicBool::new(false),
        })
    }

    fn revoke(&self) {
        self.allowed.store(false, Ordering::SeqCst);
    }

    fn revise_component(&self, component: ComponentMetadata) {
        *self.current_component.lock().expect("current component") = component;
    }

    fn set_tenant_parameter(&self, value: &str) {
        *self.mount_parameters.lock().expect("mount parameters") =
            TrustedMountParameters::new(vec![("tenant".to_owned(), value.to_owned())])
                .expect("trusted mount parameters");
    }

    fn expire_credential(&self, credential: &TransportCredential, expires_at: UnixMillis) {
        self.credentials
            .lock()
            .expect("credentials")
            .get_mut(credential.expose_authorization_bearer())
            .expect("issued credential record")
            .expires_at = expires_at;
    }

    fn set_baseline(&self, baseline: StreamPosition) {
        *self.baseline.lock().expect("baseline") = baseline;
    }

    fn use_fixed_credentials(&self) {
        self.fixed_credentials.store(true, Ordering::SeqCst);
    }
}

impl SubscriptionContinuityPort for ControlledSubscriptionPorts {
    fn authoritative_baseline<'a>(
        &'a self,
        request: SubscriptionBaselineRequest<'a>,
    ) -> TestFuture<'a, Result<AuthoritativeStreamPosition, SubscriptionError>> {
        Box::pin(async move {
            assert_eq!(request.component().as_str(), "tests.trace");
            assert_eq!(request.stream().as_str(), "orders.activity");
            Ok(AuthoritativeStreamPosition::from_host_continuity(
                *self.baseline.lock().expect("baseline"),
            ))
        })
    }
}

impl SubscriptionRegistryPort for ControlledSubscriptionPorts {
    fn resolve<'a>(
        &'a self,
        request: SubscriptionRegistryRequest<'a>,
    ) -> TestFuture<'a, Result<CurrentSubscriptionRegistration, SubscriptionError>> {
        Box::pin(async move {
            CurrentSubscriptionRegistration::from_registered(
                &self.current_component.lock().expect("current component"),
                request.stream(),
                &self.mount_parameters.lock().expect("mount parameters"),
            )
        })
    }
}

impl SubscriptionAuthorizationPort for ControlledSubscriptionPorts {
    fn authorize<'a>(
        &'a self,
        request: SubscriptionAuthorizationRequest<'a>,
    ) -> TestFuture<'a, Result<SubscriptionAuthorizationDecision, SubscriptionError>> {
        Box::pin(async move {
            assert_eq!(request.component().as_str(), "tests.trace");
            assert_eq!(request.stream().as_str(), "orders.activity");
            assert_eq!(request.topics().as_slice().len(), 2);
            self.operations
                .lock()
                .expect("operations")
                .push(request.operation());
            Ok(if self.allowed.load(Ordering::SeqCst) {
                SubscriptionAuthorizationDecision::Allow
            } else {
                SubscriptionAuthorizationDecision::Deny
            })
        })
    }
}

impl SubscriptionCredentialPort for ControlledSubscriptionPorts {
    fn issue<'a>(
        &'a self,
        request: SubscriptionCredentialRequest<'a>,
    ) -> TestFuture<'a, Result<TransportCredential, SubscriptionError>> {
        Box::pin(async move {
            assert!(request.presented().is_none());
            let mut token = CREDENTIAL_SENTINEL.to_vec();
            if !self.fixed_credentials.load(Ordering::SeqCst) {
                let counter = self.next_credential.fetch_add(1, Ordering::SeqCst);
                let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&[0x7c; 32])
                    .expect("fixed-length HMAC test key");
                mac.update(b"suprnova-live/test-subscription-credential/v1\0");
                mac.update(&counter.to_be_bytes());
                mac.update(request.binding().to_base64url().as_bytes());
                mac.update(&[match request.operation() {
                    SubscriptionAuthorizationOperation::Issue => 0,
                    SubscriptionAuthorizationOperation::Connect => 1,
                    SubscriptionAuthorizationOperation::Renew => 2,
                }]);
                mac.update(&request.expires_at().get().to_be_bytes());
                mac.update(request.scope().component().as_str().as_bytes());
                mac.update(request.scope().component_contract().as_bytes());
                mac.update(request.scope().stream().as_str().as_bytes());
                token.extend_from_slice(&mac.finalize().into_bytes());
            }
            self.credentials.lock().expect("credentials").insert(
                token.clone(),
                CredentialRecord {
                    binding: request.binding().to_base64url(),
                    operation: request.operation(),
                    scope: request.scope().clone(),
                    expires_at: request.expires_at(),
                },
            );
            TransportCredential::from_host_authority_bearer(token)
        })
    }

    fn verify_and_consume<'a>(
        &'a self,
        request: SubscriptionCredentialRequest<'a>,
    ) -> TestFuture<'a, Result<SubscriptionCredentialDecision, SubscriptionError>> {
        Box::pin(async move {
            assert!(!format!("{request:?}").contains("scoped-async-credential-sentinel"));
            let mut records = self.credentials.lock().expect("credentials");
            let accepted = request.presented().is_some_and(|credential| {
                let bearer = credential.expose_authorization_bearer();
                let accepted = records.get(bearer).is_some_and(|record| {
                    record.binding == request.binding().to_base64url()
                        && record.operation == request.operation()
                        && record.scope == *request.scope()
                        && record.expires_at == request.expires_at()
                        && request.now() < record.expires_at
                });
                if accepted {
                    records.remove(bearer);
                }
                accepted
            });
            Ok(if accepted {
                SubscriptionCredentialDecision::Accept
            } else {
                SubscriptionCredentialDecision::Reject
            })
        })
    }
}

fn metadata() -> SubscriptionMetadata {
    SubscriptionMetadata::new(
        StreamName::parse("orders.activity").expect("stream"),
        BoundedTopics::new(vec![
            TopicName::parse("tenant/:tenant/orders").expect("topic template"),
            TopicName::parse("tenant/:tenant/presence").expect("topic template"),
        ])
        .expect("topics"),
        BoundedEventNames::new(vec![
            BrowserOperationName::parse("order.updated").expect("event"),
        ])
        .expect("events"),
        suprnova_live::async_updates::SubscriptionModes::new(vec![
            suprnova_live::async_updates::SubscriptionMode::WebSocket,
        ])
        .expect("mode"),
        ReconnectPolicy::ResumeOrRefresh {
            maximum_attempts: NonZeroU8::new(4).expect("attempts"),
        },
    )
}

struct OrderUpdated;

impl EventPayloadMetadata for OrderUpdated {
    const NAME: &'static str = "order.updated";
    const VERSION: u16 = 1;
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
}

struct OrderUpdatedV2;

impl EventPayloadMetadata for OrderUpdatedV2 {
    const NAME: &'static str = "order.updated";
    const VERSION: u16 = 2;
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
}

struct OrderUpdatedPayloadRevision;

impl EventPayloadMetadata for OrderUpdatedPayloadRevision {
    const NAME: &'static str = "order.updated";
    const VERSION: u16 = 1;
    const PAYLOAD_CONTRACT: &'static str = "orders.revised-payload";
}

struct OrderUpdatedSchemaRevision;

impl EventPayloadMetadata for OrderUpdatedSchemaRevision {
    const NAME: &'static str = "order.updated";
    const VERSION: u16 = 1;
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Boolean;
}

struct OrderRenamed;

impl EventPayloadMetadata for OrderRenamed {
    const NAME: &'static str = "order.renamed";
    const VERSION: u16 = 1;
}

macro_rules! define_budget_events {
    ($($name:ident => $wire_name:literal),+ $(,)?) => {
        $(
            struct $name;

            impl EventPayloadMetadata for $name {
                const NAME: &'static str = $wire_name;
                const VERSION: u16 = 1;
            }
        )+

        fn oversized_registered_events(targets: &BoundedTargets) -> Vec<EventMetadata> {
            vec![
                $(
                    EventMetadata::from_payload_with_contract::<$name>(
                        EventSource::Stream,
                        targets.clone(),
                        EventOrder::PerSourceSequence,
                        EventCyclePolicy::ForbidRepeatedIsland,
                        16,
                    )
                    .expect("bounded stream event"),
                )+
            ]
        }
    };
}

define_budget_events!(
    BudgetEvent01 => "budget.event.01",
    BudgetEvent02 => "budget.event.02",
    BudgetEvent03 => "budget.event.03",
    BudgetEvent04 => "budget.event.04",
    BudgetEvent05 => "budget.event.05",
    BudgetEvent06 => "budget.event.06",
    BudgetEvent07 => "budget.event.07",
    BudgetEvent08 => "budget.event.08",
    BudgetEvent09 => "budget.event.09",
    BudgetEvent10 => "budget.event.10",
    BudgetEvent11 => "budget.event.11",
    BudgetEvent12 => "budget.event.12",
    BudgetEvent13 => "budget.event.13",
    BudgetEvent14 => "budget.event.14",
    BudgetEvent15 => "budget.event.15",
    BudgetEvent16 => "budget.event.16",
    BudgetEvent17 => "budget.event.17",
    BudgetEvent18 => "budget.event.18",
    BudgetEvent19 => "budget.event.19",
    BudgetEvent20 => "budget.event.20",
    BudgetEvent21 => "budget.event.21",
    BudgetEvent22 => "budget.event.22",
    BudgetEvent23 => "budget.event.23",
    BudgetEvent24 => "budget.event.24",
);

fn oversized_component_metadata() -> ComponentMetadata {
    let original = component_metadata();
    let targets = BoundedTargets::new(
        (0..16)
            .map(|index| {
                let prefix = format!("listener-{index:02}-");
                let value = format!("{prefix}{}", "x".repeat(128 - prefix.len()));
                EventTarget::Browser(
                    BrowserOperationName::parse(&value).expect("maximum-length listener"),
                )
            })
            .collect(),
    )
    .expect("maximum target set");
    let events = oversized_registered_events(&targets);
    let event_names = events.iter().map(|event| event.name().clone()).collect();
    let subscription = SubscriptionMetadata::new(
        metadata().stream().clone(),
        metadata().topics().clone(),
        BoundedEventNames::new(event_names).expect("bounded event names"),
        metadata().modes().clone(),
        metadata().reconnect(),
    );
    ComponentMetadata::new_with_async_contracts(
        original.identity().clone(),
        original.view().clone(),
        original.versions(),
        original.fields().to_vec(),
        original.actions().to_vec(),
        events,
        original.effects().to_vec(),
        vec![subscription],
        original.refresh_on_promote(),
    )
    .expect("large but individually legal component metadata")
}

fn component_metadata() -> &'static ComponentMetadata {
    static METADATA: OnceLock<ComponentMetadata> = OnceLock::new();
    METADATA.get_or_init(|| {
        let base = component_support::metadata();
        ComponentMetadata::new_with_async_contracts(
            base.identity().clone(),
            base.view().clone(),
            base.versions(),
            base.fields().to_vec(),
            base.actions().to_vec(),
            vec![
                EventMetadata::from_payload_with_contract::<OrderUpdated>(
                    EventSource::Stream,
                    BoundedTargets::new(vec![EventTarget::SelfIsland]).expect("targets"),
                    EventOrder::PerSourceSequence,
                    EventCyclePolicy::ForbidRepeatedIsland,
                    1,
                )
                .expect("stream event"),
            ],
            base.effects().to_vec(),
            vec![metadata()],
            base.refresh_on_promote(),
        )
        .expect("async component metadata")
    })
}

fn event_contract<T: EventPayloadMetadata + 'static>(
    targets: Vec<EventTarget>,
    cycle: EventCyclePolicy,
    maximum_fanout: u16,
) -> SubscriptionEventContract {
    let metadata = EventMetadata::from_payload_with_contract::<T>(
        EventSource::Stream,
        BoundedTargets::new(targets).expect("targets"),
        EventOrder::PerSourceSequence,
        cycle,
        maximum_fanout,
    )
    .expect("event metadata");
    SubscriptionEventContract::from_registered(&metadata).expect("subscription event contract")
}

fn other_component_metadata() -> ComponentMetadata {
    let original = component_metadata();
    ComponentMetadata::new_with_async_contracts(
        ComponentName::parse("tests.other").expect("component"),
        ViewName::parse("tests/other.html").expect("view"),
        original.versions(),
        original.fields().to_vec(),
        original.actions().to_vec(),
        original.events().to_vec(),
        original.effects().to_vec(),
        original.subscriptions().to_vec(),
        original.refresh_on_promote(),
    )
    .expect("other component metadata")
}

fn revised_component_metadata() -> ComponentMetadata {
    let original = component_metadata();
    ComponentMetadata::new_with_async_contracts(
        original.identity().clone(),
        original.view().clone(),
        original.versions(),
        original.fields().to_vec(),
        original.actions().to_vec(),
        vec![
            EventMetadata::from_payload_with_contract::<OrderUpdatedV2>(
                EventSource::Stream,
                BoundedTargets::new(vec![EventTarget::SelfIsland]).expect("targets"),
                EventOrder::PerSourceSequence,
                EventCyclePolicy::ForbidRepeatedIsland,
                1,
            )
            .expect("revised stream event"),
        ],
        original.effects().to_vec(),
        original.subscriptions().to_vec(),
        original.refresh_on_promote(),
    )
    .expect("same-name revised component metadata")
}

fn key_ring() -> SnapshotKeyRing {
    let active = KeyRecord::new(
        KeyId::parse("async-key-1").expect("key id"),
        RootKey::new(vec![0x41; 32]).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(50_000),
        UnixMillis::new(100_000),
    )
    .expect("key record");
    SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
}

fn issue_request(expires_at: UnixMillis) -> SubscriptionIssueRequest {
    SubscriptionIssueRequest::new(
        metadata().stream().clone(),
        CapabilityVersion::new(1).expect("capability"),
        expires_at,
        PollFallbackPolicy::new(
            10_000,
            1_500,
            PollInitialBehavior::AfterInterval,
            PollVisibilityPolicy::PauseWhenHidden,
        )
        .expect("fallback"),
    )
}

#[test]
fn trusted_registration_calculates_a_representable_worst_case_claim_budget() {
    let registration = CurrentSubscriptionRegistration::from_registered(
        component_metadata(),
        metadata().stream(),
        &TrustedMountParameters::new(vec![("tenant".to_owned(), "7".to_owned())])
            .expect("trusted mount parameters"),
    )
    .expect("ordinary registration");

    assert!(registration.canonical_claim_budget_bytes() > 0);
    assert!(registration.canonical_claim_budget_bytes() <= MAX_CANONICAL_SUBSCRIPTION_CLAIMS_BYTES);
}

#[test]
fn trusted_registration_rejects_cross_field_claim_budget_before_issuance() {
    assert_eq!(
        CurrentSubscriptionRegistration::from_registered(
            &oversized_component_metadata(),
            metadata().stream(),
            &TrustedMountParameters::new(vec![("tenant".to_owned(), "7".to_owned())])
                .expect("trusted mount parameters"),
        )
        .expect_err("individually legal event maxima exceed the canonical descriptor budget")
        .kind(),
        SubscriptionErrorKind::DescriptorBudgetExceeded
    );
}

#[tokio::test]
async fn issue_rejects_stream_absent_from_the_current_registry() {
    let ports = ControlledSubscriptionPorts::allowing();
    let context = trusted_context(ports);
    assert_eq!(
        SubscriptionService::new(key_ring())
            .issue(
                &context,
                SubscriptionIssueRequest::new(
                    StreamName::parse("orders.directive_interpolated").expect("stream"),
                    CapabilityVersion::new(1).expect("capability"),
                    UnixMillis::new(5_000),
                    PollFallbackPolicy::new(
                        10_000,
                        1_500,
                        PollInitialBehavior::AfterInterval,
                        PollVisibilityPolicy::PauseWhenHidden,
                    )
                    .expect("fallback"),
                ),
                UnixMillis::new(1_000),
            )
            .await
            .expect_err("unregistered directive-selected stream must fail")
            .kind(),
        SubscriptionErrorKind::UnregisteredSubscription
    );
}

#[tokio::test]
async fn trusted_mount_parameters_are_the_only_topic_interpolation_source() {
    let ports = ControlledSubscriptionPorts::allowing();
    let context = trusted_context(ports.clone());
    let service = SubscriptionService::new(key_ring());
    let issued = service
        .issue(
            &context,
            issue_request(UnixMillis::new(5_000)),
            UnixMillis::new(1_000),
        )
        .await
        .expect("trusted parameters resolve registered topic templates");
    let verified = SubscriptionDescriptorCodec::new(key_ring())
        .verify(issued.descriptor(), UnixMillis::new(1_001))
        .expect("descriptor verifies");
    let topics = verified
        .claims()
        .topics()
        .as_slice()
        .iter()
        .map(TopicName::as_str)
        .collect::<Vec<_>>();
    assert_eq!(topics, ["tenant/7/orders", "tenant/7/presence"]);

    ports.set_tenant_parameter("8");
    assert_eq!(
        service
            .connect(
                &context,
                issued.descriptor(),
                issued.transport_credential(),
                UnixMillis::new(1_100),
            )
            .await
            .expect_err("a current trusted topic revision revokes the descriptor")
            .kind(),
        SubscriptionErrorKind::ScopeMismatch
    );
}

#[tokio::test]
async fn issue_uses_only_the_host_continuity_baseline() {
    let ports = ControlledSubscriptionPorts::allowing();
    let authoritative = StreamPosition::new(StreamEpoch::new(9), StreamSequence::new(41));
    ports.set_baseline(authoritative);
    let context = trusted_context(ports);
    let issued = SubscriptionService::new(key_ring())
        .issue(
            &context,
            issue_request(UnixMillis::new(5_000)),
            UnixMillis::new(1_000),
        )
        .await
        .expect("issue with host continuity baseline");
    let verified = SubscriptionDescriptorCodec::new(key_ring())
        .verify(issued.descriptor(), UnixMillis::new(1_001))
        .expect("descriptor verifies");

    assert_eq!(verified.baseline(), authoritative);
}

fn trusted_context(ports: Arc<ControlledSubscriptionPorts>) -> TrustedLiveRequestContext {
    trusted_context_for(
        Some(ports),
        component_metadata(),
        component_support::fixture_host_scope(),
    )
}

fn trusted_context_for(
    ports: Option<Arc<ControlledSubscriptionPorts>>,
    component_metadata: &ComponentMetadata,
    scope: HostScopeFacts,
) -> TrustedLiveRequestContext {
    let descriptor = ComponentDescriptor::new(component_metadata.clone());
    let contract = ComponentContract::new(
        component_metadata.identity().clone(),
        descriptor.contract_digest().clone(),
        1,
        1,
        1,
    )
    .expect("component contract");
    let registry = ComponentRegistryBuilder::new()
        .register(descriptor)
        .expect("component registers")
        .build();
    let route = component_support::snapshot_support::route(0x30);
    let slot = IslandSlot::parse("trace").expect("slot identity");
    let catalog = MountCatalogBuilder::new()
        .register(
            &registry,
            MountCatalogEntry::new(
                ExpectedSeedV1::new(
                    contract,
                    BuildId::parse("build-async-tests").expect("build identity"),
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
        .expect("mount catalog entry")
        .build();
    let mut capabilities = HostCapabilities::bound_to(scope.clone());
    if let Some(ports) = ports {
        capabilities = capabilities
            .with_subscription_registry(ports.clone())
            .with_subscription_continuity(ports.clone())
            .with_subscription_authorization(ports.clone())
            .with_subscription_credentials(ports);
    }
    let expires_at = UnixMillis::new(10_000);
    let mut checks = HostCheckFacts::new();
    let overrides = BTreeMap::<CheckKind, CheckFact>::new();
    for kind in CheckKind::ALL {
        checks
            .record(
                kind,
                overrides
                    .get(&kind)
                    .copied()
                    .unwrap_or_else(|| CheckFact::new(CheckDisposition::Passed, expires_at)),
            )
            .expect("host check");
    }
    let selection = MountSelection::new(
        route.clone(),
        slot.clone(),
        component_metadata.identity().clone(),
        component_metadata.contract_digest().clone(),
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

#[tokio::test]
async fn connect_and_renew_recheck_current_authorization_and_rotate_credentials() {
    let ports = ControlledSubscriptionPorts::allowing();
    let context = trusted_context(ports.clone());
    let service = SubscriptionService::new(key_ring());
    let issued = service
        .issue(
            &context,
            issue_request(UnixMillis::new(5_000)),
            UnixMillis::new(1_000),
        )
        .await
        .expect("issue subscription");
    let sentinel = std::str::from_utf8(CREDENTIAL_SENTINEL).expect("ASCII sentinel");
    for public_surface in [
        issued.descriptor().as_str().to_owned(),
        format!("{issued:?}"),
        format!("{:?}", issued.descriptor()),
        format!(
            "<section data-live-stream=\"{}\"></section>",
            issued.descriptor().as_str()
        ),
        format!(
            "{{\"snapshot\":null,\"subscription\":{:?}}}",
            issued.descriptor()
        ),
        format!("/__live/stream?descriptor={}", issued.descriptor().as_str()),
        format!("{{\"history_subscription\":{:?}}}", issued.descriptor()),
    ] {
        assert!(!public_surface.contains(sentinel));
    }
    let connected = service
        .connect(
            &context,
            issued.descriptor(),
            issued.transport_credential(),
            UnixMillis::new(1_100),
        )
        .await
        .expect("authorize connect");
    let renewed = service
        .renew(
            &context,
            issued.descriptor(),
            connected.renewal_credential(),
            UnixMillis::new(6_000),
            UnixMillis::new(1_200),
        )
        .await
        .expect("authorize renewal");

    assert_eq!(renewed.expires_at(), UnixMillis::new(6_000));
    for redacted_surface in [
        format!("{connected:?}"),
        format!("{:?}", connected.renewal_credential()),
        format!("{renewed:?}"),
        format!("<section data-live-renewal=\"{connected:?}\"></section>"),
        format!("/stream/renew?subscription={connected:?}"),
        format!("{{\"history_renewal\":\"{connected:?}\"}}"),
        format!("{{\"snapshot_renewal\":\"{connected:?}\"}}"),
    ] {
        assert!(!redacted_surface.contains(sentinel));
    }
    assert_eq!(
        ports.operations.lock().expect("operations").as_slice(),
        &[
            SubscriptionAuthorizationOperation::Issue,
            SubscriptionAuthorizationOperation::Connect,
            SubscriptionAuthorizationOperation::Renew,
        ]
    );
}

#[tokio::test]
async fn credentials_reject_cross_descriptor_cross_operation_and_expired_use() {
    let ports = ControlledSubscriptionPorts::allowing();
    let context = trusted_context(ports.clone());
    let service = SubscriptionService::new(key_ring());
    let first = service
        .issue(
            &context,
            issue_request(UnixMillis::new(5_000)),
            UnixMillis::new(1_000),
        )
        .await
        .expect("issue first descriptor");
    let connected = service
        .connect(
            &context,
            first.descriptor(),
            first.transport_credential(),
            UnixMillis::new(1_050),
        )
        .await
        .expect("connect credential is exact");
    let second = service
        .issue(
            &context,
            issue_request(UnixMillis::new(6_000)),
            UnixMillis::new(1_060),
        )
        .await
        .expect("issue distinct descriptor");

    assert_eq!(
        service
            .connect(
                &context,
                first.descriptor(),
                second.transport_credential(),
                UnixMillis::new(1_070),
            )
            .await
            .expect_err("credential cannot cross descriptor bindings")
            .kind(),
        SubscriptionErrorKind::InvalidCredential
    );
    assert_eq!(
        service
            .renew(
                &context,
                second.descriptor(),
                second.transport_credential(),
                UnixMillis::new(7_000),
                UnixMillis::new(1_080),
            )
            .await
            .expect_err("connect credential cannot authorize renewal")
            .kind(),
        SubscriptionErrorKind::InvalidCredential
    );
    assert_eq!(
        service
            .connect(
                &context,
                first.descriptor(),
                connected.renewal_credential(),
                UnixMillis::new(1_090),
            )
            .await
            .expect_err("renewal credential cannot authorize connect")
            .kind(),
        SubscriptionErrorKind::InvalidCredential
    );

    ports.expire_credential(second.transport_credential(), UnixMillis::new(1_100));
    assert_eq!(
        service
            .connect(
                &context,
                second.descriptor(),
                second.transport_credential(),
                UnixMillis::new(1_100),
            )
            .await
            .expect_err("credential expiry is exclusive independently of descriptor expiry")
            .kind(),
        SubscriptionErrorKind::InvalidCredential
    );
}

#[tokio::test]
async fn connect_credential_is_consumed_atomically_after_one_use() {
    let ports = ControlledSubscriptionPorts::allowing();
    let context = trusted_context(ports);
    let service = SubscriptionService::new(key_ring());
    let issued = service
        .issue(
            &context,
            issue_request(UnixMillis::new(5_000)),
            UnixMillis::new(1_000),
        )
        .await
        .expect("issue connect credential");
    service
        .connect(
            &context,
            issued.descriptor(),
            issued.transport_credential(),
            UnixMillis::new(1_050),
        )
        .await
        .expect("first connect consumes credential");

    assert_eq!(
        service
            .connect(
                &context,
                issued.descriptor(),
                issued.transport_credential(),
                UnixMillis::new(1_060),
            )
            .await
            .expect_err("connect credential replay must fail")
            .kind(),
        SubscriptionErrorKind::InvalidCredential
    );
}

#[tokio::test]
async fn renewal_credential_is_consumed_atomically_after_one_use() {
    let ports = ControlledSubscriptionPorts::allowing();
    let context = trusted_context(ports);
    let service = SubscriptionService::new(key_ring());
    let issued = service
        .issue(
            &context,
            issue_request(UnixMillis::new(5_000)),
            UnixMillis::new(1_000),
        )
        .await
        .expect("issue connect credential");
    let connected = service
        .connect(
            &context,
            issued.descriptor(),
            issued.transport_credential(),
            UnixMillis::new(1_050),
        )
        .await
        .expect("connect and mint renewal credential");
    service
        .renew(
            &context,
            issued.descriptor(),
            connected.renewal_credential(),
            UnixMillis::new(6_000),
            UnixMillis::new(1_060),
        )
        .await
        .expect("first renewal consumes credential");

    assert_eq!(
        service
            .renew(
                &context,
                issued.descriptor(),
                connected.renewal_credential(),
                UnixMillis::new(6_000),
                UnixMillis::new(1_070),
            )
            .await
            .expect_err("renewal credential replay must fail")
            .kind(),
        SubscriptionErrorKind::InvalidCredential
    );
}

#[tokio::test]
async fn fixed_global_or_repeated_credential_fails_issuance_conformance() {
    let ports = ControlledSubscriptionPorts::allowing();
    ports.use_fixed_credentials();
    let context = trusted_context(ports);
    let service = SubscriptionService::new(key_ring());
    service
        .issue(
            &context,
            issue_request(UnixMillis::new(5_000)),
            UnixMillis::new(1_000),
        )
        .await
        .expect("first unique registration of provider token");

    assert_eq!(
        service
            .issue(
                &context,
                issue_request(UnixMillis::new(5_000)),
                UnixMillis::new(1_001),
            )
            .await
            .expect_err("fixed global credential must fail provider conformance")
            .kind(),
        SubscriptionErrorKind::InvalidCredential
    );
}

#[tokio::test]
async fn issued_credentials_are_mac_derived_and_unique_per_issuance() {
    let ports = ControlledSubscriptionPorts::allowing();
    let context = trusted_context(ports);
    let service = SubscriptionService::new(key_ring());
    let first = service
        .issue(
            &context,
            issue_request(UnixMillis::new(5_000)),
            UnixMillis::new(1_000),
        )
        .await
        .expect("first credential");
    let second = service
        .issue(
            &context,
            issue_request(UnixMillis::new(5_000)),
            UnixMillis::new(1_001),
        )
        .await
        .expect("second credential");
    let first_bytes = first.transport_credential().expose_authorization_bearer();
    let second_bytes = second.transport_credential().expose_authorization_bearer();

    assert_eq!(first_bytes.len(), CREDENTIAL_SENTINEL.len() + 32);
    assert_eq!(second_bytes.len(), CREDENTIAL_SENTINEL.len() + 32);
    assert_ne!(first_bytes, second_bytes);
}

#[tokio::test]
async fn revocation_and_wrong_current_scope_fail_closed_before_renewal() {
    let ports = ControlledSubscriptionPorts::allowing();
    let context = trusted_context(ports.clone());
    let service = SubscriptionService::new(key_ring());
    let issued = service
        .issue(
            &context,
            issue_request(UnixMillis::new(5_000)),
            UnixMillis::new(1_000),
        )
        .await
        .expect("issue subscription");
    let connected = service
        .connect(
            &context,
            issued.descriptor(),
            issued.transport_credential(),
            UnixMillis::new(1_100),
        )
        .await
        .expect("connect before current authorization revocation");

    ports.revoke();
    assert_eq!(
        service
            .renew(
                &context,
                issued.descriptor(),
                connected.renewal_credential(),
                UnixMillis::new(6_000),
                UnixMillis::new(1_200),
            )
            .await
            .expect_err("revoked current authorization must fail")
            .kind(),
        SubscriptionErrorKind::AuthorizationDenied
    );
}

#[tokio::test]
async fn same_name_component_contract_revision_revokes_connect_before_policy() {
    let ports = ControlledSubscriptionPorts::allowing();
    let original_context = trusted_context(ports.clone());
    let service = SubscriptionService::new(key_ring());
    let issued = service
        .issue(
            &original_context,
            issue_request(UnixMillis::new(5_000)),
            UnixMillis::new(1_000),
        )
        .await
        .expect("issue original component subscription");
    let calls_before = ports.operations.lock().expect("operations").len();
    let revised = revised_component_metadata();
    ports.revise_component(revised.clone());
    let revised_context = trusted_context_for(
        Some(ports.clone()),
        &revised,
        component_support::fixture_host_scope(),
    );

    assert_eq!(
        service
            .connect(
                &revised_context,
                issued.descriptor(),
                issued.transport_credential(),
                UnixMillis::new(1_100),
            )
            .await
            .expect_err("same-name contract revision must revoke old descriptor")
            .kind(),
        SubscriptionErrorKind::ScopeMismatch
    );
    assert_eq!(
        ports.operations.lock().expect("operations").len(),
        calls_before,
        "registry revision must fail before current policy"
    );
}

#[tokio::test]
async fn validly_signed_event_contract_field_mutations_fail_current_registry_checks() {
    let ports = ControlledSubscriptionPorts::allowing();
    let context = trusted_context(ports.clone());
    let service = SubscriptionService::new(key_ring());
    let issued = service
        .issue(
            &context,
            issue_request(UnixMillis::new(5_000)),
            UnixMillis::new(1_000),
        )
        .await
        .expect("issue current event contract");
    let verified = SubscriptionDescriptorCodec::new(key_ring())
        .verify(issued.descriptor(), UnixMillis::new(1_001))
        .expect("verify issued claims");
    let original = verified.claims();
    let mutations = vec![
        event_contract::<OrderRenamed>(
            vec![EventTarget::SelfIsland],
            EventCyclePolicy::ForbidRepeatedIsland,
            1,
        ),
        event_contract::<OrderUpdatedV2>(
            vec![EventTarget::SelfIsland],
            EventCyclePolicy::ForbidRepeatedIsland,
            1,
        ),
        event_contract::<OrderUpdatedPayloadRevision>(
            vec![EventTarget::SelfIsland],
            EventCyclePolicy::ForbidRepeatedIsland,
            1,
        ),
        event_contract::<OrderUpdatedSchemaRevision>(
            vec![EventTarget::SelfIsland],
            EventCyclePolicy::ForbidRepeatedIsland,
            1,
        ),
        event_contract::<OrderUpdated>(
            vec![EventTarget::Parent],
            EventCyclePolicy::ForbidRepeatedIsland,
            1,
        ),
        event_contract::<OrderUpdated>(
            vec![EventTarget::SelfIsland],
            EventCyclePolicy::MaximumHops(NonZeroU8::new(2).expect("hops")),
            1,
        ),
        event_contract::<OrderUpdated>(
            vec![EventTarget::SelfIsland],
            EventCyclePolicy::ForbidRepeatedIsland,
            2,
        ),
    ];

    for mutation in mutations {
        let claims = SubscriptionClaims::new(
            original.stream().clone(),
            original.protocol(),
            original.capability(),
            original.topics().clone(),
            BoundedEventContracts::new(vec![mutation]).expect("one mutated contract"),
            original.authorization_memo().clone(),
            original.baseline(),
            original.expires_at(),
            original.reconnect(),
            original.fallback_poll(),
        )
        .expect("mutated claims remain structurally valid");
        let descriptor = SubscriptionDescriptorCodec::new(key_ring())
            .sign(&claims, UnixMillis::new(1_010))
            .expect("sign mutated contract");
        assert_eq!(
            service
                .connect(
                    &context,
                    &descriptor,
                    issued.transport_credential(),
                    UnixMillis::new(1_020),
                )
                .await
                .expect_err("current registry must reject a changed event contract")
                .kind(),
            SubscriptionErrorKind::ScopeMismatch
        );
    }
}

#[tokio::test]
async fn current_registry_stream_removal_revokes_descriptor_before_policy() {
    let ports = ControlledSubscriptionPorts::allowing();
    let context = trusted_context(ports.clone());
    let service = SubscriptionService::new(key_ring());
    let issued = service
        .issue(
            &context,
            issue_request(UnixMillis::new(5_000)),
            UnixMillis::new(1_000),
        )
        .await
        .expect("issue registered stream");
    let calls_before = ports.operations.lock().expect("operations").len();
    ports.revise_component(component_support::metadata().clone());

    assert_eq!(
        service
            .connect(
                &context,
                issued.descriptor(),
                issued.transport_credential(),
                UnixMillis::new(1_100),
            )
            .await
            .expect_err("removed current stream must revoke descriptor")
            .kind(),
        SubscriptionErrorKind::UnregisteredSubscription
    );
    assert_eq!(
        ports.operations.lock().expect("operations").len(),
        calls_before,
        "removed stream must fail before policy"
    );
}

#[tokio::test]
async fn principal_session_tenant_and_component_substitution_fail_before_policy() {
    let ports = ControlledSubscriptionPorts::allowing();
    let context = trusted_context(ports.clone());
    let service = SubscriptionService::new(key_ring());
    let issued = service
        .issue(
            &context,
            issue_request(UnixMillis::new(5_000)),
            UnixMillis::new(1_000),
        )
        .await
        .expect("issue subscription");
    let calls_before = ports.operations.lock().expect("operations").len();
    let base = component_support::fixture_host_scope();
    let principal_context = trusted_context_for(
        Some(ports.clone()),
        component_metadata(),
        HostScopeFacts::new(
            base.scope().clone(),
            base.session().cloned(),
            Some(PrincipalFingerprint::from_bytes(&[0x91; 32]).expect("principal")),
            base.tenant().cloned(),
        ),
    );
    let session_context = trusted_context_for(
        Some(ports.clone()),
        component_metadata(),
        HostScopeFacts::new(
            base.scope().clone(),
            Some(SessionFingerprint::from_bytes(&[0x92; 32]).expect("session")),
            base.principal().cloned(),
            base.tenant().cloned(),
        ),
    );
    let tenant_context = trusted_context_for(
        Some(ports.clone()),
        component_metadata(),
        HostScopeFacts::new(
            base.scope().clone(),
            base.session().cloned(),
            base.principal().cloned(),
            Some(TenantFingerprint::from_bytes(&[0x93; 32]).expect("tenant")),
        ),
    );
    let other_component = other_component_metadata();
    let component_context = trusted_context_for(
        Some(ports.clone()),
        &other_component,
        component_support::fixture_host_scope(),
    );

    for wrong_context in [
        &principal_context,
        &session_context,
        &tenant_context,
        &component_context,
    ] {
        assert_eq!(
            service
                .connect(
                    wrong_context,
                    issued.descriptor(),
                    issued.transport_credential(),
                    UnixMillis::new(1_300),
                )
                .await
                .expect_err("current identity substitution must fail")
                .kind(),
            SubscriptionErrorKind::ScopeMismatch
        );
    }
    assert_eq!(
        ports.operations.lock().expect("operations").len(),
        calls_before,
        "wrong current identity must fail before policy"
    );
}

#[tokio::test]
async fn expired_descriptor_fails_before_current_policy_is_called() {
    let ports = ControlledSubscriptionPorts::allowing();
    let context = trusted_context(ports.clone());
    let service = SubscriptionService::new(key_ring());
    let issued = service
        .issue(
            &context,
            issue_request(UnixMillis::new(1_100)),
            UnixMillis::new(1_000),
        )
        .await
        .expect("issue subscription");
    let calls_before = ports.operations.lock().expect("operations").len();

    assert_eq!(
        service
            .connect(
                &context,
                issued.descriptor(),
                issued.transport_credential(),
                UnixMillis::new(1_100),
            )
            .await
            .expect_err("expiry is exclusive")
            .kind(),
        SubscriptionErrorKind::DescriptorExpired
    );
    assert_eq!(
        ports.operations.lock().expect("operations").len(),
        calls_before,
        "expired descriptors must fail before the policy port"
    );
}

#[tokio::test]
async fn missing_current_host_subscription_capabilities_fail_closed() {
    let context = trusted_context_for(
        None,
        component_metadata(),
        component_support::fixture_host_scope(),
    );

    assert_eq!(
        SubscriptionService::new(key_ring())
            .issue(
                &context,
                issue_request(UnixMillis::new(5_000)),
                UnixMillis::new(1_000),
            )
            .await
            .expect_err("subscription issuance requires current host authorization")
            .kind(),
        SubscriptionErrorKind::AuthorizationUnavailable
    );
}
