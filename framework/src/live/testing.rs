//! Focused assertion helpers for application Live tests.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::Request;

use super::attestation::{LiveOperation, SecurityCheck};
use super::{ActionOutcome, ActionResult, ComponentContract, LiveMount, LiveRuntime};

/// Signed, ledger-backed v2 child-delivery fixture for real HTTP adapter tests.
pub struct LiveChildParameterDeliveryFixture {
    runtime: LiveRuntime,
    child_snapshot: serde_json::Value,
    envelope: serde_json::Value,
    historical_v1_envelope: serde_json::Value,
    parent_snapshot: serde_json::Value,
    scope: suprnova_live::identity::ScopeFingerprint,
    parent_instance: suprnova_live::identity::InstanceId,
    child_instance: suprnova_live::identity::InstanceId,
}

impl LiveChildParameterDeliveryFixture {
    /// Returns the exact current signed child snapshot submitted by the browser.
    #[must_use]
    pub fn child_snapshot(&self) -> serde_json::Value {
        self.child_snapshot.clone()
    }

    /// Returns the canonical v2 admission carrier paired after parent commit.
    #[must_use]
    pub fn admission_carrier(&self) -> serde_json::Value {
        serde_json::json!({
            "envelope": self.envelope,
            "parent_snapshot": self.parent_snapshot,
        })
    }

    /// Returns a genuinely signed historical-v1 envelope for rejection tests only.
    #[must_use]
    pub fn historical_v1_envelope(&self) -> serde_json::Value {
        self.historical_v1_envelope.clone()
    }

    /// Reads the child ledger revision from the same runtime serving HTTP.
    pub async fn current_child_revision(&self) -> Result<u64, crate::FrameworkError> {
        self.runtime
            .child_revision_for_test(&self.scope, &self.child_instance)
            .await
    }

    /// Reads the accepted parent revision to prove later child failure is non-atomic.
    pub async fn current_parent_revision(&self) -> Result<u64, crate::FrameworkError> {
        self.runtime
            .child_revision_for_test(&self.scope, &self.parent_instance)
            .await
    }

    /// Advances only the parent ledger so this fixture's signed delivery becomes stale.
    pub async fn advance_parent_revision(&self) -> Result<u64, crate::FrameworkError> {
        self.runtime
            .advance_parent_revision_for_test(&self.scope, &self.parent_instance)
            .await
    }
}

/// Mints one exact signed child request against the runtime used by the real Live route.
#[allow(
    clippy::too_many_arguments,
    reason = "the hostile-route fixture keeps state, parameters, mounts, and all scope facts explicit"
)]
pub async fn prepare_child_parameter_delivery_for_test<P, C>(
    parent_mount: &LiveMount<P>,
    child_mount: &LiveMount<C>,
    parent_build_override: Option<&str>,
    previous_parameters: suprnova_live::canonical::CanonicalValue,
    next_parameters: suprnova_live::canonical::CanonicalValue,
    parent_state: suprnova_live::canonical::CanonicalValue,
    child_state: suprnova_live::canonical::CanonicalValue,
    session: Option<&[u8]>,
    principal: Option<&[u8]>,
    tenant: Option<&[u8]>,
) -> Result<LiveChildParameterDeliveryFixture, crate::FrameworkError>
where
    P: ComponentContract,
    C: ComponentContract,
{
    let session =
        session.map(|value| super::attestation::purpose_fingerprint(SecurityCheck::Session, value));
    let principal = principal
        .map(|value| super::attestation::purpose_fingerprint(SecurityCheck::Principal, value));
    let tenant =
        tenant.map(|value| super::attestation::purpose_fingerprint(SecurityCheck::Tenant, value));
    let scope = super::context::aggregate_scope(
        session.as_ref().map(<[u8; 32]>::as_slice),
        principal.as_ref().map(<[u8; 32]>::as_slice),
        tenant.as_ref().map(<[u8; 32]>::as_slice),
    )?;
    let runtime = LiveRuntime::bind()?;
    let parent_build_override = parent_build_override
        .map(suprnova_live::identity::BuildId::parse)
        .transpose()
        .map_err(|_| crate::FrameworkError::internal("parent fixture build rejected"))?;
    let fixture = runtime
        .prepare_child_parameter_fixture_for_test(
            parent_mount.component(),
            parent_mount.route(),
            parent_mount.slot(),
            parent_build_override,
            child_mount.component(),
            child_mount.route(),
            child_mount.slot(),
            scope.clone(),
            previous_parameters,
            next_parameters,
            parent_state,
            child_state,
        )
        .await?;
    Ok(LiveChildParameterDeliveryFixture {
        runtime,
        child_snapshot: serde_json::from_slice(&fixture.child_snapshot)
            .map_err(|_| crate::FrameworkError::internal("child fixture snapshot rejected"))?,
        envelope: serde_json::from_slice(&fixture.envelope)
            .map_err(|_| crate::FrameworkError::internal("child fixture envelope rejected"))?,
        historical_v1_envelope: serde_json::from_slice(&fixture.historical_v1_envelope).map_err(
            |_| crate::FrameworkError::internal("historical child fixture envelope rejected"),
        )?,
        parent_snapshot: serde_json::from_slice(&fixture.parent_snapshot)
            .map_err(|_| crate::FrameworkError::internal("parent fixture snapshot rejected"))?,
        scope: fixture.scope,
        parent_instance: fixture.parent_instance,
        child_instance: fixture.child_instance,
    })
}

/// Ordered framework checks required before a protected Live operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LiveSecurityCheck {
    /// Same-origin admission.
    Origin,
    /// Cross-site request-forgery token validation.
    Csrf,
    /// Session resolution.
    Session,
    /// Principal resolution.
    Principal,
    /// Tenant resolution.
    Tenant,
    /// Trusted-proxy normalization.
    Proxy,
    /// Rate-limit admission.
    RateLimit,
    /// Completion of the exact required middleware scope.
    Middleware,
}

impl LiveSecurityCheck {
    const fn index(self) -> usize {
        match self {
            Self::Origin => 0,
            Self::Csrf => 1,
            Self::Session => 2,
            Self::Principal => 3,
            Self::Tenant => 4,
            Self::Proxy => 5,
            Self::RateLimit => 6,
            Self::Middleware => 7,
        }
    }
}

/// Redacted disposition visible to framework integration tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveSecurityDisposition {
    /// The owning framework stage performed and passed the check.
    Passed,
    /// Explicit route policy declared the check inapplicable.
    NotRequired,
}

/// Closed route-policy reason observed on `NotRequired` security evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveSecurityPolicyReason {
    /// A trusted internal path owns origin admission.
    TrustedInternalOrigin,
    /// This request class is explicitly outside CSRF policy.
    StatelessCsrfPolicy,
    /// The route is explicitly stateless.
    StatelessRequest,
    /// The route explicitly permits an anonymous principal.
    AnonymousPrincipal,
    /// The route is explicitly outside tenant scoping.
    TenantlessRoute,
    /// The request arrived without trusted-proxy interpretation.
    DirectPeer,
    /// A declared upstream boundary owns rate admission.
    UpstreamRateLimited,
    /// No additional middleware prerequisite applies.
    NoAdditionalMiddleware,
}

const fn policy_reason(reason: suprnova_live::host::PolicyReason) -> LiveSecurityPolicyReason {
    match reason {
        suprnova_live::host::PolicyReason::TrustedInternalOrigin => {
            LiveSecurityPolicyReason::TrustedInternalOrigin
        }
        suprnova_live::host::PolicyReason::StatelessCsrfPolicy => {
            LiveSecurityPolicyReason::StatelessCsrfPolicy
        }
        suprnova_live::host::PolicyReason::StatelessRequest => {
            LiveSecurityPolicyReason::StatelessRequest
        }
        suprnova_live::host::PolicyReason::AnonymousPrincipal => {
            LiveSecurityPolicyReason::AnonymousPrincipal
        }
        suprnova_live::host::PolicyReason::TenantlessRoute => {
            LiveSecurityPolicyReason::TenantlessRoute
        }
        suprnova_live::host::PolicyReason::DirectPeer => LiveSecurityPolicyReason::DirectPeer,
        suprnova_live::host::PolicyReason::UpstreamRateLimited => {
            LiveSecurityPolicyReason::UpstreamRateLimited
        }
        suprnova_live::host::PolicyReason::NoAdditionalMiddleware => {
            LiveSecurityPolicyReason::NoAdditionalMiddleware
        }
    }
}

/// Live operation class used by the feature-gated hostile-request harness.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveTestOperation {
    /// State-changing component action.
    Action,
    /// Upload-control operation.
    Upload,
    /// Server-sent-event control or subscription operation.
    SseControl,
    /// WebSocket handshake admission.
    WebSocketHandshake,
}

/// Required slot in the immutable server-runtime provider graph.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveTestRuntimeProvider {
    /// Current-time source.
    Clock,
    /// Server-side random source.
    Random,
    /// Snapshot signing and verification keys.
    KeyRing,
    /// Island revision authority.
    Ledger,
    /// Current action authorization.
    Authorization,
    /// Host database transaction boundary.
    Transaction,
    /// Generated component validation dispatch.
    Validation,
    /// Post-acceptance event reporting.
    EventReporter,
    /// Bounded execution telemetry.
    Telemetry,
    /// Request-lifetime cancellation.
    Cancellation,
    /// HTTP response-intent projection.
    ResponseIntent,
    /// Authoritative upload state ledger.
    UploadLedger,
    /// Upload expiry and cleanup lease ledger.
    UploadCleanupLedger,
    /// Host-owned quarantine byte store.
    UploadQuarantine,
    /// Selected upload byte provider.
    UploadProvider,
    /// Reverse-proxy upload provider capability.
    UploadReverseProxy,
    /// Authoritative reverse-proxy progress capability.
    UploadReverseProxyProgress,
    /// Direct-upload provider capability.
    UploadDirect,
    /// Framework-owned upload authorization adapter.
    UploadAuthorizationAdapter,
    /// Upload authorization port.
    UploadAuthorization,
    /// Malware scanner port.
    UploadScanner,
    /// Application upload-validation port.
    UploadApplicationValidation,
    /// Immutable validation-evidence store.
    UploadEvidence,
    /// Host storage finalizer.
    UploadFinalizer,
    /// Current stream subscription authorization.
    SubscriptionAuthorization,
    /// Descriptor-scoped transport credential store.
    SubscriptionCredentials,
}

