//! Typed registration and request-time assembly for canonical Live documents.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;

use sha2::{Digest, Sha256};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::host::{
    MountCatalogEntry, MountScopeRequirements, MountSelection, ScopeRequirement,
};
use suprnova_live::identity::{BuildId, IslandSlot, RouteIdentity};
use suprnova_live::mount::{DocumentMountKey, DocumentMountScope, MountFlags, PrivateMountRequest};
use suprnova_live::snapshot::{
    ComponentContract as SnapshotContract, ExpectedSeedV1, MountedDocumentPath,
};

use crate::view::{
    AssetSet, DocumentResponseIntent, MountMetadata, RenderLimits, TrustedHtml, ViewName,
    ViewRenderer, ViewTemplate, document_response,
};
use crate::{App, FrameworkError, HttpResponse, Request, Router};

use super::assets::{
    BootstrapFailure, LiveBootstrap, LiveBootstrapOptions, RequiredCapability, render_bootstrap,
};
use super::attestation::LiveOperation;
use super::context::LiveRouteSecurityPolicy;
use super::{ComponentContract, LiveRuntime};

/// Initial snapshot form and identity policy for one declared island mount.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveMountKind {
    /// Reusable anonymous state promoted to an instance on first action.
    PublicSeed,
    /// Request-scoped state with ledger authority before HTML publication.
    IdentityBound,
}

/// Immutable typed route/slot declaration shared by startup and its handler.
pub struct LiveMount<C> {
    route_pattern: String,
    route: RouteIdentity,
    slot: IslandSlot,
    document_key: DocumentMountKey,
    build: BuildId,
    expected: ExpectedSeedV1,
    component: suprnova_live::identity::ComponentName,
    contract: suprnova_live::identity::ContentDigest,
    protocol: u16,
    kind: LiveMountKind,
    marker: PhantomData<fn() -> C>,
}

impl<C> Clone for LiveMount<C> {
    fn clone(&self) -> Self {
        Self {
            route_pattern: self.route_pattern.clone(),
            route: self.route.clone(),
            slot: self.slot.clone(),
            document_key: self.document_key.clone(),
            build: self.build.clone(),
            expected: self.expected.clone(),
            component: self.component.clone(),
            contract: self.contract.clone(),
            protocol: self.protocol,
            kind: self.kind,
            marker: PhantomData,
        }
    }
}

impl<C: ComponentContract> LiveMount<C> {
    /// Declares a reusable public-seed island on one canonical route and slot.
    pub fn public_seed(
        route_pattern: &str,
        slot: &str,
        document_key: &str,
    ) -> Result<Self, LiveDocumentError> {
        Self::new(route_pattern, slot, document_key, LiveMountKind::PublicSeed)
    }

    /// Declares an identity-bound island whose instance authority precedes output.
    pub fn identity_bound(
        route_pattern: &str,
        slot: &str,
        document_key: &str,
    ) -> Result<Self, LiveDocumentError> {
        Self::new(
            route_pattern,
            slot,
            document_key,
            LiveMountKind::IdentityBound,
        )
    }

