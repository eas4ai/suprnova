//! Typed registration and request-time assembly for canonical Live documents.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroU32;

use bytes::Bytes;
use sha2::{Digest, Sha256};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::host::{
    MountCatalogEntry, MountScopeRequirements, MountSelection, ScopeRequirement,
};
use suprnova_live::identity::{BuildId, ComponentName, ContentDigest, IslandSlot, RouteIdentity};
use suprnova_live::mount::{DocumentMountKey, DocumentMountScope, MountFlags, PrivateMountRequest};
use suprnova_live::render_cache::composite::{
    MAX_FALLBACK_BYTES, MAX_SLOT_PARAMETER_BYTES, Segment, SlotFailurePolicy,
};
use suprnova_live::render_cache::key::RenderKey;
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

/// What a stitched hit does when this island cannot be rendered for the
/// request.
///
/// Only a route declared as a stitched public shell ever consults this: on
/// every other route the island is rendered inline and there is nothing to
/// fall back from.
#[derive(Clone)]
pub enum StitchFailurePolicy {
    /// The cached shell is not used; the route handler renders the request
    /// uncached, exactly as it would have without the cache.
    FailDocument,
    /// The document is served without this island.
    Omit,
    /// This checked fragment takes the island's place.
    Fallback(TrustedHtml),
}

impl fmt::Debug for StitchFailurePolicy {
    /// Never prints the fallback markup: a policy is carried in declarations
    /// and errors that may be logged, and the fragment is application
    /// markup, not a diagnostic.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::FailDocument => "fail_document",
            Self::Omit => "omit",
            Self::Fallback(_) => "fallback(<checked>)",
        })
    }
}

