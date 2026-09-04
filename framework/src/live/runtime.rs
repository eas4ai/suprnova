//! Immutable process runtime assembled before Live routes are constructed.

use std::fmt;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::{App, FrameworkError};
use sha2::{Digest, Sha256};
use suprnova_live::async_updates::{
    StreamPosition, SubscriptionCredentialPort, TrustedMountParameters,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::child::{
    AcceptedParentRevision, ChildParameterLimits, ExpectedChildParametersV2,
    PreparedChildParametersV1, PreparedChildParametersV2, authorize_child_parameters_v2,
    verify_child_parameters_v2,
};
use suprnova_live::clock::{Clock, SystemClock};
use suprnova_live::component::composition::{
    ChildDeclaration, ChildKey, ChildState, CompositionAncestry, CompositionLimits,
    CompositionPlanner,
};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::endpoint::{
    EndpointError, LiveEndpointConfig, LiveEndpointService, ParsedLiveMediaType,
};
use suprnova_live::execution::ExecutionService;
use suprnova_live::host::{
    LiveRequestContextValidator, MountCatalog, MountCatalogBuilder, MountCatalogEntry,
    MountSelection, TrustedLiveRequestContext,
};
use suprnova_live::identity::{
    BuildId, ComponentName, ContentDigest, IdempotencyKey, InstanceId, IslandSlot, KeyId,
    ModelField, Revision, RouteIdentity, ScopeFingerprint, UnixMillis,
};
use suprnova_live::ledger::{
    AcceptedOutcome, AcceptedOutcomeKind, ClaimOutcome, ClaimRequest, LedgerLimits,
    LiveInstanceLedger, MemoryInstanceLedger, MountInstanceRecord,
};
use suprnova_live::limits::{InputLimits, UploadLimitConfig, UploadLimits};
use suprnova_live::mount::{
    DocumentMountKey, DocumentMountScope, MountFlags, MountLimits, MountProviders,
    PrivateMountOutput, PrivateMountRequest, PrivateMountService, PublicMountProviders,
    PublicSeedMountOutput, PublicSeedMountService,
};
use suprnova_live::promotion::{PromotionLimitConfig, PromotionLimits, PromotionService};
use suprnova_live::protocol::{ProtocolLimitConfig, ProtocolLimits};
use suprnova_live::random::{InstanceIdGenerator, SystemInstanceIdGenerator};
use suprnova_live::resource::PermitPool;
use suprnova_live::snapshot::{
    ComponentContract as SnapshotContract, CompositionChildLineageV1, CompositionLineageV1,
    CompositionOwnerLineageV1, ExpectedInstanceV1, InstanceBodyV1, InstanceFieldsV1,
    MountedDocumentPath, SnapshotLimits, verify_instance,
};
use suprnova_live::state::ProposalLimits;
use suprnova_live::upload::{
    BoundedBackoff, CleanupPolicy, TransferGrantCodec, UploadCleanupService, UploadControlKind,
    UploadFieldPolicy, UploadFinalizationService, UploadHandle, UploadIdempotencyKey, UploadRecord,
    UploadService, UploadValidationService,
};
use suprnova_live::validation::ValidationEngine;
use suprnova_live::view::{RenderLimits, ViewRenderer};
use uuid::Uuid;

use super::async_updates::{AsyncErrorKind, AsyncState};
use super::context::SubscriptionCapabilities;
use super::ports::subscription::{FixedSubscriptionBaseline, SuprnovaSubscriptionRegistry};
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
    snapshot_limits: SnapshotLimits,
    endpoint_config: LiveEndpointConfig,
    context_validator: LiveRequestContextValidator,
    ports: super::ports::HostPorts,
    uploads: UploadRuntimeGraph,
    async_state: Arc<AsyncState>,
    mount_builder: Mutex<Option<MountCatalogBuilder>>,
    mount_catalog: OnceLock<Arc<MountCatalog>>,
    upload_mount_builder: Mutex<Option<Vec<UploadMountSelector>>>,
    upload_mounts: OnceLock<Arc<[UploadMountSelector]>>,
    mount_kind_builder: Mutex<Option<Vec<MountKindRecord>>>,
    mount_kinds: OnceLock<Arc<[MountKindRecord]>>,
}

/// Framework-side record of one registered mount's declared kind.
///
/// The engine catalog owns the scope requirements; this record lets the
/// action boundary close the identity absences a public seed permits before
/// the engine validates the request against that same catalog.
#[derive(Clone)]
struct MountKindRecord {
    route: RouteIdentity,
    slot: IslandSlot,
    kind: super::document::LiveMountKind,
}

pub(crate) struct LiveMountRegistration {
    entry: MountCatalogEntry,
    upload_selector: UploadMountSelector,
    kind: super::document::LiveMountKind,
}

impl LiveMountRegistration {
    pub(crate) const fn new(
        entry: MountCatalogEntry,
        selection: MountSelection,
        document_key: DocumentMountKey,
        build: BuildId,
        kind: super::document::LiveMountKind,
    ) -> Self {
        Self {
            entry,
            upload_selector: UploadMountSelector {
                selection,
                document_key,
                build,
            },
            kind,
        }
    }
}

#[derive(Clone)]
struct UploadMountSelector {
    selection: MountSelection,
    document_key: DocumentMountKey,
    build: BuildId,
}

impl UploadMountSelector {
    fn scope_binding(
        &self,
        base_scope: ScopeFingerprint,
    ) -> super::upload::UploadMountScopeBinding {
        super::upload::UploadMountScopeBinding {
            base_scope,
            route: self.selection.route().clone(),
            slot: self.selection.slot().clone(),
            component: self.selection.component().clone(),
            contract: self.selection.contract_digest().clone(),
            build: self.build.clone(),
            document_key: self.document_key.clone(),
            protocol: self.selection.protocol(),
        }
    }
}

pub(super) struct UploadMountAuthorityTestFixture {
    pub(super) binding: super::upload::UploadMountScopeBinding,
    pub(super) scope: ScopeFingerprint,
}

pub(super) struct UploadMountResolutionTestFixture {
    pub(super) slot: IslandSlot,
    pub(super) document_key: DocumentMountKey,
}

struct UploadRuntimeGraph {
    limits: UploadLimits,
    body_budget: super::upload::UploadBodyBudget,
    authority: Arc<UploadService>,
    validation: Arc<UploadValidationService>,
    finalization: Arc<UploadFinalizationService>,
    cleanup: Arc<UploadCleanupService>,
    cleanup_runner: UploadCleanupRunner,
}

