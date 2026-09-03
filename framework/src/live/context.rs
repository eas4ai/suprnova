//! Conversion of request-bound Suprnova facts into engine-owned authority.

use std::sync::Arc;

use sha2::{Digest, Sha256};
use suprnova_live::action::ActionAuthorizationPort;
use suprnova_live::async_updates::{
    SubscriptionAuthorizationPort, SubscriptionContinuityPort, SubscriptionCredentialPort,
    SubscriptionRegistryPort,
};
use suprnova_live::host::{
    CheckFact, CheckKind, HostCapabilities, HostCheckFacts, HostScopeFacts,
    LiveRequestContextCandidate, MountSelection, PrincipalFingerprint, SessionFingerprint,
    TenantFingerprint,
};
use suprnova_live::identity::{IslandSlot, RouteIdentity, ScopeFingerprint, UnixMillis};
use suprnova_live::upload::UploadAuthorizationPort;

use crate::{FrameworkError, Request};
use crate::{Middleware, Next, Response, async_trait};

use super::attestation::SecurityCheck;

/// Route-owned operation and policy consumed before global middleware runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LiveRouteMetadata {
    operation: super::attestation::LiveOperation,
    policy: LiveRouteSecurityPolicy,
}

impl LiveRouteMetadata {
    pub(crate) const fn new(
        operation: super::attestation::LiveOperation,
        policy: LiveRouteSecurityPolicy,
    ) -> Self {
        Self { operation, policy }
    }

    pub(crate) const fn operation(self) -> super::attestation::LiveOperation {
        self.operation
    }

    pub(crate) const fn completion(self) -> LiveMiddlewareCompletion {
        LiveMiddlewareCompletion::new(self.policy)
    }

    pub(crate) fn merge_document_policy(
        &mut self,
        policy: LiveRouteSecurityPolicy,
    ) -> Result<(), crate::FrameworkError> {
        if self.operation != super::attestation::LiveOperation::Document {
            return Err(crate::FrameworkError::internal(
                "Live document metadata collided with another operation",
            ));
        }
        self.policy = self.policy.intersect(policy);
        Ok(())
    }
}

/// Final route-owned marker proving the configured Live middleware chain ran.
pub(crate) struct LiveMiddlewareCompletion {
    policy: LiveRouteSecurityPolicy,
}

/// Closed route policy for security facts that may legitimately be absent.
///
/// Owner middleware still records every positive decision. This policy only
/// permits the final route boundary to close an absent fact with the matching
/// typed `not_required` reason.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LiveRouteSecurityPolicy {
    pub(crate) trusted_internal_origin: bool,
    pub(crate) stateless_csrf: bool,
    pub(crate) stateless_session: bool,
    pub(crate) anonymous_principal: bool,
    pub(crate) tenantless: bool,
    pub(crate) direct_peer: bool,
    pub(crate) upstream_rate_limit: bool,
    pub(crate) no_additional_middleware: bool,
}

impl LiveMiddlewareCompletion {
    pub(crate) const fn new(policy: LiveRouteSecurityPolicy) -> Self {
        Self { policy }
    }

    pub(crate) fn close_policy_absences(&self, request: &mut Request) {
        use suprnova_live::host::PolicyReason;

        let allowed = [
            (
                self.policy.trusted_internal_origin,
                SecurityCheck::Origin,
                PolicyReason::TrustedInternalOrigin,
            ),
            (
                self.policy.stateless_csrf,
                SecurityCheck::Csrf,
                PolicyReason::StatelessCsrfPolicy,
            ),
            (
                self.policy.stateless_session,
                SecurityCheck::Session,
                PolicyReason::StatelessRequest,
            ),
            (
                self.policy.anonymous_principal,
                SecurityCheck::Principal,
                PolicyReason::AnonymousPrincipal,
            ),
            (
                self.policy.tenantless,
                SecurityCheck::Tenant,
                PolicyReason::TenantlessRoute,
            ),
            (
                self.policy.direct_peer,
                SecurityCheck::Proxy,
                PolicyReason::DirectPeer,
            ),
            (
                self.policy.upstream_rate_limit,
                SecurityCheck::RateLimit,
                PolicyReason::UpstreamRateLimited,
            ),
        ];
        for (permitted, check, reason) in allowed {
            if permitted
                && request
                    .live_security_attestation()
                    .disposition(check)
                    .is_none()
            {
                request.record_live_security_not_required(check, reason);
            }
        }

        if self.policy.no_additional_middleware {
            request.record_live_security_not_required(
                SecurityCheck::Middleware,
                PolicyReason::NoAdditionalMiddleware,
            );
        } else {
            request.record_live_security_check(SecurityCheck::Middleware, None);
        }
    }
}

