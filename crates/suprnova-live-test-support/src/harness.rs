//! Real mount, ledger, snapshot, and action orchestration for conformance tests.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use suprnova_live::action::RawActionArguments;
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::clock::Clock as _;
use suprnova_live::crypto::{SnapshotKeyRing, SnapshotPurpose};
use suprnova_live::execution::{
    ActionExecutionRequest, ExecutionResult, ExecutionService, InstancedActionRequest,
};
use suprnova_live::host::TrustedLiveRequestContext;
use suprnova_live::identity::{
    ActionName, ContentDigest, IdempotencyKey, InstanceId, Revision, UnixMillis,
};
use suprnova_live::ledger::{LedgerLimits, MemoryInstanceLedger};
use suprnova_live::limits::InputLimits;
use suprnova_live::mount::{
    DocumentMountKey, DocumentMountScope, MountFlags, MountLimits, MountProviders,
    PrivateMountRequest, PrivateMountService,
};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
use suprnova_live::snapshot::{
    ExpectedInstanceV1, SnapshotLimits, VerifiedInstanceV1, verify_instance,
};
use suprnova_live::state::ProposalBatch;
use suprnova_live::validation::{BagPolicy, ValidationEngine};
use suprnova_live::view::{RenderLimits, ViewRenderer};

use crate::HarnessServices;

/// Closed reason a dev-only component harness could not perform its request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HarnessErrorKind {
    /// Internal test controls could not satisfy production configuration limits.
    InvalidConfiguration,
    /// Initial component mounting failed closed.
    MountRejected,
    /// Signed mount or successor state did not verify.
    SnapshotRejected,
    /// An action was requested before a successful mount.
    NotMounted,
    /// Deterministic request identity could not satisfy its fixed contract.
    InvalidRequestIdentity,
    /// The controlled clock was unavailable while verifying a result.
    ClockUnavailable,
}

/// Redacted component-harness failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct HarnessError {
    kind: HarnessErrorKind,
}

impl HarnessError {
    const fn new(kind: HarnessErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the stable closed failure reason.
    #[must_use]
    pub const fn kind(self) -> HarnessErrorKind {
        self.kind
    }
}

impl fmt::Display for HarnessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            HarnessErrorKind::InvalidConfiguration => "invalid_harness_configuration",
            HarnessErrorKind::MountRejected => "harness_mount_rejected",
            HarnessErrorKind::SnapshotRejected => "harness_snapshot_rejected",
            HarnessErrorKind::NotMounted => "harness_not_mounted",
            HarnessErrorKind::InvalidRequestIdentity => "invalid_harness_request_identity",
            HarnessErrorKind::ClockUnavailable => "harness_clock_unavailable",
        })
    }
}

impl fmt::Debug for HarnessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for HarnessError {}

/// Explicit construction inputs for one real host-neutral component harness.
pub struct ComponentHarnessConfig {
    descriptor: ComponentDescriptor,
    context: TrustedLiveRequestContext,
    expected_instance: ExpectedInstanceV1,
    keys: SnapshotKeyRing,
    snapshot_limits: SnapshotLimits,
    services: HarnessServices,
    ledger_limits: Option<LedgerLimits>,
    mount_limits: Option<MountLimits>,
    render_limits: RenderLimits,
}

impl ComponentHarnessConfig {
    /// Creates a configuration with conservative local-only provider ceilings.
    #[must_use]
    pub fn new(
        descriptor: ComponentDescriptor,
        context: TrustedLiveRequestContext,
        expected_instance: ExpectedInstanceV1,
        keys: SnapshotKeyRing,
        snapshot_limits: SnapshotLimits,
        services: HarnessServices,
    ) -> Self {
        Self {
            descriptor,
            context,
            expected_instance,
            keys,
            snapshot_limits,
            services,
            ledger_limits: None,
            mount_limits: None,
            render_limits: RenderLimits::standard(),
        }
    }

    /// Replaces the Tier 0 ledger ceilings used by the harness.
    #[must_use]
    pub fn with_ledger_limits(mut self, limits: LedgerLimits) -> Self {
        self.ledger_limits = Some(limits);
        self
    }

    /// Replaces the initial-mount ceilings used by the harness.
    #[must_use]
    pub fn with_mount_limits(mut self, limits: MountLimits) -> Self {
        self.mount_limits = Some(limits);
        self
    }

    /// Replaces the view-output ceilings used by the harness.
    #[must_use]
    pub const fn with_render_limits(mut self, limits: RenderLimits) -> Self {
        self.render_limits = limits;
        self
    }
}

impl fmt::Debug for ComponentHarnessConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ComponentHarnessConfig")
            .field("component", self.descriptor.metadata().identity())
            .finish_non_exhaustive()
    }
}

