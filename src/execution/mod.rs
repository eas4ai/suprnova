//! Host-neutral coordination of Live actions, transactions, and accepted outcomes.

mod recovery;
mod service;
mod trace;
mod transaction;

pub use recovery::RetryLegality;
pub use service::{
    AcceptedExecution, AcceptedExecutionReport, AcceptedOutcomeReporter, ActionExecutionRequest,
    ExecutionRefreshReason, ExecutionResult, ExecutionService, InstancedActionRequest,
    InstancedFreshRenderRequest, PromotedActionRequest, RefreshRequiredExecution,
};
pub(crate) use trace::record;
pub use trace::{ExecutionPhase, ExecutionTracePort, NoopExecutionTrace};
pub(crate) use transaction::run_host_future;
pub use transaction::{HostError, HostErrorKind, HostTransaction, TransactionPort};
