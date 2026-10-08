//! Closed causes of operations the execution coordinator ended with a refresh.

use crate::child::ChildParameterErrorKind;
use crate::clock::ClockErrorKind;
use crate::component::{ActionExecutionErrorKind, LifecycleErrorKind};
use crate::identity::IdentityErrorKind;
use crate::ledger::LedgerErrorKind;
use crate::limits::SizeBreach;
use crate::snapshot::SnapshotErrorKind;
use crate::view::{ViewError, ViewErrorKind};

use super::HostErrorKind;

/// Closed cause of an operation that the engine ended with a refresh.
///
/// The browser sees at most the coarse
/// [`ExecutionRefreshReason`](super::ExecutionRefreshReason). The cause stays with the host, so
/// an operator can tell which trusted subsystem failed without reproducing the request. Every
/// variant carries closed kinds only and never text from a request, a component, or a provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ExecutionFailure {
    /// The component executor failed the registered action request.
    Action(ActionExecutionErrorKind),
    /// The component executor failed a recovery render, a model synchronization, a parameter
    /// change, a lazy completion, or a promotion mount.
    Lifecycle(LifecycleErrorKind),
    /// The instance ledger returned an error while it claimed the base revision or recorded
    /// the accepted outcome.
    Ledger(LedgerErrorKind),
    /// The instance ledger granted a successor revision other than the one the recovery render
    /// was built for.
    LedgerSuccessorMismatch,
    /// Identity construction failed: the base revision had no successor, or a child parameter
    /// digest did not have the digest length.
    Identity(IdentityErrorKind),
    /// The clock could not supply the signing time, or a deadline derived from its reading
    /// overflowed.
    Clock(ClockErrorKind),
    /// The trusted host request context expired before the successor snapshot was signed.
    ContextExpired,
    /// Composition lineage: the operation names an owner parent revision and the signed lineage
    /// has no owner.
    CompositionOwnerMissing,
    /// Composition lineage: a rendered child mount carries no transition while the signed
    /// lineage has children.
    CompositionChildUntracked,
    /// Composition lineage: a rendered child matches no signed lineage entry by key, contract,
    /// and instance.
    CompositionChildUnknown,
    /// Composition lineage: the snapshot subsystem refused an owner, child, or whole lineage.
    CompositionLineage(SnapshotErrorKind),
    /// The snapshot subsystem refused the successor state, memo, extensions, or signature.
    Snapshot(SnapshotErrorKind),
    /// The view subsystem refused the rendered fragment or the assembled island root.
    View(ViewErrorKind),
    /// The rendered island HTML was over the configured byte limit; the sizes
    /// let the host name the limit and the setting to raise.
    ViewTooLarge(SizeBreach),
    /// Outcome validation found an incomplete outcome: an empty signed snapshot, a render
    /// outcome without HTML, or HTML for an outcome that renders nothing.
    OutcomeShape,
    /// The child parameter subsystem could not seal a changed child delivery.
    ChildParameters(ChildParameterErrorKind),
    /// A host port returned an error or panicked.
    Host(HostErrorKind),
    /// Host response-intent preparation: the result needs intents and the host supplied no
    /// preparation port, or the prepared intents do not fit the result and protocol.
    ResponseIntents,
    /// Accepted-response sealing: the host supplied no request-bound sealer and binding, or a
    /// sealer for another protocol version, or the sealer refused the complete response.
    ResponseSealing,
}

impl ExecutionFailure {
    /// The cause a view error becomes: a body over its limit keeps its sizes.
    pub(crate) fn from_view(error: &ViewError) -> Self {
        match error.size() {
            Some(size) => Self::ViewTooLarge(size),
            None => Self::View(error.kind()),
        }
    }
}