impl LiveTestRuntimeProvider {
    /// Every provider that must be present before traffic begins.
    pub const ALL: [Self; 26] = [
        Self::Clock,
        Self::Random,
        Self::KeyRing,
        Self::Ledger,
        Self::Authorization,
        Self::Transaction,
        Self::Validation,
        Self::EventReporter,
        Self::Telemetry,
        Self::Cancellation,
        Self::ResponseIntent,
        Self::UploadLedger,
        Self::UploadCleanupLedger,
        Self::UploadQuarantine,
        Self::UploadProvider,
        Self::UploadReverseProxy,
        Self::UploadReverseProxyProgress,
        Self::UploadDirect,
        Self::UploadAuthorizationAdapter,
        Self::UploadAuthorization,
        Self::UploadScanner,
        Self::UploadApplicationValidation,
        Self::UploadEvidence,
        Self::UploadFinalizer,
        Self::SubscriptionAuthorization,
        Self::SubscriptionCredentials,
    ];

    /// Stable redacted provider name used in boot diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        runtime_provider(self).name()
    }
}

const fn runtime_provider(
    provider: LiveTestRuntimeProvider,
) -> super::runtime::RuntimeProviderSlot {
    match provider {
        LiveTestRuntimeProvider::Clock => super::runtime::RuntimeProviderSlot::Clock,
        LiveTestRuntimeProvider::Random => super::runtime::RuntimeProviderSlot::Random,
        LiveTestRuntimeProvider::KeyRing => super::runtime::RuntimeProviderSlot::KeyRing,
        LiveTestRuntimeProvider::Ledger => super::runtime::RuntimeProviderSlot::Ledger,
        LiveTestRuntimeProvider::Authorization => {
            super::runtime::RuntimeProviderSlot::Authorization
        }
        LiveTestRuntimeProvider::Transaction => super::runtime::RuntimeProviderSlot::Transaction,
        LiveTestRuntimeProvider::Validation => super::runtime::RuntimeProviderSlot::Validation,
        LiveTestRuntimeProvider::EventReporter => {
            super::runtime::RuntimeProviderSlot::EventReporter
        }
        LiveTestRuntimeProvider::Telemetry => super::runtime::RuntimeProviderSlot::Telemetry,
        LiveTestRuntimeProvider::Cancellation => super::runtime::RuntimeProviderSlot::Cancellation,
        LiveTestRuntimeProvider::ResponseIntent => {
            super::runtime::RuntimeProviderSlot::ResponseIntent
        }
        LiveTestRuntimeProvider::UploadLedger => super::runtime::RuntimeProviderSlot::UploadLedger,
        LiveTestRuntimeProvider::UploadCleanupLedger => {
            super::runtime::RuntimeProviderSlot::UploadCleanupLedger
        }
        LiveTestRuntimeProvider::UploadQuarantine => {
            super::runtime::RuntimeProviderSlot::UploadQuarantine
        }
        LiveTestRuntimeProvider::UploadProvider => {
            super::runtime::RuntimeProviderSlot::UploadProvider
        }
        LiveTestRuntimeProvider::UploadReverseProxy => {
            super::runtime::RuntimeProviderSlot::UploadReverseProxy
        }
        LiveTestRuntimeProvider::UploadReverseProxyProgress => {
            super::runtime::RuntimeProviderSlot::UploadReverseProxyProgress
        }
        LiveTestRuntimeProvider::UploadDirect => super::runtime::RuntimeProviderSlot::UploadDirect,
        LiveTestRuntimeProvider::UploadAuthorizationAdapter => {
            super::runtime::RuntimeProviderSlot::UploadAuthorizationAdapter
        }
        LiveTestRuntimeProvider::UploadAuthorization => {
            super::runtime::RuntimeProviderSlot::UploadAuthorization
        }
        LiveTestRuntimeProvider::UploadScanner => {
            super::runtime::RuntimeProviderSlot::UploadScanner
        }
        LiveTestRuntimeProvider::UploadApplicationValidation => {
            super::runtime::RuntimeProviderSlot::UploadApplicationValidation
        }
        LiveTestRuntimeProvider::UploadEvidence => {
            super::runtime::RuntimeProviderSlot::UploadEvidence
        }
        LiveTestRuntimeProvider::UploadFinalizer => {
            super::runtime::RuntimeProviderSlot::UploadFinalizer
        }
        LiveTestRuntimeProvider::SubscriptionAuthorization => {
            super::runtime::RuntimeProviderSlot::SubscriptionAuthorization
        }
        LiveTestRuntimeProvider::SubscriptionCredentials => {
            super::runtime::RuntimeProviderSlot::SubscriptionCredentials
        }
    }
}

const SECURITY_CHECKS: [LiveSecurityCheck; 8] = [
    LiveSecurityCheck::Origin,
    LiveSecurityCheck::Csrf,
    LiveSecurityCheck::Session,
    LiveSecurityCheck::Principal,
    LiveSecurityCheck::Tenant,
    LiveSecurityCheck::Proxy,
    LiveSecurityCheck::RateLimit,
    LiveSecurityCheck::Middleware,
];

/// Redacted observation of request-carried Live security evidence.
pub struct LiveSecurityReport {
    present_count: usize,
    missing: Vec<LiveSecurityCheck>,
    dispositions: [Option<LiveSecurityDisposition>; 8],
    policy_reasons: [Option<LiveSecurityPolicyReason>; 8],
    order_valid: bool,
}

impl LiveSecurityReport {
    /// Returns whether every required check has framework-owned evidence.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty() && self.order_valid
    }

    /// Returns required checks for which no evidence was minted.
    #[must_use]
    pub fn missing_checks(&self) -> &[LiveSecurityCheck] {
        &self.missing
    }

    /// Returns how many distinct evidence slots are present.
    #[must_use]
    pub const fn present_count(&self) -> usize {
        self.present_count
    }

    /// Returns the evidence disposition for one exact check, when present.
    #[must_use]
    pub const fn disposition(&self, check: LiveSecurityCheck) -> Option<LiveSecurityDisposition> {
        self.dispositions[check.index()]
    }

    /// Returns the exact declared reason for `NotRequired` evidence.
    #[must_use]
    pub const fn policy_reason(
        &self,
        check: LiveSecurityCheck,
    ) -> Option<LiveSecurityPolicyReason> {
        self.policy_reasons[check.index()]
    }

    /// Returns whether successful middleware stages ran in the required order.
    #[must_use]
    pub const fn order_is_valid(&self) -> bool {
        self.order_valid
    }
}

impl fmt::Debug for LiveSecurityReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<LiveSecurityReport:redacted>")
    }
}

/// Returns a redacted testing view of one request's internal attestation.
#[must_use]
pub fn inspect_request_attestation(request: &Request) -> LiveSecurityReport {
    let attestation = request.live_security_attestation();
    let present = attestation.present();
    let missing = SECURITY_CHECKS
        .iter()
        .enumerate()
        .filter_map(|(index, check)| ((present & (1 << index)) == 0).then_some(*check))
        .collect();
    let dispositions = std::array::from_fn(|index| {
        let disposition = attestation.disposition(SecurityCheck::ALL[index])?;
        Some(match disposition {
            suprnova_live::host::CheckDisposition::Passed => LiveSecurityDisposition::Passed,
            suprnova_live::host::CheckDisposition::NotRequired(_) => {
                LiveSecurityDisposition::NotRequired
            }
        })
    });
    let policy_reasons = std::array::from_fn(|index| {
        let disposition = attestation.disposition(SecurityCheck::ALL[index])?;
        match disposition {
            suprnova_live::host::CheckDisposition::Passed => None,
            suprnova_live::host::CheckDisposition::NotRequired(reason) => {
                Some(policy_reason(reason))
            }
        }
    });
    LiveSecurityReport {
        present_count: present.count_ones() as usize,
        missing,
        dispositions,
        policy_reasons,
        order_valid: attestation.order_valid(),
    }
}

/// Marks a request as a framework-owned Live route in the testing harness.
///
/// This creates no security evidence; each owning middleware must still mint
/// its own proof after its exact successful branch.
#[must_use]
pub fn prepare_live_request_for_test(
    mut request: Request,
    operation: LiveTestOperation,
) -> Request {
    let operation = match operation {
        LiveTestOperation::Action => LiveOperation::Action,
        LiveTestOperation::Upload => LiveOperation::Upload,
        LiveTestOperation::SseControl => LiveOperation::SseControl,
        LiveTestOperation::WebSocketHandshake => LiveOperation::WebSocketHandshake,
    };
    let prepared = request.prepare_live_operation(operation);
    assert!(
        prepared,
        "test Live requests require one matched route pattern and one operation"
    );
    request
}

/// Explicit route policy used only to exercise production Live admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveTestRoutePolicy {
    /// The route is admitted by a trusted internal-origin boundary.
    pub trusted_internal_origin: bool,
    /// CSRF is explicitly inapplicable for this route class.
    pub stateless_csrf: bool,
    /// The route deliberately has no session scope.
    pub stateless_session: bool,
    /// The route deliberately permits an anonymous principal.
    pub anonymous_principal: bool,
    /// The route deliberately has no tenant scope.
    pub tenantless: bool,
    /// The request uses a direct peer without proxy interpretation.
    pub direct_peer: bool,
    /// A declared upstream boundary owns rate admission.
    pub upstream_rate_limit: bool,
    /// No application-specific middleware is required after core checks.
    pub no_additional_middleware: bool,
}

fn route_policy(policy: LiveTestRoutePolicy) -> super::context::LiveRouteSecurityPolicy {
    super::context::LiveRouteSecurityPolicy {
        trusted_internal_origin: policy.trusted_internal_origin,
        stateless_csrf: policy.stateless_csrf,
        stateless_session: policy.stateless_session,
        anonymous_principal: policy.anonymous_principal,
        tenantless: policy.tenantless,
        direct_peer: policy.direct_peer,
        upstream_rate_limit: policy.upstream_rate_limit,
        no_additional_middleware: policy.no_additional_middleware,
    }
}

impl LiveTestRoutePolicy {
    /// Requires every ordinary host-owned check.
    #[must_use]
    pub const fn strict() -> Self {
        Self {
            trusted_internal_origin: false,
            stateless_csrf: false,
            stateless_session: false,
            anonymous_principal: false,
            tenantless: false,
            direct_peer: false,
            upstream_rate_limit: false,
            no_additional_middleware: false,
        }
    }
}

/// Attaches production Live admission metadata to an already registered test route.
pub fn register_live_route_for_test(
    router: &mut crate::Router,
    method: hyper::Method,
    pattern: &str,
    operation: LiveTestOperation,
    policy: LiveTestRoutePolicy,
) -> Result<(), crate::FrameworkError> {
    let operation = match operation {
        LiveTestOperation::Action => LiveOperation::Action,
        LiveTestOperation::Upload => LiveOperation::Upload,
        LiveTestOperation::SseControl => LiveOperation::SseControl,
        LiveTestOperation::WebSocketHandshake => LiveOperation::WebSocketHandshake,
    };
    let policy = route_policy(policy);
    router.register_live_route_metadata(
        method,
        pattern,
        super::context::LiveRouteMetadata::new(operation, policy),
    )
}