struct UploadCleanupRunner {
    cleanup: Arc<UploadCleanupService>,
    wake: Arc<tokio::sync::Notify>,
    started: AtomicBool,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl UploadCleanupRunner {
    fn new(cleanup: Arc<UploadCleanupService>) -> Self {
        Self {
            cleanup,
            wake: Arc::new(tokio::sync::Notify::new()),
            started: AtomicBool::new(false),
            task: Mutex::new(None),
        }
    }

    fn ensure_started(&self) -> Result<(), FrameworkError> {
        if self.started.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let handle = tokio::runtime::Handle::try_current().map_err(|_| live_boot_error())?;
        let cleanup = Arc::clone(&self.cleanup);
        let wake = Arc::clone(&self.wake);
        let task = handle.spawn(async move {
            let lease =
                suprnova_live::upload::CleanupLeaseId::parse("framework-production-upload-cleanup")
                    .expect("static cleanup lease identity");
            loop {
                tokio::select! {
                    () = wake.notified() => {}
                    () = tokio::time::sleep(Duration::from_secs(30)) => {}
                }
                let _ = cleanup.run_once(lease.clone()).await;
            }
        });
        *self
            .task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(task);
        Ok(())
    }

    fn wake(&self) -> Result<(), FrameworkError> {
        self.ensure_started()?;
        self.wake.notify_one();
        Ok(())
    }
}

impl Drop for UploadCleanupRunner {
    fn drop(&mut self) {
        self.cleanup.cancel();
        if let Some(task) = self
            .task
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            task.abort();
        }
    }
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
    UploadLedger,
    UploadCleanupLedger,
    UploadQuarantine,
    UploadProvider,
    UploadReverseProxy,
    UploadReverseProxyProgress,
    UploadDirect,
    UploadAuthorizationAdapter,
    UploadAuthorization,
    UploadScanner,
    UploadApplicationValidation,
    UploadEvidence,
    UploadFinalizer,
    SubscriptionAuthorization,
    SubscriptionCredentials,
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
            Self::UploadLedger => "upload ledger",
            Self::UploadCleanupLedger => "upload cleanup ledger",
            Self::UploadQuarantine => "upload quarantine",
            Self::UploadProvider => "upload provider",
            Self::UploadReverseProxy => "upload reverse-proxy provider",
            Self::UploadReverseProxyProgress => "upload reverse-proxy progress",
            Self::UploadDirect => "upload direct provider",
            Self::UploadAuthorizationAdapter => "upload authorization adapter",
            Self::UploadAuthorization => "upload authorization",
            Self::UploadScanner => "upload scanner",
            Self::UploadApplicationValidation => "upload application validation",
            Self::UploadEvidence => "upload validation evidence",
            Self::UploadFinalizer => "upload finalizer",
            Self::SubscriptionAuthorization => "subscription authorization",
            Self::SubscriptionCredentials => "subscription credentials",
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
            ports: super::ports::HostPortCandidates::production(registry)?,
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
            RuntimeProviderSlot::UploadLedger => self.ports.upload_ledger = None,
            RuntimeProviderSlot::UploadCleanupLedger => self.ports.upload_cleanup_ledger = None,
            RuntimeProviderSlot::UploadQuarantine => self.ports.upload_quarantine = None,
            RuntimeProviderSlot::UploadProvider => {
                self.ports.upload_provider = None;
                self.ports.upload_provider_adapter = None;
            }
            RuntimeProviderSlot::UploadReverseProxy => self.ports.upload_reverse_proxy = None,
            RuntimeProviderSlot::UploadReverseProxyProgress => {
                self.ports.upload_reverse_proxy_adapter = None;
            }
            RuntimeProviderSlot::UploadDirect => self.ports.upload_direct = None,
            RuntimeProviderSlot::UploadAuthorizationAdapter => {
                self.ports.upload_authorization_adapter = None;
            }
            RuntimeProviderSlot::UploadAuthorization => self.ports.upload_authorization = None,
            RuntimeProviderSlot::UploadScanner => self.ports.upload_scanner = None,
            RuntimeProviderSlot::UploadApplicationValidation => {
                self.ports.upload_application_validation = None;
            }
            RuntimeProviderSlot::UploadEvidence => self.ports.upload_evidence = None,
            RuntimeProviderSlot::UploadFinalizer => self.ports.upload_finalizer = None,
            RuntimeProviderSlot::SubscriptionAuthorization => {
                self.ports.subscription_authorization = None;
            }
            RuntimeProviderSlot::SubscriptionCredentials => {
                self.ports.subscription_credentials = None;
            }
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

pub(super) struct ChildParameterTestFixture {
    pub(super) child_snapshot: Vec<u8>,
    pub(super) envelope: Vec<u8>,
    pub(super) historical_v1_envelope: Vec<u8>,
    pub(super) parent_snapshot: Vec<u8>,
    pub(super) scope: ScopeFingerprint,
    pub(super) parent_instance: InstanceId,
    pub(super) child_instance: InstanceId,
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

