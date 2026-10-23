//! Application-facing event emitted after one Live outcome is accepted.

pub use suprnova_live::ledger::AcceptedOutcomeKind;

/// Post-acceptance framework event suitable for listeners, queues, and broadcasts.
///
/// The event is observability, not durable delivery authority. Effects that must
/// survive process failure still belong to the action's transaction-owned outbox.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveOutcomeAccepted {
    revision: u64,
    outcome: AcceptedOutcomeKind,
}

impl LiveOutcomeAccepted {
    pub(crate) const fn new(revision: u64, outcome: AcceptedOutcomeKind) -> Self {
        Self { revision, outcome }
    }

    /// Returns the committed successor revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns the engine-owned bounded accepted-outcome category.
    #[must_use]
    pub const fn outcome(&self) -> AcceptedOutcomeKind {
        self.outcome
    }
}

impl crate::events::Event for LiveOutcomeAccepted {
    fn event_name() -> &'static str {
        "suprnova.live.outcome.accepted"
    }
}
