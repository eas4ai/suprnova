use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroUsize;

use super::{
    UploadChecksum, UploadError, UploadErrorKind, UploadHandle, UploadIdempotencyKey,
    UploadRevision,
};

const MAX_RETAINED_OUTCOMES: usize = 64;
const MAX_CONFIGURED_OUTCOMES: usize = 100_000;

/// Authoritative lifecycle state of one temporary upload.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum UploadState {
    /// Identity exists but has not entered transfer admission.
    Created,
    /// Transfer is admitted and waiting for resources.
    Queued,
    /// One or more chunks may be accepted.
    Transferring,
    /// Byte transfer completed and authoritative validation is running.
    Verifying,
    /// Validation accepted the temporary upload for explicit finalization.
    Ready,
    /// An application action began durable finalization.
    Finalizing,
    /// Durable finalization committed.
    Finalized,
    /// Authoritative validation rejected the temporary upload.
    Rejected,
    /// The pending upload was explicitly canceled.
    Canceled,
    /// Retention policy expired the temporary upload.
    Expired,
    /// Provider or host failure closed the upload.
    Failed,
}

impl UploadState {
    /// Every state in stable fixture order.
    pub const ALL: [Self; 11] = [
        Self::Created,
        Self::Queued,
        Self::Transferring,
        Self::Verifying,
        Self::Ready,
        Self::Finalizing,
        Self::Finalized,
        Self::Rejected,
        Self::Canceled,
        Self::Expired,
        Self::Failed,
    ];

    /// Parses the stable state name.
    pub fn parse(value: &str) -> Result<Self, UploadError> {
        match value {
            "created" => Ok(Self::Created),
            "queued" => Ok(Self::Queued),
            "transferring" => Ok(Self::Transferring),
            "verifying" => Ok(Self::Verifying),
            "ready" => Ok(Self::Ready),
            "finalizing" => Ok(Self::Finalizing),
            "finalized" => Ok(Self::Finalized),
            "rejected" => Ok(Self::Rejected),
            "canceled" => Ok(Self::Canceled),
            "expired" => Ok(Self::Expired),
            "failed" => Ok(Self::Failed),
            _ => Err(UploadError::new(UploadErrorKind::InvalidField)),
        }
    }

    /// Returns the stable state name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Queued => "queued",
            Self::Transferring => "transferring",
            Self::Verifying => "verifying",
            Self::Ready => "ready",
            Self::Finalizing => "finalizing",
            Self::Finalized => "finalized",
            Self::Rejected => "rejected",
            Self::Canceled => "canceled",
            Self::Expired => "expired",
            Self::Failed => "failed",
        }
    }

    /// Returns whether no new state transition may be accepted.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Finalized | Self::Rejected | Self::Canceled | Self::Expired | Self::Failed
        )
    }

    /// Returns a monotonic lifecycle rank used by invariant tests and telemetry.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::Created => 0,
            Self::Queued => 1,
            Self::Transferring => 2,
            Self::Verifying => 3,
            Self::Ready => 4,
            Self::Finalizing => 5,
            Self::Finalized | Self::Rejected | Self::Canceled | Self::Expired | Self::Failed => 6,
        }
    }
}

/// Authoritatively accepted chunk metadata retained by the upload record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedChunk {
    index: u32,
    size: u64,
    checksum: UploadChecksum,
}

