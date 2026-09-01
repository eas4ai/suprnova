//! Immutable process runtime assembled before Live routes are constructed.

use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

use crate::{App, FrameworkError};
use sha2::{Digest, Sha256};
use suprnova_live::clock::{Clock, SystemClock};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::endpoint::{
    EndpointError, LiveEndpointConfig, LiveEndpointService, ParsedLiveMediaType,
};
use suprnova_live::execution::ExecutionService;
use suprnova_live::host::{
    LiveRequestContextValidator, MountCatalog, MountCatalogBuilder, MountCatalogEntry,
    MountSelection, TrustedLiveRequestContext,
};
use suprnova_live::identity::{IslandSlot, KeyId, RouteIdentity, UnixMillis};
use suprnova_live::ledger::{LedgerLimits, LiveInstanceLedger, MemoryInstanceLedger};
use suprnova_live::limits::InputLimits;
use suprnova_live::mount::{
    DocumentMountKey, DocumentMountScope, MountFlags, MountLimits, MountProviders,
    PrivateMountOutput, PrivateMountRequest, PrivateMountService, PublicMountProviders,
    PublicSeedMountOutput, PublicSeedMountService,
};
use suprnova_live::promotion::{PromotionLimitConfig, PromotionLimits, PromotionService};
use suprnova_live::protocol::{ProtocolLimitConfig, ProtocolLimits};
use suprnova_live::random::{InstanceIdGenerator, SystemInstanceIdGenerator};
use suprnova_live::snapshot::{MountedDocumentPath, SnapshotLimits};
use suprnova_live::state::ProposalLimits;
use suprnova_live::validation::ValidationEngine;
use suprnova_live::view::{RenderLimits, ViewRenderer};
use uuid::Uuid;

use super::{LiveConfig, LiveRegistry};
use crate::Request;

struct RuntimeGraph {
    config: LiveConfig,
    registry: LiveRegistry,
    clock: Arc<dyn Clock>,
    random: Arc<dyn InstanceIdGenerator>,
    key_ring: Arc<SnapshotKeyRing>,
    ledger: Arc<dyn LiveInstanceLedger>,
    promotion: Arc<PromotionService>,
    public_mount: Arc<PublicSeedMountService>,
    private_mount: Arc<PrivateMountService>,
    execution: Arc<ExecutionService>,
    engine_registry: Arc<suprnova_live::registry::ComponentRegistry>,
    input_limits: InputLimits,
    proposal_limits: ProposalLimits,
    validation_engine: ValidationEngine,
    endpoint_config: LiveEndpointConfig,
    context_validator: LiveRequestContextValidator,
    ports: super::ports::HostPorts,
    mount_builder: Mutex<Option<MountCatalogBuilder>>,
    mount_catalog: OnceLock<Arc<MountCatalog>>,
}

#[derive(Clone, Copy)]
pub(super) enum RuntimeProviderSlot {
    Clock,
    Random,
    KeyRing,
    Ledger,
    Authorization,
    Transaction,
    Validation,
    EventReporter,
    Telemetry,
    Cancellation,
    ResponseIntent,
}

impl RuntimeProviderSlot {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Clock => "clock",
            Self::Random => "random",
            Self::KeyRing => "key ring",
            Self::Ledger => "instance ledger",
            Self::Authorization => "authorization",
            Self::Transaction => "transaction",
            Self::Validation => "validation",
            Self::EventReporter => "event reporter",
            Self::Telemetry => "telemetry",
            Self::Cancellation => "cancellation",
            Self::ResponseIntent => "response intent",
        }
    }
}

struct RuntimeProviderCandidates {
    clock: Option<Arc<dyn Clock>>,
    random: Option<Arc<dyn InstanceIdGenerator>>,
    key_ring: Option<Arc<SnapshotKeyRing>>,
    ledger: Option<Arc<dyn LiveInstanceLedger>>,
    ports: super::ports::HostPortCandidates,
}

struct RuntimeProviders {
    clock: Arc<dyn Clock>,
    random: Arc<dyn InstanceIdGenerator>,
    key_ring: Arc<SnapshotKeyRing>,
    ledger: Arc<dyn LiveInstanceLedger>,
    ports: super::ports::HostPorts,
}

