//! Shared deterministic component lifecycle fixtures.

#![allow(
    dead_code,
    reason = "shared helpers are used by separate integration-test crates"
)]
#![allow(
    unused_imports,
    reason = "shared re-exports vary across separate integration-test crates"
)]

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll};

use bytes::Bytes;
use http::Method;
use suprnova_live::action::{
    ActionArgumentSchema, ActionAuthorizationPort, AuthorizationRequirement, TransactionPolicy,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::component::{
    ComponentError, ComponentFactory, ComponentHooks, ComponentInstance, HydrationContext,
    LiveFuture, MountContext, RenderContext,
};
use suprnova_live::endpoint::{
    LIVE_MEDIA_TYPE_V1, LIVE_MEDIA_TYPE_V2, LiveEndpointConfig, LiveEndpointRequest,
    ParsedLiveMediaType, RequestCachePolicy,
};
use suprnova_live::host::{
    HostScopeFacts, MountCatalogBuilder, MountCatalogEntry, MountScopeRequirements, MountSelection,
    PrincipalFingerprint, ScopeRequirement, SessionFingerprint, TenantFingerprint,
    TrustedLiveRequestContext,
};
use suprnova_live::identity::{
    BuildId, ComponentName, CorrelationId, InstanceId, IslandSlot, ModelField, Revision,
    ScopeFingerprint, UnixMillis, ViewName,
};
use suprnova_live::metadata::{ActionMetadata, ComponentMetadata, ContractVersions, FieldMetadata};
use suprnova_live::mount::DocumentMountKey;
use suprnova_live::protocol::{BrowserRenderContext, ProtocolLimitConfig, ProtocolLimits};
use suprnova_live::random::{InstanceIdGenerator, RandomError};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
use suprnova_live::snapshot::state::{
    FieldCategory, FieldSpec, StateCodec, StateExposure, StateSchema,
};
use suprnova_live::snapshot::{ComponentContract, ExpectedSeedV1, SnapshotSchemaSet};
use suprnova_live::upload::UploadAuthorizationPort;
use suprnova_live::validation::ValidationSelection;
use suprnova_live::view::{AssetSet, IslandRender};
use suprnova_live_test_support::SyntheticLiveRequestContextBuilder;
use suprnova_live_test_support::{VerifiedResponseSealing, capture_verified_response_sealer};
use tokio::sync::Notify;

#[path = "ledger_support.rs"]
mod ledger_support;
#[path = "snapshot_support.rs"]
pub(crate) mod snapshot_support;

pub(crate) use ledger_support::{ManualClock, digest, idempotency, ledger};

pub(crate) fn browser_context() -> BrowserRenderContext {
    let expected = DocumentMountKey::parse("test-root").expect("document key");
    BrowserRenderContext::checked("test-root", &expected).expect("browser render context")
}

pub(crate) async fn admitted_response_sealer(
    descriptor: ComponentDescriptor,
    context: TrustedLiveRequestContext,
    encoded_snapshot: &[u8],
    base_revision: Revision,
    correlation_start: u8,
    max_response_bytes: Option<usize>,
) -> VerifiedResponseSealing {
    admitted_response_sealer_with_snapshot_limits(
        descriptor,
        context,
        encoded_snapshot,
        base_revision,
        correlation_start,
        max_response_bytes,
        snapshot_limits(),
    )
    .await
}

pub(crate) async fn admitted_response_sealer_with_snapshot_limits(
    descriptor: ComponentDescriptor,
    context: TrustedLiveRequestContext,
    encoded_snapshot: &[u8],
    base_revision: Revision,
    correlation_start: u8,
    max_response_bytes: Option<usize>,
    snapshot_limits: suprnova_live::snapshot::SnapshotLimits,
) -> VerifiedResponseSealing {
    admitted_response_sealer_with_semantics_and_snapshot_limits(
        descriptor,
        context,
        encoded_snapshot,
        base_revision,
        correlation_start,
        0x65,
        "execute",
        serde_json::json!({}),
        serde_json::json!({}),
        max_response_bytes,
        snapshot_limits,
    )
    .await
}

#[allow(
    clippy::too_many_arguments,
    reason = "the hostile fixture names each independently varied semantic request field"
)]
pub(crate) async fn admitted_response_sealer_with_semantics(
    descriptor: ComponentDescriptor,
    context: TrustedLiveRequestContext,
    encoded_snapshot: &[u8],
    base_revision: Revision,
    correlation_start: u8,
    idempotency_start: u8,
    action: &str,
    arguments: serde_json::Value,
    model_proposals: serde_json::Value,
) -> VerifiedResponseSealing {
    admitted_response_sealer_with_semantics_and_snapshot_limits(
        descriptor,
        context,
        encoded_snapshot,
        base_revision,
        correlation_start,
        idempotency_start,
        action,
        arguments,
        model_proposals,
        None,
        snapshot_limits(),
    )
    .await
}