    fn new(
        route_pattern: &str,
        slot: &str,
        document_key: &str,
        kind: LiveMountKind,
    ) -> Result<Self, LiveDocumentError> {
        if !route_pattern.starts_with('/') || route_pattern.starts_with("/__live/") {
            return Err(LiveDocumentError::new(
                LiveDocumentErrorKind::InvalidDeclaration,
            ));
        }
        let descriptor = C::__live_registration()
            .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::InvalidDeclaration))?
            .into_engine();
        let schemas = descriptor
            .snapshot_schemas()
            .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::InvalidDeclaration))?;
        let route = route_identity(route_pattern)?;
        let slot = IslandSlot::parse(slot)
            .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::InvalidDeclaration))?;
        let document_key = DocumentMountKey::parse(document_key)
            .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::InvalidDeclaration))?;
        let component = descriptor.metadata().identity().clone();
        let contract = descriptor.contract_digest().clone();
        let versions = descriptor.metadata().versions();
        let snapshot_contract = SnapshotContract::new(
            component.clone(),
            contract.clone(),
            schemas.state().version(),
            schemas.memo().version(),
            schemas.mount().version(),
        )
        .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::InvalidDeclaration))?;
        let build = BuildId::parse(concat!("suprnova-", env!("CARGO_PKG_VERSION")))
            .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::InvalidDeclaration))?;
        let expected = ExpectedSeedV1::new(
            snapshot_contract,
            build.clone(),
            route.clone(),
            slot.clone(),
            schemas,
        );
        Ok(Self {
            route_pattern: route_pattern.to_owned(),
            route,
            slot,
            document_key,
            build,
            expected,
            component,
            contract,
            protocol: versions.minimum_protocol(),
            kind,
            marker: PhantomData,
        })
    }

    /// Returns the declared publication form.
    #[must_use]
    pub const fn kind(&self) -> LiveMountKind {
        self.kind
    }

    pub(crate) const fn route(&self) -> &RouteIdentity {
        &self.route
    }

    pub(crate) const fn slot(&self) -> &IslandSlot {
        &self.slot
    }

    pub(crate) const fn component(&self) -> &suprnova_live::identity::ComponentName {
        &self.component
    }

    fn route_policy(&self) -> LiveRouteSecurityPolicy {
        document_policy(self.kind == LiveMountKind::PublicSeed)
    }

    fn scope_requirements(&self) -> MountScopeRequirements {
        let requirement = if self.kind == LiveMountKind::PublicSeed {
            ScopeRequirement::Optional
        } else {
            ScopeRequirement::Required
        };
        MountScopeRequirements::new(requirement, requirement, requirement)
    }
}

const fn document_policy(public: bool) -> LiveRouteSecurityPolicy {
    LiveRouteSecurityPolicy {
        trusted_internal_origin: true,
        stateless_csrf: true,
        stateless_session: public,
        anonymous_principal: public,
        tenantless: public,
        direct_peer: true,
        upstream_rate_limit: true,
        no_additional_middleware: true,
    }
}

impl Router {
    /// Declares a Live document route that mounts no island at startup.
    ///
    /// The document still emits bootstrap markup, so islands inserted later
    /// connect through the same runtime. Routes with declared mounts use
    /// [`Router::try_live_mount`] instead.
    pub fn try_live_document(mut self, route_pattern: &str) -> Result<Self, FrameworkError> {
        if !route_pattern.starts_with('/') || route_pattern.starts_with("/__live/") {
            return Err(FrameworkError::internal(
                "Live document routes must be application paths",
            ));
        }
        self.register_live_document_metadata(
            hyper::Method::GET,
            route_pattern,
            document_policy(true),
        )?;
        Ok(self)
    }

    /// Seals one typed document mount into the startup catalog.
    pub fn try_live_mount<C: ComponentContract>(
        mut self,
        mount: &LiveMount<C>,
    ) -> Result<Self, FrameworkError> {
        self.register_live_document_metadata(
            hyper::Method::GET,
            &mount.route_pattern,
            mount.route_policy(),
        )?;
        let selection = MountSelection::new(
            mount.route.clone(),
            mount.slot.clone(),
            mount.component.clone(),
            mount.contract.clone(),
            mount.protocol,
        );
        self.register_live_mount_entry(super::runtime::LiveMountRegistration::new(
            MountCatalogEntry::new(mount.expected.clone(), mount.scope_requirements())
                .with_document_key(mount.document_key.clone()),
            selection,
            mount.document_key.clone(),
            mount.build.clone(),
        ))?;
        Ok(self)
    }
}

/// Checked mounted-island markup that can cross only the audited template filter.
pub struct MountedIsland {
    html: TrustedHtml,
}

impl MountedIsland {
    /// Returns checked island markup for `|trusted_html` template insertion.
    #[must_use]
    pub const fn html(&self) -> &TrustedHtml {
        &self.html
    }
}

impl fmt::Debug for MountedIsland {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<MountedIsland:checked>")
    }
}

/// Request-bound whole-document collector for independently mounted islands.
pub struct LiveDocument<'a> {
    request: &'a Request,
    runtime: LiveRuntime,
    scope: DocumentMountScope,
    metadata: Vec<MountMetadata>,
    bootstrapped: bool,
}

