//! Public seed publication without instance allocation or ledger authority.

use std::fmt;
use std::sync::Arc;

use bytes::Bytes;

use super::{DocumentMountKey, DocumentMountScope, MountError, MountErrorKind, MountFlags};
use crate::canonical::CanonicalValue;
use crate::clock::Clock;
use crate::component::{ComponentExecutor, MountContext, RenderContext};
use crate::crypto::SnapshotKeyRing;
use crate::host::TrustedLiveRequestContext;
use crate::identity::{Revision, UnixMillis};
use crate::registry::ComponentRegistry;
use crate::snapshot::state::StateExposure;
use crate::snapshot::{MountedDocumentPath, SeedBodyV1, SeedFieldsV1, SnapshotLimits};
use crate::view::{
    IslandRender, IslandRootFlag, IslandRootInput, IslandSnapshotForm, MountMetadata,
    MountSnapshotKind, TrustedHtml, ViewRenderer, assemble_island_root,
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
    body: String,
    metadata: MountMetadata,
    revision: Revision,
    expires_at: UnixMillis,
}

impl PublicSeedMountOutput {
    /// Returns complete engine-owned island HTML.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        self.body.as_bytes()
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

    /// Returns the non-authoritative seed publication deadline used to
    /// bound how long a host may keep this seed publishable.
    #[must_use]
    pub const fn expires_at(&self) -> UnixMillis {
        self.expires_at
    }

    /// Consumes the completed mount into checked document markup and inert metadata.
    #[must_use]
    pub fn into_document_parts(self) -> (TrustedHtml, MountMetadata) {
        (
            TrustedHtml::engine_validated_island(self.body),
            self.metadata,
        )
    }
}

impl fmt::Debug for PublicSeedMountOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublicSeedMountOutput")
            .field("body_bytes", &self.body.len())
            .field("metadata", &self.metadata)
            .field("revision", &self.revision)
            .field("expires_at", &self.expires_at)
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
    island_stream_directive: bool,
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
            island_stream_directive: false,
            registry: providers.registry,
            clock: providers.clock,
            keys: providers.keys,
            snapshot_limits,
            views,
            max_metadata_bytes,
        })
    }

    /// Emits the island-owned `live:stream` directive on every island root
    /// whose component declares exactly one stream, so the browser runtime
    /// subscribes that island without application markup. Off by default:
    /// a host that drives subscriptions itself keeps its roots unchanged.
    #[must_use]
    pub const fn with_island_stream_directive(mut self) -> Self {
        self.island_stream_directive = true;
        self
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

    /// Runs the registered component mount lifecycle and publishes only its
    /// public seed exposure, without generating or persisting an instance ID.
    pub async fn mount_component(
        &self,
        document: &mut DocumentMountScope,
        key: DocumentMountKey,
        parameters: CanonicalValue,
        flags: MountFlags,
        context: &TrustedLiveRequestContext,
    ) -> Result<PublicSeedMountOutput, MountError> {
        document.reserve(key.clone())?;
        let result = self
            .mount_component_reserved(key.clone(), parameters, flags, None, context)
            .await;
        if result.is_err() {
            document.release(&key);
        }
        result
    }

    /// Mounts a component while sealing its matched document path into the signed seed.
    pub async fn mount_component_for_document(
        &self,
        document: &mut DocumentMountScope,
        key: DocumentMountKey,
        parameters: CanonicalValue,
        flags: MountFlags,
        document_path: &MountedDocumentPath,
        context: &TrustedLiveRequestContext,
    ) -> Result<PublicSeedMountOutput, MountError> {
        document.reserve(key.clone())?;
        let result = self
            .mount_component_reserved(key.clone(), parameters, flags, Some(document_path), context)
            .await;
        if result.is_err() {
            document.release(&key);
        }
        result
    }

    async fn mount_component_reserved(
        &self,
        key: DocumentMountKey,
        parameters: CanonicalValue,
        flags: MountFlags,
        document_path: Option<&MountedDocumentPath>,
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
        expected
            .schemas()
            .mount()
            .validate(&parameters, StateExposure::PublicSeed)
            .map_err(|_| MountError::new(MountErrorKind::ParametersRejected))?;
        descriptor
            .parameter_schema()
            .validate(&parameters, self.snapshot_limits.input())
            .map_err(|_| MountError::new(MountErrorKind::ParametersRejected))?;
        let expires_at = now
            .get()
            .checked_add(self.snapshot_limits.max_seed_age_ms())
            .map(UnixMillis::new)
            .ok_or_else(|| MountError::new(MountErrorKind::ClockUnavailable))?;
        let revision = Revision::new(0);
        let render_context = RenderContext::for_public_seed(context, revision, expires_at);
        let mount_context = MountContext::new(render_context, &parameters);
        let lifecycle = ComponentExecutor::new()
            .initial_public_mount(descriptor, &mount_context)
            .await
            .map_err(|_| MountError::new(MountErrorKind::LifecycleRejected))?;
        let (render, state, memo) = lifecycle.into_parts();
        expected
            .schemas()
            .state()
            .validate(&state, StateExposure::PublicSeed)
            .and_then(|_| {
                expected
                    .schemas()
                    .memo()
                    .validate(&memo, StateExposure::PublicSeed)
            })
            .map_err(|_| MountError::new(MountErrorKind::SnapshotRejected))?;
        let extensions = document_path
            .map(MountedDocumentPath::extension)
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect();
        let seed = SeedBodyV1::new(
            SeedFieldsV1 {
                component: expected.component().clone(),
                build_id: expected.build_id().clone(),
                route: expected.route().clone(),
                slot: expected.slot().clone(),
                key_id: self.keys.active_key_id().clone(),
                issued_at: now,
                max_age_ms: self.snapshot_limits.max_seed_age_ms(),
                mount: parameters,
                state,
                memo,
                advisory_generations: Vec::new(),
                refresh_on_promote: descriptor.metadata().refresh_on_promote(),
                extensions,
            },
            expected.schemas(),
            &self.snapshot_limits,
        )
        .map_err(|_| MountError::new(MountErrorKind::SnapshotRejected))?;
        self.mount_reserved(
            PublicSeedMountRequest::new(key, seed, render, flags),
            context,
        )
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
        let expires_at = now
            .get()
            .checked_add(self.snapshot_limits.max_seed_age_ms())
            .map(UnixMillis::new)
            .ok_or_else(|| MountError::new(MountErrorKind::ClockUnavailable))?;
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
                stream: self
                    .island_stream_directive
                    .then(|| crate::view::declared_stream(descriptor.metadata()))
                    .flatten(),
            },
            self.max_metadata_bytes,
        )
        .map_err(|_| MountError::new(MountErrorKind::MetadataTooLarge))?;
        let validated = self
            .views
            .validate_island_output(descriptor.metadata().view().clone(), assembled)
            .map_err(|_| MountError::new(MountErrorKind::RenderRejected))?;
        Ok(PublicSeedMountOutput {
            body: String::from_utf8(validated.body.to_vec())
                .map_err(|_| MountError::new(MountErrorKind::RenderRejected))?,
            metadata,
            revision,
            expires_at,
        })
    }
}