/// One identity-bound island's declaration, in the framework's own typed
/// identities, as a stitched shell has to record it.
///
/// This is what a later hit needs to mount the island again: which
/// component, under which contract and protocol, with which parameters and
/// inert flags, and what to do when that mount fails. The engine's
/// `StitchSlot` spells the same facts as bounded strings; converting is the
/// publisher's job, not this type's.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StitchSlotDescriptor {
    /// Canonical route identity the declaration belongs to.
    pub route: RouteIdentity,
    /// Island slot within that route's document.
    pub slot: IslandSlot,
    /// Server-declared document mount key.
    pub document_key: DocumentMountKey,
    /// Registered component name.
    pub component: ComponentName,
    /// Component contract digest.
    pub contract_digest: ContentDigest,
    /// Protocol version the mount was declared with.
    pub protocol: u16,
    /// Build identity the declaration belongs to.
    pub build: BuildId,
    /// RFC 8785 canonical JSON of the mount parameters.
    pub parameters: String,
    /// Inert mount flags.
    pub flags: MountFlags,
    /// Declared behavior when the island cannot be rendered on a hit.
    pub on_failure: SlotFailurePolicy,
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
    stitch_failure: StitchFailurePolicy,
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
            stitch_failure: self.stitch_failure.clone(),
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
            // The safe default: an island that cannot be rendered for this
            // request means the shell is not used at all.
            stitch_failure: StitchFailurePolicy::FailDocument,
            marker: PhantomData,
        })
    }

    /// Returns the declared publication form.
    #[must_use]
    pub const fn kind(&self) -> LiveMountKind {
        self.kind
    }

    /// Declares what a stitched hit does when this island cannot be
    /// rendered for the request. The default is
    /// [`StitchFailurePolicy::FailDocument`].
    ///
    /// The policy is consulted only for an identity-bound mount on a route
    /// declared `RepresentationClass::PublicShellStitched`, the one class
    /// that re-renders islands on a hit. On a public-seed mount, or on any
    /// other class, it is accepted and inert: the island is rendered inline
    /// by the handler and there is no stitched hit to fall back from.
    ///
    /// A fallback fragment is limited to `MAX_FALLBACK_BYTES`, the bound the
    /// stored entry itself applies; a larger one is rejected here, where the
    /// declaration is written, rather than silently at publication time.
    pub fn on_stitch_failure(
        mut self,
        policy: StitchFailurePolicy,
    ) -> Result<Self, LiveDocumentError> {
        validate_stitch_failure_policy(&policy)?;
        self.stitch_failure = policy;
        Ok(self)
    }

    /// This declaration as a stitched shell has to record it, for one
    /// request's parameters and inert flags.
    ///
    /// Fails when the parameters cannot be spelled as a canonical document
    /// within the slot's own bound. That is not a mount failure: the island
    /// still renders, and the caller records the capture as one no shell can
    /// be built from instead of turning a working document into an error.
    pub(crate) fn stitch_descriptor(
        &self,
        parameters: &CanonicalValue,
        flags: &MountFlags,
    ) -> Result<StitchSlotDescriptor, LiveDocumentError> {
        let limits = suprnova_live::limits::InputLimits::new(
            MAX_SLOT_PARAMETER_BYTES,
            32,
            512,
            MAX_SLOT_PARAMETER_BYTES,
        )
        .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::InvalidMount))?;
        let canonical = suprnova_live::canonical::to_canonical_bytes(parameters, &limits)
            .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::InvalidMount))?;
        let parameters = String::from_utf8(canonical)
            .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::InvalidMount))?;
        Ok(StitchSlotDescriptor {
            route: self.route.clone(),
            slot: self.slot.clone(),
            document_key: self.document_key.clone(),
            component: self.component.clone(),
            contract_digest: self.contract.clone(),
            protocol: self.protocol,
            build: self.build.clone(),
            parameters,
            flags: flags.clone(),
            on_failure: match &self.stitch_failure {
                StitchFailurePolicy::FailDocument => SlotFailurePolicy::FailDocument,
                StitchFailurePolicy::Omit => SlotFailurePolicy::Omit,
                StitchFailurePolicy::Fallback(html) => SlotFailurePolicy::Fallback {
                    html: html.as_str().to_owned(),
                },
            },
        })
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
        match self.kind {
            LiveMountKind::PublicSeed => MountScopeRequirements::new(
                ScopeRequirement::Optional,
                ScopeRequirement::Optional,
                ScopeRequirement::Optional,
            ),
            // An identity-bound island belongs to a session and a principal.
            // The tenant is bound into the scope whenever the application's
            // resolver names one and stays absent for a single-tenant
            // deployment; a request from another tenant, or a tenant-less
            // request against a tenant-bound instance, still fails the scope
            // comparison.
            LiveMountKind::IdentityBound => MountScopeRequirements::new(
                ScopeRequirement::Required,
                ScopeRequirement::Required,
                ScopeRequirement::Optional,
            ),
        }
    }
}

/// Whether a [`LiveNestedSegment`] declares a per-request identity binding,
/// and, if it does, the one includer path length every declaration naming
/// the same inner route MUST agree on.
///
/// Carried separately from [`LiveNestedSegment`] itself so
/// [`Router::try_live_nested_segment`] can compare two declarations for the
/// same inner route without needing the rest of either one: `on_failure` is
/// a per-including-document choice (see [`LiveNestedSegment`]'s own doc)
/// and never participates in this comparison.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NestedSegmentIdentity {
    /// Declares no identity binding at all: nothing in the segment varies
    /// by includer, reauthorization is skipped at resolution, and the
    /// entry may be shared by any including document unconditionally.
    Free,
    /// Declares a per-request identity binding: the segment may be shared
    /// only among includers whose own resolved document path is exactly
    /// `includer_path_len` bytes (see [`LiveNestedSegment::identity_bound`]
    /// for why).
    Bound {
        /// The one includer path length every declaration naming this
        /// inner route under this binding MUST share.
        includer_path_len: NonZeroU32,
    },
}

