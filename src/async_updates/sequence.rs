//! Independent, scope-bound stream sequence continuity authority.

use std::error::Error;
use std::fmt;

use super::{AsyncEnvelope, AsyncEnvelopeContext, StreamName, StreamPosition, SubscriptionId};

/// Maximum number of validated envelopes accepted as one replay transcript.
pub const MAX_REPLAY_TRANSCRIPT_ENVELOPES: usize = 1_024;

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
    /// A newer epoch arrived without authoritative refresh.
    EpochChanged,
    /// The current sequence has no representable successor.
    SequenceOverflow,
}

/// Pure result of observing one membership- and registry-validated envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceDisposition {
    /// Apply this exact next sequence and advance current position.
    Apply,
    /// Ignore an already applied sequence in the current epoch.
    IgnoreDuplicate,
    /// Ignore delivery from an epoch older than the current baseline.
    IgnoreStaleEpoch,
    /// Reject an envelope for another logical subscription or stream.
    ScopeMismatch,
    /// Do not apply; explicit recovery authority is now required.
    Degraded(SequenceDegradation),
    /// Do not apply while the machine is already awaiting recovery authority.
    AwaitingRecovery,
}

/// Result of applying validated continuity authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BaselineDisposition {
    /// Authority installed a non-regressing baseline and restored current state.
    Adopted,
    /// Authority named the already-current baseline and changed no position.
    AlreadyCurrent,
}

/// Why continuity recovery was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceErrorKind {
    /// Replay was empty, incomplete, non-contiguous, duplicated, or cross-epoch.
    InvalidReplayTranscript,
    /// Replay named another logical subscription or registered stream.
    ScopeMismatch,
    /// A host baseline attempted to regress known authority.
    BaselineRegression,
    /// A host baseline did not cover the observed high-water position.
    AuthoritativeBaselineInsufficient,
    /// The trusted host continuity adapter could not establish a baseline.
    AuthoritativeRefreshUnavailable,
}

impl SequenceErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidReplayTranscript => "invalid_async_replay_transcript",
            Self::ScopeMismatch => "async_sequence_scope_mismatch",
            Self::BaselineRegression => "async_baseline_regression",
            Self::AuthoritativeBaselineInsufficient => "async_baseline_insufficient",
            Self::AuthoritativeRefreshUnavailable => "async_authoritative_refresh_unavailable",
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

/// Exact scope and continuity facts supplied only to the trusted host adapter.
#[derive(Clone, Copy)]
pub struct AsyncContinuityRequest<'a> {
    subscription: &'a SubscriptionId,
    stream: &'a StreamName,
    current: StreamPosition,
    high_water: Option<StreamPosition>,
}

impl<'a> AsyncContinuityRequest<'a> {
    /// Returns the exact logical subscription identity.
    #[must_use]
    pub const fn subscription(self) -> &'a SubscriptionId {
        self.subscription
    }

    /// Returns the exact registered stream identity.
    #[must_use]
    pub const fn stream(self) -> &'a StreamName {
        self.stream
    }

    /// Returns the last applied or authority-adopted position.
    #[must_use]
    pub const fn current(self) -> StreamPosition {
        self.current
    }

    /// Returns the highest validated position observed while degraded.
    #[must_use]
    pub const fn high_water(self) -> Option<StreamPosition> {
        self.high_water
    }
}

impl fmt::Debug for AsyncContinuityRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AsyncContinuityRequest")
            .field("subscription", &"<redacted>")
            .field("stream", &self.stream)
            .field("current", &self.current)
            .field("high_water", &self.high_water)
            .finish()
    }
}

/// Host-owned authority that establishes a fresh continuity baseline.
///
/// This port is injected by the framework host. Browser input never creates an
/// authority value and cannot call sequence adoption with a claimed position.
pub trait AsyncContinuityAuthorityPort: Send + Sync {
    /// Returns a current host-authoritative baseline for the exact request scope.
    fn authoritative_refresh(&self, request: AsyncContinuityRequest<'_>) -> Option<StreamPosition>;
}

/// Per-logical-subscription sequence authority independent of transport choice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SequenceMachine {
    subscription: SubscriptionId,
    stream: StreamName,
    current: StreamPosition,
    state: SequenceState,
    degradation: Option<SequenceDegradation>,
    high_water: Option<StreamPosition>,
}

impl SequenceMachine {
    /// Starts from one sealed membership context and authoritative descriptor baseline.
    #[must_use]
    pub fn new(context: &AsyncEnvelopeContext, authoritative_baseline: StreamPosition) -> Self {
        Self {
            subscription: context.subscription().clone(),
            stream: context.stream().clone(),
            current: authoritative_baseline,
            state: SequenceState::Current,
            degradation: None,
            high_water: None,
        }
    }

    /// Returns the last authority-backed applied or adopted position.
    #[must_use]
    pub const fn current(&self) -> StreamPosition {
        self.current
    }

    /// Returns whether exact-next delivery may currently apply.
    #[must_use]
    pub const fn state(&self) -> SequenceState {
        self.state
    }

    /// Returns the highest validated position observed while degraded.
    #[must_use]
    pub const fn high_water(&self) -> Option<StreamPosition> {
        self.high_water
    }