/// Stable deterministic idempotency and request-digest material for one action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HarnessRequestIdentity {
    idempotency: [u8; 16],
    digest: [u8; 32],
}

impl HarnessRequestIdentity {
    /// Derives fixed non-secret identity bytes from a compact fixture seed.
    #[must_use]
    pub fn from_seed(seed: u8) -> Self {
        let mut idempotency = [seed; 16];
        idempotency[15] = seed.wrapping_add(1).max(1);
        let mut digest = [seed.wrapping_add(2); 32];
        digest[31] = seed.wrapping_add(3).max(1);
        Self {
            idempotency,
            digest,
        }
    }

    fn materialize(self) -> Result<(IdempotencyKey, ContentDigest), HarnessError> {
        let idempotency = IdempotencyKey::from_bytes(&self.idempotency)
            .map_err(|_| HarnessError::new(HarnessErrorKind::InvalidRequestIdentity))?;
        let digest = ContentDigest::from_bytes(&self.digest)
            .map_err(|_| HarnessError::new(HarnessErrorKind::InvalidRequestIdentity))?;
        Ok((idempotency, digest))
    }
}

/// Redacted browser-publishable facts captured after a successful initial mount.
pub struct HarnessMount {
    body: Vec<u8>,
    instance_id: InstanceId,
    revision: Revision,
    expires_at: UnixMillis,
}

impl HarnessMount {
    /// Returns complete engine-owned island HTML.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Returns the server-controlled component instance identity.
    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        &self.instance_id
    }

    /// Returns the authoritative initial revision.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the exclusive instance expiry.
    #[must_use]
    pub const fn expires_at(&self) -> UnixMillis {
        self.expires_at
    }
}

impl fmt::Debug for HarnessMount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HarnessMount")
            .field("body_bytes", &self.body.len())
            .field("revision", &self.revision)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Stateful dev-only runner over the real mount, ledger, snapshot, and action services.
pub struct ComponentHarness {
    descriptor: ComponentDescriptor,
    context: TrustedLiveRequestContext,
    expected_instance: ExpectedInstanceV1,
    services: HarnessServices,
    snapshot_limits: SnapshotLimits,
    keys: Arc<SnapshotKeyRing>,
    mount_service: PrivateMountService,
    execution_service: ExecutionService,
    document: DocumentMountScope,
    validation_engine: ValidationEngine,
    input_limits: InputLimits,
    current: Option<VerifiedInstanceV1>,
    current_encoded: Option<Vec<u8>>,
}

impl ComponentHarness {
    /// Builds real production services around deterministic dev-only dependencies.
    pub fn new(config: ComponentHarnessConfig) -> Result<Self, HarnessError> {
        let ledger_limits = match config.ledger_limits {
            Some(limits) => limits,
            None => LedgerLimits::new(1_000, 60_000, 16, 256)
                .map_err(|_| HarnessError::new(HarnessErrorKind::InvalidConfiguration))?,
        };
        let mount_limits = match config.mount_limits {
            Some(limits) => limits,
            None => MountLimits::new(1_000, 8, 64 * 1024, 64)
                .map_err(|_| HarnessError::new(HarnessErrorKind::InvalidConfiguration))?,
        };
        let registry = Arc::new(
            ComponentRegistryBuilder::new()
                .register(config.descriptor.clone())
                .map_err(|_| HarnessError::new(HarnessErrorKind::InvalidConfiguration))?
                .build(),
        );
        let ledger = Arc::new(MemoryInstanceLedger::new(
            Arc::clone(config.services.clock()) as Arc<dyn suprnova_live::clock::Clock>,
            ledger_limits,
        ));
        let keys = Arc::new(config.keys);
        let renderer = ViewRenderer::new(config.render_limits)
            .map_err(|_| HarnessError::new(HarnessErrorKind::InvalidConfiguration))?;
        let mount_service = PrivateMountService::new(
            MountProviders::new(
                registry,
                Arc::clone(&ledger) as Arc<dyn suprnova_live::ledger::LiveInstanceLedger>,
                Arc::clone(config.services.clock()) as Arc<dyn suprnova_live::clock::Clock>,
                Arc::clone(config.services.instance_ids())
                    as Arc<dyn suprnova_live::random::InstanceIdGenerator>,
                Arc::clone(&keys),
            ),
            config.snapshot_limits.clone(),
            renderer,
            mount_limits,
        )
        .map_err(|_| HarnessError::new(HarnessErrorKind::InvalidConfiguration))?;
        let execution_service = ExecutionService::new(
            ledger,
            Arc::clone(config.services.clock()) as Arc<dyn suprnova_live::clock::Clock>,
            Arc::clone(&keys),
            config.snapshot_limits.clone(),
            renderer,
        );
        let validation_engine = ValidationEngine::new(128)
            .map_err(|_| HarnessError::new(HarnessErrorKind::InvalidConfiguration))?;
        Ok(Self {
            descriptor: config.descriptor,
            context: config.context,
            expected_instance: config.expected_instance,
            services: config.services,
            snapshot_limits: config.snapshot_limits,
            keys,
            mount_service,
            execution_service,
            document: DocumentMountScope::new(),
            validation_engine,
            input_limits: InputLimits::default(),
            current: None,
            current_encoded: None,
        })
    }