/// Declares one component mount on a prebuilt router using production catalog types.
pub fn register_live_mount_for_test<C: super::ComponentContract>(
    router: &mut crate::Router,
    route: &str,
    slot: &str,
) -> Result<(), crate::FrameworkError> {
    use sha2::{Digest, Sha256};
    use suprnova_live::host::{MountCatalogEntry, MountScopeRequirements, ScopeRequirement};
    use suprnova_live::identity::{BuildId, IslandSlot, RouteIdentity};
    use suprnova_live::snapshot::state::{SnapshotSchemaSet, StateSchema};
    use suprnova_live::snapshot::{ComponentContract as SnapshotContract, ExpectedSeedV1};

    fn fixture_error() -> crate::FrameworkError {
        crate::FrameworkError::internal("Live mount test fixture was rejected")
    }

    let descriptor = C::__live_registration()
        .map_err(|_| fixture_error())?
        .into_engine();
    let mut route_digest = Sha256::new();
    route_digest.update(b"suprnova-live/route-identity/v1\0");
    route_digest.update(route.as_bytes());
    let route_bytes: [u8; 32] = route_digest.finalize().into();
    let route = RouteIdentity::from_bytes(&route_bytes).map_err(|_| fixture_error())?;
    let slot = IslandSlot::parse(slot).map_err(|_| fixture_error())?;
    let empty_schema = || StateSchema::new(1, vec![]).map_err(|_| fixture_error());
    let schemas = SnapshotSchemaSet::new(empty_schema()?, empty_schema()?, empty_schema()?)
        .map_err(|_| fixture_error())?;
    let component = descriptor.metadata().identity().clone();
    let contract = descriptor.contract_digest().clone();
    let selection = suprnova_live::host::MountSelection::new(
        route.clone(),
        slot.clone(),
        component.clone(),
        contract.clone(),
        descriptor.metadata().versions().minimum_protocol(),
    );
    let document_key = suprnova_live::mount::DocumentMountKey::parse(slot.as_str())
        .map_err(|_| fixture_error())?;
    let expected = ExpectedSeedV1::new(
        SnapshotContract::new(component, contract, 1, 1, 1).map_err(|_| fixture_error())?,
        BuildId::parse("test-build").map_err(|_| fixture_error())?,
        route,
        slot,
        schemas,
    );
    router.register_live_mount_entry(super::runtime::LiveMountRegistration::new(
        MountCatalogEntry::new(
            expected,
            MountScopeRequirements::new(
                ScopeRequirement::Absent,
                ScopeRequirement::Absent,
                ScopeRequirement::Absent,
            ),
        ),
        selection,
        document_key,
        BuildId::parse("test-build").map_err(|_| fixture_error())?,
    ))
}

/// Consumes test router mount declarations into the same immutable runtime path
/// used by server preparation, without binding a listener.
pub fn prepare_live_router_for_test(
    router: &crate::Router,
) -> Result<super::LiveRuntime, crate::FrameworkError> {
    let config = crate::App::resolve::<super::LiveConfig>().unwrap_or_default();
    let registry = crate::App::resolve::<super::LiveRegistry>()
        .unwrap_or_else(|_| super::LiveRegistry::builder().build());
    let runtime = super::runtime::assemble_for_harness(config, registry)?;
    crate::container::testing::TestContainer::singleton(runtime.clone());
    for entry in router.take_live_mount_entries()? {
        runtime.register_mount(entry)?;
    }
    runtime.finalize_mount_catalog()?;
    Ok(runtime)
}

/// Test clock whose reading only moves when a test advances it.
pub struct AdjustableTestClock {
    now_ms: std::sync::atomic::AtomicU64,
}

impl AdjustableTestClock {
    /// Starts the clock at `now_ms` milliseconds since the Unix epoch.
    #[must_use]
    pub const fn new(now_ms: u64) -> Self {
        Self {
            now_ms: std::sync::atomic::AtomicU64::new(now_ms),
        }
    }

    /// Returns the current reading.
    #[must_use]
    pub fn now_ms(&self) -> u64 {
        self.now_ms.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Moves the clock forward by `delta_ms`.
    pub fn advance_ms(&self, delta_ms: u64) {
        self.now_ms
            .fetch_add(delta_ms, std::sync::atomic::Ordering::SeqCst);
    }
}

impl suprnova_live::clock::Clock for AdjustableTestClock {
    fn now(&self) -> Result<suprnova_live::identity::UnixMillis, suprnova_live::clock::ClockError> {
        Ok(suprnova_live::identity::UnixMillis::new(self.now_ms()))
    }
}

/// Prepares the runtime like [`prepare_live_router_for_test`] with an adjustable clock.
pub fn prepare_live_router_with_clock_for_test(
    router: &crate::Router,
    clock: Arc<AdjustableTestClock>,
) -> Result<super::LiveRuntime, crate::FrameworkError> {
    let config = crate::App::resolve::<super::LiveConfig>().unwrap_or_default();
    let registry = crate::App::resolve::<super::LiveRegistry>()
        .unwrap_or_else(|_| super::LiveRegistry::builder().build());
    let clock: Arc<dyn suprnova_live::clock::Clock> = clock;
    let runtime = super::runtime::assemble_for_harness_with_clock(config, registry, clock)?;
    crate::container::testing::TestContainer::singleton(runtime.clone());
    for entry in router.take_live_mount_entries()? {
        runtime.register_mount(entry)?;
    }
    runtime.finalize_mount_catalog()?;
    Ok(runtime)
}

/// Bounded observation of one asynchronous document transport.
pub struct AsyncTransportReport {
    kind: &'static str,
    credential: Option<String>,
    /// Number of committed logical memberships.
    pub memberships: usize,
    /// Envelopes currently retained in the bounded delivery buffer.
    pub retained_events: usize,
    /// Bytes currently retained in the bounded delivery buffer.
    pub retained_bytes: usize,
    /// Whether the document degraded under backpressure.
    pub degraded: bool,
    /// Whether a physical reader is attached.
    pub reader_active: bool,
    /// Coalesced refreshes since the reader attached.
    pub coalesced: u64,
    /// Lanes that had to re-baseline after a delivery gap.
    pub degraded_lanes: u64,
}

impl AsyncTransportReport {
    /// Returns the transport kind (`sse` or `websocket`).
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        self.kind
    }

    /// Returns whether this transport is bound to `credential`.
    #[must_use]
    pub fn credential_matches(&self, credential: &str) -> bool {
        self.credential.as_deref() == Some(credential)
    }
}

impl fmt::Debug for AsyncTransportReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<AsyncTransportReport:redacted>")
    }
}

/// Returns bounded reports for every asynchronous transport of the runtime.
#[must_use]
pub fn inspect_async_transports_for_test(runtime: &LiveRuntime) -> Vec<AsyncTransportReport> {
    runtime
        .async_state()
        .reports()
        .into_iter()
        .map(|report| AsyncTransportReport {
            kind: report.kind,
            credential: report.credential,
            memberships: report.memberships,
            retained_events: report.retained_events,
            retained_bytes: report.retained_bytes,
            degraded: report.degraded,
            reader_active: report.reader_active,
            coalesced: report.coalesced,
            degraded_lanes: report.degraded_lanes,
        })
        .collect()
}

/// Waits until the SSE transport bound to `credential` has no attached reader.
pub async fn await_async_transport_retirement_for_test(runtime: &LiveRuntime, credential: &str) {
    runtime.async_state().await_retirement(credential).await;
}

/// Returns whether macro registration attached executable request-owned hooks.
pub fn component_registration_has_runtime_hooks<C: super::ComponentContract>() -> bool {
    C::__live_registration()
        .map(|registration| registration.into_engine().hooks().is_some())
        .unwrap_or(false)
}

/// Records one positive owner-middleware fact without exposing production authority.
pub fn record_live_security_pass_for_test(
    request: &mut Request,
    check: LiveSecurityCheck,
    fact: Option<&[u8]>,
) -> bool {
    let check = SecurityCheck::ALL[check.index()];
    request.record_live_security_check(check, fact)
}

/// Records the policy-coherent `not_required` reason for one test check.
pub fn record_live_security_not_required_for_test(
    request: &mut Request,
    check: LiveSecurityCheck,
) -> bool {
    let (check, reason) = match check {
        LiveSecurityCheck::Origin => (
            SecurityCheck::Origin,
            suprnova_live::host::PolicyReason::TrustedInternalOrigin,
        ),
        LiveSecurityCheck::Csrf => (
            SecurityCheck::Csrf,
            suprnova_live::host::PolicyReason::StatelessCsrfPolicy,
        ),
        LiveSecurityCheck::Session => (
            SecurityCheck::Session,
            suprnova_live::host::PolicyReason::StatelessRequest,
        ),
        LiveSecurityCheck::Principal => (
            SecurityCheck::Principal,
            suprnova_live::host::PolicyReason::AnonymousPrincipal,
        ),
        LiveSecurityCheck::Tenant => (
            SecurityCheck::Tenant,
            suprnova_live::host::PolicyReason::TenantlessRoute,
        ),
        LiveSecurityCheck::Proxy => (
            SecurityCheck::Proxy,
            suprnova_live::host::PolicyReason::DirectPeer,
        ),
        LiveSecurityCheck::RateLimit => (
            SecurityCheck::RateLimit,
            suprnova_live::host::PolicyReason::UpstreamRateLimited,
        ),
        LiveSecurityCheck::Middleware => (
            SecurityCheck::Middleware,
            suprnova_live::host::PolicyReason::NoAdditionalMiddleware,
        ),
    };
    request.record_live_security_not_required(check, reason)
}

/// Removes one fact so hostile-adapter tests can prove omission rejection.
#[cfg(feature = "testing")]
pub fn remove_live_security_check_for_test(request: &mut Request, check: LiveSecurityCheck) {
    request.remove_live_security_check_for_test(SecurityCheck::ALL[check.index()]);
}

/// Prepares an already matched request with an explicit test expiry.
#[must_use]
pub fn prepare_live_request_until_for_test(
    mut request: Request,
    operation: LiveTestOperation,
    expires_at_ms: u64,
) -> Request {
    let operation = match operation {
        LiveTestOperation::Action => LiveOperation::Action,
        LiveTestOperation::Upload => LiveOperation::Upload,
        LiveTestOperation::SseControl => LiveOperation::SseControl,
        LiveTestOperation::WebSocketHandshake => LiveOperation::WebSocketHandshake,
    };
    assert!(request.prepare_live_operation_until(
        operation,
        suprnova_live::identity::UnixMillis::new(expires_at_ms),
    ));
    request
}

