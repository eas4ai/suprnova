//! Public seed publication without instance allocation or ledger authority.

use std::fmt;
use std::sync::Arc;

use bytes::Bytes;

use super::{DocumentMountKey, DocumentMountScope, MountError, MountErrorKind, MountFlags};
use crate::clock::Clock;
use crate::crypto::SnapshotKeyRing;
use crate::host::TrustedLiveRequestContext;
use crate::identity::Revision;
use crate::registry::ComponentRegistry;
use crate::snapshot::{SeedBodyV1, SnapshotLimits};
use crate::view::{
    IslandRender, IslandRootFlag, IslandRootInput, IslandSnapshotForm, MountMetadata,
    MountSnapshotKind, ViewRenderer, assemble_island_root,
};

const HARD_MAX_METADATA_BYTES: usize = 1_048_576;

/// Dependencies required for public seed publication, deliberately excluding ledger and identity generation.
pub struct PublicMountProviders {
    registry: Arc<ComponentRegistry>,
    clock: Arc<dyn Clock>,
    keys: Arc<SnapshotKeyRing>,
}

impl PublicMountProviders {
    /// Groups the only authority boundaries used while publishing a public seed.
    #[must_use]
    pub fn new(
        registry: Arc<ComponentRegistry>,
        clock: Arc<dyn Clock>,
        keys: Arc<SnapshotKeyRing>,
    ) -> Self {
        Self {
            registry,
            clock,
            keys,
        }
    }
}

impl fmt::Debug for PublicMountProviders {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<PublicMountProviders:redacted>")
    }
}

/// Fully validated public-state input and already rendered semantic HTML.
pub struct PublicSeedMountRequest {
    key: DocumentMountKey,
    seed: SeedBodyV1,
    render: IslandRender,
    flags: MountFlags,
}

impl PublicSeedMountRequest {
    /// Groups a public seed body, its semantic SSR fragment, and inert flags.
    #[must_use]
    pub const fn new(
        key: DocumentMountKey,
        seed: SeedBodyV1,
        render: IslandRender,
        flags: MountFlags,
    ) -> Self {
        Self {
            key,
            seed,
            render,
            flags,
        }
    }
}

impl fmt::Debug for PublicSeedMountRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<PublicSeedMountRequest:redacted>")
    }
}

/// Browser-publishable public seed with no instance or ledger authority.
pub struct PublicSeedMountOutput {
    body: Bytes,
    metadata: MountMetadata,
    revision: Revision,
}

impl PublicSeedMountOutput {
    /// Returns complete engine-owned island HTML.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Returns typed inert seed metadata.
    #[must_use]
    pub const fn metadata(&self) -> &MountMetadata {
        &self.metadata
    }

    /// Returns the fixed non-authoritative seed revision.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }
}

impl fmt::Debug for PublicSeedMountOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublicSeedMountOutput")
            .field("body_bytes", &self.body.len())
            .field("metadata", &self.metadata)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

/// Stateless seed publication service that cannot allocate server instance authority.
pub struct PublicSeedMountService {
    registry: Arc<ComponentRegistry>,
    clock: Arc<dyn Clock>,
    keys: Arc<SnapshotKeyRing>,
    snapshot_limits: SnapshotLimits,
    views: ViewRenderer,
    max_metadata_bytes: usize,
}

impl PublicSeedMountService {
    /// Creates a public seed service with an explicit metadata ceiling.
    pub fn new(
        providers: PublicMountProviders,
        snapshot_limits: SnapshotLimits,
        views: ViewRenderer,
        max_metadata_bytes: usize,
    ) -> Result<Self, MountError> {
        if !(1..=HARD_MAX_METADATA_BYTES).contains(&max_metadata_bytes) {
            return Err(MountError::new(MountErrorKind::InvalidConfiguration));
        }
        Ok(Self {
            registry: providers.registry,
            clock: providers.clock,
            keys: providers.keys,
            snapshot_limits,
            views,
            max_metadata_bytes,
        })
    }

    /// Signs and publishes a verified public seed without touching an instance ledger.
    pub fn mount(
        &self,
        document: &mut DocumentMountScope,
        request: PublicSeedMountRequest,
        context: &TrustedLiveRequestContext,
    ) -> Result<PublicSeedMountOutput, MountError> {
        let key = request.key.clone();
        document.reserve(key.clone())?;
        let result = self.mount_reserved(request, context);
        if result.is_err() {
            document.release(&key);
        }
        result
    }

    fn mount_reserved(
        &self,
        request: PublicSeedMountRequest,
        context: &TrustedLiveRequestContext,
    ) -> Result<PublicSeedMountOutput, MountError> {
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
        if request.seed.component() != expected.component()
            || request.seed.build_id() != expected.build_id()
            || request.seed.route() != expected.route()
            || request.seed.slot() != expected.slot()
            || request.seed.key_id() != self.keys.active_key_id()
        {
            return Err(MountError::new(MountErrorKind::SnapshotRejected));
        }
        self.views
            .validate_island_fragment(descriptor.metadata().view().clone(), &request.render)
            .map_err(|_| MountError::new(MountErrorKind::RenderRejected))?;
        let signed_snapshot = request
            .seed
            .sign(&self.keys, now, &self.snapshot_limits)
            .map_err(|_| MountError::new(MountErrorKind::SnapshotRejected))?;
        let metadata = MountMetadata::new(
            expected.slot().clone(),
            expected.component().name().clone(),
            MountSnapshotKind::PublicSeed,
            Bytes::from(signed_snapshot.clone()),
        )
        .map_err(|_| MountError::new(MountErrorKind::MetadataTooLarge))?;
        let revision = Revision::new(0);
        let assembled = assemble_island_root(
            request.render,
            IslandRootInput {
                component: catalog.component().clone(),
                slot: catalog.slot().clone(),
                document_key: request.key.as_str().to_owned(),
                protocol_minimum: catalog.minimum_protocol(),
                runtime_contract: 1,
                snapshot: Bytes::from(signed_snapshot),
                snapshot_form: IslandSnapshotForm::Seed,
                instance_id: None,
                revision,
                lazy_complete: false,
                flags: request
                    .flags
                    .iter()
                    .map(|(name, value)| IslandRootFlag::from_validated(name, value))
                    .collect(),
            },
            self.max_metadata_bytes,
        )
        .map_err(|_| MountError::new(MountErrorKind::MetadataTooLarge))?;
        let validated = self
            .views
            .validate_island_output(descriptor.metadata().view().clone(), assembled)
            .map_err(|_| MountError::new(MountErrorKind::RenderRejected))?;
        Ok(PublicSeedMountOutput {
            body: validated.body,
            metadata,
            revision,
        })
    }
}
