//! Independently versioned asynchronous envelope and sequence-authority tests.

use std::collections::BTreeMap;
use std::fs;
use std::future::Future;
use std::num::{NonZeroU8, NonZeroU16};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use crate::SUPPORTED_PROTOCOL_VERSIONS;
use crate::action::{ActionArgumentSchema, AuthorizationRequirement, TransactionPolicy};
use crate::async_updates::{
    AsyncCodecLimits, AsyncContinuityAuthorityPort, AsyncContinuityRequest, AsyncDispatchError,
    AsyncEnvelope, AsyncEnvelopeContext, AsyncEnvelopeDispatchPort, AsyncEnvelopeErrorKind,
    AsyncMembershipRegistryPort, AsyncMembershipRequest, AsyncMembershipValidation, AsyncPayload,
    AuthoritativeStreamPosition, AuthorizedSubscription, BaselineDisposition,
    BoundedEventContracts, BoundedEventNames, BoundedPresentationSignalContracts, BoundedTargets,
    BoundedTopics, BrowserPayloadSchema, CapabilityVersion, CompletionReason,
    CurrentSubscriptionRegistration, EventCyclePolicy, EventOrder, EventSource, EventTarget,
    MAX_ASYNC_ENVELOPE_ENTRIES, PollFallbackPolicy, PollInitialBehavior, PollVisibilityPolicy,
    PresentationSignalContract, PresentationSignalSchema, ReconnectPolicy, RegisteredBrowserEvent,
    RegisteredPresentationSignal, ReplayDispatchError, ReplayDispatchOutcome,
    ResolvedAsyncDelivery, SUPPORTED_ASYNC_PROTOCOL_VERSIONS, SequenceDisposition,
    SequenceErrorKind, SequenceMachine, SequenceState, StreamEpoch, StreamErrorCode, StreamName,
    StreamPosition, StreamSequence, SubscriptionAuthorizationDecision,
    SubscriptionAuthorizationPort, SubscriptionAuthorizationRequest, SubscriptionBaselineRequest,
    SubscriptionContinuityPort, SubscriptionCredentialPort, SubscriptionCredentialRequest,
    SubscriptionCredentialRotationOutcome, SubscriptionCredentialRotationRequest,
    SubscriptionError, SubscriptionEventContract, SubscriptionId, SubscriptionIssueRequest,
    SubscriptionMetadata, SubscriptionMode, SubscriptionModes, SubscriptionRegistryPort,
    SubscriptionRegistryRequest, SubscriptionService, TopicName, TransportCredential,
    TrustedMountParameters, decode_async_envelope, encode_async_envelope,
};
use crate::canonical::CanonicalValue;
use crate::conformance::{FixtureVersion, fixture_directory};
use crate::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use crate::host::{
    CheckDisposition, CheckFact, CheckKind, HostCapabilities, HostCheckFacts,
    LiveRequestContextCandidate, LiveRequestContextValidator, MountCatalogBuilder,
    MountCatalogEntry, MountScopeRequirements, MountSelection, PrincipalFingerprint,
    ScopeRequirement, SessionFingerprint, TenantFingerprint, TrustedLiveRequestContext,
};
use crate::identity::{
    BrowserOperationName, BuildId, ComponentName, IslandSlot, KeyId, ModelField, RouteIdentity,
    ScopeFingerprint, SignalScopeIdentity, UnixMillis, ViewName,
};
use crate::metadata::{
    ActionMetadata, ComponentMetadata, ContractVersions, EventMetadata, EventPayloadMetadata,
    FieldMetadata,
};
use crate::registry::{ComponentDescriptor, ComponentRegistryBuilder};
use crate::snapshot::state::{FieldCategory, FieldSpec, StateCodec, StateSchema};
use crate::snapshot::{ComponentContract, ExpectedSeedV1, SnapshotSchemaSet};
use crate::validation::ValidationSelection;
use proptest::prelude::*;
use serde_json::Value;

struct OrdersUpdated;

impl EventPayloadMetadata for OrdersUpdated {
    const NAME: &'static str = "orders.updated";
    const VERSION: u16 = 1;
    const PAYLOAD_CONTRACT: &'static str = "orders.updated.payload";
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
}

struct OrdersUpdatedV2;

impl EventPayloadMetadata for OrdersUpdatedV2 {
    const NAME: &'static str = "orders.updated";
    const VERSION: u16 = 2;
    const PAYLOAD_CONTRACT: &'static str = "orders.updated.payload";
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
}

fn event_contract() -> SubscriptionEventContract {
    let metadata = EventMetadata::from_payload_with_contract::<OrdersUpdated>(
        EventSource::Stream,
        BoundedTargets::new(vec![EventTarget::SelfIsland, EventTarget::Document])
            .expect("bounded targets"),
        EventOrder::PerSourceSequence,
        EventCyclePolicy::MaximumHops(NonZeroU8::new(4).expect("nonzero hops")),
        4,
    )
    .expect("registered event metadata");
    SubscriptionEventContract::from_registered(&metadata).expect("event contract")
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
        Box::pin(async { TransportCredential::from_host_authority_bearer(vec![0x51; 32]) })
    }

    fn consume_and_rotate<'a>(
        &'a self,
        _request: SubscriptionCredentialRotationRequest<'a>,
    ) -> TestFuture<'a, SubscriptionCredentialRotationOutcome> {
        Box::pin(async {
            match TransportCredential::from_host_authority_bearer(vec![0x52; 32]) {
                Ok(credential) => SubscriptionCredentialRotationOutcome::Rotated(credential),
                Err(_) => SubscriptionCredentialRotationOutcome::Failed,
            }
        })
    }
}

fn bytes<const LENGTH: usize>(start: u8) -> [u8; LENGTH] {
    std::array::from_fn(|index| start.wrapping_add(index as u8))
}

fn route(start: u8) -> RouteIdentity {
    RouteIdentity::from_bytes(&bytes::<32>(start)).expect("route identity")
}

fn fixture_host_scope() -> crate::host::HostScopeFacts {
    crate::host::HostScopeFacts::new(
        ScopeFingerprint::from_bytes(&bytes::<32>(0x40)).expect("scope identity"),
        Some(SessionFingerprint::from_bytes(&bytes::<32>(0x41)).expect("session identity")),
        Some(PrincipalFingerprint::from_bytes(&bytes::<32>(0x42)).expect("principal identity")),
        Some(TenantFingerprint::from_bytes(&bytes::<32>(0x43)).expect("tenant identity")),
    )
}

fn schema_set() -> SnapshotSchemaSet {
    SnapshotSchemaSet::new(
        StateSchema::new(
            1,
            vec![
                FieldSpec::new("serial", StateCodec::Json, FieldCategory::State, true)
                    .expect("state field"),
            ],
        )
        .expect("state schema"),
        StateSchema::new(1, vec![]).expect("memo schema"),
        StateSchema::new(1, vec![]).expect("mount schema"),
    )
    .expect("snapshot schema set")
}

fn base_metadata() -> &'static ComponentMetadata {
    static METADATA: OnceLock<ComponentMetadata> = OnceLock::new();
    METADATA.get_or_init(|| {
        ComponentMetadata::new(
            ComponentName::parse("tests.trace").expect("component identity"),
            ViewName::parse("tests/trace.html").expect("view identity"),
            ContractVersions::new(1, 1, 1, 1, 1).expect("versions"),
            vec![FieldMetadata::new(
                ModelField::parse("serial").expect("field identity"),
                FieldCategory::State,
                StateCodec::Json,
                true,
            )],
            vec![
                ActionMetadata::new_with_contract(
                    crate::identity::ActionName::parse("execute").expect("action identity"),
                    1,
                    ActionArgumentSchema::empty(),
                    AuthorizationRequirement::Current,
                    ValidationSelection::ComponentAndArguments,
                    TransactionPolicy::None,
                )
                .expect("action metadata"),
            ],
        )
        .expect("component metadata")
    })
}

fn subscription_component_metadata() -> ComponentMetadata {
    let base = base_metadata();
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
        SubscriptionModes::new(vec![SubscriptionMode::ServerSentEvents]).expect("mode"),
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
        KeyId::parse("async-envelope-key").expect("key ID"),
        RootKey::new(vec![0x39; 32]).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(20_000),
        UnixMillis::new(40_000),
    )
    .expect("key record");
    SnapshotKeyRing::new(key, Vec::new()).expect("key ring")
}

