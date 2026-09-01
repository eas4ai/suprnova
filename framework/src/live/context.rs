//! Conversion of request-bound Suprnova facts into engine-owned authority.

use std::sync::Arc;

use sha2::{Digest, Sha256};
use suprnova_live::action::ActionAuthorizationPort;
use suprnova_live::host::{
    CheckFact, CheckKind, HostCapabilities, HostCheckFacts, HostScopeFacts,
    LiveRequestContextCandidate, MountSelection, PrincipalFingerprint, SessionFingerprint,
    TenantFingerprint,
};
use suprnova_live::identity::{IslandSlot, RouteIdentity, ScopeFingerprint};

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
    pub(crate) stateless_session: bool,
    pub(crate) anonymous_principal: bool,
    pub(crate) tenantless: bool,
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

#[async_trait]
impl Middleware for LiveMiddlewareCompletion {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        if request.live_operation().is_some() {
            self.close_policy_absences(&mut request);
        }
        next(request).await
    }
}

pub(crate) fn candidate(
    request: &Request,
    current_route: RouteIdentity,
    current_slot: IslandSlot,
    selection: MountSelection,
    action_authorization: Arc<dyn ActionAuthorizationPort>,
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
    let scope = aggregate_scope(
        session_bytes.as_ref().map(<[u8; 32]>::as_slice),
        principal_bytes.as_ref().map(<[u8; 32]>::as_slice),
        tenant_bytes.as_ref().map(<[u8; 32]>::as_slice),
    )?;
    let scope = HostScopeFacts::new(scope, session, principal, tenant);
    let capabilities =
        HostCapabilities::bound_to(scope.clone()).with_action_authorization(action_authorization);

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

fn aggregate_scope(
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