impl RuntimeProviderCandidates {
    fn production(registry: &LiveRegistry) -> Result<Self, FrameworkError> {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let random: Arc<dyn InstanceIdGenerator> = Arc::new(SystemInstanceIdGenerator);
        let key_ring = Arc::new(build_key_ring()?);
        let ledger_limits =
            LedgerLimits::new(30_000, 604_800_000, 64, 100_000).map_err(|_| live_boot_error())?;
        let ledger: Arc<dyn LiveInstanceLedger> =
            Arc::new(MemoryInstanceLedger::new(Arc::clone(&clock), ledger_limits));
        Ok(Self {
            clock: Some(clock),
            random: Some(random),
            key_ring: Some(key_ring),
            ledger: Some(ledger),
            ports: super::ports::HostPortCandidates::production(registry),
        })
    }

    fn from_graph(graph: &RuntimeGraph) -> Self {
        Self {
            clock: Some(Arc::clone(&graph.clock)),
            random: Some(Arc::clone(&graph.random)),
            key_ring: Some(Arc::clone(&graph.key_ring)),
            ledger: Some(Arc::clone(&graph.ledger)),
            ports: graph.ports.candidates(),
        }
    }

    fn omit(&mut self, provider: RuntimeProviderSlot) {
        match provider {
            RuntimeProviderSlot::Clock => self.clock = None,
            RuntimeProviderSlot::Random => self.random = None,
            RuntimeProviderSlot::KeyRing => self.key_ring = None,
            RuntimeProviderSlot::Ledger => self.ledger = None,
            RuntimeProviderSlot::Authorization => self.ports.authorization = None,
            RuntimeProviderSlot::Transaction => self.ports.transaction = None,
            RuntimeProviderSlot::Validation => self.ports.validation = None,
            RuntimeProviderSlot::EventReporter => self.ports.reporter = None,
            RuntimeProviderSlot::Telemetry => self.ports.trace = None,
            RuntimeProviderSlot::Cancellation => self.ports.cancellation = None,
            RuntimeProviderSlot::ResponseIntent => self.ports.response = None,
        }
    }

    fn finalize(self) -> Result<RuntimeProviders, FrameworkError> {
        Ok(RuntimeProviders {
            clock: self.clock.ok_or_else(|| missing_provider("clock"))?,
            random: self.random.ok_or_else(|| missing_provider("random"))?,
            key_ring: self.key_ring.ok_or_else(|| missing_provider("key ring"))?,
            ledger: self
                .ledger
                .ok_or_else(|| missing_provider("instance ledger"))?,
            ports: self.ports.finalize(missing_provider)?,
        })
    }
}

/// Opaque immutable Live runtime bound in Suprnova's application container.
///
/// Applications may resolve this value to verify that Live was prepared, but
/// engine services and trust-bearing ports remain framework-owned.
#[derive(Clone)]
pub struct LiveRuntime {
    graph: Arc<RuntimeGraph>,
}

impl LiveRuntime {
    pub(crate) fn bind() -> Result<Self, FrameworkError> {
        if let Ok(runtime) = App::resolve::<Self>() {
            return Ok(runtime);
        }

        let config = App::resolve::<LiveConfig>().unwrap_or_default();
        let registry =
            App::resolve::<LiveRegistry>().unwrap_or_else(|_| LiveRegistry::builder().build());
        let candidates = RuntimeProviderCandidates::production(&registry)?;
        let runtime = assemble_runtime(config, registry, candidates)?;

        App::singleton_if_absent(runtime);
        App::resolve::<Self>().map_err(|_| {
            FrameworkError::internal("Live runtime could not be bound during server preparation")
        })
    }

    /// Returns the validated immutable Live configuration.
    #[must_use]
    pub fn config(&self) -> LiveConfig {
        self.graph.config
    }

    /// Returns the number of component contracts sealed into the runtime.
    #[must_use]
    pub fn registry_len(&self) -> usize {
        self.graph.registry.len()
    }