#[allow(
    clippy::too_many_arguments,
    reason = "the hostile fixture names each independently varied semantic request field and limit"
)]
async fn admitted_response_sealer_with_semantics_and_snapshot_limits(
    descriptor: ComponentDescriptor,
    context: TrustedLiveRequestContext,
    encoded_snapshot: &[u8],
    base_revision: Revision,
    correlation_start: u8,
    idempotency_start: u8,
    action: &str,
    arguments: serde_json::Value,
    model_proposals: serde_json::Value,
    max_response_bytes: Option<usize>,
    snapshot_limits: suprnova_live::snapshot::SnapshotLimits,
) -> VerifiedResponseSealing {
    let input = suprnova_live::limits::InputLimits::new(64 * 1024, 12, 512, 40 * 1024)
        .expect("protocol input limits");
    let protocol = ProtocolLimits::new(ProtocolLimitConfig {
        input,
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
    .expect("protocol limits");
    let mut config = LiveEndpointConfig::new(protocol, snapshot_limits).expect("endpoint config");
    if let Some(max_response_bytes) = max_response_bytes {
        config = config
            .with_max_response_bytes(max_response_bytes)
            .expect("response limit");
    }
    let protocol_version = context.mount().protocol();
    let media = ParsedLiveMediaType::parse(if protocol_version == 1 {
        LIVE_MEDIA_TYPE_V1
    } else {
        LIVE_MEDIA_TYPE_V2
    })
    .expect("live media type");
    let correlation =
        CorrelationId::from_bytes(&bytes::<16>(correlation_start)).expect("correlation identity");
    let snapshot: serde_json::Value =
        serde_json::from_slice(encoded_snapshot).expect("snapshot JSON");
    let body = serde_json::to_vec(&serde_json::json!({
        "base_revision": base_revision.get().to_string(),
        "component": descriptor.metadata().identity().as_str(),
        "correlation_id": correlation.to_base64url(),
        "extensions": {},
        "idempotency_key": CorrelationId::from_bytes(&bytes::<16>(idempotency_start))
            .expect("idempotency bytes")
            .to_base64url(),
        "model_proposals": model_proposals,
        "operations": [{"arguments": arguments, "kind": "invoke_action", "name": action}],
        "protocol_version": protocol_version,
        "runtime_contract_version": 1,
        "snapshot": {"envelope": snapshot, "kind": "instance"},
        "snapshot_schema_version": 1,
    }))
    .expect("request JSON");
    let request = LiveEndpointRequest::try_new(
        Method::POST,
        media,
        Bytes::from(body),
        Some(context),
        RequestCachePolicy::Bypass,
    )
    .expect("endpoint request");
    let registry = ComponentRegistryBuilder::new()
        .register(descriptor)
        .expect("registry entry")
        .build();
    capture_verified_response_sealer(
        config,
        Arc::new(registry),
        Arc::new(ManualClock::new(1_000)),
        Arc::new(key_ring()),
        request,
    )
    .await
    .expect("verified response sealer")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FailurePoint {
    None,
    Mount,
    MountCallPanic,
    MetadataPanic,
    Hydrate,
    Bind,
    BeforeAction,
    Action,
    AfterAction,
    Rendering,
    Render,
    RenderCallPanic,
    RenderPanic,
    RenderFutureDropPanic,
    ExecutableRender,
    Rendered,
    Dehydrating,
    Dehydrate,
    InvalidSnapshotState,
    Teardown,
    TeardownCallPanic,
    DropPanic,
}

pub(crate) struct ActionGate {
    entered: Notify,
    release: Notify,
}

impl ActionGate {
    pub(crate) fn new() -> Self {
        Self {
            entered: Notify::new(),
            release: Notify::new(),
        }
    }

    pub(crate) fn mark_entered(&self) {
        self.entered.notify_one();
    }

    pub(crate) async fn wait_until_entered(&self) {
        self.entered.notified().await;
    }

    pub(crate) fn release(&self) {
        self.release.notify_one();
    }

    pub(crate) async fn wait_for_release(&self) {
        self.release.notified().await;
    }
}

struct DropPanicRenderFuture {
    output: Option<IslandRender>,
}

impl Future for DropPanicRenderFuture {
    type Output = Result<IslandRender, ComponentError>;

    fn poll(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(Ok(self.output.take().expect("polled only once")))
    }
}

impl Drop for DropPanicRenderFuture {
    fn drop(&mut self) {
        panic!("render future drop panic");
    }
}

impl Drop for TraceFixture {
    fn drop(&mut self) {
        if self.failure == FailurePoint::DropPanic {
            self.record("drop_panic");
            panic!("component drop panic");
        }
    }
}

pub(crate) struct TraceFixture {
    pub(crate) trace: Arc<Mutex<Vec<&'static str>>>,
    pub(crate) failure: FailurePoint,
    pub(crate) serial: u64,
    pub(crate) metadata: &'static ComponentMetadata,
    pub(crate) action_gate: Option<Arc<ActionGate>>,
}

impl TraceFixture {
    pub(crate) fn record(&self, value: &'static str) {
        self.trace.lock().expect("trace lock").push(value);
    }

    pub(crate) fn action_gate(&self) -> Option<Arc<ActionGate>> {
        self.action_gate.clone()
    }

    fn fail(&self, point: FailurePoint) -> Result<(), ComponentError> {
        if self.failure == point {
            Err(ComponentError::application_failure())
        } else {
            Ok(())
        }
    }
}

impl ComponentInstance for TraceFixture {
    fn metadata(&self) -> &'static ComponentMetadata {
        assert_ne!(self.failure, FailurePoint::MetadataPanic, "metadata panic");
        self.metadata
    }

    fn action_target(&mut self) -> &mut dyn suprnova_live::action::ActionTarget {
        self
    }

    fn hydrated<'a>(
        &'a mut self,
        context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async move {
            if let Some(browser) = context.browser() {
                assert!(!browser.document_key().as_str().is_empty());
            }
            self.record("hydrated");
            self.fail(FailurePoint::Hydrate)
        })
    }

    fn bind_models(
        &mut self,
        _proposals: &suprnova_live::state::ProposalBatch,
    ) -> Result<(), ComponentError> {
        self.record("bind");
        self.fail(FailurePoint::Bind)
    }

    fn before_action<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
        _action: &'a suprnova_live::identity::ActionName,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async move {
            self.record("before_action");
            self.fail(FailurePoint::BeforeAction)
        })
    }

    fn after_action<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
        _action: &'a suprnova_live::identity::ActionName,
        _result: &'a suprnova_live::action::ActionResult,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async move {
            self.record("after_action");
            self.fail(FailurePoint::AfterAction)
        })
    }

    fn rendering<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async move {
            self.record("rendering");
            self.fail(FailurePoint::Rendering)
        })
    }

    fn params_changed<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
        _parameters: &'a suprnova_live::child::VerifiedChildParametersV1,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async move {
            self.record("params_changed");
            Ok(())
        })
    }

    fn lazy_complete<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async move {
            self.record("lazy_complete");
            Ok(())
        })
    }

    fn render<'a>(
        &'a self,
        _context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<IslandRender, ComponentError>> {
        if self.failure == FailurePoint::RenderCallPanic {
            self.record("render");
            panic!("render call panic");
        }
        if self.failure == FailurePoint::RenderFutureDropPanic {
            self.record("render");
            return Box::pin(DropPanicRenderFuture {
                output: Some(IslandRender {
                    body: Bytes::from_static(b"<p>future</p>"),
                    assets: AssetSet::empty(),
                    children: vec![],
                }),
            });
        }
        Box::pin(async move {
            self.record("render");
            assert_ne!(self.failure, FailurePoint::RenderPanic, "render panic");
            self.fail(FailurePoint::Render)?;
            let body = if self.failure == FailurePoint::ExecutableRender {
                "<script data-suprnova-live-root=\"forged\"></script>".to_owned()
            } else {
                format!("<p>{}</p>", self.serial)
            };
            Ok(IslandRender {
                body: Bytes::from(body),
                assets: AssetSet::empty(),
                children: vec![],
            })
        })
    }

    fn rendered<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async move {
            self.record("rendered");
            self.fail(FailurePoint::Rendered)
        })
    }

    fn dehydrating<'a>(
        &'a mut self,
        _context: &'a RenderContext<'a>,
    ) -> LiveFuture<'a, Result<(), ComponentError>> {
        Box::pin(async move {
            self.record("dehydrating");
            self.fail(FailurePoint::Dehydrating)
        })
    }

    fn dehydrate(&self, _exposure: StateExposure) -> Result<CanonicalValue, ComponentError> {
        self.record("dehydrate");
        self.fail(FailurePoint::Dehydrate)?;
        if self.failure == FailurePoint::InvalidSnapshotState {
            return Ok(CanonicalValue::Object(BTreeMap::from([(
                "browser_forged".to_owned(),
                CanonicalValue::Bool(true),
            )])));
        }
        Ok(CanonicalValue::Object(BTreeMap::from([(
            "serial".to_owned(),
            CanonicalValue::String(self.serial.to_string()),
        )])))
    }

    fn dehydrate_memo(&self) -> Result<CanonicalValue, ComponentError> {
        self.record("memo");
        Ok(CanonicalValue::Object(BTreeMap::new()))
    }

    fn teardown<'a>(&'a mut self) -> LiveFuture<'a, Result<(), ComponentError>> {
        if self.failure == FailurePoint::TeardownCallPanic {
            self.record("teardown");
            panic!("teardown call panic");
        }
        Box::pin(async move {
            self.record("teardown");
            self.fail(FailurePoint::Teardown)
        })
    }
}