/// One nested cached segment declaration: the typed way an author marks a
/// route's own cached representation as includable, by name, from another
/// cached document's graph, and carries the failure policy that inclusion
/// uses when it cannot be resolved - [`LiveMount`]'s counterpart for
/// something that is not an island.
///
/// `route_pattern` names which route's cached representation this
/// declaration is about, the same way [`LiveMount`]'s own declared route
/// pattern names the route an island mount belongs to; a later resolution
/// step (not built by this declaration) derives the engine's actual
/// `RenderKey` and the entry's current stored version and length, and hands
/// them to [`Self::into_segment`] to produce the typed graph fact the
/// engine's `Segment::Nested` variant needs.
///
/// Whether the segment [`Self::is_identity_bound`] governs a length
/// constraint on sharing, never a security decision: at resolution,
/// `resolve_nested_segment` decides reauthorization from the fetched
/// inner entry's own structure, not from what any declaration claims, so a
/// wrong flag here can only ever turn into a resolution failure through
/// this declaration's own policy - never a leak. See
/// [`Self::identity_bound`] for the constraint this exists to make loud.
#[derive(Clone, Debug)]
pub struct LiveNestedSegment {
    route_pattern: String,
    identity: NestedSegmentIdentity,
    on_failure: StitchFailurePolicy,
}

impl LiveNestedSegment {
    /// Declares `route_pattern`'s own cached representation as an
    /// identity-free nested segment: it declares no identity binding at
    /// all, so resolution skips reauthorization and the entry may be
    /// shared by any including document unconditionally.
    pub fn identity_free(route_pattern: &str) -> Result<Self, LiveDocumentError> {
        Ok(Self {
            route_pattern: validated_nested_pattern(route_pattern)?,
            identity: NestedSegmentIdentity::Free,
            on_failure: StitchFailurePolicy::FailDocument,
        })
    }

    /// Declares `route_pattern`'s own cached representation as an
    /// identity-bound nested segment, shareable only by includers whose own
    /// resolved document path is exactly as long as
    /// `includer_route_pattern`'s.
    ///
    /// # The length-stability constraint
    ///
    /// An identity-bound inner segment's re-mount embeds the *including*
    /// document's own resolved path into its signed snapshot (spec 16,
    /// "Nested cached segments"), so the inner entry's actual assembled
    /// length depends on that path's byte length. The engine stores one
    /// `assembled_len` per graph segment, so one inner entry cannot match
    /// two includers whose resolved paths differ in length: the mismatch
    /// is always caught safely (`verify_nested` resolves it through this
    /// declaration's own [`Self::on_failure`] policy, never a wrong
    /// document), but the sharing this segment exists to provide is lost
    /// silently, and a real deployment reads a segment that never resolves
    /// as a bug rather than a declaration error.
    ///
    /// `includer_route_pattern` closes that gap by requiring the includer's
    /// own *literal* route pattern - not a general pattern that could match
    /// several paths - up front: a pattern naming a `{parameter}` is
    /// refused outright, because such a route resolves to a different path
    /// length on almost every request, so not even one such includer could
    /// promise the constant length this segment's re-mount needs. A
    /// literal pattern's resolved path is always the pattern text itself,
    /// so its byte length is knowable, and checked, right here.
    ///
    /// A second includer of the same inner segment whose own literal path
    /// differs in length needs its own, separate `LiveNestedSegment` value,
    /// never this one reused for both, and
    /// [`Router::try_live_nested_segment`] refuses at router construction,
    /// not at a request, when two declarations naming the same inner route
    /// disagree about this.
    pub fn identity_bound(
        route_pattern: &str,
        includer_route_pattern: &str,
    ) -> Result<Self, LiveDocumentError> {
        let route_pattern = validated_nested_pattern(route_pattern)?;
        let includer_route_pattern = validated_nested_pattern(includer_route_pattern)?;
        if includer_route_pattern.contains('{') {
            return Err(LiveDocumentError::new(
                LiveDocumentErrorKind::DynamicIncluderPath,
            ));
        }
        let includer_path_len = u32::try_from(includer_route_pattern.len())
            .ok()
            .and_then(NonZeroU32::new)
            .ok_or_else(|| LiveDocumentError::new(LiveDocumentErrorKind::InvalidDeclaration))?;
        Ok(Self {
            route_pattern,
            identity: NestedSegmentIdentity::Bound { includer_path_len },
            on_failure: StitchFailurePolicy::FailDocument,
        })
    }

