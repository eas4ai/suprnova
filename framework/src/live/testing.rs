//! Focused assertion helpers for application Live tests.

use std::fmt;

use crate::Request;

use super::attestation::{LiveOperation, SecurityCheck};
use super::{ActionOutcome, ActionResult, LiveRuntime};

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
}

impl LiveTestRuntimeProvider {
    /// Every provider that must be present before traffic begins.
    pub const ALL: [Self; 11] = [
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
    let expected = ExpectedSeedV1::new(
        SnapshotContract::new(
            descriptor.metadata().identity().clone(),
            descriptor.contract_digest().clone(),
            1,
            1,
            1,
        )
        .map_err(|_| fixture_error())?,
        BuildId::parse("test-build").map_err(|_| fixture_error())?,
        route,
        slot,
        schemas,
    );
    router.register_live_mount_entry(MountCatalogEntry::new(
        expected,
        MountScopeRequirements::new(
            ScopeRequirement::Absent,
            ScopeRequirement::Absent,
            ScopeRequirement::Absent,
        ),
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
        runtime.register_mount(MountCatalogEntry::new(
            expected_seed,
            MountScopeRequirements::new(
                ScopeRequirement::Absent,
                ScopeRequirement::Absent,
                ScopeRequirement::Absent,
            ),
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
    mount_catalog: bool,
    response_and_cancellation: bool,
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
            && self.mount_catalog
            && self.response_and_cancellation
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
        mount_catalog: readiness.mount_catalog,
        response_and_cancellation: readiness.response_and_cancellation,
    }
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