impl AcceptedChunk {
    /// Constructs metadata already admitted by protocol and provider bounds.
    pub fn new(index: u32, size: u64, checksum: UploadChecksum) -> Result<Self, UploadError> {
        if size == 0 {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        Ok(Self {
            index,
            size,
            checksum,
        })
    }

    /// Returns the zero-based chunk index.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// Returns the accepted byte count.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Returns the accepted checksum.
    #[must_use]
    pub const fn checksum(&self) -> &UploadChecksum {
        &self.checksum
    }
}

/// Closed internal ledger transition vocabulary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UploadTransition {
    /// Admits a created upload into the resource queue.
    Queue,
    /// Starts the first transfer work.
    BeginTransfer,
    /// Records one accepted chunk while remaining in transfer.
    PutChunk(AcceptedChunk),
    /// Ends byte transfer and begins verification.
    Complete,
    /// Accepts authoritative validation.
    Accept,
    /// Begins explicit application finalization.
    BeginFinalize,
    /// Commits durable finalization.
    CommitFinalize,
    /// Cancels pending work.
    Cancel,
    /// Rejects authoritative validation.
    Reject,
    /// Expires temporary state.
    Expire,
    /// Closes pending work after a provider or host failure.
    Fail,
}

/// One conditional idempotent request to mutate an upload record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadTransitionRequest {
    handle: UploadHandle,
    expected_revision: UploadRevision,
    idempotency_key: UploadIdempotencyKey,
    transition: UploadTransition,
}

impl UploadTransitionRequest {
    /// Creates a fully bound conditional transition request.
    #[must_use]
    pub const fn new(
        handle: UploadHandle,
        expected_revision: UploadRevision,
        idempotency_key: UploadIdempotencyKey,
        transition: UploadTransition,
    ) -> Self {
        Self {
            handle,
            expected_revision,
            idempotency_key,
            transition,
        }
    }

    /// Returns the target temporary upload.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        &self.handle
    }

    /// Returns the required current revision.
    #[must_use]
    pub const fn expected_revision(&self) -> UploadRevision {
        self.expected_revision
    }

    /// Returns the bounded retry identity.
    #[must_use]
    pub const fn idempotency_key(&self) -> &UploadIdempotencyKey {
        &self.idempotency_key
    }

    /// Returns the requested closed lifecycle transition.
    #[must_use]
    pub const fn transition(&self) -> &UploadTransition {
        &self.transition
    }
}

/// Whether a transition newly committed or replayed its exact stored outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionDisposition {
    /// This call committed the state transition.
    Applied,
    /// This call exactly replayed an earlier accepted retry identity.
    ExistingOutcome,
}

/// Safe committed transition summary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransitionOutcome {
    disposition: TransitionDisposition,
    state: UploadState,
    revision: UploadRevision,
}

impl TransitionOutcome {
    /// Returns whether the outcome was new or replayed.
    #[must_use]
    pub const fn disposition(self) -> TransitionDisposition {
        self.disposition
    }

    /// Returns the authoritative successor state.
    #[must_use]
    pub const fn state(self) -> UploadState {
        self.state
    }

    /// Returns the authoritative successor revision.
    #[must_use]
    pub const fn revision(self) -> UploadRevision {
        self.revision
    }
}

#[derive(Clone, Eq, PartialEq)]
struct RecordedTransition {
    handle: UploadHandle,
    expected_revision: UploadRevision,
    transition: UploadTransition,
    outcome: TransitionOutcome,
}

/// Bounded in-memory reference model for conditional upload transitions.
pub struct UploadStateMachine {
    handle: UploadHandle,
    state: UploadState,
    revision: UploadRevision,
    max_outcomes: usize,
    outcomes: BTreeMap<UploadIdempotencyKey, RecordedTransition>,
}

impl UploadStateMachine {
    /// Creates a machine from one authoritative persisted record.
    #[must_use]
    pub const fn new(handle: UploadHandle, state: UploadState, revision: UploadRevision) -> Self {
        Self {
            handle,
            state,
            revision,
            max_outcomes: MAX_RETAINED_OUTCOMES,
            outcomes: BTreeMap::new(),
        }
    }