/// Runs the same final policy closure installed on a production Live route.
pub fn complete_live_route_policy_for_test(request: &mut Request, policy: LiveTestRoutePolicy) {
    super::context::LiveMiddlewareCompletion::new(route_policy(policy))
        .close_policy_absences(request);
}

/// Opaque registered mount used to exercise the production context validator.
pub struct LiveContextHarness {
    runtime: LiveRuntime,
    current_route: suprnova_live::identity::RouteIdentity,
    current_slot: suprnova_live::identity::IslandSlot,
    selection: suprnova_live::host::MountSelection,
}

impl LiveContextHarness {
    /// Builds a tenantless, stateless, anonymous mount with a sealed catalog.
    pub fn anonymous() -> Result<Self, crate::FrameworkError> {
        use sha2::{Digest, Sha256};
        use suprnova_live::host::{MountCatalogEntry, MountScopeRequirements, ScopeRequirement};
        use suprnova_live::identity::{
            BuildId, ComponentName, IslandSlot, RouteIdentity, ViewName,
        };
        use suprnova_live::metadata::{ComponentMetadata, ContractVersions};
        use suprnova_live::registry::{ComponentDescriptor, ComponentRegistryBuilder};
        use suprnova_live::snapshot::state::{SnapshotSchemaSet, StateSchema};
        use suprnova_live::snapshot::{ComponentContract, ExpectedSeedV1};

        fn test_error() -> crate::FrameworkError {
            crate::FrameworkError::internal("Live context test fixture was rejected")
        }

        let component = ComponentName::parse("tests.context").map_err(|_| test_error())?;
        let metadata = ComponentMetadata::new(
            component.clone(),
            ViewName::parse("live/tests/context.html").map_err(|_| test_error())?,
            ContractVersions::new(1, 1, 1, 1, 1).map_err(|_| test_error())?,
            vec![],
            vec![],
        )
        .map_err(|_| test_error())?;
        let engine_registry = ComponentRegistryBuilder::new()
            .register(ComponentDescriptor::new(metadata))
            .map_err(|_| test_error())?
            .build();
        let descriptor = engine_registry
            .resolve(&component)
            .map_err(|_| test_error())?;

        let mut route_digest = Sha256::new();
        route_digest.update(b"suprnova-live/test-route/v1\0");
        route_digest.update(b"/catalog");
        let route_bytes: [u8; 32] = route_digest.finalize().into();
        let current_route = RouteIdentity::from_bytes(&route_bytes).map_err(|_| test_error())?;
        let current_slot = IslandSlot::parse("root").map_err(|_| test_error())?;
        let schemas = SnapshotSchemaSet::new(
            StateSchema::new(1, vec![]).map_err(|_| test_error())?,
            StateSchema::new(1, vec![]).map_err(|_| test_error())?,
            StateSchema::new(1, vec![]).map_err(|_| test_error())?,
        )
        .map_err(|_| test_error())?;
        let expected_seed = ExpectedSeedV1::new(
            ComponentContract::new(
                component.clone(),
                descriptor.contract_digest().clone(),
                1,
                1,
                1,
            )
            .map_err(|_| test_error())?,
            BuildId::parse("test-build").map_err(|_| test_error())?,
            current_route.clone(),
            current_slot.clone(),
            schemas,
        );
        let selection = suprnova_live::host::MountSelection::new(
            current_route.clone(),
            current_slot.clone(),
            component,
            descriptor.contract_digest().clone(),
            1,
        );
        let registry = super::LiveRegistry::from_engine(engine_registry);
        let runtime = super::runtime::assemble_for_harness(super::LiveConfig::default(), registry)?;
        runtime.register_mount(super::runtime::LiveMountRegistration::new(
            MountCatalogEntry::new(
                expected_seed,
                MountScopeRequirements::new(
                    ScopeRequirement::Absent,
                    ScopeRequirement::Absent,
                    ScopeRequirement::Absent,
                ),
            ),
            selection.clone(),
            suprnova_live::mount::DocumentMountKey::parse(current_slot.as_str())
                .map_err(|_| test_error())?,
            BuildId::parse("test-build").map_err(|_| test_error())?,
        ))?;
        runtime.finalize_mount_catalog()?;

        Ok(Self {
            runtime,
            current_route,
            current_slot,
            selection,
        })
    }

    /// Returns success only when the production validator accepts every fact.
    pub fn validate(&self, request: &Request) -> Result<(), crate::FrameworkError> {
        self.runtime
            .validate_request_context(
                request,
                self.current_route.clone(),
                self.current_slot.clone(),
                self.selection.clone(),
            )
            .map(|_| ())
    }
}

/// Returns whether two request values carry the same server-minted identity.
#[must_use]
pub fn same_request_identity(first: &Request, second: &Request) -> bool {
    first.live_request_identity() == second.live_request_identity()
}

/// Returns the host-owned cancellation flag attached to a prepared request.
#[must_use]
pub fn request_cancellation_for_test(
    request: &Request,
) -> Option<suprnova_live::resource::CancellationFlag> {
    request.live_cancellation()
}

/// Returns whether two handles refer to the same immutable runtime graph.
#[must_use]
pub fn same_runtime_instance(first: &LiveRuntime, second: &LiveRuntime) -> bool {
    first.same_instance(second)
}

/// Proves that the same provider-candidate validation used by boot fails closed.
pub fn validate_runtime_provider_omission_for_test(
    runtime: &LiveRuntime,
    provider: LiveTestRuntimeProvider,
) -> Result<(), crate::FrameworkError> {
    super::runtime::validate_provider_omission(runtime, runtime_provider(provider))
}

struct FixedTestClock(suprnova_live::identity::UnixMillis);

impl suprnova_live::clock::Clock for FixedTestClock {
    fn now(&self) -> Result<suprnova_live::identity::UnixMillis, suprnova_live::clock::ClockError> {
        Ok(self.0)
    }
}

/// Injects a fixed clock through the runtime provider seam and observes expiry.
pub fn prepare_live_request_with_fixed_clock_for_test(
    runtime: &LiveRuntime,
    mut request: Request,
    operation: LiveTestOperation,
    now_ms: u64,
) -> Result<u64, crate::FrameworkError> {
    let clock: std::sync::Arc<dyn suprnova_live::clock::Clock> = std::sync::Arc::new(
        FixedTestClock(suprnova_live::identity::UnixMillis::new(now_ms)),
    );
    let runtime = super::runtime::assemble_with_clock_override(runtime, clock)?;
    let operation = match operation {
        LiveTestOperation::Action => LiveOperation::Action,
        LiveTestOperation::Upload => LiveOperation::Upload,
        LiveTestOperation::SseControl => LiveOperation::SseControl,
        LiveTestOperation::WebSocketHandshake => LiveOperation::WebSocketHandshake,
    };
    runtime.prepare_request(&mut request, operation)?;
    request
        .live_security_attestation()
        .expires_at(request.live_request_identity())
        .map(suprnova_live::identity::UnixMillis::get)
        .ok_or_else(|| crate::FrameworkError::internal("fixed-clock Live request was not prepared"))
}

#[derive(serde::Deserialize)]
struct FixtureValidationState {
    profile: FixtureProfile,
}

#[derive(serde::Deserialize, validator::Validate)]
struct FixtureProfile {
    #[validate(email)]
    email: String,
}

struct FixtureValidation;

impl suprnova_live::validation::ValidationPort for FixtureValidation {
    fn validate<'a>(
        &'a self,
        request: suprnova_live::validation::ValidationRequest<'a>,
    ) -> suprnova_live::validation::ValidationFuture<
        'a,
        Result<
            Vec<suprnova_live::validation::ValidationIssue>,
            suprnova_live::validation::ValidationPortError,
        >,
    > {
        Box::pin(async move {
            use validator::Validate;

            if request.component().as_str() != "tests.validation" {
                return Err(suprnova_live::validation::ValidationPortError::unavailable());
            }
            let serialized = serde_json::to_value(request.state())
                .map_err(|_| suprnova_live::validation::ValidationPortError::unavailable())?;
            let state: FixtureValidationState = serde_json::from_value(serialized)
                .map_err(|_| suprnova_live::validation::ValidationPortError::unavailable())?;
            let errors = match state.profile.validate() {
                Ok(()) => return Ok(Vec::new()),
                Err(errors) => errors,
            };
            if !errors.field_errors().contains_key("email") {
                return Err(suprnova_live::validation::ValidationPortError::unavailable());
            }

            Ok(vec![suprnova_live::validation::ValidationIssue::new(
                suprnova_live::state::ModelPath::parse("profile.email")
                    .map_err(|_| suprnova_live::validation::ValidationPortError::unavailable())?,
                suprnova_live::validation::ValidationMessageId::parse("validation.email_invalid")
                    .map_err(|_| suprnova_live::validation::ValidationPortError::unavailable())?,
            )])
        })
    }
}

/// Sealed component-validation fixture that exercises the production adapter.
pub struct LiveValidationHarness {
    registry: super::LiveRegistry,
    port: super::ports::validation::SuprnovaValidationPort,
}

impl LiveValidationHarness {
    /// Builds one immutable descriptor-and-validation registration.
    pub fn new() -> Result<Self, crate::FrameworkError> {
        use suprnova_live::identity::{ComponentName, ViewName};
        use suprnova_live::metadata::{ComponentMetadata, ContractVersions};
        use suprnova_live::registry::ComponentDescriptor;

        fn fixture_error() -> crate::FrameworkError {
            crate::FrameworkError::internal("Live validation test fixture was rejected")
        }

        let metadata = ComponentMetadata::new(
            ComponentName::parse("tests.validation").map_err(|_| fixture_error())?,
            ViewName::parse("live/tests/validation.html").map_err(|_| fixture_error())?,
            ContractVersions::new(1, 1, 1, 1, 1).map_err(|_| fixture_error())?,
            vec![],
            vec![],
        )
        .map_err(|_| fixture_error())?;
        let registration =
            super::__private::ComponentRegistration::new(ComponentDescriptor::new(metadata))
                .with_validation(std::sync::Arc::new(FixtureValidation));
        let registry = super::LiveRegistry::builder()
            .register_registration(registration)
            .map_err(|_| fixture_error())?
            .build();
        let port = super::ports::validation::SuprnovaValidationPort::new(registry.clone());
        Ok(Self { registry, port })
    }

    /// Uses an application-built immutable registry with generated callbacks.
    #[must_use]
    pub fn from_registry(registry: super::LiveRegistry) -> Self {
        Self {
            port: super::ports::validation::SuprnovaValidationPort::new(registry.clone()),
            registry,
        }
    }