impl<'a> LiveDocument<'a> {
    /// Opens a collector only inside a prepared Live document route.
    pub fn from_request(request: &'a Request) -> Result<Self, LiveDocumentError> {
        if request.live_operation() != Some(LiveOperation::Document) {
            return Err(LiveDocumentError::new(
                LiveDocumentErrorKind::UnpreparedRequest,
            ));
        }
        let runtime = App::resolve::<LiveRuntime>()
            .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::RuntimeUnavailable))?;
        Ok(Self {
            request,
            runtime,
            scope: DocumentMountScope::new(),
            metadata: Vec::new(),
            bootstrapped: false,
        })
    }

    /// Runs the declared mount policy and returns checked SSR island markup.
    pub async fn mount<C: ComponentContract>(
        &mut self,
        declaration: &LiveMount<C>,
        parameters: CanonicalValue,
        flags: MountFlags,
    ) -> Result<MountedIsland, LiveDocumentError> {
        if self.bootstrapped {
            return Err(LiveDocumentError::new(
                LiveDocumentErrorKind::MountAfterBootstrap,
            ));
        }
        if self.request.route_pattern() != Some(declaration.route_pattern.as_str()) {
            return Err(LiveDocumentError::new(LiveDocumentErrorKind::RouteMismatch));
        }
        let document_path = MountedDocumentPath::parse(self.request.path())
            .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::InvalidMount))?;
        let selection = MountSelection::new(
            declaration.route.clone(),
            declaration.slot.clone(),
            declaration.component.clone(),
            declaration.contract.clone(),
            declaration.protocol,
        );
        let context = self
            .runtime
            .validate_request_context(
                self.request,
                declaration.route.clone(),
                declaration.slot.clone(),
                selection,
            )
            .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::ContextRejected))?;
        let key = declaration.document_key.clone();
        let (html, metadata) = match declaration.kind {
            LiveMountKind::PublicSeed => self
                .runtime
                .mount_public_component(
                    &mut self.scope,
                    key,
                    parameters,
                    flags,
                    &document_path,
                    &context,
                )
                .await
                .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::InvalidMount))?
                .into_document_parts(),
            LiveMountKind::IdentityBound => self
                .runtime
                .mount_private_component(
                    &mut self.scope,
                    PrivateMountRequest::new(key, parameters, flags)
                        .with_document_path(document_path),
                    &context,
                )
                .await
                .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::InvalidMount))?
                .into_document_parts(),
        };
        self.metadata.push(metadata);
        Ok(MountedIsland { html })
    }

    /// Emits the inert configuration and ordered artifact tags this document needs.
    ///
    /// Roles follow every mounted component: the upload feature when a field
    /// declares an upload policy, the asynchronous feature when a component
    /// declares streams, and the Stimulus bridge only when requested. Call it
    /// once after the last mount; later mounts are rejected so the emitted
    /// roles always cover every island.
    pub fn bootstrap(
        &mut self,
        options: LiveBootstrapOptions,
    ) -> Result<LiveBootstrap, LiveDocumentError> {
        if self.bootstrapped {
            return Err(LiveDocumentError::new(
                LiveDocumentErrorKind::BootstrapRepeated,
            ));
        }
        let mut required = BTreeSet::new();
        for mount in &self.metadata {
            let metadata = self
                .runtime
                .component_metadata(mount.component())
                .ok_or_else(|| LiveDocumentError::new(LiveDocumentErrorKind::InvalidMount))?;
            if metadata
                .fields()
                .iter()
                .any(|field| field.upload_policy().is_some())
            {
                required.insert(RequiredCapability::Uploads);
            }
            if !metadata.subscriptions().is_empty() {
                required.insert(RequiredCapability::AsyncUpdates);
            }
        }
        let protocol = (
            suprnova_live::SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .copied()
                .min()
                .unwrap_or(1),
            suprnova_live::SUPPORTED_PROTOCOL_VERSIONS
                .iter()
                .copied()
                .max()
                .unwrap_or(1),
        );
        let bootstrap = render_bootstrap(&options, &required, self.runtime.config(), protocol)
            .map_err(|failure| {
                LiveDocumentError::new(match failure {
                    BootstrapFailure::AssetsUnavailable => LiveDocumentErrorKind::AssetsUnavailable,
                    BootstrapFailure::InvalidNonce => LiveDocumentErrorKind::InvalidBootstrap,
                    BootstrapFailure::MarkupRejected => LiveDocumentErrorKind::RenderRejected,
                })
            })?;
        self.bootstrapped = true;
        Ok(bootstrap)
    }

    /// Renders and adapts one complete canonical document after every mount succeeds.
    pub fn render<T: ViewTemplate + ?Sized>(
        self,
        view: ViewName,
        template: &T,
        response: DocumentResponseIntent,
        assets: AssetSet,
    ) -> Result<HttpResponse, LiveDocumentError> {
        let config = self.runtime.config();
        let limits = RenderLimits::new(
            config.max_response_bytes(),
            128,
            128,
            128,
            config.max_response_bytes().min(512 * 1024),
        )
        .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::RenderRejected))?;
        let render = ViewRenderer::new(limits)
            .and_then(|renderer| {
                renderer.render_document(view, template, response, assets, self.metadata)
            })
            .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::RenderRejected))?;
        document_response(render)
            .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::RenderRejected))
    }
}