fn subscription_trusted_context(ports: Arc<SubscriptionFixturePorts>) -> TrustedLiveRequestContext {
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
    let route = route(0x64);
    let slot = IslandSlot::parse("async-envelope").expect("slot");
    let catalog = MountCatalogBuilder::new()
        .register(
            &registry,
            MountCatalogEntry::new(
                ExpectedSeedV1::new(
                    contract,
                    BuildId::parse("build-async-envelope").expect("build"),
                    route.clone(),
                    slot.clone(),
                    schema_set(),
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
    let scope = fixture_host_scope();
    let capabilities = HostCapabilities::bound_to(scope.clone())
        .with_subscription_registry(ports.clone())
        .with_subscription_continuity(ports.clone())
        .with_subscription_authorization(ports.clone())
        .with_subscription_credentials(ports.clone());
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

fn authorized_subscription() -> &'static AuthorizedSubscription {
    static AUTHORIZED: OnceLock<AuthorizedSubscription> = OnceLock::new();
    AUTHORIZED.get_or_init(|| build_authorized_subscription(position(4, 40)))
}

fn build_authorized_subscription(baseline: StreamPosition) -> AuthorizedSubscription {
    let ports = Arc::new(SubscriptionFixturePorts {
        component: subscription_component_metadata(),
        parameters: TrustedMountParameters::new(Vec::new()).expect("mount parameters"),
        baseline,
    });
    let context = subscription_trusted_context(ports);
    let service = SubscriptionService::new(subscription_key_ring());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    runtime.block_on(async {
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
        service
            .connect(
                &context,
                issued.descriptor(),
                issued.transport_credential(),
                UnixMillis::new(1_100),
            )
            .await
            .expect("authorized subscription")
    })
}

fn subscription_id() -> SubscriptionId {
    SubscriptionId::from_bytes(b"subscription-001").expect("subscription id")
}

fn stream() -> StreamName {
    StreamName::parse("orders").expect("stream")
}

struct TestMembershipRegistry {
    active: bool,
    subscription: SubscriptionId,
    stream: StreamName,
    events: BoundedEventContracts,
    presentation_signals: BoundedPresentationSignalContracts,
}

impl AsyncMembershipRegistryPort for TestMembershipRegistry {
    fn validate_current(
        &self,
        request: AsyncMembershipRequest<'_>,
        validation: &mut AsyncMembershipValidation<'_>,
    ) {
        if self.active && request.subscription() == &self.subscription {
            validation.accept_current(&self.stream, &self.events, &self.presentation_signals);
        }
    }
}

fn membership_registry_for(authorized: &AuthorizedSubscription) -> TestMembershipRegistry {
    TestMembershipRegistry {
        active: true,
        subscription: subscription_id(),
        stream: stream(),
        events: authorized.verified().claims().events().clone(),
        presentation_signals: BoundedPresentationSignalContracts::new(vec![
            PresentationSignalContract::new(
                SignalScopeIdentity::parse("root-scope").expect("signal scope"),
                BrowserOperationName::parse("completion_percent").expect("signal name"),
                PresentationSignalSchema::U64,
            ),
            PresentationSignalContract::new(
                SignalScopeIdentity::parse("root-scope").expect("signal scope"),
                BrowserOperationName::parse("signed_count").expect("signal name"),
                PresentationSignalSchema::I64,
            ),
        ])
        .expect("signals"),
    }
}

fn membership_registry() -> TestMembershipRegistry {
    membership_registry_for(authorized_subscription())
}

fn context() -> AsyncEnvelopeContext {
    AsyncEnvelopeContext::from_authorized(
        authorized_subscription(),
        subscription_id(),
        &membership_registry(),
    )
    .expect("active current membership")
}

fn context_at(baseline: StreamPosition) -> AsyncEnvelopeContext {
    let authorized = build_authorized_subscription(baseline);
    AsyncEnvelopeContext::from_authorized(
        &authorized,
        subscription_id(),
        &membership_registry_for(&authorized),
    )
    .expect("active current membership at baseline")
}

fn limits() -> AsyncCodecLimits {
    AsyncCodecLimits::v1()
}

fn position(epoch: u64, sequence: u64) -> StreamPosition {
    StreamPosition::new(StreamEpoch::new(epoch), StreamSequence::new(sequence))
}

fn fixture_position(value: &Value) -> StreamPosition {
    StreamPosition::new(
        StreamEpoch::new(
            value["epoch"]
                .as_str()
                .expect("fixture epoch")
                .parse()
                .expect("decimal epoch"),
        ),
        StreamSequence::new(
            value["sequence"]
                .as_str()
                .expect("fixture sequence")
                .parse()
                .expect("decimal sequence"),
        ),
    )
}

fn wire(payload: &str, epoch: u64, sequence: u64) -> Vec<u8> {
    wire_for(&subscription_id(), &stream(), payload, epoch, sequence)
}

fn wire_for(
    subscription: &SubscriptionId,
    stream: &StreamName,
    payload: &str,
    epoch: u64,
    sequence: u64,
) -> Vec<u8> {
    format!(
        "{{\"payload\":{payload},\"position\":{{\"epoch\":\"{epoch}\",\"sequence\":\"{sequence}\"}},\"protocol_version\":1,\"stream\":\"{}\",\"subscription\":\"{}\"}}",
        stream.as_str(),
        subscription.to_base64url(),
    )
    .into_bytes()
}

fn decode(payload: &str, epoch: u64, sequence: u64) -> crate::async_updates::AsyncEnvelope {
    decode_async_envelope(&wire(payload, epoch, sequence), &limits(), &context())
        .expect("valid async envelope")
}

#[test]
fn async_protocol_is_independent_from_live_action_and_morph_versions() {
    assert_eq!(SUPPORTED_ASYNC_PROTOCOL_VERSIONS, &[1]);
    assert_eq!(SUPPORTED_PROTOCOL_VERSIONS, &[1, 2]);
}

#[test]
fn sequence_machine_starts_only_from_the_signed_authorized_baseline() {
    let context = context();
    let signed_baseline = position(4, 40);

    assert_eq!(context.authoritative_baseline(), signed_baseline);
    assert_eq!(SequenceMachine::new(&context).current(), signed_baseline);
}

#[test]
fn every_closed_payload_kind_decodes_and_round_trips_canonically() {
    let payloads = [
        "{\"kind\":\"refresh\",\"name\":\"refresh\"}",
        "{\"event\":\"orders.updated\",\"kind\":\"browser_event\",\"payload\":{\"count\":1},\"schema_version\":1,\"target\":\"self\"}",
        "{\"kind\":\"presentation_signal\",\"name\":\"completion_percent\",\"scope\":\"root-scope\",\"value\":50}",
        "{\"kind\":\"heartbeat\"}",
        "{\"kind\":\"complete\",\"reason\":\"server_shutdown\"}",
        "{\"code\":\"authorization_lost\",\"kind\":\"error\"}",
    ];

    for (offset, payload) in payloads.into_iter().enumerate() {
        let encoded = wire(payload, 4, 41 + offset as u64);
        let envelope = decode_async_envelope(&encoded, &limits(), &context()).expect("decode");
        assert_eq!(envelope.protocol_version(), 1);
        assert_eq!(envelope.subscription(), &subscription_id());
        assert_eq!(envelope.stream(), &stream());
        assert_eq!(
            envelope.position(),
            StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(41 + offset as u64))
        );
        assert_eq!(
            encode_async_envelope(&envelope, &limits()).expect("encode"),
            encoded
        );
    }
}

#[test]
fn server_authored_envelopes_require_the_current_registered_context() {
    let context = context();
    let event = RegisteredBrowserEvent::new(
        &context,
        BrowserOperationName::parse("orders.updated").expect("event name"),
        1,
        EventTarget::SelfIsland,
        CanonicalValue::Null,
    )
    .expect("registered event");
    let envelope = AsyncEnvelope::new(
        &context,
        StreamPosition::new(StreamEpoch::new(8), StreamSequence::new(21)),
        AsyncPayload::BrowserEvent(event),
    )
    .expect("server-authored envelope");
    let encoded = encode_async_envelope(&envelope, &limits()).expect("encode");
    assert_eq!(
        decode_async_envelope(&encoded, &limits(), &context).expect("decode"),
        envelope
    );

    let signal = RegisteredPresentationSignal::new(
        &context,
        SignalScopeIdentity::parse("root-scope").expect("signal scope"),
        BrowserOperationName::parse("completion_percent").expect("signal name"),
        CanonicalValue::String("wrong schema".to_owned()),
    )
    .expect_err("signal schema must match current registration");
    assert_eq!(signal.kind(), AsyncEnvelopeErrorKind::UnregisteredPayload);

    let oversized = RegisteredBrowserEvent::new(
        &context,
        BrowserOperationName::parse("orders.updated").expect("event name"),
        1,
        EventTarget::SelfIsland,
        CanonicalValue::String("x".repeat(32_769)),
    )
    .expect_err("server-authored payloads must be bounded before envelope construction");
    assert_eq!(oversized.kind(), AsyncEnvelopeErrorKind::StringTooLong);
}

#[test]
fn server_authored_payloads_share_the_lossless_canonical_codec_contract() {
    const MAX_BROWSER_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;
    let context = context();
    let safe = MAX_BROWSER_SAFE_INTEGER as f64;

    for (name, value) in [
        ("signed_count", -safe),
        ("signed_count", safe),
        ("completion_percent", 0.0),
        ("completion_percent", safe),
    ] {
        let signal = RegisteredPresentationSignal::new(
            &context,
            SignalScopeIdentity::parse("root-scope").expect("signal scope"),
            BrowserOperationName::parse(name).expect("registered signal"),
            CanonicalValue::number(value).expect("finite canonical number"),
        )
        .expect("lossless server-authored value");
        let envelope = AsyncEnvelope::new(
            &context,
            position(4, 41),
            AsyncPayload::PresentationSignal(signal),
        )
        .expect("server envelope");
        let encoded = encode_async_envelope(&envelope, &limits()).expect("canonical encode");
        assert_eq!(
            decode_async_envelope(&encoded, &limits(), &context).expect("canonical decode"),
            envelope,
        );
    }

    for (name, value) in [
        ("signed_count", -(safe + 1.0)),
        ("signed_count", safe + 1.0),
        ("signed_count", i64::MIN as f64),
        ("signed_count", i64::MAX as f64),
        ("completion_percent", -1.0),
        ("completion_percent", safe + 1.0),
        ("completion_percent", u64::MAX as f64),
    ] {
        assert_eq!(
            RegisteredPresentationSignal::new(
                &context,
                SignalScopeIdentity::parse("root-scope").expect("signal scope"),
                BrowserOperationName::parse(name).expect("registered signal"),
                CanonicalValue::number(value).expect("finite canonical number"),
            )
            .expect_err("integer schemas reject values outside browser-safe exact integers")
            .kind(),
            AsyncEnvelopeErrorKind::UnregisteredPayload,
        );
    }

    let normalized = RegisteredPresentationSignal::new(
        &context,
        SignalScopeIdentity::parse("root-scope").expect("signal scope"),
        BrowserOperationName::parse("completion_percent").expect("registered signal"),
        CanonicalValue::number(-0.0).expect("negative zero is finite"),
    )
    .expect("canonical negative zero represents unsigned zero");
    let CanonicalValue::Number(normalized) = normalized.value() else {
        panic!("normalized numeric signal")
    };
    assert!(!normalized.get().is_sign_negative());

    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            CanonicalValue::number(value).is_err(),
            "non-finite values cannot reach a server-authored payload constructor",
        );
    }

    let huge_integral_event = RegisteredBrowserEvent::new(
        &context,
        BrowserOperationName::parse("orders.updated").expect("event name"),
        1,
        EventTarget::SelfIsland,
        CanonicalValue::number(safe + 1.0).expect("finite canonical number"),
    )
    .expect_err("constructor must apply the same canonical parser as decode");
    assert_eq!(
        huge_integral_event.kind(),
        AsyncEnvelopeErrorKind::InvalidEnvelope,
    );
}