pub(crate) struct FixtureControl {
    pub(crate) trace: Arc<Mutex<Vec<&'static str>>>,
    pub(crate) failure: FailurePoint,
    pub(crate) next_serial: std::sync::atomic::AtomicU64,
    pub(crate) metadata: &'static ComponentMetadata,
    pub(crate) action_gate: Option<Arc<ActionGate>>,
}

impl FixtureControl {
    pub(crate) fn new(failure: FailurePoint) -> Arc<Self> {
        Arc::new(Self {
            trace: Arc::new(Mutex::new(Vec::new())),
            failure,
            next_serial: std::sync::atomic::AtomicU64::new(1),
            metadata: metadata(),
            action_gate: None,
        })
    }

    pub(crate) fn new_with_action_gate(
        failure: FailurePoint,
        action_gate: Arc<ActionGate>,
    ) -> Arc<Self> {
        Arc::new(Self {
            trace: Arc::new(Mutex::new(Vec::new())),
            failure,
            next_serial: std::sync::atomic::AtomicU64::new(1),
            metadata: metadata(),
            action_gate: Some(action_gate),
        })
    }

    pub(crate) fn new_with_metadata(
        failure: FailurePoint,
        component_metadata: &'static ComponentMetadata,
    ) -> Arc<Self> {
        Arc::new(Self {
            trace: Arc::new(Mutex::new(Vec::new())),
            failure,
            next_serial: std::sync::atomic::AtomicU64::new(1),
            metadata: component_metadata,
            action_gate: None,
        })
    }