    /// Declares what a resolution attempt does when this segment cannot be
    /// resolved for a request, including a mismatch against
    /// [`Self::identity_bound`]'s own declared includer path length. The
    /// default is [`StitchFailurePolicy::FailDocument`].
    ///
    /// A fallback fragment is limited to `MAX_FALLBACK_BYTES`, checked here
    /// rather than silently at resolution, exactly as
    /// [`LiveMount::on_stitch_failure`] checks it for an island.
    pub fn on_failure(mut self, policy: StitchFailurePolicy) -> Result<Self, LiveDocumentError> {
        validate_stitch_failure_policy(&policy)?;
        self.on_failure = policy;
        Ok(self)
    }

    /// Whether this declaration binds a per-request identity, and so
    /// constrains sharing to includers of one exact resolved path length
    /// (see [`Self::identity_bound`]). Never a security decision on its
    /// own; see this type's own doc.
    #[must_use]
    pub const fn is_identity_bound(&self) -> bool {
        matches!(self.identity, NestedSegmentIdentity::Bound { .. })
    }

    /// The nested route's own declared pattern.
    #[must_use]
    pub fn route_pattern(&self) -> &str {
        &self.route_pattern
    }

    /// Converts this declaration and the facts a resolution step already
    /// fetched into the engine's typed graph fact, the [`Segment::Nested`]
    /// variant.
    ///
    /// `includer_path` MUST be the *actual* resolved path of the document
    /// being built right now. For an identity-bound declaration this checks
    /// it against [`Self::identity_bound`]'s own declared length - the
    /// constraint this type exists to make loud rather than silent - right
    /// here, in the one place every future caller has to pass through to
    /// produce a segment at all, so the check cannot be skipped by a caller
    /// that forgets a separate step. A mismatch is a declaration error, not
    /// a trust decision: resolution itself decides reauthorization from the
    /// fetched inner entry's own structure, never from this check, so a
    /// wrong length here can only turn into a resolution failure through
    /// this declaration's own policy - never a leak.
    ///
    /// `key`, `version`, and `assembled_len` are the exact facts the
    /// engine's own [`Segment::Nested`] documents: a `RenderKey` derived for
    /// the named route, the inner entry's stored version at the moment it
    /// was fetched, and its own assembled length.
    pub fn into_segment(
        &self,
        key: RenderKey,
        version: u64,
        assembled_len: u32,
        includer_path: &MountedDocumentPath,
    ) -> Result<Segment, LiveDocumentError> {
        if let NestedSegmentIdentity::Bound { includer_path_len } = self.identity {
            let matches = u32::try_from(includer_path.as_str().len())
                .is_ok_and(|actual| actual == includer_path_len.get());
            // An error channel, not a `debug_assert!`: unlike the
            // structurally-impossible cases elsewhere in this module, the
            // includer path is per-request data crossing this API boundary
            // on every call, so a mismatch here is an ordinary, expected,
            // and testable failure mode - the very thing this fallible
            // conversion exists to report - never an invariant a panic
            // would be answering.
            if !matches {
                return Err(LiveDocumentError::new(
                    LiveDocumentErrorKind::NestedSegmentLengthMismatch,
                ));
            }
        }
        Ok(Segment::Nested {
            key,
            version,
            assembled_len,
            on_failure: match &self.on_failure {
                StitchFailurePolicy::FailDocument => SlotFailurePolicy::FailDocument,
                StitchFailurePolicy::Omit => SlotFailurePolicy::Omit,
                StitchFailurePolicy::Fallback(html) => SlotFailurePolicy::Fallback {
                    html: html.as_str().to_owned(),
                },
            },
        })
    }
}

