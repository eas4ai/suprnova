//! Validate-render-sign-authorize orchestration for private initial mounts.

use std::sync::Arc;

use bytes::Bytes;

use super::output::HARD_MAX_FLAGS;
use super::{
    DocumentMountScope, MountError, MountErrorKind, PrivateMountOutput, PrivateMountRequest,
};
use crate::canonical::to_canonical_bytes;
use crate::clock::Clock;
use crate::component::{ComponentExecutor, MountContext, RenderContext};
use crate::crypto::SnapshotKeyRing;
use crate::host::TrustedLiveRequestContext;
use crate::identity::{Revision, UnixMillis};
use crate::ledger::{LedgerErrorKind, LiveInstanceLedger, MountInstanceRecord};
use crate::random::InstanceIdGenerator;
use crate::registry::ComponentRegistry;
use crate::snapshot::state::StateExposure;
use crate::snapshot::{InstanceBodyV1, InstanceFieldsV1, SnapshotLimits};
use crate::view::{
    IslandRootFlag, IslandRootInput, IslandSnapshotForm, MountMetadata, MountSnapshotKind,
    ViewRenderer, assemble_island_root,
};

const HARD_MAX_ATTEMPTS: usize = 16;
const HARD_MAX_METADATA_BYTES: usize = 1_048_576;

/// Bounded private mount validity, retry, and metadata policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MountLimits {
    instance_lifetime_ms: u64,
    max_identity_attempts: usize,
    max_metadata_bytes: usize,
    max_flags: usize,
}

/// Host-provided registry, authority, time, randomness, and signing dependencies.
pub struct MountProviders {
    registry: Arc<ComponentRegistry>,
    ledger: Arc<dyn LiveInstanceLedger>,
    clock: Arc<dyn Clock>,
    instance_ids: Arc<dyn InstanceIdGenerator>,
    keys: Arc<SnapshotKeyRing>,
}

impl MountProviders {
    /// Groups the provider boundaries required by private mounting.
    #[must_use]
    pub fn new(
        registry: Arc<ComponentRegistry>,
        ledger: Arc<dyn LiveInstanceLedger>,
        clock: Arc<dyn Clock>,
        instance_ids: Arc<dyn InstanceIdGenerator>,
        keys: Arc<SnapshotKeyRing>,
    ) -> Self {
        Self {
            registry,
            ledger,
            clock,
            instance_ids,
            keys,
        }
    }
}

impl std::fmt::Debug for MountProviders {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("<MountProviders:redacted>")
    }
}

impl MountLimits {
    /// Creates non-zero mount limits below hard engine ceilings.
    pub fn new(
        instance_lifetime_ms: u64,
        max_identity_attempts: usize,
        max_metadata_bytes: usize,
        max_flags: usize,
    ) -> Result<Self, MountError> {
        let valid = instance_lifetime_ms > 0
            && max_identity_attempts > 0
            && max_identity_attempts <= HARD_MAX_ATTEMPTS
            && max_metadata_bytes > 0
            && max_metadata_bytes <= HARD_MAX_METADATA_BYTES
            && max_flags <= HARD_MAX_FLAGS;
        if !valid {
            return Err(MountError::new(MountErrorKind::InvalidConfiguration));
        }
        Ok(Self {
            instance_lifetime_ms,
            max_identity_attempts,
            max_metadata_bytes,
            max_flags,
        })
    }
}

/// Stateless private mount service; only ledger concurrency metadata survives requests.
pub struct PrivateMountService {
    registry: Arc<ComponentRegistry>,
    ledger: Arc<dyn LiveInstanceLedger>,
    clock: Arc<dyn Clock>,
    instance_ids: Arc<dyn InstanceIdGenerator>,
    keys: Arc<SnapshotKeyRing>,
    snapshot_limits: SnapshotLimits,
    views: ViewRenderer,
    limits: MountLimits,
    executor: ComponentExecutor,
}