    pub(crate) fn same_instance(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.graph, &other.graph)
    }

    pub(crate) fn register_mount(&self, entry: MountCatalogEntry) -> Result<(), FrameworkError> {
        if self.graph.mount_catalog.get().is_some() {
            return Err(FrameworkError::internal(
                "Live mount registration is closed after route construction",
            ));
        }
        let mut builder = self
            .graph
            .mount_builder
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let current = builder.take().ok_or_else(live_boot_error)?;
        let next = current
            .register(self.graph.registry.engine(), entry)
            .map_err(|_| FrameworkError::internal("Live mount catalog was rejected"))?;
        *builder = Some(next);
        Ok(())
    }

    pub(crate) fn finalize_mount_catalog(&self) -> Result<(), FrameworkError> {
        if self.graph.mount_catalog.get().is_some() {
            return Ok(());
        }
        let mut builder = self
            .graph
            .mount_builder
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let builder = builder.take().ok_or_else(live_boot_error)?;
        self.graph
            .mount_catalog
            .set(Arc::new(builder.build()))
            .map_err(|_| live_boot_error())
    }

    pub(crate) fn prepare_request(
        &self,
        request: &mut Request,
        operation: super::attestation::LiveOperation,
    ) -> Result<(), FrameworkError> {
        let now = self.graph.clock.now().map_err(|_| live_boot_error())?;
        let expires_at = UnixMillis::new(
            now.get()
                .saturating_add(self.graph.config.max_context_lifetime_ms()),
        );
        request
            .prepare_live_operation_until_with_cancellation(
                operation,
                expires_at,
                self.graph.ports.cancellation.attach(),
            )
            .then_some(())
            .ok_or_else(|| FrameworkError::internal("Live request preparation was rejected"))
    }

    pub(crate) fn validate_request_context(
        &self,
        request: &Request,
        current_route: RouteIdentity,
        current_slot: IslandSlot,
        selection: MountSelection,
    ) -> Result<TrustedLiveRequestContext, FrameworkError> {
        self.finalize_mount_catalog()?;
        let now = self.graph.clock.now().map_err(|_| live_boot_error())?;
        let candidate = super::context::candidate(
            request,
            current_route,
            current_slot,
            selection,
            Arc::clone(&self.graph.ports.authorization),
        )?;
        let catalog = self.graph.mount_catalog.get().ok_or_else(live_boot_error)?;
        self.graph
            .context_validator
            .validate(catalog, candidate, now)
            .map_err(|_| FrameworkError::internal("Live request context was rejected"))
    }

    pub(crate) fn inspect_mount(
        &self,
        body: &[u8],
        media: ParsedLiveMediaType,
    ) -> Result<MountSelection, EndpointError> {
        self.graph.endpoint_config.inspect_mount(body, media)
    }

    pub(crate) fn endpoint_service(
        &self,
    ) -> (
        LiveEndpointService,
        Arc<super::ports::response::PreparedResponseCompletion>,
    ) {
        let completion = Arc::new(super::ports::response::PreparedResponseCompletion::default());
        let kernel = super::action::SuprnovaEndpointKernel::new(
            Arc::clone(&self.graph.promotion),
            Arc::clone(&self.graph.execution),
            self.graph.input_limits,
            self.graph.proposal_limits,
            self.graph.validation_engine,
            &self.graph.ports,
            Arc::clone(&completion),
        );
        (
            LiveEndpointService::new(
                self.graph.endpoint_config.clone(),
                Arc::clone(&self.graph.engine_registry),
                Arc::clone(&self.graph.clock),
                Arc::clone(&self.graph.key_ring),
                Arc::new(kernel),
            ),
            completion,
        )
    }

    pub(crate) async fn mount_public_component(
        &self,
        document: &mut DocumentMountScope,
        key: DocumentMountKey,
        parameters: suprnova_live::canonical::CanonicalValue,
        flags: MountFlags,
        document_path: &MountedDocumentPath,
        context: &TrustedLiveRequestContext,
    ) -> Result<PublicSeedMountOutput, FrameworkError> {
        self.graph
            .public_mount
            .mount_component_for_document(document, key, parameters, flags, document_path, context)
            .await
            .map_err(|_| FrameworkError::internal("Live public mount was rejected"))
    }

    pub(crate) async fn mount_private_component(
        &self,
        document: &mut DocumentMountScope,
        request: PrivateMountRequest,
        context: &TrustedLiveRequestContext,
    ) -> Result<PrivateMountOutput, FrameworkError> {
        self.graph
            .private_mount
            .mount(document, request, context)
            .await
            .map_err(|_| FrameworkError::internal("Live private mount was rejected"))
    }

    pub(crate) fn readiness(&self) -> RuntimeReadiness {
        RuntimeReadiness {
            clock: Arc::strong_count(&self.graph.clock) > 0,
            random: Arc::strong_count(&self.graph.random) > 0,
            key_ring: Arc::strong_count(&self.graph.key_ring) > 0,
            ledger: Arc::strong_count(&self.graph.ledger) > 0,
            promotion: Arc::strong_count(&self.graph.promotion) > 0,
            execution: Arc::strong_count(&self.graph.execution) > 0,
            context_validator: self.graph.config.max_context_lifetime_ms() > 0,
            host_ports: Arc::strong_count(&self.graph.ports.authorization) > 0
                && Arc::strong_count(&self.graph.ports.transaction) > 0
                && Arc::strong_count(&self.graph.ports.validation) > 0
                && Arc::strong_count(&self.graph.ports.reporter) > 0
                && Arc::strong_count(&self.graph.ports.trace) > 0,
            response_and_cancellation: Arc::strong_count(&self.graph.ports.response) > 0
                && Arc::strong_count(&self.graph.ports.cancellation) > 0,
            mount_catalog: self.graph.mount_catalog.get().is_some(),
        }
    }
}