#[test]
fn decoded_payloads_are_closed_registered_values() {
    let refresh = decode("{\"kind\":\"refresh\",\"name\":\"refresh\"}", 1, 1);
    assert!(matches!(refresh.payload(), AsyncPayload::Refresh(_)));

    let event = decode(
        "{\"event\":\"orders.updated\",\"kind\":\"browser_event\",\"payload\":{\"count\":1},\"schema_version\":1,\"target\":\"self\"}",
        1,
        2,
    );
    let AsyncPayload::BrowserEvent(event) = event.payload() else {
        panic!("browser event payload")
    };
    assert_eq!(event.name().as_str(), OrdersUpdated::NAME);
    assert_eq!(event.schema_version(), OrdersUpdated::VERSION);
    assert_eq!(event.target(), &EventTarget::SelfIsland);
    assert!(matches!(event.payload(), CanonicalValue::Object(_)));

    let signal = decode(
        "{\"kind\":\"presentation_signal\",\"name\":\"completion_percent\",\"scope\":\"root-scope\",\"value\":50}",
        1,
        3,
    );
    let AsyncPayload::PresentationSignal(signal) = signal.payload() else {
        panic!("presentation signal payload")
    };
    assert_eq!(signal.name().as_str(), "completion_percent");

    assert!(matches!(
        decode(
            "{\"kind\":\"complete\",\"reason\":\"server_shutdown\"}",
            1,
            4
        )
        .payload(),
        AsyncPayload::Complete(CompletionReason::ServerShutdown)
    ));
    assert!(matches!(
        decode("{\"code\":\"authorization_lost\",\"kind\":\"error\"}", 1, 5).payload(),
        AsyncPayload::Error(StreamErrorCode::AuthorizationLost)
    ));
}

#[test]
fn envelope_debug_output_never_exposes_raw_payload_values() {
    const SENTINEL: &str = "async_payload_secret_sentinel";
    let envelope = decode(
        &format!(
            "{{\"event\":\"orders.updated\",\"kind\":\"browser_event\",\"payload\":{{\"secret\":\"{SENTINEL}\"}},\"schema_version\":1,\"target\":\"self\"}}"
        ),
        1,
        1,
    );

    assert!(!format!("{envelope:?}").contains(SENTINEL));
    assert!(!format!("{:?}", envelope.payload()).contains(SENTINEL));
}

#[test]
fn unknown_major_duplicate_unknown_and_noncanonical_fields_fail_closed() {
    let id = subscription_id().to_base64url();
    let cases = [
        (
            format!(
                "{{\"payload\":{{\"kind\":\"heartbeat\"}},\"position\":{{\"epoch\":\"4\",\"sequence\":\"41\"}},\"protocol_version\":2,\"stream\":\"orders\",\"subscription\":\"{id}\"}}"
            ),
            AsyncEnvelopeErrorKind::UnsupportedProtocol,
        ),
        (
            format!(
                "{{\"payload\":{{\"kind\":\"heartbeat\"}},\"position\":{{\"epoch\":\"4\",\"sequence\":\"41\"}},\"protocol_version\":1,\"protocol_version\":1,\"stream\":\"orders\",\"subscription\":\"{id}\"}}"
            ),
            AsyncEnvelopeErrorKind::DuplicateField,
        ),
        (
            format!(
                "{{\"payload\":{{\"kind\":\"heartbeat\"}},\"position\":{{\"epoch\":\"4\",\"sequence\":\"41\"}},\"protocol_version\":1,\"stream\":\"orders\",\"subscription\":\"{id}\",\"unexpected\":true}}"
            ),
            AsyncEnvelopeErrorKind::InvalidEnvelope,
        ),
        (
            format!(
                "{{ \"payload\":{{\"kind\":\"heartbeat\"}},\"position\":{{\"epoch\":\"4\",\"sequence\":\"41\"}},\"protocol_version\":1,\"stream\":\"orders\",\"subscription\":\"{id}\"}}"
            ),
            AsyncEnvelopeErrorKind::NonCanonical,
        ),
    ];

    for (encoded, expected) in cases {
        assert_eq!(
            decode_async_envelope(encoded.as_bytes(), &limits(), &context())
                .expect_err("hostile envelope")
                .kind(),
            expected,
        );
    }
}

