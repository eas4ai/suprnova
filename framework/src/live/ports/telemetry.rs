//! Bounded execution-phase telemetry without outcome authority.

use suprnova_live::execution::{ExecutionPhase, ExecutionTracePort};

pub(crate) struct SuprnovaExecutionTrace;

impl ExecutionTracePort for SuprnovaExecutionTrace {
    fn record(&self, phase: ExecutionPhase) {
        tracing::trace!(phase = ?phase, "Live execution phase");
    }
}
