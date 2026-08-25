#![allow(
    dead_code,
    reason = "each fuzz binary uses one half of this shared deterministic setup"
)]

use std::future::Future;
use std::num::NonZeroU8;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll, Wake, Waker};

use suprnova_live::async_updates::{
    AsyncEnvelopeContext, AsyncMembershipRegistryPort, AsyncMembershipRequest,
    AsyncMembershipValidation, AuthoritativeStreamPosition, BoundedEventContracts,
    BoundedEventNames, BoundedPresentationSignalContracts, BoundedTargets, BoundedTopics,
    BrowserPayloadSchema, CapabilityVersion, CurrentSubscriptionRegistration, EventCyclePolicy,
    EventOrder, EventSource, EventTarget, PollFallbackPolicy, PollInitialBehavior,
    PollVisibilityPolicy, ReconnectPolicy, StreamEpoch, StreamName, StreamPosition, StreamSequence,
    SubscriptionAuthorizationDecision, SubscriptionAuthorizationPort,
    SubscriptionAuthorizationRequest, SubscriptionBaselineRequest, SubscriptionContinuityPort,
    SubscriptionCredentialPort, SubscriptionCredentialRequest,
    SubscriptionCredentialRotationOutcome, SubscriptionCredentialRotationRequest,
    SubscriptionError, SubscriptionFuture, SubscriptionId,
    SubscriptionIssueRequest, SubscriptionMetadata, SubscriptionMode, SubscriptionModes,
    SubscriptionRegistryPort, SubscriptionRegistryRequest, SubscriptionService, TopicName,
    TransportCredential, TrustedMountParameters,
};
use suprnova_live::checker::{CheckReport, CheckerLimits, TemplateCatalog, TemplateChecker};
use suprnova_live::child::{ChildParameterLimits, ExpectedChildParametersV1};
use suprnova_live::component::composition::{
    ChildKey, ChildParameterField, ChildParameterSchema,
};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::host::{
    CheckDisposition, CheckFact, CheckKind, HostCapabilities, HostCheckFacts, HostScopeFacts,
    LiveRequestContextCandidate, LiveRequestContextValidator, MountCatalogBuilder,
    MountCatalogEntry, MountScopeRequirements, MountSelection, PolicyReason, ScopeRequirement,
    TrustedLiveRequestContext,
};
use suprnova_live::identity::{
    BrowserOperationName, BuildId, ComponentName, ContentDigest, InstanceId, IslandSlot, KeyId,
    ModelField, Revision, RouteIdentity, ScopeFingerprint, UnixMillis, ViewName,
};
use suprnova_live::limits::InputLimits;
use suprnova_live::metadata::{
    ComponentMetadata, ContractVersions, EventMetadata, EventPayloadMetadata,
};
use suprnova_live::protocol::{ProtocolLimitConfig, ProtocolLimits};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
use suprnova_live::snapshot::state::{FieldCategory, FieldSpec, StateCodec, StateSchema};
use suprnova_live::snapshot::{
    ComponentContract, ExpectedInstanceV1, ExpectedSeedV1, SnapshotLimits, SnapshotSchemaSet,
};
use suprnova_live::state::ModelCodec;

pub(crate) struct SnapshotSetup {
    pub(crate) keys: SnapshotKeyRing,
    pub(crate) seed: ExpectedSeedV1,
    pub(crate) instance: ExpectedInstanceV1,
    pub(crate) limits: SnapshotLimits,
}

pub(crate) struct ChildSetup {
    pub(crate) expected: ExpectedChildParametersV1,
    pub(crate) limits: ChildParameterLimits,
}

struct FuzzEvent;

impl EventPayloadMetadata for FuzzEvent {
    const NAME: &'static str = "fuzz.event";
    const VERSION: u16 = 1;
    const PAYLOAD_CONTRACT: &'static str = "fuzz.event.payload";
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
}

struct FuzzSubscriptionPorts {
    component: ComponentMetadata,
    parameters: TrustedMountParameters,
}

impl SubscriptionRegistryPort for FuzzSubscriptionPorts {
    fn resolve<'a>(
        &'a self,
        request: SubscriptionRegistryRequest<'a>,
    ) -> SubscriptionFuture<'a, Result<CurrentSubscriptionRegistration, SubscriptionError>> {
        Box::pin(async move {
            CurrentSubscriptionRegistration::from_registered(
                &self.component,
                request.stream(),
                &self.parameters,
            )
        })
    }
}

