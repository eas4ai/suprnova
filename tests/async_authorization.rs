//! Current authorization and credential-renewal tests for subscriptions.

use std::collections::BTreeMap;
use std::future::Future;
use std::num::NonZeroU8;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use suprnova_live::async_updates::{
    BoundedEventContracts, BoundedTargets, BoundedTopics, BrowserPayloadSchema, CapabilityVersion,
    EventCyclePolicy, EventOrder, EventSource, EventTarget, PollFallbackPolicy,
    PollInitialBehavior, PollVisibilityPolicy, ReconnectPolicy, StreamEpoch, StreamName,
    StreamPosition, StreamSequence, SubscriptionAuthorizationDecision,
    SubscriptionAuthorizationOperation, SubscriptionAuthorizationPort,
    SubscriptionAuthorizationRequest, SubscriptionCredentialDecision, SubscriptionCredentialPort,
    SubscriptionCredentialRequest, SubscriptionError, SubscriptionErrorKind,
    SubscriptionIssueRequest, SubscriptionMetadata, SubscriptionService, TopicName,
    TransportCredential,
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

#[derive(Default)]
struct ControlledSubscriptionPorts {
    allowed: AtomicBool,
    operations: Mutex<Vec<SubscriptionAuthorizationOperation>>,
}

impl ControlledSubscriptionPorts {
    fn allowing() -> Arc<Self> {
        Arc::new(Self {
            allowed: AtomicBool::new(true),
            operations: Mutex::new(Vec::new()),
        })
    }

    fn revoke(&self) {
        self.allowed.store(false, Ordering::SeqCst);
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
        _request: SubscriptionCredentialRequest<'a>,
    ) -> TestFuture<'a, Result<TransportCredential, SubscriptionError>> {
        Box::pin(async {
            TransportCredential::from_host_authority_bearer(CREDENTIAL_SENTINEL.to_vec())
        })
    }

    fn verify<'a>(
        &'a self,
        request: SubscriptionCredentialRequest<'a>,
    ) -> TestFuture<'a, Result<SubscriptionCredentialDecision, SubscriptionError>> {
        Box::pin(async move {
            Ok(
                if request.presented().is_some_and(|credential| {
                    credential.expose_authorization_bearer() == CREDENTIAL_SENTINEL
                }) {
                    SubscriptionCredentialDecision::Accept
                } else {
                    SubscriptionCredentialDecision::Reject
                },
            )
        })
    }
}

fn metadata() -> SubscriptionMetadata {
    SubscriptionMetadata::new(
        StreamName::parse("orders.activity").expect("stream"),
        BoundedTopics::new(vec![
            TopicName::parse("tenant/7/orders").expect("topic"),
            TopicName::parse("tenant/7/presence").expect("topic"),
        ])
        .expect("topics"),
        BoundedEventContracts::new(vec![
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
    SubscriptionIssueRequest::from_registered(
        component_metadata(),
        metadata().stream(),
        CapabilityVersion::new(1).expect("capability"),
        StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(19)),
        expires_at,
        PollFallbackPolicy::new(
            10_000,
            1_500,
            PollInitialBehavior::AfterInterval,
            PollVisibilityPolicy::PauseWhenHidden,
        )
        .expect("fallback"),
    )
    .expect("registered subscription")
}

#[test]
fn issue_request_selects_only_registry_digested_stream_metadata() {
    assert_eq!(
        SubscriptionIssueRequest::from_registered(
            component_metadata(),
            &StreamName::parse("orders.directive_interpolated").expect("stream"),
            CapabilityVersion::new(1).expect("capability"),
            StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(19)),
            UnixMillis::new(5_000),
            PollFallbackPolicy::new(
                10_000,
                1_500,
                PollInitialBehavior::AfterInterval,
                PollVisibilityPolicy::PauseWhenHidden,
            )
            .expect("fallback"),
        )
        .expect_err("unregistered directive-selected stream must fail")
        .kind(),
        SubscriptionErrorKind::UnregisteredSubscription
    );
}

#[tokio::test]
async fn issue_rejects_metadata_from_a_different_registered_component_contract() {
    let ports = ControlledSubscriptionPorts::allowing();
    let context = trusted_context(ports);
    let other = other_component_metadata();
    let request = SubscriptionIssueRequest::from_registered(
        &other,
        metadata().stream(),
        CapabilityVersion::new(1).expect("capability"),
        StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(19)),
        UnixMillis::new(5_000),
        PollFallbackPolicy::new(
            10_000,
            1_500,
            PollInitialBehavior::AfterInterval,
            PollVisibilityPolicy::PauseWhenHidden,
        )
        .expect("fallback"),
    )
    .expect("stream is registered on other component");

    assert_eq!(
        SubscriptionService::new(key_ring())
            .issue(&context, request, UnixMillis::new(1_000))
            .await
            .expect_err("cross-component metadata must fail before signing")
            .kind(),
        SubscriptionErrorKind::UnregisteredSubscription
    );
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
    service
        .connect(
            &context,
            issued.descriptor(),
            issued.transport_credential(),
            &metadata(),
            UnixMillis::new(1_100),
        )
        .await
        .expect("authorize connect");
    let renewed = service
        .renew(
            &context,
            issued.descriptor(),
            issued.transport_credential(),
            &metadata(),
            UnixMillis::new(6_000),
            UnixMillis::new(1_200),
        )
        .await
        .expect("authorize renewal");

    assert_eq!(renewed.expires_at(), UnixMillis::new(6_000));
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

    ports.revoke();
    assert_eq!(
        service
            .renew(
                &context,
                issued.descriptor(),
                issued.transport_credential(),
                &metadata(),
                UnixMillis::new(6_000),
                UnixMillis::new(1_200),
            )
            .await
            .expect_err("revoked current authorization must fail")
            .kind(),
        SubscriptionErrorKind::AuthorizationDenied
    );

    let wrong_topics = SubscriptionMetadata::new(
        StreamName::parse("orders.activity").expect("stream"),
        BoundedTopics::new(vec![TopicName::parse("tenant/8/orders").expect("topic")])
            .expect("topics"),
        metadata().events().clone(),
        metadata().modes().clone(),
        metadata().reconnect(),
    );
    assert_eq!(
        service
            .connect(
                &context,
                issued.descriptor(),
                issued.transport_credential(),
                &wrong_topics,
                UnixMillis::new(1_300),
            )
            .await
            .expect_err("topic substitution must fail")
            .kind(),
        SubscriptionErrorKind::ScopeMismatch
    );

    let wrong_stream = SubscriptionMetadata::new(
        StreamName::parse("orders.other").expect("stream"),
        metadata().topics().clone(),
        metadata().events().clone(),
        metadata().modes().clone(),
        metadata().reconnect(),
    );
    assert_eq!(
        service
            .connect(
                &context,
                issued.descriptor(),
                issued.transport_credential(),
                &wrong_stream,
                UnixMillis::new(1_300),
            )
            .await
            .expect_err("stream substitution must fail")
            .kind(),
        SubscriptionErrorKind::ScopeMismatch
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
                    &metadata(),
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
                &metadata(),
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