/// Validates a nested segment's own route pattern, or an
/// [`LiveNestedSegment::identity_bound`] includer pattern before its own
/// dynamic-segment check: the same application-path shape `LiveMount::new`
/// requires.
fn validated_nested_pattern(pattern: &str) -> Result<String, LiveDocumentError> {
    if !pattern.starts_with('/') || pattern.starts_with("/__live/") {
        return Err(LiveDocumentError::new(
            LiveDocumentErrorKind::InvalidDeclaration,
        ));
    }
    Ok(pattern.to_owned())
}

impl Router {
    /// Registers one nested cached segment declaration, checked against
    /// every earlier declaration naming the same inner route pattern.
    ///
    /// This is where the length-stability trap
    /// [`LiveNestedSegment::identity_bound`]'s own doc describes is made
    /// loud rather than silent: two includers naming the same inner route
    /// MUST agree on whether it is identity-bound and, if so, on the
    /// includer path length it was bound to. Disagreement is refused here,
    /// at router construction - before any request is ever served - rather
    /// than discovered later as a resolution failure that reads like a bug
    /// at the point of use. A different [`StitchFailurePolicy`] across
    /// includers is not a conflict: it is a per-including-document
    /// decision (see [`LiveNestedSegment`]'s own doc), so it never
    /// participates in this check.
    pub fn try_live_nested_segment(
        mut self,
        segment: &LiveNestedSegment,
    ) -> Result<Self, FrameworkError> {
        self.register_live_nested_segment_entry(
            segment.route_pattern.clone(),
            segment.identity.clone(),
        )?;
        Ok(self)
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
            mount.kind,
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
            .validate_request_context(self.request, selection)
            .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::ContextRejected))?;
        let key = declaration.document_key.clone();
        // Recorded here, at the mount that just succeeded, not later at
        // `render`: `MountedIsland::html()` is `pub` and `TrustedHtml`
        // implements `Display`, so a handler can take the returned island
        // and hand-build its own response without ever calling `render` at
        // all. The mount kind and (for a public seed) its deadline are
        // security-relevant the moment the mount succeeds, so the fact
        // must exist regardless of whether `render` is ever reached.
        let (html, metadata) = match declaration.kind {
            LiveMountKind::PublicSeed => {
                let output = self
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
                    .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::InvalidMount))?;
                crate::render_cache::live::record_mount(
                    LiveMountKind::PublicSeed,
                    Some(output.expires_at().get()),
                );
                // A public seed is the same for everybody, so a stitched
                // shell keeps its bytes: only the fact that the island is
                // in there is recorded, never a slot to re-mount. Guarded
                // on an active collector because the recording copies the
                // slot and the key into owned strings; the recording itself
                // is already a no-op outside a scope, so this only skips
                // the two allocations an uncached request would waste.
                if crate::render_cache::collector::is_active() {
                    crate::render_cache::live::record_shell_island(
                        &declaration.slot,
                        &declaration.document_key,
                    );
                }
                output.into_document_parts()
            }
            LiveMountKind::IdentityBound => {
                // Built before the parameters are moved into the request,
                // and never with `?`: an island whose parameters cannot be
                // spelled within the slot's bound still renders here, it
                // just cannot be stitched later.
                //
                // Only inside a collector scope, the read-site idiom
                // `collector::is_active` documents: the descriptor
                // canonicalizes the parameters and clones the mount's
                // identities, and the recording it feeds below is a no-op
                // without a scope, so an uncached request should pay one
                // `try_with` and nothing else. The check has to happen
                // here, before `parameters` moves into the request, so the
                // `Option` it produces is also what gates the island copy
                // after the mount; a scope cannot start or end in between.
                let descriptor = crate::render_cache::collector::is_active()
                    .then(|| declaration.stitch_descriptor(&parameters, &flags));
                // The mount runs in the slot bucket: whatever it reads is
                // re-read on every stitched hit and must not be recorded as
                // something the shared shell depends on.
                let output = crate::render_cache::collector::slot_scope(
                    self.runtime.mount_private_component(
                        &mut self.scope,
                        PrivateMountRequest::new(key, parameters, flags)
                            .with_document_path(document_path),
                        &context,
                    ),
                )
                .await
                .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::InvalidMount))?;
                crate::render_cache::live::record_mount(LiveMountKind::IdentityBound, None);
                let (html, metadata) = output.into_document_parts();
                match descriptor {
                    // The recorded bytes are the island's own markup, the
                    // same `TrustedHtml` the template is about to insert.
                    Some(Ok(descriptor)) => crate::render_cache::live::record_stitch_slot(
                        crate::render_cache::live::CapturedSlot {
                            descriptor,
                            html: Bytes::from(html.as_str().as_bytes().to_vec()),
                        },
                    ),
                    Some(Err(_)) => crate::render_cache::live::record_stitch_capture_invalid(),
                    // No collector was active above, so there is nothing to
                    // record and the island's markup is never copied.
                    None => {}
                }
                (html, metadata)
            }
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
        // After `render_bootstrap` accepted the options, so the recorded
        // value is the nonce the emitted script elements actually carry.
        crate::render_cache::live::record_bootstrap_nonce(options.nonce());
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
        // The bytes a stitched shell would be cut from are known only
        // here, and these are the same bytes `document_response` hands to
        // the response below. Hashed only inside a collector scope: the
        // recording is a no-op without one, so an uncached response would
        // pay a whole-body SHA-256 for nothing.
        if crate::render_cache::collector::is_active() {
            crate::render_cache::live::record_document_digest(Sha256::digest(&render.body).into());
        }
        // The mount kind and seed deadline facts are already recorded, at
        // `mount` (see its own doc for why); only the document's cache
        // intent is known here, so this call records only that.
        crate::render_cache::live::record_document_intent(&render.response);
        document_response(render)
            .map_err(|_| LiveDocumentError::new(LiveDocumentErrorKind::RenderRejected))
    }
}