    /// Runs selected validation for an exact component name through the adapter.
    pub async fn validate(
        &self,
        component: &str,
    ) -> Result<Vec<(String, String)>, crate::FrameworkError> {
        use suprnova_live::validation::{ValidationPort, ValidationRequest, ValidationSelection};

        let component = suprnova_live::identity::ComponentName::parse(component).map_err(|_| {
            crate::FrameworkError::internal("Live validation component identity was rejected")
        })?;
        let state = suprnova_live::canonical::CanonicalValue::Object(
            [(
                "profile".to_owned(),
                suprnova_live::canonical::CanonicalValue::Object(
                    [(
                        "email".to_owned(),
                        suprnova_live::canonical::CanonicalValue::String("not-an-email".to_owned()),
                    )]
                    .into_iter()
                    .collect(),
                ),
            )]
            .into_iter()
            .collect(),
        );
        let arguments =
            suprnova_live::canonical::CanonicalValue::Object(std::collections::BTreeMap::new());
        let request = ValidationRequest::new(
            &component,
            ValidationSelection::WholeComponent,
            &state,
            &arguments,
        );
        self.port
            .validate(request)
            .await
            .map_err(|_| crate::FrameworkError::internal("Live validation provider rejected"))
            .map(|issues| {
                issues
                    .into_iter()
                    .map(|issue| {
                        (
                            issue.path().as_str().to_owned(),
                            issue.message().as_str().to_owned(),
                        )
                    })
                    .collect()
            })
    }

    /// Runs a generated callback against the exact request-owned typed target.
    pub async fn validate_target<T: Send + 'static>(
        &self,
        component: &str,
        action: &str,
        target: &mut T,
    ) -> Result<Vec<(String, String)>, crate::FrameworkError> {
        use suprnova_live::validation::{ValidationPort, ValidationRequest, ValidationSelection};

        let component = suprnova_live::identity::ComponentName::parse(component).map_err(|_| {
            crate::FrameworkError::internal("Live validation component identity was rejected")
        })?;
        let action = suprnova_live::identity::ActionName::parse(action).map_err(|_| {
            crate::FrameworkError::internal("Live validation action identity was rejected")
        })?;
        let state =
            suprnova_live::canonical::CanonicalValue::Object(std::collections::BTreeMap::new());
        let arguments =
            suprnova_live::canonical::CanonicalValue::Object(std::collections::BTreeMap::new());
        let target: &mut dyn suprnova_live::action::ActionTarget = target;
        let request = ValidationRequest::new(
            &component,
            ValidationSelection::WholeComponent,
            &state,
            &arguments,
        )
        .with_action(&action)
        .with_target(target);
        self.port
            .validate(request)
            .await
            .map_err(|_| crate::FrameworkError::internal("Live validation provider rejected"))
            .map(|issues| {
                issues
                    .into_iter()
                    .map(|issue| {
                        (
                            issue.path().as_str().to_owned(),
                            issue.message().as_str().to_owned(),
                        )
                    })
                    .collect()
            })
    }

    /// Runs generated component validation through the bounded engine.
    pub async fn validate_target_with_issue_limit<T: Send + 'static>(
        &self,
        component: &str,
        action: &str,
        max_issues: usize,
        target: &mut T,
    ) -> Result<Vec<(String, String)>, crate::FrameworkError> {
        use suprnova_live::validation::{
            BagPolicy, ErrorBag, ValidationEngine, ValidationRequest, ValidationSelection,
        };

        let component = suprnova_live::identity::ComponentName::parse(component).map_err(|_| {
            crate::FrameworkError::internal("Live validation component identity was rejected")
        })?;
        let action = suprnova_live::identity::ActionName::parse(action).map_err(|_| {
            crate::FrameworkError::internal("Live validation action identity was rejected")
        })?;
        let state =
            suprnova_live::canonical::CanonicalValue::Object(std::collections::BTreeMap::new());
        let arguments =
            suprnova_live::canonical::CanonicalValue::Object(std::collections::BTreeMap::new());
        let target: &mut dyn suprnova_live::action::ActionTarget = target;
        let request = ValidationRequest::new(
            &component,
            ValidationSelection::WholeComponent,
            &state,
            &arguments,
        )
        .with_action(&action)
        .with_target(target);
        let engine = ValidationEngine::new(max_issues)
            .map_err(|_| crate::FrameworkError::internal("invalid validation issue limit"))?;
        let mut bag = ErrorBag::default();
        engine
            .validate(&self.port, request, &mut bag, BagPolicy::Replace)
            .await
            .map_err(|_| crate::FrameworkError::internal("Live validation engine rejected"))?;
        Ok(bag
            .issues()
            .iter()
            .map(|issue| {
                (
                    issue.path().as_str().to_owned(),
                    issue.message().as_str().to_owned(),
                )
            })
            .collect())
    }

    /// Runs only the declared stable model paths through a generated hook.
    pub async fn validate_selected_target<'a, T, I>(
        &self,
        component: &str,
        action: &str,
        selected: I,
        target: &mut T,
    ) -> Result<Vec<(String, String)>, crate::FrameworkError>
    where
        T: Send + 'static,
        I: IntoIterator<Item = &'a str>,
    {
        use suprnova_live::validation::{ValidationPort, ValidationRequest, ValidationSelection};

        let component = suprnova_live::identity::ComponentName::parse(component).map_err(|_| {
            crate::FrameworkError::internal("Live validation component identity was rejected")
        })?;
        let action = suprnova_live::identity::ActionName::parse(action).map_err(|_| {
            crate::FrameworkError::internal("Live validation action identity was rejected")
        })?;
        let selected = selected
            .into_iter()
            .map(|path| {
                suprnova_live::state::ModelPath::parse(path).map_err(|_| {
                    crate::FrameworkError::internal("Live validation model path was rejected")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let state =
            suprnova_live::canonical::CanonicalValue::Object(std::collections::BTreeMap::new());
        let arguments =
            suprnova_live::canonical::CanonicalValue::Object(std::collections::BTreeMap::new());
        let target: &mut dyn suprnova_live::action::ActionTarget = target;
        let request = ValidationRequest::new(
            &component,
            ValidationSelection::Selected(selected),
            &state,
            &arguments,
        )
        .with_action(&action)
        .with_target(target);
        self.port
            .validate(request)
            .await
            .map_err(|_| crate::FrameworkError::internal("Live validation provider rejected"))
            .map(|issues| {
                issues
                    .into_iter()
                    .map(|issue| {
                        (
                            issue.path().as_str().to_owned(),
                            issue.message().as_str().to_owned(),
                        )
                    })
                    .collect()
            })
    }

    /// Runs the registered selection with schema-checked string arguments.
    pub async fn validate_string_action_target<'a, T, I>(
        &self,
        component: &str,
        action: &str,
        arguments: I,
        target: &mut T,
    ) -> Result<Vec<(String, String)>, crate::FrameworkError>
    where
        T: Send + 'static,
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        use suprnova_live::action::RawActionArguments;
        use suprnova_live::validation::{ValidationPort, ValidationRequest};

        let component = suprnova_live::identity::ComponentName::parse(component).map_err(|_| {
            crate::FrameworkError::internal("Live validation component identity was rejected")
        })?;
        let action = suprnova_live::identity::ActionName::parse(action).map_err(|_| {
            crate::FrameworkError::internal("Live validation action identity was rejected")
        })?;
        let descriptor =
            self.registry.engine().resolve(&component).map_err(|_| {
                crate::FrameworkError::internal("Live validation component missing")
            })?;
        let metadata = descriptor
            .metadata()
            .actions()
            .iter()
            .find(|metadata| metadata.name() == &action)
            .ok_or_else(|| crate::FrameworkError::internal("Live validation action missing"))?;
        let canonical = suprnova_live::canonical::CanonicalValue::Object(
            arguments
                .into_iter()
                .map(|(name, value)| {
                    (
                        name.to_owned(),
                        suprnova_live::canonical::CanonicalValue::String(value.to_owned()),
                    )
                })
                .collect(),
        );
        let prepared = descriptor
            .actions()
            .prepare(
                &action,
                RawActionArguments::new(canonical),
                &suprnova_live::limits::InputLimits::default(),
            )
            .map_err(|_| crate::FrameworkError::internal("Live validation arguments rejected"))?;
        let state =
            suprnova_live::canonical::CanonicalValue::Object(std::collections::BTreeMap::new());
        let target: &mut dyn suprnova_live::action::ActionTarget = target;
        let request = ValidationRequest::new(
            &component,
            metadata.validation().clone(),
            &state,
            prepared.canonical(),
        )
        .with_action(&action)
        .with_prepared_arguments(&prepared)
        .with_target(target);
        self.port
            .validate(request)
            .await
            .map_err(|_| crate::FrameworkError::internal("Live validation provider rejected"))
            .map(|issues| {
                issues
                    .into_iter()
                    .map(|issue| {
                        (
                            issue.path().as_str().to_owned(),
                            issue.message().as_str().to_owned(),
                        )
                    })
                    .collect()
            })
    }
}

/// Redacted readiness observation for the immutable runtime graph.
pub struct LiveRuntimeReport {
    clock: bool,
    random: bool,
    key_ring: bool,
    ledger: bool,
    promotion: bool,
    execution: bool,
    context_validator: bool,
    host_ports: bool,
    upload_ports: bool,
    upload_services: bool,
    mount_catalog: bool,
    response_and_cancellation: bool,
    subscription_ports: bool,
    async_state: bool,
}

impl LiveRuntimeReport {
    /// Returns whether every required runtime service was assembled and sealed.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.clock
            && self.random
            && self.key_ring
            && self.ledger
            && self.promotion
            && self.execution
            && self.context_validator
            && self.host_ports
            && self.upload_ports
            && self.upload_services
            && self.mount_catalog
            && self.response_and_cancellation
            && self.subscription_ports
            && self.async_state
    }

    /// Returns whether the subscription authorization and credential ports are installed.
    #[must_use]
    pub const fn has_subscription_ports(&self) -> bool {
        self.subscription_ports
    }

    /// Returns whether the asynchronous-update runtime state was assembled.
    #[must_use]
    pub const fn has_async_state(&self) -> bool {
        self.async_state
    }

    /// Returns whether the runtime owns a wall-clock provider.
    #[must_use]
    pub const fn has_clock(&self) -> bool {
        self.clock
    }

    /// Returns whether the runtime owns a server-side random source.
    #[must_use]
    pub const fn has_random_source(&self) -> bool {
        self.random
    }

    /// Returns whether the runtime owns a purpose-derived snapshot key ring.
    #[must_use]
    pub const fn has_key_ring(&self) -> bool {
        self.key_ring
    }

    /// Returns whether the runtime owns an instance revision ledger.
    #[must_use]
    pub const fn has_instance_ledger(&self) -> bool {
        self.ledger
    }

    /// Returns whether the runtime owns bounded public-seed promotion.
    #[must_use]
    pub const fn has_seed_promotion_service(&self) -> bool {
        self.promotion
    }

    /// Returns whether the runtime owns the action execution coordinator.
    #[must_use]
    pub const fn has_execution_kernel(&self) -> bool {
        self.execution
    }

    /// Returns whether the runtime owns the trusted-context validator.
    #[must_use]
    pub const fn has_context_validator(&self) -> bool {
        self.context_validator
    }

    /// Returns whether framework-owned engine port adapters are installed.
    #[must_use]
    pub const fn has_host_ports(&self) -> bool {
        self.host_ports
    }

    /// Returns whether every distinct framework-owned upload port was assembled.
    ///
    /// This reports immutable assembly only; it does not probe provider health or
    /// turn an unavailable direct-provider default into a usable capability.
    #[must_use]
    pub const fn has_upload_ports(&self) -> bool {
        self.upload_ports
    }

    /// Returns whether all engine upload coordinators were assembled from host ports.
    #[must_use]
    pub const fn has_upload_services(&self) -> bool {
        self.upload_services
    }

    /// Returns whether route construction sealed the mount catalog.
    #[must_use]
    pub const fn has_mount_catalog(&self) -> bool {
        self.mount_catalog
    }
}