    pub(crate) fn values(&self) -> Vec<&'static str> {
        self.trace.lock().expect("trace lock").clone()
    }

    fn instance(&self) -> TraceFixture {
        TraceFixture {
            trace: self.trace.clone(),
            failure: self.failure,
            serial: self
                .next_serial
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst),
            metadata: self.metadata,
            action_gate: self.action_gate.clone(),
        }
    }
}

pub(crate) fn install(control: Arc<FixtureControl>) -> ComponentHooks {
    ComponentHooks::new(Arc::new(FixtureFactory { control }))
}

struct FixtureFactory {
    control: Arc<FixtureControl>,
}

impl ComponentFactory for FixtureFactory {
    fn mount<'a>(
        &'a self,
        _context: &'a MountContext<'a>,
    ) -> LiveFuture<'a, Result<Box<dyn ComponentInstance>, ComponentError>> {
        if self.control.failure == FailurePoint::MountCallPanic {
            panic!("mount call panic");
        }
        Box::pin(async move {
            self.control.trace.lock().expect("trace lock").push("mount");
            if self.control.failure == FailurePoint::Mount {
                return Err(ComponentError::application_failure());
            }
            Ok(Box::new(self.control.instance()) as Box<dyn ComponentInstance>)
        })
    }

    fn hydrate<'a>(
        &'a self,
        _context: &'a HydrationContext<'a>,
    ) -> LiveFuture<'a, Result<Box<dyn ComponentInstance>, ComponentError>> {
        Box::pin(async move {
            self.control
                .trace
                .lock()
                .expect("trace lock")
                .push("reconstruct");
            Ok(Box::new(self.control.instance()) as Box<dyn ComponentInstance>)
        })
    }
}

pub(crate) fn metadata() -> &'static ComponentMetadata {
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
                    suprnova_live::identity::ActionName::parse("execute").expect("action identity"),
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

pub(crate) fn trusted_context() -> TrustedLiveRequestContext {
    trusted_context_with_port(None)
}

pub(crate) fn trusted_context_with_authorization(
    authorization: Arc<dyn ActionAuthorizationPort>,
) -> TrustedLiveRequestContext {
    trusted_context_with_port(Some(authorization))
}