impl SubscriptionContinuityPort for FuzzSubscriptionPorts {
    fn authoritative_baseline<'a>(
        &'a self,
        _request: SubscriptionBaselineRequest<'a>,
    ) -> SubscriptionFuture<'a, Result<AuthoritativeStreamPosition, SubscriptionError>> {
        Box::pin(async {
            Ok(AuthoritativeStreamPosition::from_host_continuity(
                StreamPosition::new(StreamEpoch::new(1), StreamSequence::new(0)),
            ))
        })
    }
}

impl SubscriptionAuthorizationPort for FuzzSubscriptionPorts {
    fn authorize<'a>(
        &'a self,
        _request: SubscriptionAuthorizationRequest<'a>,
    ) -> SubscriptionFuture<'a, Result<SubscriptionAuthorizationDecision, SubscriptionError>> {
        Box::pin(async { Ok(SubscriptionAuthorizationDecision::Allow) })
    }
}

impl SubscriptionCredentialPort for FuzzSubscriptionPorts {
    fn issue<'a>(
        &'a self,
        _request: SubscriptionCredentialRequest<'a>,
    ) -> SubscriptionFuture<'a, Result<TransportCredential, SubscriptionError>> {
        Box::pin(async { TransportCredential::from_host_authority_bearer(vec![0x61; 32]) })
    }

    fn consume_and_rotate<'a>(
        &'a self,
        _request: SubscriptionCredentialRotationRequest<'a>,
    ) -> SubscriptionFuture<'a, SubscriptionCredentialRotationOutcome> {
        Box::pin(async {
            match TransportCredential::from_host_authority_bearer(vec![0x62; 32]) {
                Ok(credential) => SubscriptionCredentialRotationOutcome::Rotated(credential),
                Err(_) => SubscriptionCredentialRotationOutcome::Failed,
            }
        })
    }
}

struct FuzzMembershipRegistry {
    subscription: SubscriptionId,
    stream: StreamName,
    events: BoundedEventContracts,
}

impl AsyncMembershipRegistryPort for FuzzMembershipRegistry {
    fn validate_current(
        &self,
        request: AsyncMembershipRequest<'_>,
        validation: &mut AsyncMembershipValidation<'_>,
    ) {
        if request.subscription() == &self.subscription {
            let signals = BoundedPresentationSignalContracts::new(Vec::new())
                .expect("empty signal contracts");
            let _ = validation.accept_current(&self.stream, &self.events, &signals);
        }
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn block_on_ready<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut task_context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    for _ in 0..64 {
        if let Poll::Ready(output) = future.as_mut().poll(&mut task_context) {
            return output;
        }
    }
    panic!("deterministic fuzz authority fixture did not become ready")
}

pub(crate) fn async_subscription_id() -> SubscriptionId {
    SubscriptionId::from_bytes(b"fuzz-subscription").expect("static subscription identity")
}

pub(crate) fn async_context() -> &'static AsyncEnvelopeContext {
    static CONTEXT: OnceLock<AsyncEnvelopeContext> = OnceLock::new();
    CONTEXT.get_or_init(|| {
        let ports = Arc::new(FuzzSubscriptionPorts {
            component: async_component_metadata(),
            parameters: TrustedMountParameters::new(Vec::new()).expect("empty mount parameters"),
        });
        let trusted = async_trusted_context(ports.clone());
        let service = SubscriptionService::new(async_subscription_key_ring());
        let issued = block_on_ready(service.issue(
            &trusted,
            SubscriptionIssueRequest::new(
                async_stream(),
                CapabilityVersion::new(1).expect("static capability"),
                UnixMillis::new(5_000),
                PollFallbackPolicy::new(
                    10_000,
                    0,
                    PollInitialBehavior::AfterInterval,
                    PollVisibilityPolicy::PauseWhenHidden,
                )
                .expect("static fallback"),
            ),
            UnixMillis::new(1_000),
        ))
        .expect("fuzz subscription issuance");
        let authorized = block_on_ready(service.connect(
            &trusted,
            issued.descriptor(),
            issued.transport_credential(),
            UnixMillis::new(1_100),
        ))
        .expect("fuzz subscription authorization");
        let membership = FuzzMembershipRegistry {
            subscription: async_subscription_id(),
            stream: async_stream(),
            events: authorized.verified().claims().events().clone(),
        };
        AsyncEnvelopeContext::from_authorized(
            &authorized,
            async_subscription_id(),
            &membership,
        )
        .expect("active fuzz membership")
    })
}

fn async_event_metadata() -> EventMetadata {
    EventMetadata::from_payload_with_contract::<FuzzEvent>(
        EventSource::Stream,
        BoundedTargets::new(vec![EventTarget::SelfIsland]).expect("static target"),
        EventOrder::PerSourceSequence,
        EventCyclePolicy::MaximumHops(NonZeroU8::new(1).expect("static hop")),
        1,
    )
    .expect("static event metadata")
}