impl fmt::Debug for LiveRuntimeReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<LiveRuntimeReport:redacted>")
    }
}

/// Returns a redacted readiness observation for one runtime graph.
#[must_use]
pub fn inspect_runtime(runtime: &LiveRuntime) -> LiveRuntimeReport {
    let readiness = runtime.readiness();
    LiveRuntimeReport {
        clock: readiness.clock,
        random: readiness.random,
        key_ring: readiness.key_ring,
        ledger: readiness.ledger,
        promotion: readiness.promotion,
        execution: readiness.execution,
        context_validator: readiness.context_validator,
        host_ports: readiness.host_ports,
        upload_ports: readiness.upload_ports,
        upload_services: readiness.upload_services,
        mount_catalog: readiness.mount_catalog,
        response_and_cancellation: readiness.response_and_cancellation,
        subscription_ports: readiness.subscription_ports,
        async_state: readiness.async_state,
    }
}

/// Resolves one browser-selectable upload mount only through the finalized catalog facts.
pub fn select_upload_mount_for_test(
    runtime: &LiveRuntime,
    component: &str,
    slot: &str,
    document_key: &str,
) -> Result<(), crate::FrameworkError> {
    runtime.select_upload_mount_for_test(component, slot, document_key)
}

/// Redacted proof that one upload authority scope was derived from trusted mount facts.
pub struct UploadMountAuthorityReport {
    fixture: super::runtime::UploadMountAuthorityTestFixture,
}

/// Redacted deterministic upload-handle derivation and accepted key-rotation candidates.
pub struct UploadHandleDerivationReport {
    current: suprnova_live::upload::UploadHandle,
    accepted: Vec<suprnova_live::upload::UploadHandle>,
}

/// Redacted read-only observation of one deterministic upload identity's host residue.
pub struct UploadResidueReport {
    ledger: bool,
    provider: bool,
    metadata: bool,
}

impl UploadResidueReport {
    /// Returns whether no authoritative record, provider checkpoint, or create memo exists.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        !self.ledger && !self.provider && !self.metadata
    }
}

impl fmt::Debug for UploadResidueReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadResidueReport:redacted>")
    }
}

impl UploadHandleDerivationReport {
    /// Returns whether both derivations mint the same current handle.
    #[must_use]
    pub fn same_current(&self, other: &Self) -> bool {
        self.current == other.current
    }

    /// Returns whether this key ring accepts a handle issued by the other derivation.
    #[must_use]
    pub fn accepts(&self, other: &Self) -> bool {
        self.accepted.contains(&other.current)
    }

    /// Returns whether the opaque wire identity retains the required UUIDv4 grammar.
    #[must_use]
    pub fn is_uuid_v4(&self) -> bool {
        uuid::Uuid::parse_str(&self.current.to_string())
            .is_ok_and(|uuid| uuid.get_version_num() == 4)
    }
}

impl fmt::Debug for UploadHandleDerivationReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadHandleDerivationReport:redacted>")
    }
}

impl UploadMountAuthorityReport {
    /// Returns whether two authorities are bound to the same exact mount and host scope.
    #[must_use]
    pub fn same_scope(&self, other: &Self) -> bool {
        self.fixture.scope == other.fixture.scope
    }

    /// Tests build-drift rejection without exposing the opaque scope value.
    #[must_use]
    pub fn matches_build(&self, build: &str) -> bool {
        let Ok(build) = suprnova_live::identity::BuildId::parse(build) else {
            return false;
        };
        let mut binding = self.fixture.binding.clone();
        binding.build = build;
        super::upload::derive_mount_scope(&binding).is_ok_and(|scope| scope == self.fixture.scope)
    }

    /// Tests generated-contract drift rejection without exposing the opaque scope value.
    #[must_use]
    pub fn matches_contract(&self, contract: &[u8]) -> bool {
        let Ok(contract) = suprnova_live::identity::ContentDigest::from_bytes(contract) else {
            return false;
        };
        let mut binding = self.fixture.binding.clone();
        binding.contract = contract;
        super::upload::derive_mount_scope(&binding).is_ok_and(|scope| scope == self.fixture.scope)
    }
}

impl fmt::Debug for UploadMountAuthorityReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadMountAuthorityReport:redacted>")
    }
}

/// Redacted unique mount resolution produced from opaque upload authority.
pub struct UploadMountResolutionReport {
    fixture: super::runtime::UploadMountResolutionTestFixture,
}

impl UploadMountResolutionReport {
    /// Returns whether resolution selected this test-declared slot and document key.
    #[must_use]
    pub fn matches(&self, slot: &str, document_key: &str) -> bool {
        self.fixture.slot.as_str() == slot && self.fixture.document_key.as_str() == document_key
    }
}

impl fmt::Debug for UploadMountResolutionReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadMountResolutionReport:redacted>")
    }
}

/// Derives one upload authority through the same finalized mount selector as production.
#[allow(
    clippy::too_many_arguments,
    reason = "the hostile test keeps every independently mutable host scope fact explicit"
)]
pub fn inspect_upload_mount_authority_for_test(
    runtime: &LiveRuntime,
    component: &str,
    slot: &str,
    document_key: &str,
    session: Option<&[u8]>,
    principal: Option<&[u8]>,
    tenant: Option<&[u8]>,
) -> Result<UploadMountAuthorityReport, crate::FrameworkError> {
    let scope = upload_test_scope(session, principal, tenant)?;
    runtime
        .inspect_upload_mount_authority_for_test(component, slot, document_key, scope)
        .map(|fixture| UploadMountAuthorityReport { fixture })
}

/// Resolves opaque upload authority by bounded enumeration of finalized eligible mounts.
pub fn resolve_upload_mount_authority_for_test(
    runtime: &LiveRuntime,
    component: &str,
    authority: &UploadMountAuthorityReport,
    session: Option<&[u8]>,
    principal: Option<&[u8]>,
    tenant: Option<&[u8]>,
) -> Result<UploadMountResolutionReport, crate::FrameworkError> {
    let scope = upload_test_scope(session, principal, tenant)?;
    runtime
        .resolve_upload_mount_authority_for_test(component, scope, &authority.fixture.scope)
        .map(|fixture| UploadMountResolutionReport { fixture })
}

/// Derives upload handles through the production purpose-separated keyed helper.
#[allow(
    clippy::too_many_arguments,
    reason = "the hostile test mutates each trusted authority and rotation input independently"
)]
pub fn inspect_deterministic_upload_handle_for_test(
    authority: &UploadMountAuthorityReport,
    field: &str,
    idempotency_key: &str,
    current_key: &[u8],
    previous_keys: &[&[u8]],
    build_override: Option<&str>,
    contract_override: Option<&[u8]>,
) -> Result<UploadHandleDerivationReport, crate::FrameworkError> {
    let mut binding = authority.fixture.binding.clone();
    if let Some(build) = build_override {
        binding.build = suprnova_live::identity::BuildId::parse(build)
            .map_err(|_| crate::FrameworkError::internal("upload handle test build rejected"))?;
    }
    if let Some(contract) = contract_override {
        binding.contract = suprnova_live::identity::ContentDigest::from_bytes(contract)
            .map_err(|_| crate::FrameworkError::internal("upload handle test contract rejected"))?;
    }
    let scope = super::upload::derive_mount_scope(&binding)?;
    let field = suprnova_live::identity::ModelField::parse(field)
        .map_err(|_| crate::FrameworkError::internal("upload handle test field rejected"))?;
    let idempotency_key = suprnova_live::upload::UploadIdempotencyKey::parse(idempotency_key)
        .map_err(|_| crate::FrameworkError::internal("upload handle test retry key rejected"))?;
    let accepted = super::upload::derive_upload_handle_candidates(
        current_key,
        previous_keys,
        &scope,
        &field,
        &idempotency_key,
    )?;
    let current = accepted.first().cloned().ok_or_else(|| {
        crate::FrameworkError::internal("upload handle test derivation was empty")
    })?;
    Ok(UploadHandleDerivationReport { current, accepted })
}

/// Proves pre-semantic rejection did not touch any upload storage boundary.
pub async fn inspect_configured_upload_residue_for_test(
    runtime: &LiveRuntime,
    authority: &UploadMountAuthorityReport,
    field: &str,
    idempotency_key: &str,
) -> Result<UploadResidueReport, crate::FrameworkError> {
    let field = suprnova_live::identity::ModelField::parse(field)
        .map_err(|_| crate::FrameworkError::internal("upload residue field rejected"))?;
    let idempotency_key = suprnova_live::upload::UploadIdempotencyKey::parse(idempotency_key)
        .map_err(|_| crate::FrameworkError::internal("upload residue retry key rejected"))?;
    let candidates = runtime.derive_upload_handle_candidates(
        &authority.fixture.scope,
        &field,
        &idempotency_key,
    )?;
    let now = runtime.upload_now()?;
    let mut report = UploadResidueReport {
        ledger: false,
        provider: false,
        metadata: false,
    };
    for handle in candidates {
        report.ledger |= runtime
            .upload_ledger()
            .load(&handle)
            .await
            .map_err(|_| crate::FrameworkError::internal("upload residue ledger read failed"))?
            .is_some();
        report.provider |= runtime
            .upload_reverse_proxy_adapter()
            .progress(&handle)
            .is_ok();
        report.metadata |= runtime
            .upload_reverse_proxy_adapter()
            .create_metadata(&handle, now)
            .is_ok();
    }
    Ok(report)
}