#[test]
fn nested_duplicate_fields_and_semantic_key_misordering_fail_closed() {
    let duplicate_cases = [
        "{\"kind\":\"heartbeat\",\"kind\":\"heartbeat\"}",
        "{\"kind\":\"refresh\",\"name\":\"refresh\",\"name\":\"refresh\"}",
        "{\"event\":\"orders.updated\",\"kind\":\"browser_event\",\"payload\":null,\"schema_version\":1,\"schema_version\":1,\"target\":\"self\"}",
        "{\"kind\":\"presentation_signal\",\"name\":\"completion_percent\",\"name\":\"completion_percent\",\"scope\":\"root-scope\",\"value\":1}",
        "{\"kind\":\"complete\",\"reason\":\"server_shutdown\",\"reason\":\"server_shutdown\"}",
        "{\"code\":\"authorization_lost\",\"code\":\"authorization_lost\",\"kind\":\"error\"}",
        "{\"event\":\"orders.updated\",\"kind\":\"browser_event\",\"payload\":{\"count\":1,\"count\":2},\"schema_version\":1,\"target\":\"self\"}",
        "{\"kind\":\"presentation_signal\",\"name\":\"completion_percent\",\"scope\":\"root-scope\",\"value\":{\"count\":1,\"count\":2}}",
    ];
    for payload in duplicate_cases {
        assert_eq!(
            decode_async_envelope(&wire(payload, 1, 1), &limits(), &context())
                .expect_err("nested duplicate key")
                .kind(),
            AsyncEnvelopeErrorKind::DuplicateField
        );
    }

    let id = subscription_id().to_base64url();
    let duplicate_position = format!(
        "{{\"payload\":{{\"kind\":\"heartbeat\"}},\"position\":{{\"epoch\":\"1\",\"epoch\":\"1\",\"sequence\":\"1\"}},\"protocol_version\":1,\"stream\":\"orders\",\"subscription\":\"{id}\"}}"
    );
    assert_eq!(
        decode_async_envelope(duplicate_position.as_bytes(), &limits(), &context())
            .expect_err("duplicate position key")
            .kind(),
        AsyncEnvelopeErrorKind::DuplicateField
    );

    let noncanonical = [
        format!(
            "{{\"payload\":{{\"kind\":\"heartbeat\"}},\"position\":{{\"sequence\":\"1\",\"epoch\":\"1\"}},\"protocol_version\":1,\"stream\":\"orders\",\"subscription\":\"{id}\"}}"
        ),
        format!(
            "{{\"payload\":{{\"kind\":\"browser_event\",\"event\":\"orders.updated\",\"payload\":{{\"z\":1,\"a\":2}},\"schema_version\":1,\"target\":\"self\"}},\"position\":{{\"epoch\":\"1\",\"sequence\":\"1\"}},\"protocol_version\":1,\"stream\":\"orders\",\"subscription\":\"{id}\"}}"
        ),
        format!(
            "{{\"payload\":{{\"kind\":\"presentation_signal\",\"name\":\"completion_percent\",\"scope\":\"root-scope\",\"value\":{{\"z\":1,\"a\":2}}}},\"position\":{{\"epoch\":\"1\",\"sequence\":\"1\"}},\"protocol_version\":1,\"stream\":\"orders\",\"subscription\":\"{id}\"}}"
        ),
    ];
    for encoded in noncanonical {
        assert_eq!(
            decode_async_envelope(encoded.as_bytes(), &limits(), &context())
                .expect_err("nested semantic key order must be canonical")
                .kind(),
            AsyncEnvelopeErrorKind::NonCanonical
        );
    }
}

#[test]
fn subscription_id_rejects_encoded_length_and_shape_before_decode() {
    assert_eq!(SubscriptionId::MIN_ENCODED_LEN, 22);
    assert_eq!(SubscriptionId::MAX_ENCODED_LEN, 43);
    assert!(SubscriptionId::parse(&"A".repeat(SubscriptionId::MIN_ENCODED_LEN)).is_ok());
    assert!(SubscriptionId::parse(&"A".repeat(SubscriptionId::MAX_ENCODED_LEN)).is_ok());
    assert!(SubscriptionId::from_bytes(&[0x5a; 32]).is_ok());
    for invalid in [
        "A".repeat(SubscriptionId::MIN_ENCODED_LEN - 1),
        "A".repeat(SubscriptionId::MAX_ENCODED_LEN + 1),
        format!("{}=", "A".repeat(SubscriptionId::MIN_ENCODED_LEN - 1)),
        format!("{}+", "A".repeat(SubscriptionId::MIN_ENCODED_LEN - 1)),
        format!("{}/", "A".repeat(SubscriptionId::MIN_ENCODED_LEN - 1)),
    ] {
        assert_eq!(
            SubscriptionId::parse(&invalid)
                .expect_err("invalid encoded subscription ID")
                .kind(),
            AsyncEnvelopeErrorKind::InvalidEnvelope
        );
    }
}

#[test]
fn unsupported_or_malformed_operations_cannot_become_dispatch_authority() {
    let payloads = [
        "{\"html\":\"<p>unsafe</p>\",\"kind\":\"html\"}",
        "{\"action\":\"delete\",\"kind\":\"action\"}",
        "{\"kind\":\"effect\",\"name\":\"eval\"}",
        "{\"kind\":\"snapshot\",\"value\":\"secret\"}",
    ];
    for payload in payloads {
        assert_eq!(
            decode_async_envelope(&wire(payload, 1, 1), &limits(), &context())
                .expect_err("unsupported operation")
                .kind(),
            AsyncEnvelopeErrorKind::UnsupportedPayload,
        );
    }

    for payload in [
        "{\"extra\":true,\"kind\":\"heartbeat\"}",
        "{\"kind\":\"refresh\",\"name\":\"save\"}",
        "{\"kind\":\"complete\",\"reason\":\"run_action\"}",
        "{\"code\":\"arbitrary\",\"kind\":\"error\"}",
    ] {
        assert_eq!(
            decode_async_envelope(&wire(payload, 1, 1), &limits(), &context())
                .expect_err("malformed operation")
                .kind(),
            AsyncEnvelopeErrorKind::InvalidPayload,
        );
    }
}

#[test]
fn event_and_signal_payloads_require_current_registered_contracts() {
    let cases = [
        "{\"event\":\"orders.deleted\",\"kind\":\"browser_event\",\"payload\":{},\"schema_version\":1,\"target\":\"self\"}",
        "{\"event\":\"orders.updated\",\"kind\":\"browser_event\",\"payload\":{},\"schema_version\":2,\"target\":\"self\"}",
        "{\"event\":\"orders.updated\",\"kind\":\"browser_event\",\"payload\":{},\"schema_version\":1,\"target\":\"parent\"}",
        "{\"kind\":\"presentation_signal\",\"name\":\"unknown_signal\",\"scope\":\"root-scope\",\"value\":50}",
        "{\"kind\":\"presentation_signal\",\"name\":\"completion_percent\",\"scope\":\"foreign-scope\",\"value\":50}",
        "{\"kind\":\"presentation_signal\",\"name\":\"completion_percent\",\"scope\":\"root-scope\",\"value\":\"fifty\"}",
    ];
    for payload in cases {
        assert_eq!(
            decode_async_envelope(&wire(payload, 1, 1), &limits(), &context())
                .expect_err("unregistered payload")
                .kind(),
            AsyncEnvelopeErrorKind::UnregisteredPayload,
        );
    }
}

#[test]
fn byte_depth_entry_string_and_payload_limits_are_enforced() {
    let tiny = AsyncCodecLimits::new(256, 4, 16, 32, 64).expect("tiny limits");
    assert_eq!(
        decode_async_envelope(&vec![b'x'; 257], &tiny, &context())
            .expect_err("raw byte limit")
            .kind(),
        AsyncEnvelopeErrorKind::TooLarge,
    );

    let deeply_nested = wire(
        "{\"event\":\"orders.updated\",\"kind\":\"browser_event\",\"payload\":[[[[[null]]]]],\"schema_version\":1,\"target\":\"self\"}",
        1,
        1,
    );
    assert_eq!(
        decode_async_envelope(&deeply_nested, &tiny, &context())
            .expect_err("depth limit")
            .kind(),
        AsyncEnvelopeErrorKind::TooDeep,
    );

    let large_payload = format!(
        "{{\"event\":\"orders.updated\",\"kind\":\"browser_event\",\"payload\":\"{}\",\"schema_version\":1,\"target\":\"self\"}}",
        "x".repeat(65),
    );
    let payload_limited = AsyncCodecLimits::new(1_024, 8, 64, 128, 64).expect("limits");
    assert_eq!(
        decode_async_envelope(&wire(&large_payload, 1, 1), &payload_limited, &context())
            .expect_err("payload byte limit")
            .kind(),
        AsyncEnvelopeErrorKind::PayloadTooLarge,
    );

    let long_string = AsyncCodecLimits::new(1_024, 8, 64, 8, 512).expect("limits");
    assert_eq!(
        decode_async_envelope(
            &wire("{\"kind\":\"heartbeat\"}", 1, 1),
            &long_string,
            &context()
        )
        .expect_err("string limit")
        .kind(),
        AsyncEnvelopeErrorKind::StringTooLong,
    );

    let duplicate_with_oversized_second_payload = format!(
        "{{\"payload\":{{\"kind\":\"heartbeat\"}},\"payload\":\"{}\",\"position\":{{\"epoch\":\"1\",\"sequence\":\"1\"}},\"protocol_version\":1,\"stream\":\"orders\",\"subscription\":\"{}\"}}",
        "x".repeat(65),
        subscription_id().to_base64url(),
    );
    assert_eq!(
        decode_async_envelope(
            duplicate_with_oversized_second_payload.as_bytes(),
            &payload_limited,
            &context(),
        )
        .expect_err("payload is bounded before duplicate-field parsing")
        .kind(),
        AsyncEnvelopeErrorKind::PayloadTooLarge,
    );
}