impl LiveRouteSecurityPolicy {
    pub(crate) const fn intersect(self, other: Self) -> Self {
        Self {
            trusted_internal_origin: self.trusted_internal_origin && other.trusted_internal_origin,
            stateless_csrf: self.stateless_csrf && other.stateless_csrf,
            stateless_session: self.stateless_session && other.stateless_session,
            anonymous_principal: self.anonymous_principal && other.anonymous_principal,
            tenantless: self.tenantless && other.tenantless,
            direct_peer: self.direct_peer && other.direct_peer,
            upstream_rate_limit: self.upstream_rate_limit && other.upstream_rate_limit,
            no_additional_middleware: self.no_additional_middleware
                && other.no_additional_middleware,
        }
    }
}

#[async_trait]
impl Middleware for LiveMiddlewareCompletion {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        if request.live_operation().is_some() {
            self.close_policy_absences(&mut request);
        }
        next(request).await
    }
}

/// Host ports installed only for the asynchronous subscription boundaries.
pub(crate) struct SubscriptionCapabilities {
    pub(crate) registry: Arc<dyn SubscriptionRegistryPort>,
    pub(crate) authorization: Arc<dyn SubscriptionAuthorizationPort>,
    pub(crate) continuity: Arc<dyn SubscriptionContinuityPort>,
    pub(crate) credentials: Arc<dyn SubscriptionCredentialPort>,
}

#[allow(
    clippy::too_many_arguments,
    reason = "the candidate names every trusted authority input explicitly"
)]
pub(crate) fn candidate(
    request: &Request,
    current_route: RouteIdentity,
    current_slot: IslandSlot,
    selection: MountSelection,
    scope_override: Option<ScopeFingerprint>,
    action_authorization: Arc<dyn ActionAuthorizationPort>,
    upload_authorization: Arc<dyn UploadAuthorizationPort>,
    subscription: Option<SubscriptionCapabilities>,
) -> Result<LiveRequestContextCandidate, FrameworkError> {
    let identity = request.live_request_identity();
    let attestation = request.live_security_attestation();
    if !attestation.order_valid() {
        return Err(context_error());
    }
    let expires_at = attestation.expires_at(identity).ok_or_else(context_error)?;
    let mut checks = HostCheckFacts::new();
    for (security, engine) in check_pairs() {
        if let Some(fact) = attestation.fact(identity, security) {
            checks
                .record(engine, CheckFact::new(fact.disposition, fact.expires_at))
                .map_err(|_| context_error())?;
        }
    }

    let scope = scope_facts(request, scope_override)?;
    let capabilities = HostCapabilities::bound_to(scope.clone())
        .with_action_authorization(action_authorization)
        .with_upload_authorization(upload_authorization);
    let capabilities = match subscription {
        Some(subscription) => capabilities
            .with_subscription_registry(subscription.registry)
            .with_subscription_authorization(subscription.authorization)
            .with_subscription_continuity(subscription.continuity)
            .with_subscription_credentials(subscription.credentials),
        None => capabilities,
    };

    Ok(LiveRequestContextCandidate::new(
        current_route,
        current_slot,
        selection,
        scope,
        checks,
        capabilities,
        expires_at,
    ))
}

/// Returns the normalized identity facts of one attested request without a mount.
///
/// A mount-bound request proves its complete check set inside the engine
/// validator; this boundary has no mount, so it requires the same complete,
/// unexpired, well-formed check set itself before any fact is trusted.
pub(crate) fn request_host_scope_facts(
    request: &Request,
    now: UnixMillis,
) -> Result<HostScopeFacts, FrameworkError> {
    let identity = request.live_request_identity();
    let attestation = request.live_security_attestation();
    if !attestation.order_valid() {
        return Err(context_error());
    }
    let mut checks = HostCheckFacts::new();
    for (security, engine) in check_pairs() {
        if let Some(fact) = attestation.fact(identity, security) {
            checks
                .record(engine, CheckFact::new(fact.disposition, fact.expires_at))
                .map_err(|_| context_error())?;
        }
    }
    checks.require_complete(now).map_err(|_| context_error())?;
    scope_facts(request, None)
}