fn upload_test_scope(
    session: Option<&[u8]>,
    principal: Option<&[u8]>,
    tenant: Option<&[u8]>,
) -> Result<suprnova_live::identity::ScopeFingerprint, crate::FrameworkError> {
    let session =
        session.map(|value| super::attestation::purpose_fingerprint(SecurityCheck::Session, value));
    let principal = principal
        .map(|value| super::attestation::purpose_fingerprint(SecurityCheck::Principal, value));
    let tenant =
        tenant.map(|value| super::attestation::purpose_fingerprint(SecurityCheck::Tenant, value));
    super::context::aggregate_scope(
        session.as_ref().map(<[u8; 32]>::as_slice),
        principal.as_ref().map(<[u8; 32]>::as_slice),
        tenant.as_ref().map(<[u8; 32]>::as_slice),
    )
}

/// Redacted evidence from exercising Suprnova's production upload-provider adapters.
pub struct UploadProviderConformanceReport {
    received_bytes: u64,
    next_chunk_index: u32,
    cancel_removed_quarantine: bool,
    direct_provider_fails_closed: bool,
    storage_provider_fails_closed: bool,
    quarantine_permissions_are_private: bool,
    memo_exact_replay: bool,
    memo_mismatch_fails_closed: bool,
    memo_missing_fails_closed: bool,
    memo_exhaustion_fails_closed: bool,
    memo_scope_isolation: bool,
    memo_lifecycle_deletion: bool,
    memo_partial_order_recovered: bool,
    memo_redacted: bool,
}

impl UploadProviderConformanceReport {
    /// Returns the byte count accepted by the real reverse-proxy provider.
    #[must_use]
    pub const fn received_bytes(&self) -> u64 {
        self.received_bytes
    }

    /// Returns the sequential chunk index following the accepted chunk.
    #[must_use]
    pub const fn next_chunk_index(&self) -> u32 {
        self.next_chunk_index
    }

    /// Returns whether cancellation removed the opaque quarantine object.
    #[must_use]
    pub const fn cancel_removed_quarantine(&self) -> bool {
        self.cancel_removed_quarantine
    }

    /// Returns whether the default direct-provider adapter rejected preparation.
    #[must_use]
    pub const fn direct_provider_fails_closed(&self) -> bool {
        self.direct_provider_fails_closed
    }

    /// Returns whether a failed quarantine store rejected work without fallback storage.
    #[must_use]
    pub const fn storage_provider_fails_closed(&self) -> bool {
        self.storage_provider_fails_closed
    }

    /// Returns whether quarantine directories and objects deny group/other access on Unix.
    #[must_use]
    pub const fn quarantine_permissions_are_private(&self) -> bool {
        self.quarantine_permissions_are_private
    }

    /// Returns whether exact normalized create metadata replays are immutable.
    #[must_use]
    pub const fn memo_exact_replay(&self) -> bool {
        self.memo_exact_replay
    }

    /// Returns whether a conflicting create replay failed closed.
    #[must_use]
    pub const fn memo_mismatch_fails_closed(&self) -> bool {
        self.memo_mismatch_fails_closed
    }

    /// Returns whether missing create metadata failed with the closed recovery class.
    #[must_use]
    pub const fn memo_missing_fails_closed(&self) -> bool {
        self.memo_missing_fails_closed
    }

    /// Returns whether entry exhaustion was bounded and closed.
    #[must_use]
    pub const fn memo_exhaustion_fails_closed(&self) -> bool {
        self.memo_exhaustion_fails_closed
    }

    /// Returns whether one full scope leaves independently bounded capacity for another.
    #[must_use]
    pub const fn memo_scope_isolation(&self) -> bool {
        self.memo_scope_isolation
    }

    /// Returns whether terminal cleanup deleted retained create metadata.
    #[must_use]
    pub const fn memo_lifecycle_deletion(&self) -> bool {
        self.memo_lifecycle_deletion
    }

    /// Returns whether provider-before-memo partial ordering recovered only from an exact replay.
    #[must_use]
    pub const fn memo_partial_order_recovered(&self) -> bool {
        self.memo_partial_order_recovered
    }

    /// Returns whether memo diagnostics redact browser metadata.
    #[must_use]
    pub const fn memo_redacted(&self) -> bool {
        self.memo_redacted
    }
}

impl fmt::Debug for UploadProviderConformanceReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadProviderConformanceReport:redacted>")
    }
}

/// Redacted evidence from one host-owned upload cleanup pass.
pub struct UploadCleanupConformanceReport {
    claimed: usize,
    reclaimed: usize,
    residue_is_empty: bool,
}

impl UploadCleanupConformanceReport {
    /// Returns the bounded number of cleanup claims admitted by the engine.
    #[must_use]
    pub const fn claimed(&self) -> usize {
        self.claimed
    }

    /// Returns the number of claims whose host residue was reclaimed.
    #[must_use]
    pub const fn reclaimed(&self) -> usize {
        self.reclaimed
    }

    /// Returns whether ledger, provider, and metadata residue are all absent.
    #[must_use]
    pub const fn residue_is_empty(&self) -> bool {
        self.residue_is_empty
    }
}

impl fmt::Debug for UploadCleanupConformanceReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadCleanupConformanceReport:redacted>")
    }
}

/// Runs the production cleanup coordinator for one terminal upload and checks residue.
pub async fn run_upload_cleanup_for_test(
    runtime: &LiveRuntime,
    handle: &str,
) -> Result<UploadCleanupConformanceReport, crate::FrameworkError> {
    use suprnova_live::upload::{CleanupLeaseId, ReadUpload, UploadErrorKind, UploadHandle};

    let handle = UploadHandle::parse(handle)
        .map_err(|_| crate::FrameworkError::internal("upload cleanup handle was rejected"))?;
    let outcome = runtime
        .upload_cleanup_for_test()
        .run_once(
            CleanupLeaseId::parse("framework-upload-conformance").map_err(|_| {
                crate::FrameworkError::internal("upload cleanup lease was rejected")
            })?,
        )
        .await
        .map_err(|_| crate::FrameworkError::internal("Live upload cleanup was rejected"))?;
    let ledger_absent = runtime
        .upload_ledger()
        .load(&handle)
        .await
        .map_err(|_| crate::FrameworkError::internal("upload cleanup ledger check failed"))?
        .is_none();
    let provider_absent = runtime
        .upload_provider()
        .read(ReadUpload::new(&handle, 0, 1))
        .await
        .is_err();
    let metadata_absent = runtime
        .upload_reverse_proxy_adapter()
        .create_metadata(
            &handle,
            runtime.upload_now().map_err(|_| {
                crate::FrameworkError::internal("upload cleanup clock check failed")
            })?,
        )
        .is_err_and(|error| error.kind() == UploadErrorKind::ValidationEvidenceUnavailable);

    Ok(UploadCleanupConformanceReport {
        claimed: outcome.claimed(),
        reclaimed: outcome.reclaimed(),
        residue_is_empty: ledger_absent && provider_absent && metadata_absent,
    })
}