fn route_identity(pattern: &str) -> Result<RouteIdentity, LiveDocumentError> {
    let mut digest = Sha256::new();
    digest.update(b"suprnova-live/route-identity/v1\0");
    digest.update(pattern.as_bytes());
    let bytes: [u8; 32] = digest.finalize().into();
    RouteIdentity::from_bytes(&bytes)
        .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::InvalidDeclaration))
}

/// Closed document registration, mount, and render failure classes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LiveDocumentErrorKind {
    /// Startup route, slot, or generated component metadata was invalid.
    InvalidDeclaration,
    /// The handler was not entered through a prepared Live document route.
    UnpreparedRequest,
    /// The immutable runtime was not available in the application container.
    RuntimeUnavailable,
    /// The declaration was used from another route.
    RouteMismatch,
    /// Current request authority could not satisfy the declared mount.
    ContextRejected,
    /// Component lifecycle, identity, snapshot, or duplicate-key checks failed.
    InvalidMount,
    /// The complete checked document could not be rendered or adapted.
    RenderRejected,
    /// The embedded browser artifacts failed validation and cannot be served.
    AssetsUnavailable,
    /// The bootstrap options carried an invalid value such as a malformed nonce.
    InvalidBootstrap,
    /// Bootstrap markup was requested twice for one document.
    BootstrapRepeated,
    /// An island was mounted after the bootstrap markup was already emitted.
    MountAfterBootstrap,
}

/// Redacted Live document failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct LiveDocumentError {
    kind: LiveDocumentErrorKind,
}

impl LiveDocumentError {
    const fn new(kind: LiveDocumentErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the stable closed failure class.
    #[must_use]
    pub const fn kind(self) -> LiveDocumentErrorKind {
        self.kind
    }
}

impl fmt::Display for LiveDocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            LiveDocumentErrorKind::InvalidDeclaration => "invalid_live_document_declaration",
            LiveDocumentErrorKind::UnpreparedRequest => "unprepared_live_document_request",
            LiveDocumentErrorKind::RuntimeUnavailable => "live_runtime_unavailable",
            LiveDocumentErrorKind::RouteMismatch => "live_document_route_mismatch",
            LiveDocumentErrorKind::ContextRejected => "live_document_context_rejected",
            LiveDocumentErrorKind::InvalidMount => "live_document_mount_rejected",
            LiveDocumentErrorKind::RenderRejected => "live_document_render_rejected",
            LiveDocumentErrorKind::AssetsUnavailable => "live_document_assets_unavailable",
            LiveDocumentErrorKind::InvalidBootstrap => "invalid_live_bootstrap",
            LiveDocumentErrorKind::BootstrapRepeated => "live_bootstrap_repeated",
            LiveDocumentErrorKind::MountAfterBootstrap => "live_mount_after_bootstrap",
        })
    }
}

impl fmt::Debug for LiveDocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for LiveDocumentError {}

impl From<LiveDocumentError> for FrameworkError {
    fn from(_: LiveDocumentError) -> Self {
        FrameworkError::internal("Live document request was rejected")
    }
}