fn scope_facts(
    request: &Request,
    scope_override: Option<ScopeFingerprint>,
) -> Result<HostScopeFacts, FrameworkError> {
    let identity = request.live_request_identity();
    let attestation = request.live_security_attestation();
    if !attestation.order_valid() {
        return Err(context_error());
    }
    let session_bytes = identity_fingerprint(attestation, identity, SecurityCheck::Session)?;
    let principal_bytes = identity_fingerprint(attestation, identity, SecurityCheck::Principal)?;
    let tenant_bytes = identity_fingerprint(attestation, identity, SecurityCheck::Tenant)?;
    let session = session_bytes
        .map(|bytes| SessionFingerprint::from_bytes(&bytes))
        .transpose()
        .map_err(|_| context_error())?;
    let principal = principal_bytes
        .map(|bytes| PrincipalFingerprint::from_bytes(&bytes))
        .transpose()
        .map_err(|_| context_error())?;
    let tenant = tenant_bytes
        .map(|bytes| TenantFingerprint::from_bytes(&bytes))
        .transpose()
        .map_err(|_| context_error())?;
    let scope = scope_override.unwrap_or(aggregate_scope(
        session_bytes.as_ref().map(<[u8; 32]>::as_slice),
        principal_bytes.as_ref().map(<[u8; 32]>::as_slice),
        tenant_bytes.as_ref().map(<[u8; 32]>::as_slice),
    )?);
    Ok(HostScopeFacts::new(scope, session, principal, tenant))
}

pub(crate) fn request_scope(request: &Request) -> Result<ScopeFingerprint, FrameworkError> {
    let identity = request.live_request_identity();
    let attestation = request.live_security_attestation();
    if !attestation.order_valid() {
        return Err(context_error());
    }
    let session = identity_fingerprint(attestation, identity, SecurityCheck::Session)?;
    let principal = identity_fingerprint(attestation, identity, SecurityCheck::Principal)?;
    let tenant = identity_fingerprint(attestation, identity, SecurityCheck::Tenant)?;
    aggregate_scope(
        session.as_ref().map(<[u8; 32]>::as_slice),
        principal.as_ref().map(<[u8; 32]>::as_slice),
        tenant.as_ref().map(<[u8; 32]>::as_slice),
    )
}

fn identity_fingerprint(
    attestation: &super::attestation::LiveSecurityAttestation,
    identity: super::attestation::LiveRequestIdentity,
    check: SecurityCheck,
) -> Result<Option<[u8; 32]>, FrameworkError> {
    let Some(fact) = attestation.fact(identity, check) else {
        return Ok(None);
    };
    match fact.disposition {
        suprnova_live::host::CheckDisposition::Passed => {
            fact.fingerprint.map(Some).ok_or_else(context_error)
        }
        suprnova_live::host::CheckDisposition::NotRequired(_) => Ok(None),
    }
}

pub(super) fn aggregate_scope(
    session: Option<&[u8]>,
    principal: Option<&[u8]>,
    tenant: Option<&[u8]>,
) -> Result<ScopeFingerprint, FrameworkError> {
    let mut digest = Sha256::new();
    digest.update(b"suprnova-live/host-scope/v1\0");
    for value in [session, principal, tenant] {
        match value {
            Some(value) => {
                digest.update([1]);
                digest.update(value);
            }
            None => digest.update([0]),
        }
    }
    let bytes: [u8; 32] = digest.finalize().into();
    ScopeFingerprint::from_bytes(&bytes).map_err(|_| context_error())
}

const fn check_pairs() -> [(SecurityCheck, CheckKind); 8] {
    [
        (SecurityCheck::Origin, CheckKind::Origin),
        (SecurityCheck::Csrf, CheckKind::Csrf),
        (SecurityCheck::Session, CheckKind::Session),
        (SecurityCheck::Principal, CheckKind::Principal),
        (SecurityCheck::Tenant, CheckKind::Tenant),
        (SecurityCheck::Proxy, CheckKind::Proxy),
        (SecurityCheck::RateLimit, CheckKind::RateLimit),
        (SecurityCheck::Middleware, CheckKind::Middleware),
    ]
}

fn context_error() -> FrameworkError {
    FrameworkError::internal("Live request context was rejected")
}