    /// Performs an identity-bound initial mount and verifies its signed snapshot.
    pub async fn mount(
        &mut self,
        parameters: CanonicalValue,
    ) -> Result<HarnessMount, HarnessError> {
        let output = self
            .mount_service
            .mount(
                &mut self.document,
                PrivateMountRequest::new(
                    DocumentMountKey::parse("harness-root")
                        .map_err(|_| HarnessError::new(HarnessErrorKind::InvalidConfiguration))?,
                    parameters,
                    MountFlags::empty(),
                ),
                &self.context,
            )
            .await
            .map_err(|_| HarnessError::new(HarnessErrorKind::MountRejected))?;
        let now = self
            .services
            .clock()
            .now()
            .map_err(|_| HarnessError::new(HarnessErrorKind::ClockUnavailable))?;
        let encoded = output.metadata().signed_snapshot().to_vec();
        let verified = verify_instance(
            &encoded,
            &self.expected_instance,
            &self.keys,
            now,
            &self.snapshot_limits,
        )
        .map_err(|_| HarnessError::new(HarnessErrorKind::SnapshotRejected))?;
        self.current = Some(verified);
        self.current_encoded = Some(encoded);
        Ok(HarnessMount {
            body: output.body().to_vec(),
            instance_id: output.instance_id().clone(),
            revision: output.revision(),
            expires_at: output.expires_at(),
        })
    }

    /// Executes one registered action through real ledger acceptance and successor signing.
    pub async fn execute_action(
        &mut self,
        action: &ActionName,
        arguments: RawActionArguments,
        proposals: Option<&ProposalBatch>,
        identity: HarnessRequestIdentity,
    ) -> Result<ExecutionResult, HarnessError> {
        let snapshot = self
            .current
            .as_ref()
            .ok_or_else(|| HarnessError::new(HarnessErrorKind::NotMounted))?;
        let (idempotency, digest) = identity.materialize()?;
        let mut request = ActionExecutionRequest::new(
            action,
            arguments,
            &self.input_limits,
            &self.validation_engine,
            self.services.validation().as_ref(),
            BagPolicy::Replace,
            Some(self.services.transactions().as_ref()),
            self.services.trace(),
        );
        if let Some(proposals) = proposals {
            request = request.with_proposals(proposals);
        }
        let result = self
            .execution_service
            .execute_instanced(InstancedActionRequest::new(
                &self.descriptor,
                &self.context,
                snapshot,
                idempotency,
                digest,
                request,
            ))
            .await;
        if let ExecutionResult::Accepted(accepted) = &result {
            let now = self
                .services
                .clock()
                .now()
                .map_err(|_| HarnessError::new(HarnessErrorKind::ClockUnavailable))?;
            let encoded = accepted.signed_snapshot().to_vec();
            let verified = verify_instance(
                &encoded,
                &self.expected_instance,
                &self.keys,
                now,
                &self.snapshot_limits,
            )
            .map_err(|_| HarnessError::new(HarnessErrorKind::SnapshotRejected))?;
            self.current = Some(verified);
            self.current_encoded = Some(encoded);
        }
        Ok(result)
    }

    /// Returns the currently verified state capability after mount or acceptance.
    #[must_use]
    pub const fn current_snapshot(&self) -> Option<&VerifiedInstanceV1> {
        self.current.as_ref()
    }

    /// Returns the current signed envelope exactly as a browser request carries it.
    #[must_use]
    pub fn current_encoded_snapshot(&self) -> Option<&[u8]> {
        self.current_encoded.as_deref()
    }

    /// Returns the deterministic service controls used by this harness.
    #[must_use]
    pub const fn services(&self) -> &HarnessServices {
        &self.services
    }
}

impl fmt::Debug for ComponentHarness {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ComponentHarness")
            .field("component", self.descriptor.metadata().identity())
            .field(
                "mounted",
                &self
                    .current
                    .as_ref()
                    .map(|snapshot| (snapshot.body().instance_id(), snapshot.body().revision())),
            )
            .field("snapshot_purpose", &SnapshotPurpose::InstanceV1)
            .finish()
    }
}