impl PrivateMountService {
    /// Creates a service after cross-checking snapshot and mount validity limits.
    pub fn new(
        providers: MountProviders,
        snapshot_limits: SnapshotLimits,
        views: ViewRenderer,
        limits: MountLimits,
    ) -> Result<Self, MountError> {
        if limits.instance_lifetime_ms > snapshot_limits.max_instance_lifetime_ms() {
            return Err(MountError::new(MountErrorKind::InvalidConfiguration));
        }
        Ok(Self {
            registry: providers.registry,
            ledger: providers.ledger,
            clock: providers.clock,
            instance_ids: providers.instance_ids,
            keys: providers.keys,
            snapshot_limits,
            views,
            limits,
            executor: ComponentExecutor::new(),
        })
    }

    /// Produces output only after a complete lifecycle and atomic ledger creation.
    pub async fn mount(
        &self,
        document: &mut DocumentMountScope,
        request: PrivateMountRequest,
        context: &TrustedLiveRequestContext,
    ) -> Result<PrivateMountOutput, MountError> {
        document.reserve(request.key.clone())?;
        let result = self.mount_reserved(&request, context).await;
        if result.is_err() {
            document.release(&request.key);
        }
        result
    }

    async fn mount_reserved(
        &self,
        request: &PrivateMountRequest,
        context: &TrustedLiveRequestContext,
    ) -> Result<PrivateMountOutput, MountError> {
        let now = self
            .clock
            .now()
            .map_err(|_| MountError::new(MountErrorKind::ClockUnavailable))?;
        if !context.is_current(now) {
            return Err(MountError::new(MountErrorKind::ContextRejected));
        }
        let catalog = context.mount();
        let descriptor = self
            .registry
            .require_contract(catalog.component(), catalog.contract_digest())
            .map_err(|_| MountError::new(MountErrorKind::ComponentRejected))?;
        let expected = catalog.expected_seed();
        expected
            .schemas()
            .mount()
            .validate(&request.parameters, StateExposure::PublicSeed)
            .map_err(|_| MountError::new(MountErrorKind::ParametersRejected))?;
        to_canonical_bytes(&request.parameters, self.snapshot_limits.input())
            .map_err(|_| MountError::new(MountErrorKind::ParametersRejected))?;
        if request.flags.len() > self.limits.max_flags
            || preflight_metadata_bytes(request) > self.limits.max_metadata_bytes
        {
            return Err(MountError::new(MountErrorKind::MetadataTooLarge));
        }
        let expires_at = now
            .get()
            .checked_add(self.limits.instance_lifetime_ms)
            .map(UnixMillis::new)
            .ok_or_else(|| MountError::new(MountErrorKind::ClockUnavailable))?;

        for attempt in 0..self.limits.max_identity_attempts {
            let instance_id = self
                .instance_ids
                .generate()
                .map_err(|_| MountError::new(MountErrorKind::RandomUnavailable))?;
            let revision = Revision::new(0);
            let render_context = RenderContext::new(context, &instance_id, revision, expires_at);
            let mount_context = MountContext::new(render_context, &request.parameters);
            let lifecycle = self
                .executor
                .initial_mount(descriptor, &mount_context)
                .await
                .map_err(|_| MountError::new(MountErrorKind::LifecycleRejected))?;
            let (render, state, memo) = lifecycle.into_parts();
            self.views
                .validate_island_fragment(descriptor.metadata().view().clone(), &render)
                .map_err(|_| MountError::new(MountErrorKind::RenderRejected))?;
            let extensions = request
                .document_path
                .as_ref()
                .map(crate::snapshot::MountedDocumentPath::extension)
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect();
            let signed_snapshot = InstanceBodyV1::new(
                InstanceFieldsV1 {
                    component: expected.component().clone(),
                    build_id: expected.build_id().clone(),
                    route: expected.route().clone(),
                    slot: expected.slot().clone(),
                    key_id: self.keys.active_key_id().clone(),
                    scope: context.scope().clone(),
                    instance_id: instance_id.clone(),
                    revision,
                    issued_at: now,
                    expires_at,
                    state,
                    memo,
                    extensions,
                },
                expected.schemas(),
                &self.snapshot_limits,
            )
            .and_then(|body| body.sign(&self.keys, now, &self.snapshot_limits))
            .map_err(|_| MountError::new(MountErrorKind::SnapshotRejected))?;
            if signed_snapshot
                .len()
                .saturating_add(preflight_metadata_bytes(request))
                > self.limits.max_metadata_bytes
            {
                return Err(MountError::new(MountErrorKind::MetadataTooLarge));
            }
            let metadata = MountMetadata::new(
                expected.slot().clone(),
                expected.component().name().clone(),
                MountSnapshotKind::Instance,
                Bytes::from(signed_snapshot.clone()),
            )
            .map_err(|_| MountError::new(MountErrorKind::MetadataTooLarge))?;
            let assembled = assemble_island_root(
                render,
                IslandRootInput {
                    component: context.mount().component().clone(),
                    slot: context.mount().slot().clone(),
                    document_key: request.key.as_str().to_owned(),
                    protocol_minimum: context.mount().minimum_protocol(),
                    runtime_contract: 1,
                    snapshot: Bytes::from(signed_snapshot.clone()),
                    snapshot_form: IslandSnapshotForm::Instance,
                    instance_id: Some(instance_id.clone()),
                    revision,
                    lazy_complete: false,
                    flags: request
                        .flags
                        .iter()
                        .map(|(name, value)| IslandRootFlag::from_validated(name, value))
                        .collect(),
                },
                self.limits.max_metadata_bytes,
            )
            .map_err(|_| MountError::new(MountErrorKind::MetadataTooLarge))?;
            let validated = self
                .views
                .validate_island_output(descriptor.metadata().view().clone(), assembled)
                .map_err(|_| MountError::new(MountErrorKind::RenderRejected))?;

            let completed_at = self
                .clock
                .now()
                .map_err(|_| MountError::new(MountErrorKind::ClockUnavailable))?;
            if completed_at < now {
                return Err(MountError::new(MountErrorKind::ClockUnavailable));
            }
            if !context.is_current(completed_at) || expires_at <= completed_at {
                return Err(MountError::new(MountErrorKind::ContextRejected));
            }

            let ledger = self
                .ledger
                .mount_instance(MountInstanceRecord::new(
                    context.scope().clone(),
                    instance_id.clone(),
                    descriptor.contract_digest().clone(),
                    revision,
                    expires_at,
                ))
                .await;
            match ledger {
                Ok(authority) => {
                    if authority.instance_id() != &instance_id
                        || authority.revision() != revision
                        || authority.expires_at() != expires_at
                    {
                        return Err(MountError::new(MountErrorKind::LedgerRejected));
                    }
                    return Ok(PrivateMountOutput {
                        body: String::from_utf8(validated.body.to_vec())
                            .map_err(|_| MountError::new(MountErrorKind::RenderRejected))?,
                        metadata,
                        instance_id,
                        revision,
                        expires_at,
                    });
                }
                Err(error) if error.kind() == LedgerErrorKind::InstanceConflict => {
                    if attempt + 1 == self.limits.max_identity_attempts {
                        return Err(MountError::new(MountErrorKind::IdentityCollision));
                    }
                }
                Err(_) => return Err(MountError::new(MountErrorKind::LedgerRejected)),
            }
        }
        Err(MountError::new(MountErrorKind::IdentityCollision))
    }
}

fn preflight_metadata_bytes(request: &PrivateMountRequest) -> usize {
    request
        .flags
        .iter()
        .fold(request.key.as_str().len(), |total, (name, value)| {
            total.saturating_add(name.len()).saturating_add(value.len())
        })
}
