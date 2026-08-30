//! Validation-only construction of request-scoped Live authority.

use std::fmt;

use crate::identity::{IslandSlot, RouteIdentity, ScopeFingerprint, UnixMillis};
use crate::promotion::TrustedPromotionContext;

use super::{
    CheckDisposition, CheckKind, HostCapabilities, HostCheckFacts, HostContextError,
    HostContextErrorKind, HostScopeFacts, MountCatalog, MountSelection, RequiredChecks,
    ScopeRequirement, VerifiedMountCatalogMatch,
};

const MAX_CONTEXT_LIFETIME_MS: u64 = 300_000;

/// Complete but non-authoritative host candidate passed to the production validator.
pub struct LiveRequestContextCandidate {
    current_route: RouteIdentity,
    current_slot: IslandSlot,
    selection: MountSelection,
    scope: HostScopeFacts,
    checks: HostCheckFacts,
    capabilities: HostCapabilities,
    expires_at: UnixMillis,
}

impl LiveRequestContextCandidate {
    /// Groups normalized facts without granting any Live authority.
    #[must_use]
    pub const fn new(
        current_route: RouteIdentity,
        current_slot: IslandSlot,
        selection: MountSelection,
        scope: HostScopeFacts,
        checks: HostCheckFacts,
        capabilities: HostCapabilities,
        expires_at: UnixMillis,
    ) -> Self {
        Self {
            current_route,
            current_slot,
            selection,
            scope,
            checks,
            capabilities,
            expires_at,
        }
    }
}

impl fmt::Debug for LiveRequestContextCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<LiveRequestContextCandidate:redacted>")
    }
}

/// Configured production validator for host facts and catalog ownership.
#[derive(Clone, Copy, Debug)]
pub struct LiveRequestContextValidator {
    max_lifetime_ms: u64,
}

impl LiveRequestContextValidator {
    /// Creates a nonzero context lifetime policy below the engine hard ceiling.
    pub fn new(max_lifetime_ms: u64) -> Result<Self, HostContextError> {
        if max_lifetime_ms == 0 || max_lifetime_ms > MAX_CONTEXT_LIFETIME_MS {
            return Err(HostContextError::new(
                HostContextErrorKind::InvalidConfiguration,
            ));
        }
        Ok(Self { max_lifetime_ms })
    }

    /// Validates all host facts and produces the only production request authority.
    pub fn validate(
        &self,
        catalog: &MountCatalog,
        candidate: LiveRequestContextCandidate,
        now: UnixMillis,
    ) -> Result<TrustedLiveRequestContext, HostContextError> {
        if candidate.expires_at <= now {
            return Err(HostContextError::new(HostContextErrorKind::ContextExpired));
        }
        if candidate.expires_at.get().saturating_sub(now.get()) > self.max_lifetime_ms {
            return Err(HostContextError::new(
                HostContextErrorKind::ContextLifetimeExceeded,
            ));
        }
        if candidate.current_route != *candidate.selection.route() {
            return Err(HostContextError::new(HostContextErrorKind::RouteMismatch));
        }
        if candidate.current_slot != *candidate.selection.slot() {
            return Err(HostContextError::new(HostContextErrorKind::SlotMismatch));
        }
        let mount = catalog.resolve(&candidate.selection)?;
        let (checks, expires_at) =
            RequiredChecks::validate(candidate.checks, now, candidate.expires_at)?;
        validate_mount_scope(&candidate.scope, &checks, mount.requirements())?;
        validate_capability_scope(&candidate.scope, candidate.capabilities.scope())?;
        Ok(TrustedLiveRequestContext {
            scope: candidate.scope,
            mount,
            checks,
            capabilities: candidate.capabilities,
            expires_at,
        })
    }
}

/// Request-scoped capability created only by complete production validation.
pub struct TrustedLiveRequestContext {
    scope: HostScopeFacts,
    mount: VerifiedMountCatalogMatch,
    checks: RequiredChecks,
    capabilities: HostCapabilities,
    expires_at: UnixMillis,
}

impl TrustedLiveRequestContext {
    pub(crate) const fn host_scope_facts(&self) -> &HostScopeFacts {
        &self.scope
    }

    /// Returns the current aggregate principal/session/tenant scope.
    #[must_use]
    pub const fn scope(&self) -> &ScopeFingerprint {
        self.scope.scope()
    }

    /// Returns the registry-verified mount catalog match.
    #[must_use]
    pub const fn mount(&self) -> &VerifiedMountCatalogMatch {
        &self.mount
    }

    /// Returns all exact validated check dispositions.
    #[must_use]
    pub const fn checks(&self) -> &RequiredChecks {
        &self.checks
    }

    /// Returns opaque host services bound to this request identity.
    #[must_use]
    pub const fn capabilities(&self) -> &HostCapabilities {
        &self.capabilities
    }

    /// Returns the exclusive authority expiration deadline.
    #[must_use]
    pub const fn expires_at(&self) -> UnixMillis {
        self.expires_at
    }

    /// Projects least-privilege seed-promotion authority from this request.
    #[must_use]
    pub fn for_promotion(&self) -> TrustedPromotionContext {
        TrustedPromotionContext::from_request(self)
    }

    /// Returns whether authority is current at the supplied host clock value.
    #[must_use]
    pub fn is_current(&self, now: UnixMillis) -> bool {
        now < self.expires_at
    }
}

impl fmt::Debug for TrustedLiveRequestContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<TrustedLiveRequestContext:redacted>")
    }
}

fn validate_mount_scope(
    scope: &HostScopeFacts,
    checks: &RequiredChecks,
    requirements: super::MountScopeRequirements,
) -> Result<(), HostContextError> {
    validate_presence(
        requirements.session(),
        scope.session().is_some(),
        checks.get(CheckKind::Session),
        HostContextErrorKind::SessionRequirement,
    )?;
    validate_presence(
        requirements.principal(),
        scope.principal().is_some(),
        checks.get(CheckKind::Principal),
        HostContextErrorKind::PrincipalRequirement,
    )?;
    validate_presence(
        requirements.tenant(),
        scope.tenant().is_some(),
        checks.get(CheckKind::Tenant),
        HostContextErrorKind::TenantRequirement,
    )
}

fn validate_presence(
    requirement: ScopeRequirement,
    present: bool,
    disposition: CheckDisposition,
    error: HostContextErrorKind,
) -> Result<(), HostContextError> {
    let coherent = match requirement {
        ScopeRequirement::Required => present && disposition == CheckDisposition::Passed,
        ScopeRequirement::Optional => present == (disposition == CheckDisposition::Passed),
        ScopeRequirement::Absent => !present && disposition != CheckDisposition::Passed,
    };
    if !coherent {
        return Err(HostContextError::new(error));
    }
    Ok(())
}

fn validate_capability_scope(
    request: &HostScopeFacts,
    capabilities: &HostScopeFacts,
) -> Result<(), HostContextError> {
    if request.scope() != capabilities.scope() {
        return Err(HostContextError::new(HostContextErrorKind::ScopeMismatch));
    }
    if request.session() != capabilities.session() {
        return Err(HostContextError::new(HostContextErrorKind::SessionMismatch));
    }
    if request.principal() != capabilities.principal() {
        return Err(HostContextError::new(
            HostContextErrorKind::PrincipalMismatch,
        ));
    }
    if request.tenant() != capabilities.tenant() {
        return Err(HostContextError::new(HostContextErrorKind::TenantMismatch));
    }
    Ok(())
}