    #[allow(
        clippy::too_many_arguments,
        reason = "the test-only fixture keeps every independently signed binding explicit"
    )]
    pub(super) async fn prepare_child_parameter_fixture_for_test(
        &self,
        parent_component: &ComponentName,
        parent_route: &RouteIdentity,
        parent_slot: &IslandSlot,
        parent_build_override: Option<BuildId>,
        child_component: &ComponentName,
        child_route: &RouteIdentity,
        child_slot: &IslandSlot,
        scope: ScopeFingerprint,
        previous_parameters: CanonicalValue,
        next_parameters: CanonicalValue,
        parent_state: CanonicalValue,
        child_state: CanonicalValue,
    ) -> Result<ChildParameterTestFixture, FrameworkError> {
        let parent = self
            .graph
            .engine_registry
            .resolve(parent_component)
            .map_err(|_| FrameworkError::internal("child fixture parent component rejected"))?;
        let child = self
            .graph
            .engine_registry
            .resolve(child_component)
            .map_err(|_| FrameworkError::internal("child fixture child component rejected"))?;
        let planner = CompositionPlanner::new(
            CompositionLimits::new(8, 8, 64 * 1024, 8).map_err(|_| live_boot_error())?,
        );
        let ancestry = CompositionAncestry::root(parent_component.clone());
        let declaration = |parameters| {
            ChildDeclaration::new(
                ChildKey::parse("delivered-child").expect("static child key"),
                child_component.clone(),
                parameters,
            )
        };
        let initial = planner
            .reconcile(
                &self.graph.engine_registry,
                &ancestry,
                &[],
                vec![declaration(previous_parameters)],
            )
            .map_err(|_| FrameworkError::internal("child fixture initial composition rejected"))?;
        let [ChildState::Remount(prepared)] = initial.as_slice() else {
            return Err(live_boot_error());
        };
        let child_instance = self
            .graph
            .random
            .generate()
            .map_err(|_| live_boot_error())?;
        let handle = prepared.clone().into_handle(child_instance.clone());
        let changed = planner
            .reconcile(
                &self.graph.engine_registry,
                &ancestry,
                std::slice::from_ref(&handle),
                vec![declaration(next_parameters)],
            )
            .map_err(|_| FrameworkError::internal("child fixture changed composition rejected"))?;
        let [ChildState::PendingParams(pending)] = changed.as_slice() else {
            return Err(live_boot_error());
        };

        let parent_instance = self
            .graph
            .random
            .generate()
            .map_err(|_| live_boot_error())?;
        let now = self.graph.clock.now().map_err(|_| live_boot_error())?;
        let expires_at = UnixMillis::new(now.get().saturating_add(300_000));
        self.graph
            .ledger
            .mount_instance(MountInstanceRecord::new(
                scope.clone(),
                parent_instance.clone(),
                parent.contract_digest().clone(),
                Revision::new(0),
                expires_at,
            ))
            .await
            .map_err(|_| live_boot_error())?;
        self.graph
            .ledger
            .mount_instance(MountInstanceRecord::new(
                scope.clone(),
                child_instance.clone(),
                child.contract_digest().clone(),
                Revision::new(0),
                expires_at,
            ))
            .await
            .map_err(|_| live_boot_error())?;
        let idempotency_bytes: [u8; 16] = Sha256::digest(b"framework-child-parent-acceptance")
            [..16]
            .try_into()
            .map_err(|_| live_boot_error())?;
        let claim = ClaimRequest::new(
            scope.clone(),
            parent_instance.clone(),
            Revision::new(0),
            IdempotencyKey::from_bytes(&idempotency_bytes).map_err(|_| live_boot_error())?,
            ContentDigest::from_bytes(&Sha256::digest(b"framework-child-parent-request"))
                .map_err(|_| live_boot_error())?,
        );
        let grant = match self
            .graph
            .ledger
            .claim(claim.clone())
            .await
            .map_err(|_| live_boot_error())?
        {
            ClaimOutcome::Granted(grant) => grant,
            _ => return Err(live_boot_error()),
        };
        self.graph
            .ledger
            .commit(
                &grant.into_token(),
                AcceptedOutcome::new(
                    AcceptedOutcomeKind::Rendered,
                    ContentDigest::from_bytes(&Sha256::digest(b"framework-child-parent-output"))
                        .map_err(|_| live_boot_error())?,
                ),
            )
            .await
            .map_err(|_| live_boot_error())?;
        let accepted = match self
            .graph
            .ledger
            .claim(claim)
            .await
            .map_err(|_| live_boot_error())?
        {
            ClaimOutcome::Accepted(accepted) => accepted,
            _ => return Err(live_boot_error()),
        };
        let parent_revision = accepted.successor_revision();
        let accepted_parent = AcceptedParentRevision::from_accepted_outcome(&accepted);
        let build = BuildId::parse(concat!("suprnova-", env!("CARGO_PKG_VERSION")))
            .map_err(|_| live_boot_error())?;
        let parent_build = parent_build_override.unwrap_or_else(|| build.clone());
        let parent_contract = SnapshotContract::new(
            parent_component.clone(),
            parent.contract_digest().clone(),
            parent
                .snapshot_schemas()
                .map_err(|_| live_boot_error())?
                .state()
                .version(),
            parent
                .snapshot_schemas()
                .map_err(|_| live_boot_error())?
                .memo()
                .version(),
            parent
                .snapshot_schemas()
                .map_err(|_| live_boot_error())?
                .mount()
                .version(),
        )
        .map_err(|_| live_boot_error())?;
        let child_contract = SnapshotContract::new(
            child_component.clone(),
            child.contract_digest().clone(),
            child
                .snapshot_schemas()
                .map_err(|_| live_boot_error())?
                .state()
                .version(),
            child
                .snapshot_schemas()
                .map_err(|_| live_boot_error())?
                .memo()
                .version(),
            child
                .snapshot_schemas()
                .map_err(|_| live_boot_error())?
                .mount()
                .version(),
        )
        .map_err(|_| live_boot_error())?;
        let mut parent_fields = InstanceFieldsV1 {
            component: parent_contract.clone(),
            build_id: parent_build.clone(),
            route: parent_route.clone(),
            slot: parent_slot.clone(),
            key_id: self.graph.key_ring.active_key_id().clone(),
            scope: scope.clone(),
            instance_id: parent_instance.clone(),
            revision: parent_revision,
            issued_at: now,
            expires_at,
            state: parent_state,
            memo: CanonicalValue::Object(Default::default()),
            extensions: Default::default(),
        };
        parent_fields
            .set_composition_lineage(
                CompositionLineageV1::new(
                    None,
                    vec![
                        CompositionChildLineageV1::new(
                            parent_instance.clone(),
                            parent_revision,
                            pending.child().key().clone(),
                            child.contract_digest().clone(),
                            child_instance.clone(),
                            1,
                        )
                        .map_err(|_| live_boot_error())?,
                    ],
                )
                .map_err(|_| live_boot_error())?,
            )
            .map_err(|_| live_boot_error())?;
        let parent_snapshot = InstanceBodyV1::new(
            parent_fields,
            &parent.snapshot_schemas().map_err(|_| live_boot_error())?,
            &self.graph.snapshot_limits,
        )
        .map_err(|error| {
            FrameworkError::internal(format!("child fixture parent snapshot rejected: {error}"))
        })?
        .sign(&self.graph.key_ring, now, &self.graph.snapshot_limits)
        .map_err(|_| live_boot_error())?;
        let verified_parent = verify_instance(
            &parent_snapshot,
            &ExpectedInstanceV1::new(
                parent_contract,
                parent_build,
                parent_route.clone(),
                parent_slot.clone(),
                scope.clone(),
                parent.snapshot_schemas().map_err(|_| live_boot_error())?,
            ),
            &self.graph.key_ring,
            now,
            &self.graph.snapshot_limits,
        )
        .map_err(|error| {
            FrameworkError::internal(format!(
                "parent fixture self-verification rejected: {error}"
            ))
        })?;
        let mut child_fields = InstanceFieldsV1 {
            component: child_contract.clone(),
            build_id: build.clone(),
            route: child_route.clone(),
            slot: child_slot.clone(),
            key_id: self.graph.key_ring.active_key_id().clone(),
            scope: scope.clone(),
            instance_id: child_instance.clone(),
            revision: Revision::new(0),
            issued_at: now,
            expires_at,
            state: child_state,
            memo: CanonicalValue::Object(Default::default()),
            extensions: Default::default(),
        };
        child_fields
            .set_composition_lineage(
                CompositionLineageV1::new(
                    Some(
                        CompositionOwnerLineageV1::new(
                            parent_instance.clone(),
                            Revision::new(0),
                            pending.child().key().clone(),
                            child.contract_digest().clone(),
                            child_instance.clone(),
                            1,
                        )
                        .map_err(|_| live_boot_error())?,
                    ),
                    Vec::new(),
                )
                .map_err(|_| live_boot_error())?,
            )
            .map_err(|_| live_boot_error())?;
        let child_snapshot = InstanceBodyV1::new(
            child_fields,
            &child.snapshot_schemas().map_err(|_| live_boot_error())?,
            &self.graph.snapshot_limits,
        )
        .map_err(|error| {
            FrameworkError::internal(format!("child fixture child snapshot rejected: {error}"))
        })?
        .sign(&self.graph.key_ring, now, &self.graph.snapshot_limits)
        .map_err(|_| live_boot_error())?;
        verify_instance(
            &child_snapshot,
            &ExpectedInstanceV1::new(
                child_contract,
                build,
                child_route.clone(),
                child_slot.clone(),
                scope.clone(),
                child.snapshot_schemas().map_err(|_| live_boot_error())?,
            ),
            &self.graph.key_ring,
            now,
            &self.graph.snapshot_limits,
        )
        .map_err(|error| {
            FrameworkError::internal(format!("child fixture self-verification rejected: {error}"))
        })?;
        let parameter_limits = ChildParameterLimits::new(self.graph.input_limits, 0, 300_000)
            .map_err(|_| live_boot_error())?;
        let historical_v1_envelope = PreparedChildParametersV1::new(
            scope.clone(),
            parent_instance.clone(),
            parent_revision,
            pending.clone(),
            now,
            expires_at,
            self.graph.key_ring.active_key_id().clone(),
            &parameter_limits,
        )
        .map_err(|_| FrameworkError::internal("historical child fixture envelope rejected"))?
        .publish(
            &accepted_parent,
            &self.graph.key_ring,
            now,
            &parameter_limits,
        )
        .map_err(|_| {
            FrameworkError::internal("historical child fixture envelope signing rejected")
        })?;
        let envelope = PreparedChildParametersV2::new(
            scope.clone(),
            parent_instance.clone(),
            parent_revision,
            child_instance.clone(),
            pending.clone(),
            now,
            expires_at,
            self.graph.key_ring.active_key_id().clone(),
            &parameter_limits,
        )
        .map_err(|_| FrameworkError::internal("child fixture envelope rejected"))?
        .publish(
            &accepted_parent,
            &self.graph.key_ring,
            now,
            &parameter_limits,
        )
        .map_err(|_| FrameworkError::internal("child fixture envelope signing rejected"))?;
        let verified_parameters = verify_child_parameters_v2(
            &envelope,
            &ExpectedChildParametersV2::new(
                scope.clone(),
                parent_instance.clone(),
                parent_revision,
                pending.child().key().clone(),
                child.contract_digest().clone(),
                child_instance.clone(),
                child.parameter_schema().clone(),
            )
            .after_applied_parent_revision(Revision::new(0)),
            &self.graph.key_ring,
            now,
            &parameter_limits,
        )
        .map_err(|error| {
            FrameworkError::internal(format!(
                "child fixture envelope verification rejected: {error}"
            ))
        })?;
        authorize_child_parameters_v2(
            &verified_parameters,
            &verified_parent,
            self.graph.ledger.as_ref(),
        )
        .await
        .map_err(|error| {
            FrameworkError::internal(format!("child fixture eligibility rejected: {error}"))
        })?;
        Ok(ChildParameterTestFixture {
            child_snapshot,
            envelope,
            historical_v1_envelope,
            parent_snapshot,
            scope,
            parent_instance,
            child_instance,
        })
    }

    pub(super) async fn advance_parent_revision_for_test(
        &self,
        scope: &ScopeFingerprint,
        parent: &InstanceId,
    ) -> Result<u64, FrameworkError> {
        let revision = self
            .graph
            .ledger
            .current_accepted_revision(scope, parent)
            .await
            .map_err(|_| live_boot_error())?
            .ok_or_else(live_boot_error)?;
        let idempotency_bytes: [u8; 16] = Sha256::digest(b"framework-child-parent-supersede")[..16]
            .try_into()
            .map_err(|_| live_boot_error())?;
        let claim = ClaimRequest::new(
            scope.clone(),
            parent.clone(),
            revision,
            IdempotencyKey::from_bytes(&idempotency_bytes).map_err(|_| live_boot_error())?,
            ContentDigest::from_bytes(&Sha256::digest(b"framework-child-parent-supersede-request"))
                .map_err(|_| live_boot_error())?,
        );
        let grant = match self
            .graph
            .ledger
            .claim(claim)
            .await
            .map_err(|_| live_boot_error())?
        {
            ClaimOutcome::Granted(grant) => grant,
            _ => return Err(live_boot_error()),
        };
        let successor = grant.successor_revision();
        self.graph
            .ledger
            .commit(
                &grant.into_token(),
                AcceptedOutcome::new(
                    AcceptedOutcomeKind::Rendered,
                    ContentDigest::from_bytes(&Sha256::digest(
                        b"framework-child-parent-supersede-output",
                    ))
                    .map_err(|_| live_boot_error())?,
                ),
            )
            .await
            .map_err(|_| live_boot_error())?;
        Ok(successor.get())
    }

    pub(super) async fn child_revision_for_test(
        &self,
        scope: &ScopeFingerprint,
        child: &InstanceId,
    ) -> Result<u64, FrameworkError> {
        self.graph
            .ledger
            .current_accepted_revision(scope, child)
            .await
            .map_err(|_| live_boot_error())?
            .map(Revision::get)
            .ok_or_else(live_boot_error)
    }

    pub(crate) fn register_mount(
        &self,
        registration: LiveMountRegistration,
    ) -> Result<(), FrameworkError> {
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
        let LiveMountRegistration {
            entry,
            upload_selector,
            kind,
        } = registration;
        let kind_record = MountKindRecord {
            route: upload_selector.selection.route().clone(),
            slot: upload_selector.selection.slot().clone(),
            kind,
        };
        let next = current
            .register(self.graph.registry.engine(), entry)
            .map_err(|_| FrameworkError::internal("Live mount catalog was rejected"))?;
        *builder = Some(next);
        self.graph
            .upload_mount_builder
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_mut()
            .ok_or_else(live_boot_error)?
            .push(upload_selector);
        self.graph
            .mount_kind_builder
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_mut()
            .ok_or_else(live_boot_error)?
            .push(kind_record);
        Ok(())
    }

    /// Closes the identity absences the request's mount kind permits.
    ///
    /// A public seed accepts anonymous, sessionless, and tenantless requests
    /// and an identity-bound mount accepts a tenantless one; the route policy
    /// cannot know the mount before the body is inspected, so the action
    /// boundary records those typed absences here. The engine then validates
    /// the closed facts against the catalog's own requirements, so a closure
    /// the catalog does not permit still fails.
    pub(crate) fn close_mount_scope_absences(
        &self,
        request: &mut Request,
        selection: &MountSelection,
    ) -> Result<(), FrameworkError> {
        use super::attestation::SecurityCheck;
        use super::document::LiveMountKind;
        use suprnova_live::host::PolicyReason;

        self.finalize_mount_catalog()?;
        let kinds = self.graph.mount_kinds.get().ok_or_else(live_boot_error)?;
        let record = kinds
            .iter()
            .find(|record| &record.route == selection.route() && &record.slot == selection.slot())
            .ok_or_else(|| FrameworkError::internal("Live request context was rejected"))?;
        let closable: &[(SecurityCheck, PolicyReason)] = match record.kind {
            LiveMountKind::PublicSeed => &[
                (SecurityCheck::Session, PolicyReason::StatelessRequest),
                (SecurityCheck::Principal, PolicyReason::AnonymousPrincipal),
                (SecurityCheck::Tenant, PolicyReason::TenantlessRoute),
            ],
            LiveMountKind::IdentityBound => {
                &[(SecurityCheck::Tenant, PolicyReason::TenantlessRoute)]
            }
        };
        for (check, reason) in closable {
            if request
                .live_security_attestation()
                .disposition(*check)
                .is_none()
            {
                request.record_live_security_not_required(*check, *reason);
            }
        }
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
        let selectors = self
            .graph
            .upload_mount_builder
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .ok_or_else(live_boot_error)?;
        self.graph
            .upload_mounts
            .set(Arc::from(selectors))
            .map_err(|_| live_boot_error())?;
        let kinds = self
            .graph
            .mount_kind_builder
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .ok_or_else(live_boot_error)?;
        self.graph
            .mount_kinds
            .set(Arc::from(kinds))
            .map_err(|_| live_boot_error())?;
        self.graph
            .mount_catalog
            .set(Arc::new(builder.build()))
            .map_err(|_| live_boot_error())
    }

    pub(crate) fn validate_upload_request_context(
        &self,
        request: &Request,
        component: &str,
        slot: &str,
        document_key: &str,
    ) -> Result<TrustedLiveRequestContext, FrameworkError> {
        let selector = self.select_mount(component, slot, document_key)?;
        let base_scope = super::context::request_scope(request)?;
        let scope = super::upload::derive_mount_scope(&selector.scope_binding(base_scope))?;
        self.validate_request_context_with_scope(
            request,
            selector.selection.clone(),
            Some(scope),
            None,
        )
    }

    /// Validates one asynchronous-update request against the browser-selected mount.
    ///
    /// The subscription ports are installed only here, so action and upload
    /// contexts can never mint or renew subscription authority.
    pub(crate) fn validate_async_request_context(
        &self,
        request: &Request,
        component: &str,
        slot: &str,
        document_key: &str,
        baseline: StreamPosition,
    ) -> Result<(TrustedLiveRequestContext, TrustedMountParameters), AsyncErrorKind> {
        let selector = self
            .select_mount(component, slot, document_key)
            .map_err(|_| AsyncErrorKind::MountUnknown)?;
        let parameters = trusted_mount_parameters(request, &selector)
            .map_err(|_| AsyncErrorKind::ContextRejected)?;
        let subscription = SubscriptionCapabilities {
            registry: Arc::new(SuprnovaSubscriptionRegistry::new(
                Arc::clone(&self.graph.engine_registry),
                parameters.clone(),
            )),
            authorization: Arc::clone(&self.graph.ports.subscription_authorization),
            continuity: Arc::new(FixedSubscriptionBaseline::new(baseline)),
            credentials: Arc::clone(&self.graph.ports.subscription_credentials)
                as Arc<dyn SubscriptionCredentialPort>,
        };
        let context = self
            .validate_request_context_with_scope(
                request,
                selector.selection.clone(),
                None,
                Some(subscription),
            )
            .map_err(|_| AsyncErrorKind::ContextRejected)?;
        Ok((context, parameters))
    }

    /// Returns the generated metadata of one registered component.
    pub(crate) fn component_metadata(
        &self,
        component: &ComponentName,
    ) -> Option<suprnova_live::metadata::ComponentMetadata> {
        self.graph
            .engine_registry
            .resolve(component)
            .ok()
            .map(|descriptor| descriptor.metadata().clone())
    }

    /// Returns the shared asynchronous-update runtime state.
    pub(crate) fn async_state(&self) -> &Arc<AsyncState> {
        &self.graph.async_state
    }

    pub(crate) fn validate_upload_action_context(
        &self,
        request: &Request,
        selection: &MountSelection,
    ) -> Result<Option<TrustedLiveRequestContext>, FrameworkError> {
        let descriptor = self
            .graph
            .engine_registry
            .resolve(selection.component())
            .map_err(|_| live_boot_error())?;
        if !descriptor
            .metadata()
            .fields()
            .iter()
            .any(|field| field.upload_policy().is_some())
        {
            return Ok(None);
        }
        self.finalize_mount_catalog()?;
        let selectors = self.graph.upload_mounts.get().ok_or_else(live_boot_error)?;
        // The registered mount identifies the upload authority by route,
        // slot, component, and contract. The catalog validates the request's
        // protocol version against the component below, and the browser
        // runtime negotiates the newest one, so the protocol never takes part
        // in this match.
        let mut matches = selectors.iter().filter(|candidate| {
            candidate.selection.route() == selection.route()
                && candidate.selection.slot() == selection.slot()
                && candidate.selection.component() == selection.component()
                && candidate.selection.contract_digest() == selection.contract_digest()
        });
        let selector = matches.next().ok_or_else(live_boot_error)?;
        if matches.next().is_some() {
            return Err(FrameworkError::internal(
                "Live upload action mount authority was ambiguous",
            ));
        }
        let base_scope = super::context::request_scope(request)?;
        let scope = super::upload::derive_mount_scope(&selector.scope_binding(base_scope))?;
        self.validate_request_context_with_scope(request, selection.clone(), Some(scope), None)
            .map(Some)
    }

    pub(crate) async fn resolve_upload_request_context(
        &self,
        request: &Request,
        handle: &UploadHandle,
    ) -> Result<(TrustedLiveRequestContext, UploadRecord), FrameworkError> {
        self.finalize_mount_catalog()?;
        let base_scope = super::context::request_scope(request)?;
        let selectors = self.graph.upload_mounts.get().ok_or_else(live_boot_error)?;
        let mut contexts = Vec::with_capacity(selectors.len());
        for selector in selectors.iter() {
            let scope =
                super::upload::derive_mount_scope(&selector.scope_binding(base_scope.clone()))?;
            contexts.push(self.validate_request_context_with_scope(
                request,
                selector.selection.clone(),
                Some(scope),
                None,
            )?);
        }
        if contexts.is_empty() {
            return Err(live_boot_error());
        }

        let record = self
            .graph
            .ports
            .uploads
            .ledger
            .load(handle)
            .await
            .map_err(|_| live_boot_error())?
            .ok_or_else(live_boot_error)?;
        let mut matches = contexts.into_iter().filter(|context| {
            context.mount().component() == record.authority().component()
                && context.scope() == record.authority().host_scope().scope()
        });
        let context = matches.next().ok_or_else(live_boot_error)?;
        if matches.next().is_some() {
            return Err(FrameworkError::internal(
                "Live upload mount authority was ambiguous",
            ));
        }
        Ok((context, record))
    }

    pub(crate) fn upload_policy(
        &self,
        component: &ComponentName,
        field: &ModelField,
    ) -> Result<UploadFieldPolicy, FrameworkError> {
        self.graph
            .engine_registry
            .resolve(component)
            .map_err(|_| live_boot_error())?
            .metadata()
            .fields()
            .iter()
            .find(|candidate| candidate.name() == field)
            .and_then(|candidate| candidate.upload_policy())
            .cloned()
            .ok_or_else(live_boot_error)
    }

    pub(crate) async fn authorize_upload_create(
        &self,
        component: &ComponentName,
        field: &ModelField,
    ) -> Result<(), FrameworkError> {
        self.graph
            .ports
            .uploads
            .authorization_adapter
            .authorize_registered(component, field, UploadControlKind::Create)
            .await
            .map_err(|_| live_boot_error())
    }

    pub(crate) fn derive_upload_handle_candidates(
        &self,
        scope: &ScopeFingerprint,
        field: &ModelField,
        idempotency_key: &UploadIdempotencyKey,
    ) -> Result<Vec<UploadHandle>, FrameworkError> {
        let current = crate::crypto::Crypt::current_key_bytes().ok_or_else(live_boot_error)?;
        let previous = crate::crypto::Crypt::previous_key_bytes();
        let previous = previous.iter().map(Vec::as_slice).collect::<Vec<_>>();
        super::upload::derive_upload_handle_candidates(
            &current,
            &previous,
            scope,
            field,
            idempotency_key,
        )
    }

    pub(crate) fn upload_now(&self) -> Result<UnixMillis, FrameworkError> {
        self.graph.clock.now().map_err(|_| live_boot_error())
    }

    pub(crate) fn upload_limits(&self) -> UploadLimits {
        self.graph.uploads.limits
    }

    pub(crate) fn upload_body_budget(&self) -> &super::upload::UploadBodyBudget {
        &self.graph.uploads.body_budget
    }

    pub(crate) fn upload_operation_locks(&self) -> &super::upload::UploadOperationLocks {
        self.graph.ports.uploads.operation_locks.as_ref()
    }

    pub(crate) fn upload_authority(&self) -> &UploadService {
        self.graph.uploads.authority.as_ref()
    }

    pub(crate) fn upload_validation(&self) -> &UploadValidationService {
        self.graph.uploads.validation.as_ref()
    }

    pub(super) fn upload_cleanup_for_test(&self) -> &UploadCleanupService {
        self.graph.uploads.cleanup.as_ref()
    }

    pub(crate) fn ensure_upload_cleanup_runner(&self) -> Result<(), FrameworkError> {
        self.graph.uploads.cleanup_runner.ensure_started()
    }

    pub(crate) fn wake_upload_cleanup(&self) -> Result<(), FrameworkError> {
        self.graph.uploads.cleanup_runner.wake()
    }

    pub(crate) fn upload_ledger(&self) -> &dyn suprnova_live::upload::UploadLedger {
        self.graph.ports.uploads.ledger.as_ref()
    }

    pub(crate) fn upload_provider(&self) -> &dyn suprnova_live::upload::UploadProvider {
        self.graph.ports.uploads.provider.as_ref()
    }

    pub(crate) fn upload_provider_adapter(
        &self,
    ) -> &super::ports::upload_provider::SuprnovaUploadProviderRouter {
        self.graph.ports.uploads.provider_adapter.as_ref()
    }

    pub(crate) fn upload_reverse_proxy(
        &self,
    ) -> &dyn suprnova_live::upload::ReverseProxyUploadProvider {
        self.graph.ports.uploads.reverse_proxy.as_ref()
    }

    pub(crate) fn upload_reverse_proxy_adapter(
        &self,
    ) -> &super::ports::upload_provider::SuprnovaReverseProxyUploadProvider {
        self.graph.ports.uploads.reverse_proxy_adapter.as_ref()
    }

    fn select_mount(
        &self,
        component: &str,
        slot: &str,
        document_key: &str,
    ) -> Result<UploadMountSelector, FrameworkError> {
        let component = ComponentName::parse(component).map_err(|_| live_boot_error())?;
        let slot = IslandSlot::parse(slot).map_err(|_| live_boot_error())?;
        let document_key = DocumentMountKey::parse(document_key).map_err(|_| live_boot_error())?;
        let selectors = self.graph.upload_mounts.get().ok_or_else(live_boot_error)?;
        let mut matches = selectors.iter().filter(|candidate| {
            candidate.selection.component() == &component
                && candidate.selection.slot() == &slot
                && candidate.document_key == document_key
        });
        let selected = matches.next().ok_or_else(live_boot_error)?;
        if matches.next().is_some() {
            return Err(FrameworkError::internal(
                "Live upload mount selection was ambiguous",
            ));
        }
        Ok(selected.clone())
    }

    pub(super) fn select_upload_mount_for_test(
        &self,
        component: &str,
        slot: &str,
        document_key: &str,
    ) -> Result<(), FrameworkError> {
        self.select_mount(component, slot, document_key).map(|_| ())
    }

    pub(super) fn inspect_upload_mount_authority_for_test(
        &self,
        component: &str,
        slot: &str,
        document_key: &str,
        base_scope: ScopeFingerprint,
    ) -> Result<UploadMountAuthorityTestFixture, FrameworkError> {
        let selector = self.select_mount(component, slot, document_key)?;
        let binding = selector.scope_binding(base_scope);
        let scope = super::upload::derive_mount_scope(&binding)?;
        Ok(UploadMountAuthorityTestFixture { binding, scope })
    }

    pub(super) fn resolve_upload_mount_authority_for_test(
        &self,
        component: &str,
        base_scope: ScopeFingerprint,
        expected_scope: &ScopeFingerprint,
    ) -> Result<UploadMountResolutionTestFixture, FrameworkError> {
        let component = ComponentName::parse(component).map_err(|_| live_boot_error())?;
        let selectors = self.graph.upload_mounts.get().ok_or_else(live_boot_error)?;
        let mut matches = selectors.iter().filter(|selector| {
            selector.selection.component() == &component
                && super::upload::derive_mount_scope(&selector.scope_binding(base_scope.clone()))
                    .is_ok_and(|scope| &scope == expected_scope)
        });
        let selected = matches.next().ok_or_else(live_boot_error)?;
        if matches.next().is_some() {
            return Err(FrameworkError::internal(
                "Live upload mount authority was ambiguous",
            ));
        }
        Ok(UploadMountResolutionTestFixture {
            slot: selected.selection.slot().clone(),
            document_key: selected.document_key.clone(),
        })
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
        _current_route: RouteIdentity,
        _current_slot: IslandSlot,
        selection: MountSelection,
    ) -> Result<TrustedLiveRequestContext, FrameworkError> {
        self.validate_request_context_with_scope(request, selection, None, None)
    }

    fn validate_request_context_with_scope(
        &self,
        request: &Request,
        selection: MountSelection,
        scope_override: Option<ScopeFingerprint>,
        subscription: Option<SubscriptionCapabilities>,
    ) -> Result<TrustedLiveRequestContext, FrameworkError> {
        self.finalize_mount_catalog()?;
        let now = self.graph.clock.now().map_err(|_| live_boot_error())?;
        let candidate = super::context::candidate(
            request,
            selection.route().clone(),
            selection.slot().clone(),
            selection,
            scope_override,
            Arc::clone(&self.graph.ports.authorization),
            Arc::clone(&self.graph.ports.uploads.authorization),
            subscription,
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
        upload_context: Option<TrustedLiveRequestContext>,
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
            Arc::clone(&self.graph.clock),
            Arc::clone(&self.graph.uploads.finalization),
            upload_context,
            Arc::clone(&completion),
        );
        let mount_catalog = Arc::clone(
            self.graph
                .mount_catalog
                .get()
                .expect("Live mount catalog is finalized before endpoint dispatch"),
        );
        (
            LiveEndpointService::new(
                self.graph.endpoint_config.clone(),
                Arc::clone(&self.graph.engine_registry),
                mount_catalog,
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
            upload_ports: Arc::strong_count(&self.graph.ports.uploads.ledger) > 0
                && Arc::strong_count(&self.graph.ports.uploads.cleanup_ledger) > 0
                && Arc::strong_count(&self.graph.ports.uploads.quarantine) > 0
                && Arc::strong_count(&self.graph.ports.uploads.provider) > 0
                && Arc::strong_count(&self.graph.ports.uploads.reverse_proxy) > 0
                && Arc::strong_count(&self.graph.ports.uploads.direct) > 0
                && Arc::strong_count(&self.graph.ports.uploads.authorization) > 0
                && Arc::strong_count(&self.graph.ports.uploads.scanner) > 0
                && Arc::strong_count(&self.graph.ports.uploads.application_validation) > 0
                && Arc::strong_count(&self.graph.ports.uploads.evidence) > 0
                && Arc::strong_count(&self.graph.ports.uploads.finalizer) > 0,
            upload_services: Arc::strong_count(&self.graph.uploads.authority) > 0
                && Arc::strong_count(&self.graph.uploads.validation) > 0
                && Arc::strong_count(&self.graph.uploads.finalization) > 0
                && Arc::strong_count(&self.graph.uploads.cleanup) > 0
                && self.graph.uploads.limits.max_chunk_bytes() > 0,
            response_and_cancellation: Arc::strong_count(&self.graph.ports.response) > 0
                && Arc::strong_count(&self.graph.ports.cancellation) > 0,
            mount_catalog: self.graph.mount_catalog.get().is_some(),
            subscription_ports: Arc::strong_count(&self.graph.ports.subscription_authorization) > 0
                && Arc::strong_count(&self.graph.ports.subscription_credentials) > 0,
            async_state: Arc::strong_count(&self.graph.async_state) > 0,
        }
    }
}

