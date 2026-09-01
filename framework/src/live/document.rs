//! Typed registration and request-time assembly for canonical Live documents.

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
            build,
            route.clone(),
            slot.clone(),
            schemas,
        );
        Ok(Self {
            route_pattern: route_pattern.to_owned(),
            route,
            slot,
            document_key,
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
        let public = self.kind == LiveMountKind::PublicSeed;
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

    fn scope_requirements(&self) -> MountScopeRequirements {
        let requirement = if self.kind == LiveMountKind::PublicSeed {
            ScopeRequirement::Optional
        } else {
            ScopeRequirement::Required
        };
        MountScopeRequirements::new(requirement, requirement, requirement)
    }
}

impl Router {
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
        self.register_live_mount_entry(
            MountCatalogEntry::new(mount.expected.clone(), mount.scope_requirements())
                .with_document_key(mount.document_key.clone()),
        )?;
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
        })
    }

    /// Runs the declared mount policy and returns checked SSR island markup.
    pub async fn mount<C: ComponentContract>(
        &mut self,
        declaration: &LiveMount<C>,
        parameters: CanonicalValue,
        flags: MountFlags,
    ) -> Result<MountedIsland, LiveDocumentError> {
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
