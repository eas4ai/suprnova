//! Closed causes of mounts the engine refused.

use crate::canonical::{CanonicalError, CanonicalErrorKind};
use crate::clock::{ClockError, ClockErrorKind};
use crate::component::composition::{CompositionError, CompositionErrorKind};
use crate::component::{LifecycleError, LifecycleErrorKind};
use crate::ledger::{LedgerError, LedgerErrorKind};
use crate::random::{RandomError, RandomErrorKind};
use crate::registry::{RegistryError, RegistryErrorKind};
use crate::snapshot::{SnapshotError, SnapshotErrorKind};
use crate::view::{ViewError, ViewErrorKind};

/// Closed cause of a mount that the engine refused.
///
/// The coarse [`MountErrorKind`](super::MountErrorKind) is all a mount failure says outside the
/// host. The cause stays with the host, so an operator can tell which trusted subsystem failed
/// without reproducing the request. Every variant carries closed kinds only and never text from
/// a request, a component, or a provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MountFailure {
    /// The clock could not supply the mount time or the completion time.
    Clock(ClockErrorKind),
    /// The component registry holds no component with the contract the catalog selected.
    Registry(RegistryErrorKind),
    /// The snapshot subsystem refused the mount parameters, the public state or memo, or the
    /// instance or seed body and its signature.
    Snapshot(SnapshotErrorKind),
    /// The canonical codec refused the mount parameters.
    Canonical(CanonicalErrorKind),
    /// The composition subsystem: the component's registered parameter schema refused the mount
    /// parameters.
    Composition(CompositionErrorKind),
    /// The server identity generator could not produce an instance identity.
    Random(RandomErrorKind),
    /// The component executor failed the initial mount lifecycle.
    Lifecycle(LifecycleErrorKind),
    /// The view subsystem refused the rendered fragment, the mount metadata, or the assembled
    /// island root.
    View(ViewErrorKind),
    /// The instance ledger refused to create the instance authority.
    Ledger(LedgerErrorKind),
}

/// Subsystem error whose closed kind a refused mount keeps as its cause.
pub(crate) trait MountCause {
    /// Names the failed subsystem and its closed kind.
    fn mount_failure(&self) -> MountFailure;
}

impl MountCause for ClockError {
    fn mount_failure(&self) -> MountFailure {
        MountFailure::Clock(self.kind())
    }
}

impl MountCause for RegistryError {
    fn mount_failure(&self) -> MountFailure {
        MountFailure::Registry(self.kind())
    }
}

impl MountCause for SnapshotError {
    fn mount_failure(&self) -> MountFailure {
        MountFailure::Snapshot(self.kind())
    }
}

impl MountCause for CanonicalError {
    fn mount_failure(&self) -> MountFailure {
        MountFailure::Canonical(self.kind())
    }
}

impl MountCause for CompositionError {
    fn mount_failure(&self) -> MountFailure {
        MountFailure::Composition(self.kind())
    }
}

impl MountCause for RandomError {
    fn mount_failure(&self) -> MountFailure {
        MountFailure::Random(self.kind())
    }
}

impl MountCause for LifecycleError {
    fn mount_failure(&self) -> MountFailure {
        MountFailure::Lifecycle(self.kind())
    }
}

impl MountCause for ViewError {
    fn mount_failure(&self) -> MountFailure {
        MountFailure::View(self.kind())
    }
}

impl MountCause for LedgerError {
    fn mount_failure(&self) -> MountFailure {
        MountFailure::Ledger(self.kind())
    }
}