/// Trusted topic parameters of one request for one finalized mount.
///
/// Every value is a single topic segment; identities that cannot be spelled
/// as one segment are omitted so templates needing them fail closed.
fn trusted_mount_parameters(
    request: &Request,
    selector: &UploadMountSelector,
) -> Result<TrustedMountParameters, FrameworkError> {
    let mut values = vec![
        (
            "component".to_owned(),
            selector.selection.component().as_str().to_owned(),
        ),
        (
            "slot".to_owned(),
            selector.selection.slot().as_str().to_owned(),
        ),
        (
            "document_key".to_owned(),
            selector.document_key.as_str().to_owned(),
        ),
    ];
    if let Some(principal) = crate::auth::guard::Auth::id() {
        values.push(("principal".to_owned(), principal));
    }
    if let Some(tenant) = request.live_tenant() {
        values.push(("tenant".to_owned(), tenant.to_owned()));
    }
    values.retain(|(_, value)| topic_segment(value));
    TrustedMountParameters::new(values)
        .map_err(|_| FrameworkError::internal("Live mount parameters were rejected"))
}

fn topic_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !matches!(value, "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
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
        .map_err(|_| live_boot_error())?
        .with_island_stream_directive(),
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
        .map_err(|_| live_boot_error())?
        .with_island_stream_directive(),
    );
    let execution = Arc::new(
        ExecutionService::new(
            Arc::clone(&ledger),
            Arc::clone(&clock),
            Arc::clone(&key_ring),
            snapshot_limits.clone(),
            renderer,
        )
        .with_reporter(Arc::clone(&ports.reporter))
        .with_island_stream_directive(),
    );
    let context_validator = LiveRequestContextValidator::new(config.max_context_lifetime_ms())
        .map_err(|_| live_boot_error())?;
    let uploads = assemble_upload_runtime(&ports, Arc::clone(&clock))?;
    let async_state = AsyncState::new(
        build_key_ring()?,
        Arc::clone(&clock),
        Arc::clone(&engine_registry),
    )?;

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
            snapshot_limits,
            endpoint_config,
            context_validator,
            ports,
            uploads,
            async_state,
            mount_builder: Mutex::new(Some(MountCatalogBuilder::new())),
            mount_catalog: OnceLock::new(),
            upload_mount_builder: Mutex::new(Some(Vec::new())),
            upload_mounts: OnceLock::new(),
            mount_kind_builder: Mutex::new(Some(Vec::new())),
            mount_kinds: OnceLock::new(),
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