#[test]
fn production_entry_and_replay_limits_accept_exactly_1024_and_reject_1025() {
    const ENVELOPE_ENTRY_OVERHEAD: usize = 12;
    assert_eq!(MAX_ASYNC_ENVELOPE_ENTRIES, 1_024);
    let exact_nested_entries = MAX_ASYNC_ENVELOPE_ENTRIES - ENVELOPE_ENTRY_OVERHEAD;

    let exact_array = (0..exact_nested_entries)
        .map(|_| "0")
        .collect::<Vec<_>>()
        .join(",");
    let over_array = format!("{exact_array},0");
    for (nested, expected) in [
        (format!("[{exact_array}]"), Ok(())),
        (
            format!("[{over_array}]"),
            Err(AsyncEnvelopeErrorKind::TooManyEntries),
        ),
    ] {
        let payload = format!(
            "{{\"event\":\"orders.updated\",\"kind\":\"browser_event\",\"payload\":{nested},\"schema_version\":1,\"target\":\"self\"}}"
        );
        let result = decode_async_envelope(&wire(&payload, 4, 41), &limits(), &context());
        assert_eq!(result.map(|_| ()).map_err(|error| error.kind()), expected);
    }

    let exact_object = (0..exact_nested_entries)
        .map(|index| format!("\"k{index:04}\":0"))
        .collect::<Vec<_>>()
        .join(",");
    let over_object = format!("{exact_object},\"z\":0");
    for (nested, expected) in [
        (format!("{{{exact_object}}}"), Ok(())),
        (
            format!("{{{over_object}}}"),
            Err(AsyncEnvelopeErrorKind::TooManyEntries),
        ),
    ] {
        let payload = format!(
            "{{\"event\":\"orders.updated\",\"kind\":\"browser_event\",\"payload\":{nested},\"schema_version\":1,\"target\":\"self\"}}"
        );
        let result = decode_async_envelope(&wire(&payload, 4, 41), &limits(), &context());
        assert_eq!(result.map(|_| ()).map_err(|error| error.kind()), expected);
    }

    for replay_len in [1_024_usize, 1_025] {
        let context = context();
        let registry = membership_registry();
        let mut machine = SequenceMachine::new(&context);
        let high_water = 40 + replay_len as u64;
        let gap = decode("{\"kind\":\"heartbeat\"}", 4, high_water);
        let mut dispatcher = RecordingDispatcher::default();
        assert!(matches!(
            machine
                .dispatch(
                    admit(&context, &gap, &registry),
                    UnixMillis::new(1_200),
                    &mut dispatcher,
                )
                .expect("fresh gap classification"),
            SequenceDisposition::Degraded(_),
        ));
        let transcript = (41..=high_water)
            .map(|sequence| decode("{\"kind\":\"heartbeat\"}", 4, sequence))
            .collect::<Vec<_>>();
        let guards = transcript
            .iter()
            .map(|envelope| admit(&context, envelope, &registry))
            .collect::<Vec<_>>();
        let result = machine.recover_from_replay(guards, UnixMillis::new(1_200), &mut dispatcher);
        if replay_len == 1_024 {
            let outcome = result.expect("exact replay limit");
            assert_eq!(outcome.applied(), 1_024);
            assert_eq!(outcome.current(), position(4, high_water));
        } else {
            let error = result.expect_err("first replay beyond production limit");
            assert_eq!(error.kind(), SequenceErrorKind::InvalidReplayTranscript);
            assert_eq!(error.applied(), 0);
            assert_eq!(dispatcher.attempts.len(), 0);
            assert_eq!(machine.current(), position(4, 40));
        }
    }
}

#[test]
fn membership_and_stream_binding_are_validated_before_sequence_observation() {
    let context = context();
    let mut machine = SequenceMachine::new(&context);
    let current = machine.current();
    let other_id = SubscriptionId::from_bytes(b"subscription-002").expect("other id");
    let registry = membership_registry();
    assert_eq!(
        AsyncEnvelopeContext::from_authorized(authorized_subscription(), other_id, &registry,)
            .expect_err("inactive membership cannot create decode authority")
            .kind(),
        AsyncEnvelopeErrorKind::SubscriptionMismatch,
    );
    assert_eq!(machine.current(), current);

    let mut wrong_stream = membership_registry();
    wrong_stream.stream = StreamName::parse("other").expect("other stream");
    assert_eq!(
        AsyncEnvelopeContext::from_authorized(
            authorized_subscription(),
            subscription_id(),
            &wrong_stream,
        )
        .expect_err("cross-stream registry cannot create decode authority")
        .kind(),
        AsyncEnvelopeErrorKind::StreamMismatch,
    );
    assert_eq!(machine.current(), current);

    let mut stale_registry = membership_registry();
    stale_registry.events = BoundedEventContracts::new(vec![
        SubscriptionEventContract::from_registered(
            &EventMetadata::from_payload_with_contract::<OrdersUpdatedV2>(
                EventSource::Stream,
                BoundedTargets::new(vec![EventTarget::SelfIsland]).expect("target"),
                EventOrder::PerSourceSequence,
                EventCyclePolicy::MaximumHops(NonZeroU8::new(4).expect("hops")),
                4,
            )
            .expect("stale event metadata"),
        )
        .expect("stale event contract"),
    ])
    .expect("stale registry events");
    assert_eq!(
        AsyncEnvelopeContext::from_authorized(
            authorized_subscription(),
            subscription_id(),
            &stale_registry,
        )
        .expect_err("stale registry cannot create decode authority")
        .kind(),
        AsyncEnvelopeErrorKind::UnregisteredPayload,
    );

    assert_eq!(
        decode_async_envelope(
            &wire(
                "{\"kind\":\"presentation_signal\",\"name\":\"caller_invented\",\"scope\":\"root-scope\",\"value\":1}",
                4,
                41,
            ),
            &limits(),
            &context,
        )
        .expect_err("caller cannot add presentation authority")
        .kind(),
        AsyncEnvelopeErrorKind::UnregisteredPayload,
    );

    let valid = decode("{\"kind\":\"heartbeat\"}", 4, 41);
    assert_eq!(
        dispatch_without_failure(&context, &mut machine, &valid),
        SequenceDisposition::Apply,
    );
    assert_eq!(machine.current(), valid.position());
}

#[test]
fn sequence_machine_applies_only_next_and_degrades_on_gaps_or_new_epochs() {
    let context = context();
    let mut machine = SequenceMachine::new(&context);
    let duplicate = decode("{\"kind\":\"heartbeat\"}", 4, 40);

    assert_eq!(
        dispatch_without_failure(&context, &mut machine, &duplicate),
        SequenceDisposition::IgnoreDuplicate
    );
    let stale = decode("{\"kind\":\"heartbeat\"}", 3, 99);
    assert_eq!(
        dispatch_without_failure(&context, &mut machine, &stale),
        SequenceDisposition::IgnoreStaleEpoch
    );
    let next = decode("{\"kind\":\"heartbeat\"}", 4, 41);
    assert_eq!(
        dispatch_without_failure(&context, &mut machine, &next),
        SequenceDisposition::Apply
    );
    assert_eq!(machine.state(), SequenceState::Current);

    let gap = decode("{\"kind\":\"heartbeat\"}", 4, 43);
    assert!(matches!(
        dispatch_without_failure(&context, &mut machine, &gap),
        SequenceDisposition::Degraded(_)
    ));
    assert_eq!(machine.state(), SequenceState::Degraded);
    let duplicate = decode("{\"kind\":\"heartbeat\"}", 4, 41);
    assert_eq!(
        dispatch_without_failure(&context, &mut machine, &duplicate),
        SequenceDisposition::IgnoreDuplicate
    );
    let waiting = decode("{\"kind\":\"heartbeat\"}", 4, 42);
    assert_eq!(
        dispatch_without_failure(&context, &mut machine, &waiting),
        SequenceDisposition::AwaitingRecovery
    );
    assert_eq!(machine.current().sequence(), StreamSequence::new(41));

    let replay = [
        decode("{\"kind\":\"heartbeat\"}", 4, 42),
        decode("{\"kind\":\"heartbeat\"}", 4, 43),
    ];
    let outcome = recover_without_failure(&context, &mut machine, &replay)
        .expect("complete contiguous replay");
    assert_eq!(outcome.applied(), 2);
    assert_eq!(machine.state(), SequenceState::Current);
    assert_eq!(
        machine.current(),
        StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(43))
    );

    let next_epoch = decode("{\"kind\":\"heartbeat\"}", 5, 1);
    assert!(matches!(
        dispatch_without_failure(&context, &mut machine, &next_epoch),
        SequenceDisposition::Degraded(_)
    ));
    let authority = StaticContinuityAuthority(StreamPosition::new(
        StreamEpoch::new(5),
        StreamSequence::new(7),
    ));
    assert_eq!(
        machine.recover_from_authoritative_refresh(&authority),
        Ok(BaselineDisposition::Adopted)
    );
    assert_eq!(machine.state(), SequenceState::Current);
}