fn assemble_runtime(
    config: LiveConfig,
    registry: LiveRegistry,
    candidates: RuntimeProviderCandidates,
) -> Result<LiveRuntime, FrameworkError> {
    let RuntimeProviders {
        clock,
        random,
        key_ring,
        ledger,
        ports,
    } = candidates.finalize()?;
    let input = InputLimits::new(
        config.max_request_bytes(),
        32,
        8_192,
        config.max_request_bytes().min(1024 * 1024),
    )
    .map_err(|_| live_boot_error())?;
    let snapshot_limits = SnapshotLimits::new(input, 5_000, 86_400_000, 604_800_000, 1_024, 1_024)
        .map_err(|_| live_boot_error())?;
    let protocol_limits = ProtocolLimits::new(ProtocolLimitConfig {
        input,
        max_snapshot_bytes: config.max_request_bytes(),
        max_html_bytes: config.max_response_bytes(),
        max_model_proposals: 128,
        max_operations: 128,
        max_arguments: 128,
        max_validation_entries: 128,
        max_events: 128,
        max_effects: 128,
        max_extensions: 128,
    })
    .map_err(|_| live_boot_error())?;
    let endpoint_config = LiveEndpointConfig::new(protocol_limits, snapshot_limits.clone())
        .and_then(|endpoint| endpoint.with_max_response_bytes(config.max_response_bytes()))
        .map_err(|_| live_boot_error())?;
    let render_limits = RenderLimits::new(
        config.max_response_bytes(),
        128,
        128,
        128,
        config.max_response_bytes().min(512 * 1024),
    )
    .map_err(|_| live_boot_error())?;
    let renderer = ViewRenderer::new(render_limits).map_err(|_| live_boot_error())?;
    let proposal_limits = ProposalLimits::new(128, 32, input).map_err(|_| live_boot_error())?;
    let validation_engine = ValidationEngine::new(128).map_err(|_| live_boot_error())?;
    let engine_registry = Arc::new(registry.engine().clone());
    let promotion_limits = PromotionLimits::new(PromotionLimitConfig {
        max_seed_bytes: config.max_request_bytes(),
        window_ms: 60_000,
        max_promotions_per_window: 256,
        max_outstanding_per_scope: 1_024,
        max_outstanding_per_route_component: 512,
        promotion_lease_ms: 30_000,
        abandoned_retention_ms: 300_000,
        instance_lifetime_ms: 604_800_000,
        max_reservations: 100_000,
        max_rate_buckets: 100_000,
    })
    .map_err(|_| live_boot_error())?;
    let promotion = Arc::new(
        PromotionService::new(
            Arc::clone(&ledger),
            Arc::clone(&clock),
            Arc::clone(&random),
            Arc::clone(&key_ring),
            snapshot_limits.clone(),
            promotion_limits,
        )
        .map_err(|_| live_boot_error())?,
    );
    let public_mount = Arc::new(
        PublicSeedMountService::new(
            PublicMountProviders::new(
                Arc::clone(&engine_registry),
                Arc::clone(&clock),
                Arc::clone(&key_ring),
            ),
            snapshot_limits.clone(),
            renderer,
            config.max_response_bytes(),
        )
        .map_err(|_| live_boot_error())?,
    );
    let private_mount = Arc::new(
        PrivateMountService::new(
            MountProviders::new(
                Arc::clone(&engine_registry),
                Arc::clone(&ledger),
                Arc::clone(&clock),
                Arc::clone(&random),
                Arc::clone(&key_ring),
            ),
            snapshot_limits.clone(),
            renderer,
            MountLimits::new(604_800_000, 8, config.max_response_bytes(), 64)
                .map_err(|_| live_boot_error())?,
        )
        .map_err(|_| live_boot_error())?,
    );
    let execution = Arc::new(
        ExecutionService::new(
            Arc::clone(&ledger),
            Arc::clone(&clock),
            Arc::clone(&key_ring),
            snapshot_limits,
            renderer,
        )
        .with_reporter(Arc::clone(&ports.reporter)),
    );
    let context_validator = LiveRequestContextValidator::new(config.max_context_lifetime_ms())
        .map_err(|_| live_boot_error())?;

    Ok(LiveRuntime {
        graph: Arc::new(RuntimeGraph {
            config,
            registry,
            clock,
            random,
            key_ring,
            ledger,
            promotion,
            public_mount,
            private_mount,
            execution,
            engine_registry,
            input_limits: input,
            proposal_limits,
            validation_engine,
            endpoint_config,
            context_validator,
            ports,
            mount_builder: Mutex::new(Some(MountCatalogBuilder::new())),
            mount_catalog: OnceLock::new(),
        }),
    })
}