pub(super) fn assemble_for_harness_with_clock(
    config: LiveConfig,
    registry: LiveRegistry,
    clock: Arc<dyn Clock>,
) -> Result<LiveRuntime, FrameworkError> {
    let mut candidates = RuntimeProviderCandidates::production(&registry)?;
    let ledger_limits =
        LedgerLimits::new(30_000, 604_800_000, 64, 100_000).map_err(|_| live_boot_error())?;
    candidates.clock = Some(Arc::clone(&clock));
    candidates.ledger = Some(Arc::new(MemoryInstanceLedger::new(clock, ledger_limits)));
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
    pub(crate) upload_ports: bool,
    pub(crate) upload_services: bool,
    pub(crate) mount_catalog: bool,
    pub(crate) response_and_cancellation: bool,
    pub(crate) subscription_ports: bool,
    pub(crate) async_state: bool,
}

fn assemble_upload_runtime(
    ports: &super::ports::HostPorts,
    clock: Arc<dyn Clock>,
) -> Result<UploadRuntimeGraph, FrameworkError> {
    let limits = UploadLimits::new(UploadLimitConfig::reference())
        .map_err(|_| FrameworkError::internal("Live upload limits were rejected"))?;
    let authority = Arc::new(
        UploadService::new(
            Arc::clone(&ports.uploads.ledger),
            TransferGrantCodec::new(build_key_ring()?),
            limits,
        )
        .map_err(|_| FrameworkError::internal("Live upload authority was rejected"))?,
    );
    let validation = Arc::new(
        UploadValidationService::new(
            Arc::clone(&authority),
            Arc::clone(&ports.uploads.provider),
            Arc::clone(&ports.uploads.evidence),
            Some(Arc::clone(&ports.uploads.scanner)),
            Some(Arc::clone(&ports.uploads.application_validation)),
            limits,
        )
        .map_err(|_| FrameworkError::internal("Live upload validation was rejected"))?,
    );
    let finalization = Arc::new(UploadFinalizationService::new(
        Arc::clone(&authority),
        Arc::clone(&ports.uploads.evidence),
        Arc::clone(&ports.uploads.finalizer),
    ));
    let retry = BoundedBackoff::new(Duration::from_secs(1), Duration::from_secs(3_600), 8)
        .map_err(|_| FrameworkError::internal("Live upload cleanup retry was rejected"))?;
    let batch_items = NonZeroUsize::new(limits.max_cleanup_batch())
        .ok_or_else(|| FrameworkError::internal("Live upload cleanup batch was rejected"))?;
    let batch_bytes = usize::try_from(limits.max_aggregate_bytes())
        .ok()
        .and_then(NonZeroUsize::new)
        .ok_or_else(|| FrameworkError::internal("Live upload cleanup bytes were rejected"))?;
    let cleanup_policy =
        CleanupPolicy::new(batch_items, batch_bytes, Duration::from_secs(30), retry)
            .map_err(|_| FrameworkError::internal("Live upload cleanup policy was rejected"))?;
    let cleanup_permits = PermitPool::new(limits.max_concurrent_transfers())
        .map_err(|_| FrameworkError::internal("Live upload cleanup permits were rejected"))?;
    let cleanup = Arc::new(
        UploadCleanupService::new(
            Arc::clone(&ports.uploads.cleanup_ledger),
            Arc::clone(&ports.uploads.provider),
            Arc::clone(&ports.uploads.evidence),
            clock,
            cleanup_permits,
            cleanup_policy,
            limits,
        )
        .map_err(|_| FrameworkError::internal("Live upload cleanup was rejected"))?,
    );

    let cleanup_runner = UploadCleanupRunner::new(Arc::clone(&cleanup));
    Ok(UploadRuntimeGraph {
        limits,
        body_budget: super::upload::UploadBodyBudget::new(limits.max_in_flight_bytes())?,
        authority,
        validation,
        finalization,
        cleanup,
        cleanup_runner,
    })
}

/// Builds a `SnapshotKeyRing` from the framework's own configured root key
/// material (`Crypt::current_key_bytes` / `previous_key_bytes`).
///
/// `pub(crate)`, not private: `RenderCache::install`
/// (`crate::render_cache::mod`) derives its own key ring from this same root
/// material so render-cache key, variance, and entry MACs are
/// cryptographically distinct from Live's snapshot and child-parameter MACs
/// (purpose separation - see `SnapshotPurpose`) while still sharing one root
/// secret rather than requiring a second one to configure. See ruling R56.
pub(crate) fn build_key_ring() -> Result<SnapshotKeyRing, FrameworkError> {
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