    /// Creates a machine with a host-validated retained-outcome bound.
    pub fn with_outcome_limit(
        handle: UploadHandle,
        state: UploadState,
        revision: UploadRevision,
        max_outcomes: NonZeroUsize,
    ) -> Result<Self, UploadError> {
        if max_outcomes.get() > MAX_CONFIGURED_OUTCOMES {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        Ok(Self {
            handle,
            state,
            revision,
            max_outcomes: max_outcomes.get(),
            outcomes: BTreeMap::new(),
        })
    }

    /// Returns the current authoritative state.
    #[must_use]
    pub const fn state(&self) -> UploadState {
        self.state
    }

    /// Returns the current authoritative revision.
    #[must_use]
    pub const fn revision(&self) -> UploadRevision {
        self.revision
    }

    /// Applies one exact conditional transition or replays its accepted outcome.
    pub fn apply(
        &mut self,
        request: UploadTransitionRequest,
    ) -> Result<TransitionOutcome, UploadError> {
        if request.handle != self.handle {
            return Err(UploadError::new(UploadErrorKind::ScopeMismatch));
        }
        if let Some(recorded) = self.outcomes.get(&request.idempotency_key) {
            if recorded.handle == request.handle
                && recorded.expected_revision == request.expected_revision
                && recorded.transition == request.transition
            {
                return Ok(TransitionOutcome {
                    disposition: TransitionDisposition::ExistingOutcome,
                    ..recorded.outcome
                });
            }
            return Err(UploadError::new(UploadErrorKind::UploadConflict));
        }
        if request.expected_revision != self.revision {
            return Err(UploadError::new(UploadErrorKind::UploadConflict));
        }
        if self.outcomes.len() == self.max_outcomes {
            return Err(UploadError::new(UploadErrorKind::IdempotencyHistoryFull));
        }

        let next_state = next_state(self.state, &request.transition)?;
        let next_revision = self.revision.checked_next()?;
        let outcome = TransitionOutcome {
            disposition: TransitionDisposition::Applied,
            state: next_state,
            revision: next_revision,
        };
        self.outcomes.insert(
            request.idempotency_key,
            RecordedTransition {
                handle: request.handle,
                expected_revision: request.expected_revision,
                transition: request.transition,
                outcome,
            },
        );
        self.state = next_state;
        self.revision = next_revision;
        Ok(outcome)
    }
}

impl fmt::Debug for UploadStateMachine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UploadStateMachine")
            .field("state", &self.state)
            .field("revision", &self.revision)
            .field("retained_outcomes", &self.outcomes.len())
            .finish()
    }
}

fn next_state(
    state: UploadState,
    transition: &UploadTransition,
) -> Result<UploadState, UploadError> {
    if state.is_terminal() {
        return Err(UploadError::new(UploadErrorKind::InvalidTransition));
    }
    match (state, transition) {
        (UploadState::Created, UploadTransition::Queue) => Ok(UploadState::Queued),
        (UploadState::Queued, UploadTransition::BeginTransfer) => Ok(UploadState::Transferring),
        (UploadState::Transferring, UploadTransition::PutChunk(_)) => Ok(UploadState::Transferring),
        (UploadState::Transferring, UploadTransition::Complete) => Ok(UploadState::Verifying),
        (UploadState::Verifying, UploadTransition::Accept) => Ok(UploadState::Ready),
        (UploadState::Ready, UploadTransition::BeginFinalize) => Ok(UploadState::Finalizing),
        (UploadState::Finalizing, UploadTransition::CommitFinalize) => Ok(UploadState::Finalized),
        (
            UploadState::Created
            | UploadState::Queued
            | UploadState::Transferring
            | UploadState::Verifying
            | UploadState::Ready,
            UploadTransition::Cancel,
        ) => Ok(UploadState::Canceled),
        (UploadState::Verifying, UploadTransition::Reject) => Ok(UploadState::Rejected),
        (
            UploadState::Created
            | UploadState::Queued
            | UploadState::Transferring
            | UploadState::Verifying
            | UploadState::Ready,
            UploadTransition::Expire,
        ) => Ok(UploadState::Expired),
        (_, UploadTransition::Fail) => Ok(UploadState::Failed),
        _ => Err(UploadError::new(UploadErrorKind::InvalidTransition)),
    }
}
