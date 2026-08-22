//! Typed, redacted observation of one harness run.

use std::sync::{Arc, Mutex, MutexGuard};

use suprnova_live::execution::{ExecutionPhase, ExecutionTracePort};

/// One closed observation emitted by a controlled harness dependency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HarnessTraceEvent {
    /// The test advanced its controlled wall clock.
    ClockAdvanced,
    /// The deterministic instance generator issued an identity.
    InstanceGenerated,
    /// Current action authorization was consulted.
    Authorization,
    /// Application validation was consulted.
    Validation,
    /// A host transaction was requested.
    TransactionBegin,
    /// A host transaction committed.
    TransactionCommit,
    /// A host transaction rolled back.
    TransactionRollback,
    /// A registered session field was read.
    SessionRead,
    /// A registered session intent was applied.
    SessionApply,
    /// The production execution coordinator entered one typed phase.
    Execution(ExecutionPhase),
}

/// Cloneable deterministic trace shared by every harness service.
#[derive(Clone, Default)]
pub struct HarnessTrace {
    events: Arc<Mutex<Vec<HarnessTraceEvent>>>,
}

impl HarnessTrace {
    /// Records one closed event without accepting an arbitrary string or payload.
    pub fn record(&self, event: HarnessTraceEvent) {
        self.lock().push(event);
    }

    /// Returns a stable snapshot of events in observation order.
    #[must_use]
    pub fn events(&self) -> Vec<HarnessTraceEvent> {
        self.lock().clone()
    }

    /// Clears prior observations while retaining the same shared trace.
    pub fn clear(&self) {
        self.lock().clear();
    }

    fn lock(&self) -> MutexGuard<'_, Vec<HarnessTraceEvent>> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl std::fmt::Debug for HarnessTrace {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HarnessTrace")
            .field("event_count", &self.lock().len())
            .finish()
    }
}

impl ExecutionTracePort for HarnessTrace {
    fn record(&self, phase: ExecutionPhase) {
        self.record(HarnessTraceEvent::Execution(phase));
    }
}