fn async_stream() -> StreamName {
    StreamName::parse("fuzz").expect("static stream")
}

fn async_component_metadata() -> ComponentMetadata {
    let event = async_event_metadata();
    let subscription = SubscriptionMetadata::new(
        async_stream(),
        BoundedTopics::new(vec![TopicName::parse("fuzz").expect("static topic")])
            .expect("static topics"),
        BoundedEventNames::new(vec![
            BrowserOperationName::parse(FuzzEvent::NAME).expect("static event name"),
        ])
        .expect("static event names"),
        SubscriptionModes::new(vec![SubscriptionMode::ServerSentEvents])
            .expect("static subscription mode"),
        ReconnectPolicy::RefreshOnReconnect,
    );
    ComponentMetadata::new_with_async_contracts(
        ComponentName::parse("fuzz.async").expect("static component"),
        ViewName::parse("fuzz/async.html").expect("static view"),
        ContractVersions::new(1, 1, 1, 1, 1).expect("static versions"),
        Vec::new(),
        Vec::new(),
        vec![event],
        Vec::new(),
        vec![subscription],
        false,
    )
    .expect("static async component metadata")
}

fn async_subscription_key_ring() -> SnapshotKeyRing {
    let key = KeyRecord::new(
        KeyId::parse("fuzz-async-key").expect("static key ID"),
        RootKey::new(vec![0x63; 32]).expect("static root key"),
        UnixMillis::new(0),
        UnixMillis::new(20_000),
        UnixMillis::new(40_000),
    )
    .expect("static key record");
    SnapshotKeyRing::new(key, Vec::new()).expect("static key ring")
}

fn async_trusted_context(ports: Arc<FuzzSubscriptionPorts>) -> TrustedLiveRequestContext {
    let component = &ports.component;
    let descriptor = ComponentDescriptor::new(component.clone());
    let contract = ComponentContract::new(
        component.identity().clone(),
        descriptor.contract_digest().clone(),
        1,
        1,
        1,
    )
    .expect("static component contract");
    let registry = ComponentRegistryBuilder::new()
        .register(descriptor)
        .expect("static component registration")
        .build();
    let route = RouteIdentity::from_bytes(&[0x64; 32]).expect("static route");
    let slot = IslandSlot::parse("fuzz-async").expect("static slot");
    let catalog = MountCatalogBuilder::new()
        .register(
            &registry,
            MountCatalogEntry::new(
                ExpectedSeedV1::new(
                    contract,
                    BuildId::parse("build-fuzz-async").expect("static build"),
                    route.clone(),
                    slot.clone(),
                    schemas().expect("static schemas"),
                ),
                MountScopeRequirements::new(
                    ScopeRequirement::Absent,
                    ScopeRequirement::Absent,
                    ScopeRequirement::Absent,
                ),
            ),
        )
        .expect("static mount registration")
        .build();
    let scope = HostScopeFacts::new(
        ScopeFingerprint::from_bytes(&[0x65; 32]).expect("static scope"),
        None,
        None,
        None,
    );
    let capabilities = HostCapabilities::bound_to(scope.clone())
        .with_subscription_registry(ports.clone())
        .with_subscription_continuity(ports.clone())
        .with_subscription_authorization(ports.clone())
        .with_subscription_credentials(ports.clone());
    let expires_at = UnixMillis::new(10_000);
    let mut checks = HostCheckFacts::new();
    for kind in CheckKind::ALL {
        let disposition = match kind {
            CheckKind::Session => CheckDisposition::NotRequired(PolicyReason::StatelessRequest),
            CheckKind::Principal => {
                CheckDisposition::NotRequired(PolicyReason::AnonymousPrincipal)
            }
            CheckKind::Tenant => CheckDisposition::NotRequired(PolicyReason::TenantlessRoute),
            CheckKind::Origin
            | CheckKind::Csrf
            | CheckKind::Proxy
            | CheckKind::RateLimit
            | CheckKind::Middleware => CheckDisposition::Passed,
        };
        checks
            .record(kind, CheckFact::new(disposition, expires_at))
            .expect("static host check");
    }
    let selection = MountSelection::new(
        route.clone(),
        slot.clone(),
        component.identity().clone(),
        component.contract_digest().clone(),
        1,
    );
    LiveRequestContextValidator::new(300_000)
        .expect("static validator")
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
        .expect("static trusted context")
}

