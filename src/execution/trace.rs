//! Closed execution phases exposed to host-neutral tracing.

use std::panic::{AssertUnwindSafe, catch_unwind};

/// One ordered boundary in an accepted Live operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionPhase {
    /// Expected-revision authority is claimed.
    Claim,
    /// A promoted seed performs its trusted fresh mount initialization.
    PromotionMount,
    /// Verified component state is reconstructed.
    Hydrate,
    /// Registered model proposals are applied.
    Bind,
    /// Current action authorization is checked.
    Authorize,
    /// Registered validation is evaluated.
    Validate,
    /// An explicitly required host transaction begins.
    TransactionBegin,
    /// Component before-action behavior runs.
    BeforeAction,
    /// The registered action body runs.
    Action,
    /// Component after-action behavior runs.
    AfterAction,
    /// Successor island HTML is rendered when required.
    Render,
    /// Complete successor state and memo are dehydrated.
    Dehydrate,
    /// The successor instanced snapshot is signed.
    Sign,
    /// The complete semantic response is validated.
    OutcomeValidation,
    /// The owning host transaction commits.
    HostCommit,
    /// Bounded accepted metadata is committed to the instance ledger.
    LedgerAcceptance,
    /// Non-outcome-changing post-acceptance reporting runs.
    Reporting,
}

/// Infallible, non-authoritative observer for exact execution order.
pub trait ExecutionTracePort: Send + Sync {
    /// Observes an attempted phase. Implementations must not perform domain work.
    fn record(&self, phase: ExecutionPhase);
}

/// Trace implementation used when no observer is installed.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopExecutionTrace;

impl ExecutionTracePort for NoopExecutionTrace {
    fn record(&self, _phase: ExecutionPhase) {}
}

pub(crate) fn record(trace: &dyn ExecutionTracePort, phase: ExecutionPhase) {
    let _ = catch_unwind(AssertUnwindSafe(|| trace.record(phase)));
}
