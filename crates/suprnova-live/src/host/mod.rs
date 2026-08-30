//! Host-produced request authority and exact mount-catalog resolution.

mod capabilities;
mod catalog;
mod checks;
mod context;

use std::error::Error;
use std::fmt;

pub use capabilities::{
    HostCapabilities, HostScopeFacts, PrincipalFingerprint, SessionFingerprint, TenantFingerprint,
};
pub use catalog::{
    MountCatalog, MountCatalogBuilder, MountCatalogEntry, MountScopeRequirements, MountSelection,
    ScopeRequirement, VerifiedMountCatalogMatch,
};
pub use checks::{
    CheckDisposition, CheckFact, CheckKind, HostCheckFacts, PolicyReason, RequiredChecks,
};
pub use context::{
    LiveRequestContextCandidate, LiveRequestContextValidator, TrustedLiveRequestContext,
};

/// Closed reason untrusted or inconsistent host facts did not become Live authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostContextErrorKind {
    /// Context-validation policy was zero or above its hard lifetime ceiling.
    InvalidConfiguration,
    /// A host check was recorded more than once.
    DuplicateCheck,
    /// One of the exact required host check kinds was absent.
    MissingCheck,
    /// A not-required reason did not belong to its check kind or current scope.
    InvalidCheckDisposition,
    /// A host check fact was already expired.
    CheckExpired,
    /// The requested trusted context was already expired.
    ContextExpired,
    /// The requested trusted-context lifetime exceeded its configured bound.
    ContextLifetimeExceeded,
    /// No catalog route matched the trusted current route.
    RouteMismatch,
    /// The route existed but did not own the selected island slot.
    SlotMismatch,
    /// The selected component did not match the registered mount.
    ComponentMismatch,
    /// Generated registry, mount catalog, or selected contract digests disagreed.
    ContractMismatch,
    /// The selected protocol was unsupported or below the component minimum.
    ProtocolMismatch,
    /// The mount's session-presence policy was not satisfied.
    SessionRequirement,
    /// The mount's principal-presence policy was not satisfied.
    PrincipalRequirement,
    /// The mount's tenant-presence policy was not satisfied.
    TenantRequirement,
    /// Opaque capabilities were issued for another aggregate scope.
    ScopeMismatch,
    /// Opaque capabilities were issued for another session.
    SessionMismatch,
    /// Opaque capabilities were issued for another principal.
    PrincipalMismatch,
    /// Opaque capabilities were issued for another tenant.
    TenantMismatch,
    /// The mount catalog exceeded its hard capacity or duplicated a route/slot.
    CatalogConflict,
}

impl HostContextErrorKind {
    /// Returns a stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidConfiguration => "invalid_host_context_configuration",
            Self::DuplicateCheck => "duplicate_host_check",
            Self::MissingCheck => "missing_host_check",
            Self::InvalidCheckDisposition => "invalid_host_check_disposition",
            Self::CheckExpired => "expired_host_check",
            Self::ContextExpired => "expired_host_context",
            Self::ContextLifetimeExceeded => "host_context_lifetime_exceeded",
            Self::RouteMismatch => "host_route_mismatch",
            Self::SlotMismatch => "host_slot_mismatch",
            Self::ComponentMismatch => "host_component_mismatch",
            Self::ContractMismatch => "host_contract_mismatch",
            Self::ProtocolMismatch => "host_protocol_mismatch",
            Self::SessionRequirement => "host_session_requirement",
            Self::PrincipalRequirement => "host_principal_requirement",
            Self::TenantRequirement => "host_tenant_requirement",
            Self::ScopeMismatch => "host_scope_mismatch",
            Self::SessionMismatch => "host_session_mismatch",
            Self::PrincipalMismatch => "host_principal_mismatch",
            Self::TenantMismatch => "host_tenant_mismatch",
            Self::CatalogConflict => "host_catalog_conflict",
        }
    }
}

/// Redacted host-context construction failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct HostContextError {
    kind: HostContextErrorKind,
}

impl HostContextError {
    pub(crate) const fn new(kind: HostContextErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed failure class.
    #[must_use]
    pub const fn kind(self) -> HostContextErrorKind {
        self.kind
    }
}

impl fmt::Display for HostContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for HostContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for HostContextError {}