/// Exercises the actual framework quarantine, reverse-proxy, and unavailable-direct adapters.
pub async fn run_upload_provider_conformance_for_test()
-> Result<UploadProviderConformanceReport, crate::FrameworkError> {
    use suprnova_live::identity::{ScopeFingerprint, UnixMillis};
    use suprnova_live::limits::{UploadLimitConfig, UploadLimits};
    use suprnova_live::upload::{
        ChunkBody, ClientUploadMetadata, DirectUploadProvider, PrepareTransfer, QuarantineBytes,
        ReadUpload, ReverseProxyUploadProvider, UploadChecksum, UploadError, UploadErrorKind,
        UploadFuture, UploadHandle, UploadProvider, VerifyTransfer, WriteChunk,
    };

    struct TestChunkBody {
        parts: VecDeque<QuarantineBytes>,
    }

    impl ChunkBody for TestChunkBody {
        fn next_chunk<'a>(
            &'a mut self,
            maximum_bytes: usize,
        ) -> UploadFuture<'a, Result<Option<QuarantineBytes>, UploadError>> {
            Box::pin(async move {
                let next = self.parts.pop_front();
                if next.as_ref().is_some_and(|part| part.len() > maximum_bytes) {
                    return Err(UploadError::new(UploadErrorKind::InputTooLarge));
                }
                Ok(next)
            })
        }
    }

    fn framework_error(stage: &'static str) -> crate::FrameworkError {
        crate::FrameworkError::internal(format!(
            "Live upload provider conformance was rejected during {stage}"
        ))
    }

    let limits = UploadLimits::new(UploadLimitConfig::reference())
        .map_err(|_| framework_error("limit validation"))?;
    let root = tempfile::tempdir().map_err(|_| framework_error("temporary root creation"))?;
    let quarantine_root = root.path().join("quarantine");
    let store = Arc::new(
        super::ports::upload_provider::SuprnovaQuarantineStore::open(
            &quarantine_root,
            4,
            limits.max_chunk_bytes(),
        )
        .map_err(|_| framework_error("quarantine store creation"))?,
    );
    let provider =
        super::ports::upload_provider::SuprnovaReverseProxyUploadProvider::new(store, limits)
            .map_err(|_| framework_error("reverse-proxy provider creation"))?;
    let handle = UploadHandle::parse("018f47c1-2af0-7cc4-a001-000000000001")
        .map_err(|_| framework_error("handle parsing"))?;
    let bytes = b"hello world";
    let checksum = UploadChecksum::parse(&hex::encode(Sha256::digest(bytes)))
        .map_err(|_| framework_error("checksum parsing"))?;

    provider
        .prepare(PrepareTransfer::new(
            &handle,
            bytes.len() as u64,
            "client-name.txt",
            UnixMillis::new(1_000),
        ))
        .await
        .map_err(|_| framework_error("transfer preparation"))?;
    #[cfg(unix)]
    let quarantine_permissions_are_private = {
        use std::os::unix::fs::PermissionsExt as _;

        let directory_private = std::fs::metadata(&quarantine_root)
            .map(|metadata| metadata.permissions().mode() & 0o077 == 0)
            .unwrap_or(false);
        let objects_private = std::fs::read_dir(&quarantine_root)
            .map(|entries| {
                entries.filter_map(Result::ok).all(|entry| {
                    entry
                        .metadata()
                        .map(|metadata| metadata.permissions().mode() & 0o077 == 0)
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        directory_private && objects_private
    };
    #[cfg(not(unix))]
    let quarantine_permissions_are_private = true;
    let missing_before_bind = provider
        .create_metadata(&handle, UnixMillis::new(1_000))
        .is_err_and(|error| error.kind() == UploadErrorKind::ValidationEvidenceUnavailable);
    let client = ClientUploadMetadata::new("client-name.txt", Some("text/plain"))
        .map_err(|_| framework_error("client metadata normalization"))?;
    let metadata_scope =
        ScopeFingerprint::from_bytes(&[7_u8; 32]).map_err(|_| framework_error("metadata scope"))?;
    let metadata = super::ports::upload_provider::UploadCreateMetadata::new(
        client,
        bytes.len() as u64,
        7,
        UnixMillis::new(2_000),
        metadata_scope.clone(),
    );
    let inserted = provider
        .bind_create_metadata(handle.clone(), metadata.clone(), UnixMillis::new(1_000))
        .map_err(|_| framework_error("create metadata bind"))?;
    let existing = provider
        .bind_create_metadata(handle.clone(), metadata.clone(), UnixMillis::new(1_000))
        .map_err(|_| framework_error("exact create metadata replay"))?;
    let mismatch = provider.bind_create_metadata(
        handle.clone(),
        super::ports::upload_provider::UploadCreateMetadata::new(
            ClientUploadMetadata::new("other-name.txt", Some("text/plain"))
                .map_err(|_| framework_error("mismatch metadata normalization"))?,
            bytes.len() as u64,
            7,
            UnixMillis::new(2_000),
            metadata_scope,
        ),
        UnixMillis::new(1_000),
    );
    let memo_partial_order_recovered = missing_before_bind
        && provider
            .create_metadata(&handle, UnixMillis::new(1_000))
            .is_ok();
    let memo_redacted = !format!("{metadata:?}").contains("client-name.txt");

    let bounded =
        super::ports::upload_provider::UploadCreateMetadataMemo::new(1, 2_048, 1, 2_048, 1_000)
            .map_err(|_| framework_error("bounded metadata memo"))?;
    let bounded_first = UploadHandle::parse("018f47c1-2af0-7cc4-a001-000000000011")
        .map_err(|_| framework_error("bounded first handle"))?;
    let bounded_second = UploadHandle::parse("018f47c1-2af0-7cc4-a001-000000000012")
        .map_err(|_| framework_error("bounded second handle"))?;
    bounded
        .bind(bounded_first, metadata.clone(), UnixMillis::new(1_000))
        .map_err(|_| framework_error("bounded first metadata"))?;
    let memo_exhaustion_fails_closed = bounded
        .bind(bounded_second, metadata.clone(), UnixMillis::new(1_000))
        .is_err_and(|error| error.kind() == UploadErrorKind::ResourceExhausted);
    let isolated =
        super::ports::upload_provider::UploadCreateMetadataMemo::new(2, 4_096, 1, 2_048, 1_000)
            .map_err(|_| framework_error("scope-isolated metadata memo"))?;
    let isolated_a = UploadHandle::parse("018f47c1-2af0-7cc4-a001-000000000021")
        .map_err(|_| framework_error("isolated first handle"))?;
    let isolated_b = UploadHandle::parse("018f47c1-2af0-7cc4-a001-000000000022")
        .map_err(|_| framework_error("isolated second handle"))?;
    let isolated_a_overflow = UploadHandle::parse("018f47c1-2af0-7cc4-a001-000000000023")
        .map_err(|_| framework_error("isolated overflow handle"))?;
    let scope_a = ScopeFingerprint::from_bytes(&[8_u8; 32])
        .map_err(|_| framework_error("isolated first scope"))?;
    let scope_b = ScopeFingerprint::from_bytes(&[9_u8; 32])
        .map_err(|_| framework_error("isolated second scope"))?;
    let isolated_metadata = |scope| {
        super::ports::upload_provider::UploadCreateMetadata::new(
            ClientUploadMetadata::new("isolated.txt", Some("text/plain"))
                .expect("static isolated metadata"),
            bytes.len() as u64,
            8,
            UnixMillis::new(2_000),
            scope,
        )
    };
    let isolated_first = isolated.bind(
        isolated_a,
        isolated_metadata(scope_a.clone()),
        UnixMillis::new(1_000),
    );
    let isolated_second = isolated.bind(
        isolated_b,
        isolated_metadata(scope_b),
        UnixMillis::new(1_000),
    );
    let isolated_overflow = isolated.bind(
        isolated_a_overflow,
        isolated_metadata(scope_a),
        UnixMillis::new(1_000),
    );
    let memo_scope_isolation = isolated_first.is_ok()
        && isolated_second.is_ok()
        && isolated_overflow.is_err_and(|error| error.kind() == UploadErrorKind::ResourceExhausted);
    let mut body = TestChunkBody {
        parts: VecDeque::from([
            QuarantineBytes::copy_from_slice(b"hello "),
            QuarantineBytes::copy_from_slice(b"world"),
        ]),
    };
    let receipt = provider
        .write_chunk(
            WriteChunk::new(&handle, 0, 0, bytes.len() as u64, &checksum),
            &mut body,
        )
        .await
        .map_err(|_| framework_error("chunk write"))?;
    let progress = provider
        .progress(&handle)
        .map_err(|_| framework_error("authoritative progress checkpoint"))?;
    if progress.expected_bytes != bytes.len() as u64 || progress.committed_bytes != receipt.bytes()
    {
        return Err(framework_error("authoritative progress comparison"));
    }
    provider
        .verify(VerifyTransfer::new(&handle, &checksum))
        .await
        .map_err(|_| framework_error("whole-file verification"))?;
    let stored = provider
        .read(ReadUpload::new(&handle, 0, bytes.len()))
        .await
        .map_err(|_| framework_error("quarantine read"))?;
    if stored.as_ref() != bytes {
        return Err(framework_error("quarantine content comparison"));
    }
    provider
        .cancel(&handle)
        .await
        .map_err(|_| framework_error("quarantine cancellation"))?;
    provider.remove_create_metadata(&handle);
    let memo_lifecycle_deletion = provider
        .create_metadata(&handle, UnixMillis::new(1_000))
        .is_err_and(|error| error.kind() == UploadErrorKind::ValidationEvidenceUnavailable);
    let cancel_removed_quarantine = provider.read(ReadUpload::new(&handle, 0, 1)).await.is_err();

    let direct = super::ports::upload_provider::UnavailableDirectUploadProvider;
    let direct_provider_fails_closed = matches!(
        UploadProvider::prepare(
            &direct,
            PrepareTransfer::new(
                &handle,
                bytes.len() as u64,
                "client-name.txt",
                UnixMillis::new(1_000),
            ),
        )
        .await,
        Err(error) if error.kind() == UploadErrorKind::ProviderUnavailable
    );
    let _direct_marker: &dyn DirectUploadProvider = &direct;

    let failed_root =
        tempfile::tempdir().map_err(|_| framework_error("failed-store temporary root creation"))?;
    let failed_store = Arc::new(
        super::ports::upload_provider::SuprnovaQuarantineStore::open(
            failed_root.path(),
            2,
            limits.max_chunk_bytes(),
        )
        .map_err(|_| framework_error("failed quarantine store creation"))?,
    );
    let failed_provider = super::ports::upload_provider::SuprnovaReverseProxyUploadProvider::new(
        failed_store,
        limits,
    )
    .map_err(|_| framework_error("failed reverse-proxy provider creation"))?;
    failed_root
        .close()
        .map_err(|_| framework_error("failed-store root removal"))?;
    let failed_handle = UploadHandle::parse("018f47c1-2af0-7cc4-a001-000000000099")
        .map_err(|_| framework_error("failed-store handle parsing"))?;
    let storage_provider_fails_closed = failed_provider
        .prepare(PrepareTransfer::new(
            &failed_handle,
            bytes.len() as u64,
            "failed-store.txt",
            UnixMillis::new(1_000),
        ))
        .await
        .is_err_and(|error| error.kind() == UploadErrorKind::ProviderUnavailable);

    Ok(UploadProviderConformanceReport {
        received_bytes: progress.committed_bytes,
        next_chunk_index: progress.next_chunk_index,
        cancel_removed_quarantine,
        direct_provider_fails_closed,
        storage_provider_fails_closed,
        quarantine_permissions_are_private,
        memo_exact_replay: inserted
            == super::ports::upload_provider::UploadCreateMetadataDisposition::Inserted
            && existing
                == super::ports::upload_provider::UploadCreateMetadataDisposition::ExistingOutcome,
        memo_mismatch_fails_closed: mismatch
            .is_err_and(|error| error.kind() == UploadErrorKind::UploadConflict),
        memo_missing_fails_closed: missing_before_bind,
        memo_exhaustion_fails_closed,
        memo_scope_isolation,
        memo_lifecycle_deletion,
        memo_partial_order_recovered,
        memo_redacted,
    })
}

/// Projects a synthetic engine response through the production HTTP adapter.
pub fn project_live_response_for_test<'header>(
    status: u16,
    headers: impl IntoIterator<Item = (&'header str, &'header str)>,
    body: &[u8],
) -> Result<crate::HttpResponse, crate::FrameworkError> {
    let status = http::StatusCode::from_u16(status)
        .map_err(|_| crate::FrameworkError::internal("invalid test response status"))?;
    let mut header_map = http::HeaderMap::new();
    for (name, value) in headers {
        let name = http::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| crate::FrameworkError::internal("invalid test response header"))?;
        let value = http::HeaderValue::from_str(value)
            .map_err(|_| crate::FrameworkError::internal("invalid test response header"))?;
        header_map.append(name, value);
    }
    super::ports::response::SuprnovaResponseIntentPort.project(
        suprnova_live::endpoint::LiveEndpointResponse {
            status,
            headers: header_map,
            body: bytes::Bytes::copy_from_slice(body),
        },
    )
}

/// Runs the production post-acceptance event adapter with bounded metadata.
pub async fn report_live_outcome_for_test(
    revision: u64,
    outcome: suprnova_live::ledger::AcceptedOutcomeKind,
) -> Result<(), crate::FrameworkError> {
    super::ports::events::dispatch_accepted_outcome(revision, outcome)
        .await
        .map_err(|_| crate::FrameworkError::internal("Live outcome reporting failed"))
}

/// Fluent assertions over one semantic Live action result.
#[derive(Clone, Copy, Debug)]
pub struct ActionAssertion<'result> {
    result: &'result ActionResult,
}

impl<'result> ActionAssertion<'result> {
    /// Starts assertions for one completed semantic action result.
    #[must_use]
    pub const fn new(result: &'result ActionResult) -> Self {
        Self { result }
    }

    /// Asserts that the action requested fresh island rendering.
    pub fn assert_rendered(self) {
        assert!(
            matches!(self.result.outcome(), ActionOutcome::Render),
            "expected Live action to render, got {:?}",
            self.result.outcome()
        );
    }

    /// Asserts that the action completed without fresh island rendering.
    pub fn assert_not_rendered(self) {
        assert!(
            matches!(self.result.outcome(), ActionOutcome::NoRender),
            "expected Live action not to render, got {:?}",
            self.result.outcome()
        );
    }

    /// Asserts that the action requested an ordinary registered-route navigation.
    pub fn assert_redirected(self) {
        assert!(
            self.result.outcome().redirects(),
            "expected Live action redirect, got {:?}",
            self.result.outcome()
        );
    }
}
