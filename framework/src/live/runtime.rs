//! Immutable process runtime assembled before Live routes are constructed.

use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

use crate::{App, FrameworkError};
use sha2::{Digest, Sha256};
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
    BuildId, ComponentName, ContentDigest, IdempotencyKey, InstanceId, IslandSlot, KeyId, Revision,
    RouteIdentity, ScopeFingerprint, UnixMillis,
};
use suprnova_live::ledger::{
    AcceptedOutcome, AcceptedOutcomeKind, ClaimOutcome, ClaimRequest, LedgerLimits,
    LiveInstanceLedger, MemoryInstanceLedger, MountInstanceRecord,
};
use suprnova_live::limits::InputLimits;
use suprnova_live::mount::{
    DocumentMountKey, DocumentMountScope, MountFlags, MountLimits, MountProviders,
    PrivateMountOutput, PrivateMountRequest, PrivateMountService, PublicMountProviders,
    PublicSeedMountOutput, PublicSeedMountService,
};
use suprnova_live::promotion::{PromotionLimitConfig, PromotionLimits, PromotionService};
use suprnova_live::protocol::{ProtocolLimitConfig, ProtocolLimits};
use suprnova_live::random::{InstanceIdGenerator, SystemInstanceIdGenerator};
use suprnova_live::snapshot::{
    ComponentContract as SnapshotContract, CompositionChildLineageV1, CompositionLineageV1,
    CompositionOwnerLineageV1, ExpectedInstanceV1, InstanceBodyV1, InstanceFieldsV1,
    MountedDocumentPath, SnapshotLimits, verify_instance,
};
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
    snapshot_limits: SnapshotLimits,
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
            snapshot_limits.clone(),
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
            snapshot_limits,
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