pub(crate) fn protocol_limits() -> Option<ProtocolLimits> {
    ProtocolLimits::new(ProtocolLimitConfig {
        input: InputLimits::new(2_048, 8, 128, 1_024).ok()?,
        max_snapshot_bytes: 1_024,
        max_html_bytes: 1_024,
        max_model_proposals: 8,
        max_operations: 8,
        max_arguments: 8,
        max_validation_entries: 8,
        max_events: 8,
        max_effects: 8,
        max_extensions: 8,
    })
    .ok()
}

pub(crate) fn snapshot_setup() -> Option<&'static SnapshotSetup> {
    static SETUP: OnceLock<Option<SnapshotSetup>> = OnceLock::new();
    SETUP.get_or_init(build_snapshot_setup).as_ref()
}

pub(crate) fn child_setup() -> Option<&'static ChildSetup> {
    static SETUP: OnceLock<Option<ChildSetup>> = OnceLock::new();
    SETUP.get_or_init(build_child_setup).as_ref()
}

pub(crate) fn check_template_source(source: &str) -> Option<CheckReport> {
    let component = ComponentName::parse("fuzz.component").ok()?;
    let view = ViewName::parse("fuzz/component.html").ok()?;
    let versions = ContractVersions::new(1, 1, 1, 1, 1).ok()?;
    let metadata = ComponentMetadata::new(
        component.clone(),
        view.clone(),
        versions,
        vec![],
        vec![],
    )
    .ok()?;
    let registry = ComponentRegistryBuilder::new()
        .register(ComponentDescriptor::new(metadata))
        .ok()?
        .build();
    let catalog = TemplateCatalog::new(vec![(view, source.to_owned())]).ok()?;
    Some(
        TemplateChecker::new(&registry, &catalog, CheckerLimits::default())
            .check_component(&component),
    )
}

fn build_child_setup() -> Option<ChildSetup> {
    let parameter_schema = ChildParameterSchema::new(
        1,
        vec![ChildParameterField::new(
            ModelField::parse("query").ok()?,
            ModelCodec::String,
            true,
        )],
    )
    .ok()?;
    let expected = ExpectedChildParametersV1::new(
        ScopeFingerprint::from_bytes(&[0x30; 32]).ok()?,
        InstanceId::from_bytes(&[0x40; 16]).ok()?,
        Revision::new(1),
        ChildKey::parse("results").ok()?,
        ContentDigest::from_bytes(&[0x50; 32]).ok()?,
        parameter_schema,
    );
    let limits = ChildParameterLimits::new(
        InputLimits::new(2_048, 8, 128, 512).ok()?,
        50,
        10_000,
    )
    .ok()?;
    Some(ChildSetup { expected, limits })
}

fn build_snapshot_setup() -> Option<SnapshotSetup> {
    let active = KeyRecord::new(
        KeyId::parse("snapshot-v1").ok()?,
        RootKey::new(vec![0x42; 32]).ok()?,
        UnixMillis::new(0),
        UnixMillis::new(10_000),
        UnixMillis::new(20_000),
    )
    .ok()?;
    let keys = SnapshotKeyRing::new(active, Vec::new()).ok()?;
    let contract = ComponentContract::new(
        ComponentName::parse("catalog.search").ok()?,
        ContentDigest::from_bytes(&[0x20; 32]).ok()?,
        1,
        1,
        1,
    )
    .ok()?;
    let build = BuildId::parse("build-fuzz-v1").ok()?;
    let route = RouteIdentity::from_bytes(&[0x10; 32]).ok()?;
    let slot = IslandSlot::parse("search-results").ok()?;
    let scope = ScopeFingerprint::from_bytes(&[0x30; 32]).ok()?;
    let schemas = schemas()?;
    let seed = ExpectedSeedV1::new(
        contract.clone(),
        build.clone(),
        route.clone(),
        slot.clone(),
        schemas.clone(),
    );
    let instance = ExpectedInstanceV1::new(contract, build, route, slot, scope, schemas);
    let limits = SnapshotLimits::new(
        InputLimits::new(2_048, 8, 128, 512).ok()?,
        50,
        10_000,
        20_000,
        8,
        8,
    )
    .ok()?;
    Some(SnapshotSetup {
        keys,
        seed,
        instance,
        limits,
    })
}

fn schemas() -> Option<SnapshotSchemaSet> {
    let state = StateSchema::new(
        1,
        vec![FieldSpec::new("query", StateCodec::Json, FieldCategory::Public, true).ok()?],
    )
    .ok()?;
    let memo = StateSchema::new(1, Vec::new()).ok()?;
    let mount = StateSchema::new(1, Vec::new()).ok()?;
    SnapshotSchemaSet::new(state, memo, mount).ok()
}