#[test]
fn replay_recovery_requires_a_complete_contiguous_same_scope_transcript() {
    let baseline = StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(41));
    let context = context_at(baseline);
    let mut machine = SequenceMachine::new(&context);
    let gap = decode("{\"kind\":\"heartbeat\"}", 4, 43);
    let _ = dispatch_without_failure(&context, &mut machine, &gap);
    assert_eq!(
        machine.high_water(),
        Some(StreamPosition::new(
            StreamEpoch::new(4),
            StreamSequence::new(43)
        ))
    );

    let invalid = [
        Vec::new(),
        vec![decode("{\"kind\":\"heartbeat\"}", 4, 41)],
        vec![decode("{\"kind\":\"heartbeat\"}", 4, 42)],
        vec![
            decode("{\"kind\":\"heartbeat\"}", 4, 42),
            decode("{\"kind\":\"heartbeat\"}", 4, 42),
            decode("{\"kind\":\"heartbeat\"}", 4, 43),
        ],
        vec![
            decode("{\"kind\":\"heartbeat\"}", 4, 42),
            decode("{\"kind\":\"heartbeat\"}", 4, 44),
        ],
        vec![
            decode("{\"kind\":\"heartbeat\"}", 5, 1),
            decode("{\"kind\":\"heartbeat\"}", 5, 2),
        ],
    ];
    for transcript in invalid {
        assert_eq!(
            recover_without_failure(&context, &mut machine, &transcript)
                .expect_err("invalid replay transcript")
                .kind(),
            SequenceErrorKind::InvalidReplayTranscript
        );
        assert_eq!(machine.current(), baseline);
        assert_eq!(machine.state(), SequenceState::Degraded);
    }

    let valid = [
        decode("{\"kind\":\"heartbeat\"}", 4, 42),
        decode("{\"kind\":\"heartbeat\"}", 4, 43),
    ];
    let outcome =
        recover_without_failure(&context, &mut machine, &valid).expect("valid replay transcript");
    assert_eq!(outcome.applied(), valid.len());
    assert_eq!(machine.current(), valid[1].position());
}

struct StaticContinuityAuthority(StreamPosition);

impl AsyncContinuityAuthorityPort for StaticContinuityAuthority {
    fn authoritative_refresh(
        &self,
        _request: AsyncContinuityRequest<'_>,
    ) -> Option<StreamPosition> {
        Some(self.0)
    }
}

#[derive(Default)]
struct RecordingDispatcher {
    attempts: Vec<StreamPosition>,
    applied: Vec<StreamPosition>,
    fail_once_at: Option<StreamPosition>,
}

impl RecordingDispatcher {
    fn failing_once_at(position: StreamPosition) -> Self {
        Self {
            fail_once_at: Some(position),
            ..Self::default()
        }
    }
}

impl AsyncEnvelopeDispatchPort for RecordingDispatcher {
    fn dispatch(&mut self, delivery: ResolvedAsyncDelivery<'_>) -> Result<(), AsyncDispatchError> {
        let envelope = delivery.envelope();
        self.attempts.push(envelope.position());
        if self.fail_once_at == Some(envelope.position()) {
            self.fail_once_at = None;
            return Err(AsyncDispatchError::failed());
        }
        self.applied.push(envelope.position());
        Ok(())
    }
}

fn admit<'a>(
    context: &'a AsyncEnvelopeContext,
    envelope: &'a AsyncEnvelope,
    registry: &TestMembershipRegistry,
) -> ResolvedAsyncDelivery<'a> {
    let guard = context
        .admit(envelope, registry, UnixMillis::new(1_200))
        .expect("fresh active membership");
    ResolvedAsyncDelivery::new(guard, None, usize::from(super::MAX_EVENT_FANOUT))
}

fn dispatch_without_failure(
    context: &AsyncEnvelopeContext,
    machine: &mut SequenceMachine,
    envelope: &AsyncEnvelope,
) -> SequenceDisposition {
    let registry = membership_registry();
    let mut dispatcher = RecordingDispatcher::default();
    machine
        .dispatch(
            admit(context, envelope, &registry),
            UnixMillis::new(1_200),
            &mut dispatcher,
        )
        .expect("closed test dispatch")
}

fn recover_without_failure<'a>(
    context: &'a AsyncEnvelopeContext,
    machine: &mut SequenceMachine,
    transcript: &'a [AsyncEnvelope],
) -> Result<ReplayDispatchOutcome, ReplayDispatchError> {
    let registry = membership_registry();
    let guards = transcript
        .iter()
        .map(|envelope| admit(context, envelope, &registry))
        .collect::<Vec<_>>();
    let mut dispatcher = RecordingDispatcher::default();
    machine.recover_from_replay(guards, UnixMillis::new(1_200), &mut dispatcher)
}

#[test]
fn fresh_membership_and_successful_dispatch_are_required_before_sequence_commit() {
    let context = context();
    let envelope = decode("{\"kind\":\"heartbeat\"}", 4, 41);
    let mut machine = SequenceMachine::new(&context);
    let mut dispatcher = RecordingDispatcher::failing_once_at(envelope.position());

    let mut revoked = membership_registry();
    revoked.active = false;
    assert!(
        context
            .admit(&envelope, &revoked, UnixMillis::new(1_200))
            .is_err()
    );
    assert_eq!(machine.current(), position(4, 40));
    assert!(dispatcher.attempts.is_empty());

    assert_eq!(
        context
            .admit(&envelope, &membership_registry(), UnixMillis::new(5_000))
            .expect_err("exclusive descriptor expiry rejects admission")
            .kind(),
        AsyncEnvelopeErrorKind::MembershipExpired,
    );

    let admitted_before_expiry = context
        .admit(&envelope, &membership_registry(), UnixMillis::new(4_999))
        .expect("membership is active immediately before expiry");
    assert_eq!(
        machine
            .dispatch(
                ResolvedAsyncDelivery::new(
                    admitted_before_expiry,
                    None,
                    usize::from(super::MAX_EVENT_FANOUT),
                ),
                UnixMillis::new(5_000),
                &mut dispatcher,
            )
            .expect_err("a retained guard expires before dispatch")
            .kind(),
        SequenceErrorKind::MembershipExpired,
    );
    assert_eq!(machine.current(), position(4, 40));
    assert!(dispatcher.attempts.is_empty());

    let mut stale_signals = membership_registry();
    stale_signals.presentation_signals =
        BoundedPresentationSignalContracts::new(Vec::new()).expect("empty current signals");
    assert!(
        context
            .admit(&envelope, &stale_signals, UnixMillis::new(1_200))
            .is_err()
    );

    let mut stale_events = membership_registry();
    stale_events.events = BoundedEventContracts::new(vec![
        SubscriptionEventContract::from_registered(
            &EventMetadata::from_payload_with_contract::<OrdersUpdatedV2>(
                EventSource::Stream,
                BoundedTargets::new(vec![EventTarget::SelfIsland]).expect("target"),
                EventOrder::PerSourceSequence,
                EventCyclePolicy::MaximumHops(NonZeroU8::new(4).expect("hops")),
                4,
            )
            .expect("changed event metadata"),
        )
        .expect("changed event contract"),
    ])
    .expect("changed current events");
    assert!(
        context
            .admit(&envelope, &stale_events, UnixMillis::new(1_200))
            .is_err()
    );

    let dispatch_error = machine
        .dispatch(
            admit(&context, &envelope, &membership_registry()),
            UnixMillis::new(1_200),
            &mut dispatcher,
        )
        .expect_err("failed dispatch cannot commit sequence");
    assert_eq!(dispatch_error.kind(), SequenceErrorKind::DispatchFailed);
    assert_eq!(machine.current(), position(4, 40));
    assert_eq!(machine.state(), SequenceState::Current);
    assert_eq!(machine.high_water(), None);

    assert_eq!(
        machine
            .dispatch(
                admit(&context, &envelope, &membership_registry()),
                UnixMillis::new(1_200),
                &mut dispatcher,
            )
            .expect("fresh retry dispatch"),
        SequenceDisposition::Apply,
    );
    assert_eq!(machine.current(), position(4, 41));

    assert_eq!(
        machine
            .dispatch(
                admit(&context, &envelope, &membership_registry()),
                UnixMillis::new(1_200),
                &mut dispatcher,
            )
            .expect("stale admitted duplicate is classified without dispatch"),
        SequenceDisposition::IgnoreDuplicate,
    );
    assert_eq!(dispatcher.applied, vec![position(4, 41)]);
}