pub(super) fn assemble_for_harness(
    config: LiveConfig,
    registry: LiveRegistry,
) -> Result<LiveRuntime, FrameworkError> {
    let candidates = RuntimeProviderCandidates::production(&registry)?;
    assemble_runtime(config, registry, candidates)
}

pub(super) fn validate_provider_omission(
    runtime: &LiveRuntime,
    provider: RuntimeProviderSlot,
) -> Result<(), FrameworkError> {
    let mut candidates = RuntimeProviderCandidates::from_graph(&runtime.graph);
    candidates.omit(provider);
    candidates.finalize().map(|_| ())
}

pub(super) fn assemble_with_clock_override(
    runtime: &LiveRuntime,
    clock: Arc<dyn Clock>,
) -> Result<LiveRuntime, FrameworkError> {
    let mut candidates = RuntimeProviderCandidates::from_graph(&runtime.graph);
    let ledger_limits =
        LedgerLimits::new(30_000, 604_800_000, 64, 100_000).map_err(|_| live_boot_error())?;
    candidates.clock = Some(Arc::clone(&clock));
    candidates.ledger = Some(Arc::new(MemoryInstanceLedger::new(clock, ledger_limits)));
    assemble_runtime(
        runtime.graph.config,
        runtime.graph.registry.clone(),
        candidates,
    )
}

pub(crate) struct RuntimeReadiness {
    pub(crate) clock: bool,
    pub(crate) random: bool,
    pub(crate) key_ring: bool,
    pub(crate) ledger: bool,
    pub(crate) promotion: bool,
    pub(crate) execution: bool,
    pub(crate) context_validator: bool,
    pub(crate) host_ports: bool,
    pub(crate) mount_catalog: bool,
    pub(crate) response_and_cancellation: bool,
}

fn build_key_ring() -> Result<SnapshotKeyRing, FrameworkError> {
    let current = crate::crypto::Crypt::current_key_bytes().ok_or_else(live_boot_error)?;
    let active = key_record(current, true)?;
    let verification = crate::crypto::Crypt::previous_key_bytes()
        .into_iter()
        .map(|bytes| key_record(bytes, false))
        .collect::<Result<Vec<_>, _>>()?;
    SnapshotKeyRing::new(active, verification).map_err(|_| live_boot_error())
}

fn key_record(bytes: Vec<u8>, active: bool) -> Result<KeyRecord, FrameworkError> {
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    let key_id_text = Uuid::from_bytes(digest[..16].try_into().expect("fixed digest"))
        .simple()
        .to_string();
    let key_id = KeyId::parse(&key_id_text).map_err(|_| live_boot_error())?;
    let root = RootKey::new(bytes).map_err(|_| live_boot_error())?;
    let sign_until = if active { u64::MAX - 1 } else { 1 };
    KeyRecord::new(
        key_id,
        root,
        UnixMillis::new(0),
        UnixMillis::new(sign_until),
        UnixMillis::new(u64::MAX),
    )
    .map_err(|_| live_boot_error())
}

fn live_boot_error() -> FrameworkError {
    FrameworkError::internal("Live runtime configuration was rejected")
}

fn missing_provider(name: &'static str) -> FrameworkError {
    FrameworkError::internal(format!("missing Live runtime provider: {name}"))
}

impl fmt::Debug for LiveRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<LiveRuntime:redacted>")
    }
}
