//! Redacted component-registry failures.

use std::error::Error;
use std::fmt;

/// Closed reason explicit component registration or lookup failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryErrorKind {
    /// Two descriptors claimed the same component identity.
    DuplicateComponent,
    /// Two component descriptors claimed the same checked root view.
    DuplicateView,
    /// Startup registration exceeded the hard component-count bound.
    CapacityExceeded,
    /// The component was not explicitly registered at startup.
    NotRegistered,
    /// The registered component no longer matches the required contract digest.
    ContractMismatch,
}

impl RegistryErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DuplicateComponent => "duplicate_component_registration",
            Self::DuplicateView => "duplicate_component_view",
            Self::CapacityExceeded => "component_registry_capacity_exceeded",
            Self::NotRegistered => "component_not_registered",
            Self::ContractMismatch => "component_contract_mismatch",
        }
    }
}

/// Redacted component registry error.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct RegistryError {
    kind: RegistryErrorKind,
}

impl RegistryError {
    pub(crate) const fn new(kind: RegistryErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed registry failure.
    #[must_use]
    pub const fn kind(self) -> RegistryErrorKind {
        self.kind
    }
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for RegistryError {}
