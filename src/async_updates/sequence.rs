//! Independent, scope-bound stream sequence continuity authority.

use std::error::Error;
use std::fmt;

use crate::identity::{ContentDigest, UnixMillis};

use super::{
    ActiveAsyncMembershipGuard, AsyncEnvelope, AsyncEnvelopeContext, ResolvedEventFanout,
    StreamName, StreamPosition, SubscriptionId,
};

/// Maximum number of validated envelopes accepted as one replay transcript.
pub const MAX_REPLAY_TRANSCRIPT_ENVELOPES: usize = 1_024;

/// Whether the current logical subscription may apply ordered payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceState {
    /// The baseline is authoritative and the next exact sequence may apply.
    Current,
    /// A gap or new epoch requires explicit continuity authority.
    Degraded,
}

/// Why observation stopped application and entered degraded state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceDegradation {
    /// A same-epoch observation skipped at least one required sequence.
    Gap,
    /// A newer epoch arrived without authoritative refresh.
    EpochChanged,
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
    /// A once-current membership guard was retained through descriptor expiry.
    MembershipExpired,
    /// A host baseline attempted to regress known authority.
    BaselineRegression,
    /// A host baseline did not cover the observed high-water position.
    AuthoritativeBaselineInsufficient,
    /// The trusted host continuity adapter could not establish a baseline.
    AuthoritativeRefreshUnavailable,
    /// The closed dispatcher rejected an otherwise applicable envelope.
    DispatchRejected,
    /// The closed dispatcher failed before an applicable envelope committed.
    DispatchFailed,
}

impl SequenceErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidReplayTranscript => "invalid_async_replay_transcript",
            Self::ScopeMismatch => "async_sequence_scope_mismatch",
            Self::MembershipExpired => "async_membership_expired",
            Self::BaselineRegression => "async_baseline_regression",
            Self::AuthoritativeBaselineInsufficient => "async_baseline_insufficient",
            Self::AuthoritativeRefreshUnavailable => "async_authoritative_refresh_unavailable",
            Self::DispatchRejected => "async_dispatch_rejected",
            Self::DispatchFailed => "async_dispatch_failed",
        }
    }
}

/// Closed failure returned by the host-owned asynchronous envelope dispatcher.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsyncDispatchErrorKind {
    /// Current dispatch policy rejected the registered envelope.
    Rejected,
    /// Dispatch began no durable application and failed operationally.
    Failed,
}

/// Redacted closed dispatcher failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AsyncDispatchError {
    kind: AsyncDispatchErrorKind,
}

impl AsyncDispatchError {
    /// Creates a closed current-policy rejection.
    #[must_use]
    pub const fn rejected() -> Self {
        Self {
            kind: AsyncDispatchErrorKind::Rejected,
        }
    }

    /// Creates a closed operational dispatch failure.
    #[must_use]
    pub const fn failed() -> Self {
        Self {
            kind: AsyncDispatchErrorKind::Failed,
        }
    }

    /// Returns the stable closed failure kind.
    #[must_use]
    pub const fn kind(self) -> AsyncDispatchErrorKind {
        self.kind
    }
}

impl fmt::Display for AsyncDispatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            AsyncDispatchErrorKind::Rejected => "async_dispatch_rejected",
            AsyncDispatchErrorKind::Failed => "async_dispatch_failed",
        })
    }
}

impl fmt::Debug for AsyncDispatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for AsyncDispatchError {}

/// Non-forgeable proof of one current, fanout-bounded registered delivery.
pub struct ResolvedAsyncDelivery<'a> {
    guard: ActiveAsyncMembershipGuard<'a>,
    resolved_event: Option<&'a ResolvedEventFanout>,
    deployment_fanout_limit: usize,
}

impl<'a> ResolvedAsyncDelivery<'a> {
    pub(crate) const fn new(
        guard: ActiveAsyncMembershipGuard<'a>,
        resolved_event: Option<&'a ResolvedEventFanout>,
        deployment_fanout_limit: usize,
    ) -> Self {
        Self {
            guard,
            resolved_event,
            deployment_fanout_limit,
        }
    }

    /// Returns the exact registered envelope covered by this proof.
    #[must_use]
    pub const fn envelope(&self) -> &AsyncEnvelope {
        self.guard.envelope()
    }

    /// Returns the trusted current recipient count for a browser event.
    #[must_use]
    pub fn resolved_recipients(&self) -> Option<std::num::NonZeroU16> {
        self.resolved_event.map(ResolvedEventFanout::recipients)
    }