pub(crate) fn trusted_context_with_upload_authorization(
    authorization: Arc<dyn UploadAuthorizationPort>,
) -> TrustedLiveRequestContext {
    trusted_context_for_with_ports(metadata(), None, Some(authorization), schema_set())
}

fn trusted_context_with_port(
    authorization: Option<Arc<dyn ActionAuthorizationPort>>,
) -> TrustedLiveRequestContext {
    trusted_context_for(metadata(), authorization)
}

pub(crate) fn trusted_context_for(
    component_metadata: &'static ComponentMetadata,
    authorization: Option<Arc<dyn ActionAuthorizationPort>>,
) -> TrustedLiveRequestContext {
    trusted_context_for_with_schemas(component_metadata, authorization, schema_set())
}

pub(crate) fn trusted_context_for_with_schemas(
    component_metadata: &'static ComponentMetadata,
    authorization: Option<Arc<dyn ActionAuthorizationPort>>,
    schemas: SnapshotSchemaSet,
) -> TrustedLiveRequestContext {
    trusted_context_for_with_ports(component_metadata, authorization, None, schemas)
}

fn trusted_context_for_with_ports(
    component_metadata: &'static ComponentMetadata,
    authorization: Option<Arc<dyn ActionAuthorizationPort>>,
    upload_authorization: Option<Arc<dyn UploadAuthorizationPort>>,
    schemas: SnapshotSchemaSet,
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
    let route = snapshot_support::route(0x30);
    let slot = IslandSlot::parse("trace").expect("slot identity");
    let catalog = MountCatalogBuilder::new()
        .register(
            &registry,
            MountCatalogEntry::new(
                ExpectedSeedV1::new(
                    contract,
                    BuildId::parse("build-lifecycle-tests").expect("build identity"),
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
        .expect("mount catalog entry")
        .build();
    let facts = fixture_host_scope();
    let mut builder = SyntheticLiveRequestContextBuilder::new(
        catalog,
        MountSelection::new(
            route,
            slot,
            component_metadata.identity().clone(),
            component_metadata.contract_digest().clone(),
            1,
        ),
        facts,
        UnixMillis::new(1_000),
        UnixMillis::new(2_000),
    );
    if let Some(authorization) = authorization {
        builder = builder.with_action_authorization(authorization);
    }
    if let Some(authorization) = upload_authorization {
        builder = builder.with_upload_authorization(authorization);
    }
    builder.build().expect("trusted context")
}

pub(crate) fn fixture_host_scope() -> HostScopeFacts {
    let scope =
        ScopeFingerprint::from_bytes(&snapshot_support::bytes::<32>(0x40)).expect("scope identity");
    HostScopeFacts::new(
        scope,
        Some(
            SessionFingerprint::from_bytes(&snapshot_support::bytes::<32>(0x41))
                .expect("session identity"),
        ),
        Some(
            PrincipalFingerprint::from_bytes(&snapshot_support::bytes::<32>(0x42))
                .expect("principal identity"),
        ),
        Some(
            TenantFingerprint::from_bytes(&snapshot_support::bytes::<32>(0x43))
                .expect("tenant identity"),
        ),
    )
}

pub(crate) fn schema_set() -> SnapshotSchemaSet {
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

pub(crate) fn key_ring() -> suprnova_live::crypto::SnapshotKeyRing {
    snapshot_support::key_ring()
}

pub(crate) fn snapshot_limits() -> suprnova_live::snapshot::SnapshotLimits {
    snapshot_support::snapshot_limits()
}

pub(crate) fn bytes<const LENGTH: usize>(start: u8) -> [u8; LENGTH] {
    snapshot_support::bytes::<LENGTH>(start)
}

#[derive(Debug)]
pub(crate) struct SequenceGenerator {
    next: AtomicU8,
    calls: AtomicUsize,
    fixed: bool,
}

impl SequenceGenerator {
    pub(crate) fn new(next: u8) -> Self {
        Self {
            next: AtomicU8::new(next),
            calls: AtomicUsize::new(0),
            fixed: false,
        }
    }

    pub(crate) fn fixed(value: u8) -> Self {
        Self {
            next: AtomicU8::new(value),
            calls: AtomicUsize::new(0),
            fixed: true,
        }
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl InstanceIdGenerator for SequenceGenerator {
    fn generate(&self) -> Result<InstanceId, RandomError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let start = if self.fixed {
            self.next.load(Ordering::SeqCst)
        } else {
            self.next.fetch_add(1, Ordering::SeqCst)
        };
        InstanceId::from_bytes(&snapshot_support::bytes::<16>(start))
            .map_err(|_| RandomError::generation_failed())
    }
}