    /// Observes one already membership- and registry-validated envelope.
    ///
    /// Scope is checked before the position is read. A gap never mutates
    /// `current`, and receipt alone cannot restore currentness.
    pub fn observe(&mut self, envelope: &AsyncEnvelope) -> SequenceDisposition {
        if !self.matches_scope(envelope) {
            return SequenceDisposition::ScopeMismatch;
        }
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
            self.record_high_water(observed);
            if observed.epoch() > self.current.epoch() {
                self.degradation = Some(SequenceDegradation::EpochChanged);
            }
            return SequenceDisposition::AwaitingRecovery;
        }
        if observed.epoch() > self.current.epoch() {
            self.degrade(SequenceDegradation::EpochChanged, observed);
            return SequenceDisposition::Degraded(SequenceDegradation::EpochChanged);
        }
        let Some(expected) = self.current.sequence().get().checked_add(1) else {
            self.degrade(SequenceDegradation::SequenceOverflow, observed);
            return SequenceDisposition::Degraded(SequenceDegradation::SequenceOverflow);
        };
        if observed.sequence().get() != expected {
            self.degrade(SequenceDegradation::Gap, observed);
            return SequenceDisposition::Degraded(SequenceDegradation::Gap);
        }
        self.current = observed;
        SequenceDisposition::Apply
    }

    /// Restores currentness only from a complete contiguous validated transcript.
    pub fn recover_from_replay(
        &mut self,
        transcript: &[AsyncEnvelope],
    ) -> Result<BaselineDisposition, SequenceError> {
        if self.state != SequenceState::Degraded
            || self.degradation != Some(SequenceDegradation::Gap)
            || transcript.is_empty()
            || transcript.len() > MAX_REPLAY_TRANSCRIPT_ENVELOPES
        {
            return Err(SequenceError::new(
                SequenceErrorKind::InvalidReplayTranscript,
            ));
        }
        let high_water = self
            .high_water
            .ok_or_else(|| SequenceError::new(SequenceErrorKind::InvalidReplayTranscript))?;
        if high_water.epoch() != self.current.epoch() {
            return Err(SequenceError::new(
                SequenceErrorKind::InvalidReplayTranscript,
            ));
        }

        let mut expected = self
            .current
            .sequence()
            .get()
            .checked_add(1)
            .ok_or_else(|| SequenceError::new(SequenceErrorKind::InvalidReplayTranscript))?;
        let mut through = self.current;
        for (index, envelope) in transcript.iter().enumerate() {
            if !self.matches_scope(envelope) {
                return Err(SequenceError::new(SequenceErrorKind::ScopeMismatch));
            }
            let position = envelope.position();
            if position.epoch() != self.current.epoch() || position.sequence().get() != expected {
                return Err(SequenceError::new(
                    SequenceErrorKind::InvalidReplayTranscript,
                ));
            }
            through = position;
            if index + 1 < transcript.len() {
                expected = expected.checked_add(1).ok_or_else(|| {
                    SequenceError::new(SequenceErrorKind::InvalidReplayTranscript)
                })?;
            }
        }
        if through.sequence() < high_water.sequence() {
            return Err(SequenceError::new(
                SequenceErrorKind::InvalidReplayTranscript,
            ));
        }
        self.restore(through);
        Ok(BaselineDisposition::Adopted)
    }

    /// Requests and installs a baseline only through trusted host continuity authority.
    pub fn recover_from_authoritative_refresh(
        &mut self,
        authority: &dyn AsyncContinuityAuthorityPort,
    ) -> Result<BaselineDisposition, SequenceError> {
        let baseline = authority
            .authoritative_refresh(AsyncContinuityRequest {
                subscription: &self.subscription,
                stream: &self.stream,
                current: self.current,
                high_water: self.high_water,
            })
            .ok_or_else(|| {
                SequenceError::new(SequenceErrorKind::AuthoritativeRefreshUnavailable)
            })?;
        if position_precedes(baseline, self.current) {
            return Err(SequenceError::new(SequenceErrorKind::BaselineRegression));
        }
        if self
            .high_water
            .is_some_and(|high_water| position_precedes(baseline, high_water))
        {
            return Err(SequenceError::new(
                SequenceErrorKind::AuthoritativeBaselineInsufficient,
            ));
        }
        let disposition = if baseline == self.current {
            BaselineDisposition::AlreadyCurrent
        } else {
            BaselineDisposition::Adopted
        };
        self.restore(baseline);
        Ok(disposition)
    }

    fn matches_scope(&self, envelope: &AsyncEnvelope) -> bool {
        envelope.subscription() == &self.subscription && envelope.stream() == &self.stream
    }

    fn degrade(&mut self, reason: SequenceDegradation, observed: StreamPosition) {
        self.state = SequenceState::Degraded;
        self.degradation = Some(reason);
        self.high_water = Some(observed);
    }

    fn record_high_water(&mut self, observed: StreamPosition) {
        if self
            .high_water
            .is_none_or(|current| position_precedes(current, observed))
        {
            self.high_water = Some(observed);
        }
    }

    fn restore(&mut self, baseline: StreamPosition) {
        self.current = baseline;
        self.state = SequenceState::Current;
        self.degradation = None;
        self.high_water = None;
    }
}

fn position_precedes(left: StreamPosition, right: StreamPosition) -> bool {
    left.epoch() < right.epoch()
        || (left.epoch() == right.epoch() && left.sequence() < right.sequence())
}