#[test]
fn replay_dispatch_failure_reports_truth_and_resumes_only_the_remaining_suffix() {
    for failed_at in [41, 42, 44] {
        let context = context();
        let registry = membership_registry();
        let mut machine = SequenceMachine::new(&context);
        let gap = decode("{\"kind\":\"heartbeat\"}", 4, 44);
        let mut dispatcher = RecordingDispatcher::failing_once_at(position(4, failed_at));

        assert!(matches!(
            machine
                .dispatch(
                    admit(&context, &gap, &registry),
                    UnixMillis::new(1_200),
                    &mut dispatcher,
                )
                .expect("gap classification does not dispatch"),
            SequenceDisposition::Degraded(_),
        ));
        assert!(dispatcher.attempts.is_empty());

        let replay = (41..=44)
            .map(|sequence| decode("{\"kind\":\"heartbeat\"}", 4, sequence))
            .collect::<Vec<_>>();
        let guards = replay
            .iter()
            .map(|envelope| admit(&context, envelope, &registry))
            .collect::<Vec<_>>();
        let error = machine
            .recover_from_replay(guards, UnixMillis::new(1_200), &mut dispatcher)
            .expect_err("selected replay dispatch fails once");
        assert_eq!(error.kind(), SequenceErrorKind::DispatchFailed);
        assert_eq!(error.applied(), (failed_at - 41) as usize);
        assert_eq!(error.current(), position(4, failed_at - 1));
        assert_eq!(error.state(), SequenceState::Degraded);
        assert_eq!(error.high_water(), Some(position(4, 44)));

        if failed_at > 41 {
            let attempts_before_invalid_prefix = dispatcher.attempts.len();
            let full_retry = replay
                .iter()
                .map(|envelope| admit(&context, envelope, &registry))
                .collect::<Vec<_>>();
            assert_eq!(
                machine
                    .recover_from_replay(full_retry, UnixMillis::new(1_200), &mut dispatcher,)
                    .expect_err("committed prefix cannot be redispatched")
                    .kind(),
                SequenceErrorKind::InvalidReplayTranscript,
            );
            assert_eq!(dispatcher.attempts.len(), attempts_before_invalid_prefix);
        }

        let remaining = replay
            .iter()
            .filter(|envelope| envelope.position().sequence().get() >= failed_at)
            .map(|envelope| admit(&context, envelope, &registry))
            .collect::<Vec<_>>();
        let outcome = machine
            .recover_from_replay(remaining, UnixMillis::new(1_200), &mut dispatcher)
            .expect("fresh remaining replay suffix");
        assert_eq!(outcome.applied(), (45 - failed_at) as usize);
        assert_eq!(outcome.current(), position(4, 44));
        assert_eq!(outcome.state(), SequenceState::Current);
        assert_eq!(machine.high_water(), None);

        assert_eq!(
            dispatcher.applied,
            (41..=44)
                .map(|sequence| position(4, sequence))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn pressure_only_replay_retirement_keeps_effective_high_water_and_truthful_kind() {
    let context = context_at(position(4, 40));
    let registry = membership_registry();
    let replay = [
        decode("{\"kind\":\"heartbeat\"}", 4, 41),
        decode("{\"kind\":\"heartbeat\"}", 4, 42),
    ];
    let mut machine = SequenceMachine::new(&context);
    let envelopes = replay.iter().collect::<Vec<_>>();
    let mut recovery = machine
        .prepare_replay(&envelopes, Some(position(4, 42)))
        .expect("pressure-only recovery transcript");
    let first = admit(&context, &replay[0], &registry);
    machine
        .dispatch_replay_entry(
            &mut recovery,
            first,
            UnixMillis::new(1_200),
            &mut RecordingDispatcher::default(),
        )
        .expect("first replay entry commits");

    let error = machine.interrupt_replay(&recovery, SequenceErrorKind::DeliveryRetired);
    assert_eq!(error.kind(), SequenceErrorKind::DeliveryRetired);
    assert_eq!(error.applied(), 1);
    assert_eq!(error.current(), position(4, 41));
    assert_eq!(error.state(), SequenceState::Current);
    assert_eq!(error.high_water(), Some(position(4, 42)));
}

#[test]
fn cross_scope_envelopes_and_replay_cannot_change_sequence_authority() {
    let context_a = context();
    let other_id = SubscriptionId::from_bytes(b"subscription-002").expect("other id");
    let mut registry_b = membership_registry();
    registry_b.subscription = other_id.clone();
    let context_b = AsyncEnvelopeContext::from_authorized(
        authorized_subscription(),
        other_id.clone(),
        &registry_b,
    )
    .expect("active B membership");
    let envelope_b = decode_async_envelope(
        &wire_for(&other_id, &stream(), "{\"kind\":\"heartbeat\"}", 4, 43),
        &limits(),
        &context_b,
    )
    .expect("valid B envelope");
    let baseline = StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(40));
    let mut machine = SequenceMachine::new(&context_a);
    let state = machine.state();

    let mut dispatcher = RecordingDispatcher::default();
    assert_eq!(
        machine
            .dispatch(
                ResolvedAsyncDelivery::new(
                    context_b
                        .admit(&envelope_b, &registry_b, UnixMillis::new(1_200))
                        .expect("fresh B guard"),
                    None,
                    usize::from(super::MAX_EVENT_FANOUT),
                ),
                UnixMillis::new(1_200),
                &mut dispatcher,
            )
            .expect_err("B guard cannot enter A machine")
            .kind(),
        SequenceErrorKind::ScopeMismatch,
    );
    assert_eq!(machine.current(), baseline);
    assert_eq!(machine.high_water(), None);
    assert_eq!(machine.state(), state);

    let gap = decode("{\"kind\":\"heartbeat\"}", 4, 43);
    let _ = dispatch_without_failure(&context_a, &mut machine, &gap);
    let high_water = machine.high_water();
    let cross_scope_guards = vec![ResolvedAsyncDelivery::new(
        context_b
            .admit(&envelope_b, &registry_b, UnixMillis::new(1_200))
            .expect("fresh B replay guard"),
        None,
        usize::from(super::MAX_EVENT_FANOUT),
    )];
    assert_eq!(
        machine
            .recover_from_replay(cross_scope_guards, UnixMillis::new(1_200), &mut dispatcher,)
            .expect_err("cross-scope replay")
            .kind(),
        SequenceErrorKind::ScopeMismatch
    );
    assert_eq!(machine.current(), baseline);
    assert_eq!(machine.high_water(), high_water);
    assert_eq!(machine.state(), SequenceState::Degraded);
}

#[test]
fn only_host_continuity_authority_can_establish_a_covering_new_baseline() {
    let baseline = StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(41));
    let context = context_at(baseline);
    let mut machine = SequenceMachine::new(&context);
    let next_epoch = decode("{\"kind\":\"heartbeat\"}", 5, 3);
    let _ = dispatch_without_failure(&context, &mut machine, &next_epoch);
    let high_water = machine.high_water();

    let invalid_replay = [
        decode("{\"kind\":\"heartbeat\"}", 4, 42),
        decode("{\"kind\":\"heartbeat\"}", 4, 43),
    ];
    assert_eq!(
        recover_without_failure(&context, &mut machine, &invalid_replay)
            .expect_err("same-epoch replay cannot prove a new epoch")
            .kind(),
        SequenceErrorKind::InvalidReplayTranscript
    );
    assert_eq!(machine.current(), baseline);
    assert_eq!(machine.high_water(), high_water);

    assert_eq!(
        machine
            .recover_from_authoritative_refresh(&StaticContinuityAuthority(StreamPosition::new(
                StreamEpoch::new(5),
                StreamSequence::new(2)
            ),))
            .expect_err("host baseline must cover observed high-water")
            .kind(),
        SequenceErrorKind::AuthoritativeBaselineInsufficient
    );
    assert_eq!(machine.current(), baseline);
    assert_eq!(machine.high_water(), high_water);

    assert_eq!(
        machine.recover_from_authoritative_refresh(&StaticContinuityAuthority(
            StreamPosition::new(StreamEpoch::new(5), StreamSequence::new(3)),
        )),
        Ok(BaselineDisposition::Adopted)
    );
    assert_eq!(
        machine.current(),
        StreamPosition::new(StreamEpoch::new(5), StreamSequence::new(3))
    );
    assert_eq!(machine.high_water(), None);
    assert_eq!(machine.state(), SequenceState::Current);
}

#[test]
fn sequence_overflow_never_wraps_or_applies() {
    let baseline = StreamPosition::new(StreamEpoch::new(9), StreamSequence::new(u64::MAX));
    let context = context_at(baseline);
    let mut machine = SequenceMachine::new(&context);
    let duplicate = decode("{\"kind\":\"heartbeat\"}", 9, u64::MAX);
    assert_eq!(
        dispatch_without_failure(&context, &mut machine, &duplicate),
        SequenceDisposition::IgnoreDuplicate
    );
    let stale = decode("{\"kind\":\"heartbeat\"}", 9, u64::MAX - 1);
    assert_eq!(
        dispatch_without_failure(&context, &mut machine, &stale),
        SequenceDisposition::IgnoreDuplicate
    );
    let next_epoch = decode("{\"kind\":\"heartbeat\"}", 10, 0);
    assert_eq!(
        dispatch_without_failure(&context, &mut machine, &next_epoch),
        SequenceDisposition::Degraded(super::SequenceDegradation::EpochChanged)
    );
    assert_eq!(machine.current(), baseline);
}

proptest! {
    #[test]
    fn sequence_machine_never_applies_a_gap(observed in prop::collection::vec((0_u64..4, any::<u64>()), 0..128)) {
        let baseline = StreamPosition::new(StreamEpoch::new(2), StreamSequence::new(10));
        let context = context_at(baseline);
        let mut machine = SequenceMachine::new(&context);
        for (epoch, sequence) in observed {
            let before = machine.current();
            let envelope = decode("{\"kind\":\"heartbeat\"}", epoch, sequence);
            if dispatch_without_failure(&context, &mut machine, &envelope) == SequenceDisposition::Apply {
                prop_assert_eq!(epoch, before.epoch().get());
                prop_assert_eq!(sequence, before.sequence().get() + 1);
                prop_assert_eq!(machine.current(), envelope.position());
            }
        }
    }

    #[test]
    fn incomplete_replay_transcripts_never_adopt_or_clear_a_gap(
        gap in 2_u64..64,
        omitted_offset in 1_u64..64,
    ) {
        let baseline = StreamPosition::new(StreamEpoch::new(7), StreamSequence::new(10));
        let context = context_at(baseline);
        let high_sequence = 10 + gap;
        let mut machine = SequenceMachine::new(&context);
        let observed_gap = decode(
            "{\"kind\":\"heartbeat\"}",
            7,
            high_sequence,
        );
        let _ = dispatch_without_failure(&context, &mut machine, &observed_gap);
        let omitted = 11 + (omitted_offset % gap);
        let transcript = (11..=high_sequence)
            .filter(|sequence| *sequence != omitted)
            .map(|sequence| decode("{\"kind\":\"heartbeat\"}", 7, sequence))
            .collect::<Vec<_>>();

        prop_assert_eq!(
            recover_without_failure(&context, &mut machine, &transcript)
                .expect_err("incomplete transcript")
                .kind(),
            SequenceErrorKind::InvalidReplayTranscript
        );
        prop_assert_eq!(machine.current(), baseline);
        prop_assert_eq!(machine.state(), SequenceState::Degraded);
        prop_assert_eq!(
            machine.high_water(),
            Some(StreamPosition::new(
                StreamEpoch::new(7),
                StreamSequence::new(high_sequence),
            ))
        );
    }

    #[test]
    fn canonical_round_trip_is_stable(epoch in any::<u64>(), sequence in any::<u64>()) {
        let encoded = wire("{\"kind\":\"heartbeat\"}", epoch, sequence);
        let envelope = decode_async_envelope(&encoded, &limits(), &context()).expect("decode");
        prop_assert_eq!(encode_async_envelope(&envelope, &limits()).expect("encode"), encoded);
    }
}

#[test]
fn version_four_async_fixture_is_executable_not_documentary() {
    let root: Value = serde_json::from_slice(
        &fs::read(fixture_directory(FixtureVersion::V4).join("async-envelope.json"))
            .expect("fixture bytes"),
    )
    .expect("fixture JSON");

    assert_eq!(root["protocol_versions"], serde_json::json!([1]));
    assert_eq!(root["live_protocol_versions"], serde_json::json!([1, 2]));
    for case in root["envelope_cases"].as_array().expect("envelope cases") {
        let encoded = case["encoded"].as_str().expect("encoded case").as_bytes();
        let result = decode_async_envelope(encoded, &limits(), &context());
        match case["expected"].as_str().expect("expected disposition") {
            "accepted" => {
                let envelope = result.expect("accepted fixture");
                assert_eq!(
                    encode_async_envelope(&envelope, &limits()).expect("encode fixture"),
                    encoded
                );
            }
            "unsupported_protocol" => assert_eq!(
                result.expect_err("unsupported protocol").kind(),
                AsyncEnvelopeErrorKind::UnsupportedProtocol
            ),
            "duplicate_field" => assert_eq!(
                result.expect_err("duplicate field").kind(),
                AsyncEnvelopeErrorKind::DuplicateField
            ),
            "unsupported_payload" => assert_eq!(
                result.expect_err("unsupported payload").kind(),
                AsyncEnvelopeErrorKind::UnsupportedPayload
            ),
            other => panic!("unknown expected fixture disposition: {other}"),
        }
    }

    for case in root["continuity_cases"]
        .as_array()
        .expect("continuity cases")
    {
        let baseline = fixture_position(&case["baseline"]);
        let context = context_at(baseline);
        let mut machine = SequenceMachine::new(&context);
        let disposition = if let Some(observed) = case.get("observed") {
            let observed = fixture_position(observed);
            let envelope = decode(
                "{\"kind\":\"heartbeat\"}",
                observed.epoch().get(),
                observed.sequence().get(),
            );
            match dispatch_without_failure(&context, &mut machine, &envelope) {
                SequenceDisposition::Apply => "apply",
                SequenceDisposition::IgnoreDuplicate => "ignore_duplicate",
                SequenceDisposition::Degraded(_) => "degrade",
                SequenceDisposition::IgnoreStaleEpoch => "ignore_stale_epoch",
                SequenceDisposition::AwaitingRecovery => "awaiting_recovery",
                SequenceDisposition::ScopeMismatch => "scope_mismatch",
            }
        } else {
            let observed_gap = fixture_position(&case["observed_gap"]);
            let gap = decode(
                "{\"kind\":\"heartbeat\"}",
                observed_gap.epoch().get(),
                observed_gap.sequence().get(),
            );
            assert!(matches!(
                dispatch_without_failure(&context, &mut machine, &gap),
                SequenceDisposition::Degraded(_)
            ));
            let recovery = &case["recovery"];
            match recovery["kind"].as_str().expect("recovery kind") {
                "replay" => {
                    let transcript = recovery["transcript"]
                        .as_array()
                        .expect("replay transcript")
                        .iter()
                        .map(|position| {
                            let position = fixture_position(position);
                            decode(
                                "{\"kind\":\"heartbeat\"}",
                                position.epoch().get(),
                                position.sequence().get(),
                            )
                        })
                        .collect::<Vec<_>>();
                    recover_without_failure(&context, &mut machine, &transcript)
                        .expect("fixture replay recovery");
                }
                "authoritative_refresh" => {
                    let authority = fixture_position(&recovery["baseline"]);
                    assert_eq!(
                        machine.recover_from_authoritative_refresh(&StaticContinuityAuthority(
                            authority
                        )),
                        Ok(BaselineDisposition::Adopted)
                    );
                }
                other => panic!("unknown recovery kind: {other}"),
            }
            "adopt_baseline"
        };
        assert_eq!(
            disposition,
            case["expected"].as_str().expect("continuity expected"),
            "continuity fixture {}",
            case["id"].as_str().expect("continuity id"),
        );
        assert_eq!(
            match machine.state() {
                SequenceState::Current => "current",
                SequenceState::Degraded => "degraded",
            },
            case["state"].as_str().expect("continuity state"),
        );
    }
}

#[test]
fn event_fanout_remains_nonzero_and_bounded_in_the_registered_contract() {
    assert_eq!(
        event_contract().maximum_fanout(),
        NonZeroU16::new(4).unwrap()
    );
}
