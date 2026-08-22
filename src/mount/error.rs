//! Closed redacted private-mount failures.

use std::error::Error;
use std::fmt;

/// Stable category for an identity-bound initial mount failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MountErrorKind {
    /// Mount limits are zero, inconsistent, or above hard ceilings.
    InvalidConfiguration,
    /// The trusted host request is expired or no longer matches the catalog.
    ContextRejected,
    /// The selected component is absent or its generated contract drifted.
    ComponentRejected,
    /// Explicit mount parameters violate the registered schema or byte bounds.
    ParametersRejected,
    /// A document-local mount key was already published or reserved.
    DuplicateDocumentKey,
    /// A document attempted to reserve more mount identities than its hard bound.
    DocumentCapacity,
    /// Inert mount metadata exceeds its bounded count or byte budget.
    MetadataTooLarge,
    /// The server identity source failed.
    RandomUnavailable,
    /// The host clock failed or could not produce a bounded deadline.
    ClockUnavailable,
    /// Component construction, lifecycle, rendering, or dehydration failed.
    LifecycleRejected,
    /// The complete instanced snapshot could not be validated or signed.
    SnapshotRejected,
    /// The assembled engine-owned island wrapper failed structural validation.
    RenderRejected,
    /// The create-only ledger rejected a non-collision authority write.
    LedgerRejected,
    /// Every bounded candidate identity collided with existing authority.
    IdentityCollision,
}

impl MountErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidConfiguration => "invalid_mount_configuration",
            Self::ContextRejected => "mount_context_rejected",
            Self::ComponentRejected => "mount_component_rejected",
            Self::ParametersRejected => "mount_parameters_rejected",
            Self::DuplicateDocumentKey => "duplicate_document_mount_key",
            Self::DocumentCapacity => "document_mount_capacity_exceeded",
            Self::MetadataTooLarge => "mount_metadata_too_large",
            Self::RandomUnavailable => "mount_random_unavailable",
            Self::ClockUnavailable => "mount_clock_unavailable",
            Self::LifecycleRejected => "mount_lifecycle_rejected",
            Self::SnapshotRejected => "mount_snapshot_rejected",
            Self::RenderRejected => "mount_render_rejected",
            Self::LedgerRejected => "mount_ledger_rejected",
            Self::IdentityCollision => "mount_identity_collision",
        }
    }
}

/// Redacted private-mount error.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct MountError {
    kind: MountErrorKind,
}

impl MountError {
    pub(crate) const fn new(kind: MountErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed failure category.
    #[must_use]
    pub const fn kind(self) -> MountErrorKind {
        self.kind
    }
}

impl fmt::Display for MountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for MountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for MountError {}