    /// Returns the trusted exact target-set digest for a browser event.
    #[must_use]
    pub fn resolved_target_scope(&self) -> Option<&ContentDigest> {
        self.resolved_event.map(ResolvedEventFanout::target_scope)
    }

    /// Returns the document deployment's validated fanout ceiling.
    #[must_use]
    pub const fn deployment_fanout_limit(&self) -> usize {
        self.deployment_fanout_limit
    }

    pub(crate) fn is_current_at(&self, now: UnixMillis) -> bool {
        self.guard.is_current_at(now)
    }

    pub(crate) const fn context(&self) -> &AsyncEnvelopeContext {
        self.guard.context()
    }
}

impl fmt::Debug for ResolvedAsyncDelivery<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<ResolvedAsyncDelivery:redacted>")
    }
}

/// Host-owned closed dispatcher for one fully registered asynchronous envelope.
pub trait AsyncEnvelopeDispatchPort {
    /// Applies the closed, resolved delivery or returns without committing it.
    fn dispatch(&mut self, delivery: ResolvedAsyncDelivery<'_>) -> Result<(), AsyncDispatchError>;
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

pub(crate) struct ReplayRecovery {
    through: StreamPosition,
    total: usize,
    applied: usize,
    restore_on_finish: bool,
}

/// Per-logical-subscription sequence authority independent of transport choice.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct SequenceMachine {
    context: AsyncEnvelopeContext,
    current: StreamPosition,
    state: SequenceState,
    degradation: Option<SequenceDegradation>,
    high_water: Option<StreamPosition>,
}

impl SequenceMachine {
    /// Starts from one sealed membership context and authoritative descriptor baseline.
    #[must_use]
    pub(crate) fn new(context: &AsyncEnvelopeContext) -> Self {
        Self {
            context: context.clone(),
            current: context.authoritative_baseline(),
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
    #[cfg(test)]
    pub const fn high_water(&self) -> Option<StreamPosition> {
        self.high_water
    }

    /// Classifies and dispatches one freshly admitted envelope.
    ///
    /// Scope is checked before the position is read. Exact-next delivery commits
    /// only after the closed dispatcher succeeds. Gaps record continuity state
    /// only after fresh admission and never invoke application dispatch.
    pub(crate) fn dispatch(
        &mut self,
        delivery: ResolvedAsyncDelivery<'_>,
        now: UnixMillis,
        dispatcher: &mut dyn AsyncEnvelopeDispatchPort,
    ) -> Result<SequenceDisposition, SequenceError> {
        if !delivery.is_current_at(now) {
            return Err(SequenceError::new(SequenceErrorKind::MembershipExpired));
        }
        if delivery.context() != &self.context || !self.matches_scope(delivery.envelope()) {
            return Err(SequenceError::new(SequenceErrorKind::ScopeMismatch));
        }
        let envelope = delivery.envelope();
        let observed = envelope.position();
        if observed.epoch() < self.current.epoch() {
            return Ok(SequenceDisposition::IgnoreStaleEpoch);
        }
        if observed.epoch() == self.current.epoch()
            && observed.sequence() <= self.current.sequence()
        {
            return Ok(SequenceDisposition::IgnoreDuplicate);
        }
        if self.state == SequenceState::Degraded {
            self.record_high_water(observed);
            if observed.epoch() > self.current.epoch() {
                self.degradation = Some(SequenceDegradation::EpochChanged);
            }
            return Ok(SequenceDisposition::AwaitingRecovery);
        }
        if observed.epoch() > self.current.epoch() {
            self.degrade(SequenceDegradation::EpochChanged, observed);
            return Ok(SequenceDisposition::Degraded(
                SequenceDegradation::EpochChanged,
            ));
        }
        // When the current sequence is `u64::MAX`, every same-epoch value was
        // classified as duplicate above. Only a newer epoch can advance, and
        // it takes the authoritative-refresh path. The subtraction is
        // therefore unreachable at this point unless a successor exists.
        let expected = self.current.sequence().get() + 1;
        if observed.sequence().get() != expected {
            self.degrade(SequenceDegradation::Gap, observed);
            return Ok(SequenceDisposition::Degraded(SequenceDegradation::Gap));
        }
        dispatcher
            .dispatch(delivery)
            .map_err(|error| SequenceError::new(dispatch_error_kind(error)))?;
        self.current = observed;
        Ok(SequenceDisposition::Apply)
    }

    /// Replays one fully prevalidated transcript and commits only dispatched prefixes.
    #[cfg(test)]
    pub(crate) fn recover_from_replay(
        &mut self,
        transcript: Vec<ResolvedAsyncDelivery<'_>>,
        now: UnixMillis,
        dispatcher: &mut dyn AsyncEnvelopeDispatchPort,
    ) -> Result<ReplayDispatchOutcome, ReplayDispatchError> {
        let envelopes = transcript
            .iter()
            .map(ResolvedAsyncDelivery::envelope)
            .collect::<Vec<_>>();
        let mut recovery = self.prepare_replay(&envelopes, None)?;
        for delivery in transcript {
            self.dispatch_replay_entry(&mut recovery, delivery, now, dispatcher)?;
        }
        self.finish_replay(recovery)
    }

    pub(crate) fn prepare_replay(
        &self,
        transcript: &[&AsyncEnvelope],
        pressure_high_water: Option<StreamPosition>,
    ) -> Result<ReplayRecovery, ReplayDispatchError> {
        if transcript.is_empty() || transcript.len() > MAX_REPLAY_TRANSCRIPT_ENVELOPES {
            return Err(self.replay_error(SequenceErrorKind::InvalidReplayTranscript, 0));
        }
        let (required_high_water, restore_on_finish) = match pressure_high_water {
            Some(required)
                if self.state == SequenceState::Current
                    && required.epoch() == self.current.epoch()
                    && required.sequence() > self.current.sequence() =>
            {
                (required, false)
            }
            None if self.state == SequenceState::Degraded
                && self.degradation == Some(SequenceDegradation::Gap) =>
            {
                let required = self.high_water.ok_or_else(|| {
                    self.replay_error(SequenceErrorKind::InvalidReplayTranscript, 0)
                })?;
                if required.epoch() != self.current.epoch() {
                    return Err(self.replay_error(SequenceErrorKind::InvalidReplayTranscript, 0));
                }
                (required, true)
            }
            _ => {
                return Err(self.replay_error(SequenceErrorKind::InvalidReplayTranscript, 0));
            }
        };

        let mut expected = self
            .current
            .sequence()
            .get()
            .checked_add(1)
            .ok_or_else(|| self.replay_error(SequenceErrorKind::InvalidReplayTranscript, 0))?;
        let mut through = self.current;
        for (index, envelope) in transcript.iter().enumerate() {
            if !self.matches_scope(envelope) {
                return Err(self.replay_error(SequenceErrorKind::ScopeMismatch, 0));
            }
            let position = envelope.position();
            if position.epoch() != self.current.epoch() || position.sequence().get() != expected {
                return Err(self.replay_error(SequenceErrorKind::InvalidReplayTranscript, 0));
            }
            through = position;
            if index + 1 < transcript.len() {
                expected = expected.checked_add(1).ok_or_else(|| {
                    self.replay_error(SequenceErrorKind::InvalidReplayTranscript, 0)
                })?;
            }
        }
        if through.sequence() < required_high_water.sequence() {
            return Err(self.replay_error(SequenceErrorKind::InvalidReplayTranscript, 0));
        }
        Ok(ReplayRecovery {
            through,
            total: transcript.len(),
            applied: 0,
            restore_on_finish,
        })
    }

    pub(crate) fn dispatch_replay_entry(
        &mut self,
        recovery: &mut ReplayRecovery,
        delivery: ResolvedAsyncDelivery<'_>,
        now: UnixMillis,
        dispatcher: &mut dyn AsyncEnvelopeDispatchPort,
    ) -> Result<(), ReplayDispatchError> {
        if !delivery.is_current_at(now) {
            return Err(self.replay_error(SequenceErrorKind::MembershipExpired, recovery.applied));
        }
        if delivery.context() != &self.context || !self.matches_scope(delivery.envelope()) {
            return Err(self.replay_error(SequenceErrorKind::ScopeMismatch, recovery.applied));
        }
        let position = delivery.envelope().position();
        if let Err(error) = dispatcher.dispatch(delivery) {
            return Err(self.replay_error(dispatch_error_kind(error), recovery.applied));
        }
        self.current = position;
        recovery.applied += 1;
        Ok(())
    }

    pub(crate) fn finish_replay(
        &mut self,
        recovery: ReplayRecovery,
    ) -> Result<ReplayDispatchOutcome, ReplayDispatchError> {
        if recovery.applied != recovery.total {
            return Err(
                self.replay_error(SequenceErrorKind::InvalidReplayTranscript, recovery.applied)
            );
        }
        if recovery.restore_on_finish {
            self.restore(recovery.through);
        }
        Ok(ReplayDispatchOutcome {
            applied: recovery.applied,
            current: self.current,
            state: self.state,
        })
    }

    /// Requests and installs a baseline only through trusted host continuity authority.
    #[cfg(test)]
    pub(crate) fn recover_from_authoritative_refresh(
        &mut self,
        authority: &dyn AsyncContinuityAuthorityPort,
    ) -> Result<BaselineDisposition, SequenceError> {
        self.recover_from_authoritative_refresh_covering(authority, None)
    }

    pub(crate) fn recover_from_authoritative_refresh_covering(
        &mut self,
        authority: &dyn AsyncContinuityAuthorityPort,
        pressure_high_water: Option<StreamPosition>,
    ) -> Result<BaselineDisposition, SequenceError> {
        let required_high_water = match (self.high_water, pressure_high_water) {
            (Some(left), Some(right)) if position_precedes(left, right) => Some(right),
            (Some(left), _) => Some(left),
            (None, right) => right,
        };
        let baseline = authority
            .authoritative_refresh(AsyncContinuityRequest {
                subscription: self.context.subscription(),
                stream: self.context.stream(),
                current: self.current,
                high_water: required_high_water,
            })
            .ok_or_else(|| {
                SequenceError::new(SequenceErrorKind::AuthoritativeRefreshUnavailable)
            })?;
        if position_precedes(baseline, self.current) {
            return Err(SequenceError::new(SequenceErrorKind::BaselineRegression));
        }
        if required_high_water.is_some_and(|high_water| position_precedes(baseline, high_water)) {
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
        envelope.subscription() == self.context.subscription()
            && envelope.stream() == self.context.stream()
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

    fn replay_error(&self, kind: SequenceErrorKind, applied: usize) -> ReplayDispatchError {
        ReplayDispatchError {
            kind,
            applied,
            current: self.current,
            state: self.state,
            high_water: self.high_water,
        }
    }
}

/// Truthful outcome after a complete replay dispatch commits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplayDispatchOutcome {
    applied: usize,
    current: StreamPosition,
    state: SequenceState,
}

impl ReplayDispatchOutcome {
    /// Returns the number of envelopes successfully dispatched and committed.
    #[must_use]
    pub const fn applied(self) -> usize {
        self.applied
    }

    /// Returns the resulting authoritative current position.
    #[must_use]
    pub const fn current(self) -> StreamPosition {
        self.current
    }

    /// Returns the resulting sequence state.
    #[must_use]
    pub const fn state(self) -> SequenceState {
        self.state
    }
}

/// Truthful replay failure including the successfully committed prefix.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ReplayDispatchError {
    kind: SequenceErrorKind,
    applied: usize,
    current: StreamPosition,
    state: SequenceState,
    high_water: Option<StreamPosition>,
}

impl ReplayDispatchError {
    /// Returns the closed replay or dispatch failure kind.
    #[must_use]
    pub const fn kind(self) -> SequenceErrorKind {
        self.kind
    }

    /// Returns the successfully dispatched and committed prefix length.
    #[must_use]
    pub const fn applied(self) -> usize {
        self.applied
    }

    /// Returns the last successfully committed position.
    #[must_use]
    pub const fn current(self) -> StreamPosition {
        self.current
    }

    /// Returns the truthful recovery state after failure.
    #[must_use]
    pub const fn state(self) -> SequenceState {
        self.state
    }

    /// Returns the original or expanded observed high-water retained for recovery.
    #[must_use]
    pub const fn high_water(self) -> Option<StreamPosition> {
        self.high_water
    }
}

impl fmt::Display for ReplayDispatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for ReplayDispatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReplayDispatchError")
            .field("kind", &self.kind)
            .field("applied", &self.applied)
            .field("current", &self.current)
            .field("state", &self.state)
            .field("high_water", &self.high_water)
            .finish()
    }
}

impl Error for ReplayDispatchError {}

const fn dispatch_error_kind(error: AsyncDispatchError) -> SequenceErrorKind {
    match error.kind() {
        AsyncDispatchErrorKind::Rejected => SequenceErrorKind::DispatchRejected,
        AsyncDispatchErrorKind::Failed => SequenceErrorKind::DispatchFailed,
    }
}

fn position_precedes(left: StreamPosition, right: StreamPosition) -> bool {
    left.epoch() < right.epoch()
        || (left.epoch() == right.epoch() && left.sequence() < right.sequence())
}
