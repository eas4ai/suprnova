//! Closed redacted private-mount failures.

use std::error::Error;
use std::fmt;

use super::failure::{MountCause, MountFailure};
use crate::ledger::LedgerErrorKind;

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
    cause: Option<MountFailure>,
}

impl MountError {
    pub(crate) const fn new(kind: MountErrorKind) -> Self {
        Self { kind, cause: None }
    }

    /// Keeps the closed kind of the subsystem error behind the coarse mount kind.
    pub(crate) fn caused_by(kind: MountErrorKind, error: &impl MountCause) -> Self {
        Self {
            kind,
            cause: Some(error.mount_failure()),
        }
    }

    /// Returns the closed failure category.
    #[must_use]
    pub const fn kind(self) -> MountErrorKind {
        self.kind
    }

    /// Returns the closed cause when a trusted subsystem failed the mount.
    ///
    /// `None` means the kind says everything: the refusal has no subsystem error behind it, such
    /// as an authority mismatch or an identity collision. The kind stays coarse outside the host;
    /// the cause lets an operator tell, for example, a store outage from a capacity fault.
    #[must_use]
    pub const fn cause(self) -> Option<MountFailure> {
        self.cause
    }

    /// Returns the closed reason the instance ledger gave when it refused the mount.
    ///
    /// Shorthand for a [`MountFailure::Ledger`] cause, which only a
    /// [`MountErrorKind::LedgerRejected`] carries.
    #[must_use]
    pub const fn ledger_kind(self) -> Option<LedgerErrorKind> {
        match self.cause {
            Some(MountFailure::Ledger(kind)) => Some(kind),
            _ => None,
        }
    }
}

impl fmt::Display for MountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for MountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())?;
        if let Some(cause) = self.cause {
            write!(formatter, ":{cause:?}")?;
        }
        Ok(())
    }
}

impl Error for MountError {}