/// Rejects a [`StitchFailurePolicy::Fallback`] fragment larger than
/// `MAX_FALLBACK_BYTES`, the bound the stored entry itself applies -
/// checked here, where the declaration is written, rather than silently at
/// publication time. Shared by [`LiveMount::on_stitch_failure`] and
/// [`LiveNestedSegment::on_failure`], the two declarations that carry a
/// [`StitchFailurePolicy`].
fn validate_stitch_failure_policy(policy: &StitchFailurePolicy) -> Result<(), LiveDocumentError> {
    if let StitchFailurePolicy::Fallback(html) = policy
        && html.as_str().len() > MAX_FALLBACK_BYTES
    {
        return Err(LiveDocumentError::new(
            LiveDocumentErrorKind::StitchFallbackTooLarge,
        ));
    }
    Ok(())
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
    /// A declared stitch fallback fragment exceeded the stored entry's bound.
    StitchFallbackTooLarge,
    /// An identity-bound nested segment's declared includer route pattern
    /// names a path parameter, so its resolved path length is not constant.
    DynamicIncluderPath,
    /// An identity-bound nested segment's actual includer path did not
    /// match the length its declaration was bound to.
    NestedSegmentLengthMismatch,
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
            LiveDocumentErrorKind::StitchFallbackTooLarge => "live_stitch_fallback_too_large",
            LiveDocumentErrorKind::DynamicIncluderPath => {
                "live_nested_segment_dynamic_includer_path"
            }
            LiveDocumentErrorKind::NestedSegmentLengthMismatch => {
                "live_nested_segment_length_mismatch"
            }
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
