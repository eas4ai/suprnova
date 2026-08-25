//! Independent stream sequence continuity authority.

use std::error::Error;
use std::fmt;

use super::{AsyncEnvelope, StreamPosition};

/// Whether the current logical subscription may apply ordered payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceState {
    /// The baseline is authoritative and the next exact sequence may apply.
    Current,
    /// A gap, overflow, or new epoch requires explicit continuity authority.
    Degraded,
}

/// Why observation stopped application and entered degraded state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceDegradation {
    /// A same-epoch observation skipped at least one required sequence.
    Gap,
    /// A newer epoch arrived without replay proof or authoritative refresh.
    EpochChanged,
    /// The current sequence has no representable successor.
    SequenceOverflow,
}

/// Pure result of observing one membership-validated envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceDisposition {
    /// Apply this exact next sequence and advance current position.
    Apply,
    /// Ignore an already applied sequence in the current epoch.
    IgnoreDuplicate,
    /// Ignore delivery from an epoch older than the current baseline.
    IgnoreStaleEpoch,
    /// Do not apply; explicit recovery authority is now required.
    Degraded(SequenceDegradation),
    /// Do not apply while the machine is already awaiting recovery authority.
    AwaitingRecovery,
}

/// Explicit trusted evidence permitted to install a new sequence baseline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContinuityProof {
    /// Trusted replay covered every position after `from` through `through`.
    Replay {
        /// Baseline from which replay began.
        from: StreamPosition,
        /// Last position whose continuous replay was proved.
        through: StreamPosition,
    },
    /// An ordinary authoritative island refresh established this baseline.
    AuthoritativeRefresh {
        /// Fresh server position paired with the accepted refresh.
        baseline: StreamPosition,
    },
}

/// Result of applying explicit continuity authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BaselineDisposition {
    /// The proof installed a non-regressing baseline and restored current state.
    Adopted,
    /// The proof named the already-current baseline and changed no authority.
    AlreadyCurrent,
}

/// Why explicit continuity evidence was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceErrorKind {
    /// Replay did not begin at current or remain ordered within that epoch.
    InvalidReplayProof,
    /// An authoritative refresh attempted to regress the known baseline.
    BaselineRegression,
}

impl SequenceErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidReplayProof => "invalid_async_replay_proof",
            Self::BaselineRegression => "async_baseline_regression",
        }
    }
}

/// Redacted sequence-authority rejection.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct SequenceError {
    kind: SequenceErrorKind,
}

impl SequenceError {
    const fn new(kind: SequenceErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed safe rejection reason.
    #[must_use]
    pub const fn kind(self) -> SequenceErrorKind {
        self.kind
    }
}

impl fmt::Display for SequenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for SequenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for SequenceError {}

/// Per-logical-subscription sequence authority independent of transport choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SequenceMachine {
    current: StreamPosition,
    state: SequenceState,
}

impl SequenceMachine {
    /// Starts from the server-authoritative descriptor baseline.
    #[must_use]
    pub const fn new(authoritative_baseline: StreamPosition) -> Self {
        Self {
            current: authoritative_baseline,
            state: SequenceState::Current,
        }
    }

    /// Returns the last authority-backed applied or adopted position.
    #[must_use]
    pub const fn current(self) -> StreamPosition {
        self.current
    }

    /// Returns whether exact-next delivery may currently apply.
    #[must_use]
    pub const fn state(self) -> SequenceState {
        self.state
    }

    /// Observes one already membership- and registry-validated envelope.
    ///
    /// A gap never mutates `current`. Once degraded, receipt alone cannot restore
    /// currentness; [`Self::adopt`] requires replay proof or authoritative refresh.
    pub fn observe(&mut self, envelope: &AsyncEnvelope) -> SequenceDisposition {
        let observed = envelope.position();
        if observed.epoch() < self.current.epoch() {
            return SequenceDisposition::IgnoreStaleEpoch;
        }
        if observed.epoch() == self.current.epoch()
            && observed.sequence() <= self.current.sequence()
        {
            return SequenceDisposition::IgnoreDuplicate;
        }
        if self.state == SequenceState::Degraded {
            return SequenceDisposition::AwaitingRecovery;
        }
        if observed.epoch() > self.current.epoch() {
            self.state = SequenceState::Degraded;
            return SequenceDisposition::Degraded(SequenceDegradation::EpochChanged);
        }
        let Some(expected) = self.current.sequence().get().checked_add(1) else {
            self.state = SequenceState::Degraded;
            return SequenceDisposition::Degraded(SequenceDegradation::SequenceOverflow);
        };
        if observed.sequence().get() != expected {
            self.state = SequenceState::Degraded;
            return SequenceDisposition::Degraded(SequenceDegradation::Gap);
        }
        self.current = observed;
        SequenceDisposition::Apply
    }

    /// Applies explicit trusted continuity evidence without ever regressing authority.
    pub fn adopt(&mut self, proof: ContinuityProof) -> Result<BaselineDisposition, SequenceError> {
        let baseline = match proof {
            ContinuityProof::Replay { from, through } => {
                if from != self.current
                    || through.epoch() != from.epoch()
                    || through.sequence() < from.sequence()
                {
                    return Err(SequenceError::new(SequenceErrorKind::InvalidReplayProof));
                }
                through
            }
            ContinuityProof::AuthoritativeRefresh { baseline } => {
                if baseline.epoch() < self.current.epoch()
                    || (baseline.epoch() == self.current.epoch()
                        && baseline.sequence() < self.current.sequence())
                {
                    return Err(SequenceError::new(SequenceErrorKind::BaselineRegression));
                }
                baseline
            }
        };
        let disposition = if baseline == self.current {
            BaselineDisposition::AlreadyCurrent
        } else {
            self.current = baseline;
            BaselineDisposition::Adopted
        };
        self.state = SequenceState::Current;
        Ok(disposition)
    }
}
