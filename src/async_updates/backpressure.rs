//! Bounded server-side admission for typed asynchronous delivery.

use std::error::Error;
use std::fmt;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::identity::{BrowserOperationName, ContentDigest};
use crate::resource::{
    Permit, PermitPool, ResourceError, ResourceOwner, ResourceQueue, Retirement, TailAdmission,
    TailAdmissionOutcome,
};

use super::envelope::{OwnedActiveAsyncMembershipGuard, canonical_async_payload_len};
use super::telemetry::AsyncTelemetry;
use super::{
    AsyncCodecLimits, AsyncEnvelope, AsyncEnvelopeDispatchPort, AsyncPayload,
    AsyncTelemetryCounter, AsyncTelemetrySnapshot, AuthorizationMemo, BrowserPayloadSchema,
    DocumentAuthorizationScope, MAX_EVENT_FANOUT, MAX_REPLAY_TRANSCRIPT_ENVELOPES,
    ReplayDispatchError, ReplayDispatchOutcome, ResolvedAsyncDelivery, SequenceDisposition,
    SequenceError, SequenceMachine, StreamEpoch, StreamName, StreamPosition, SubscriptionBinding,
    SubscriptionId, encode_async_envelope,
};

const PRESSURE_CAUSE_KIND_COUNT: usize = 4;
const MAX_TRACKED_PRESSURE_CAUSES: usize =
    super::MAX_DOCUMENT_TRANSPORT_MEMBERSHIPS * PRESSURE_CAUSE_KIND_COUNT;

/// Maximum unapplied entries retained by one server document delivery queue.
pub const MAX_ASYNC_BUFFER_EVENTS: usize = 64;
/// Maximum canonical envelope bytes retained by one server document delivery queue.
pub const MAX_ASYNC_BUFFER_BYTES: usize = 256 * 1024;
/// Independently locked maximum canonical payload bytes in async protocol v1.
pub const MAX_ASYNC_PAYLOAD_BYTES: usize = 32 * 1024;

/// Closed reason that permanently stops one bounded delivery scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsyncCloseCode {
    /// Delivery policy or shared resource bounds exceeded engine ceilings.
    InvalidPolicy,
    /// A registered message could not be represented by the async codec.
    InvalidEnvelope,
    /// The registered payload exceeded the configured delivery boundary.
    PayloadTooLarge,
    /// Trusted fanout exceeded the descriptor-bound policy.
    FanoutExceeded,
    /// The owning document or subscription retired.
    Retired,
}

impl AsyncCloseCode {
    /// Returns the stable low-cardinality machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidPolicy => "invalid_async_policy",
            Self::InvalidEnvelope => "invalid_async_envelope",
            Self::PayloadTooLarge => "async_payload_too_large",
            Self::FanoutExceeded => "async_fanout_exceeded",
            Self::Retired => "async_delivery_retired",
        }
    }
}

/// Result of offering one registered asynchronous message to bounded delivery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BufferDisposition {
    /// The message owns one new queue position.
    Queued,
    /// Exact replaceable work was absorbed by the current queue tail.
    Coalesced,
    /// Continuity is no longer provable and authoritative recovery is required.
    Degraded,
    /// The delivery scope is permanently closed with a safe typed reason.
    Closed(AsyncCloseCode),
}

pub(crate) enum ReplayPreflight {
    Ready,
    Invalid,
    Closed(AsyncCloseCode),
}

/// Descriptor- and deployment-bounded asynchronous delivery policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AsyncPolicy {
    /// Maximum canonical payload bytes admitted for one message.
    pub max_payload_bytes: NonZeroUsize,
    /// Maximum events accepted in one replay transcript.
    pub max_replay_events: NonZeroUsize,
    /// Maximum trusted targets for one fanout operation.
    pub max_fanout: NonZeroUsize,
}

/// Opaque value retained only by the shared resource queue.
pub(crate) struct AsyncBufferEntry {
    authorized: AuthorizedAsyncBufferEntry,
    group: AsyncBufferGroup,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AsyncBufferGroup {
    Single,
    Replay { index: usize, count: usize },
}

impl AsyncBufferEntry {
    const fn single(authorized: AuthorizedAsyncBufferEntry) -> Self {
        Self {
            authorized,
            group: AsyncBufferGroup::Single,
        }
    }

    const fn replay(authorized: AuthorizedAsyncBufferEntry, index: usize, count: usize) -> Self {
        Self {
            authorized,
            group: AsyncBufferGroup::Replay { index, count },
        }
    }

    fn dequeue_group_len(&self, expected_index: usize) -> Option<usize> {
        match self.group {
            AsyncBufferGroup::Single => (expected_index == 0).then_some(1),
            AsyncBufferGroup::Replay { index, count } => {
                (index == expected_index && count > 0).then_some(count)
            }
        }
    }
}

impl fmt::Debug for AsyncBufferEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<AsyncBufferEntry:redacted>")
    }
}

#[derive(Clone, Eq, PartialEq)]
enum CoalescingKey {
    Refresh {
        binding: SubscriptionBinding,
        document_scope: DocumentAuthorizationScope,
        component_memo: AuthorizationMemo,
        subscription: SubscriptionId,
        stream: StreamName,
        epoch: StreamEpoch,
    },
    PresentationSignal {
        binding: SubscriptionBinding,
        document_scope: DocumentAuthorizationScope,
        component_memo: AuthorizationMemo,
        subscription: SubscriptionId,
        stream: StreamName,
        epoch: StreamEpoch,
        signal: BrowserOperationName,
        schema: BrowserPayloadSchema,
    },
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct PressureMembership {
    subscription: SubscriptionId,
    binding: SubscriptionBinding,
    document_scope: DocumentAuthorizationScope,
    component_memo: AuthorizationMemo,
}

impl PressureMembership {
    fn from_authorized(authorized: &AuthorizedAsyncBufferEntry) -> Self {
        Self {
            subscription: authorized.envelope().subscription().clone(),
            binding: authorized.binding.clone(),
            document_scope: authorized.document_scope.clone(),
            component_memo: authorized.component_memo.clone(),
        }
    }

    pub(crate) fn new(
        subscription: SubscriptionId,
        binding: SubscriptionBinding,
        document_scope: DocumentAuthorizationScope,
        component_memo: AuthorizationMemo,
    ) -> Self {
        Self {
            subscription,
            binding,
            document_scope,
            component_memo,
        }
    }
}

impl fmt::Debug for PressureMembership {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<PressureMembership:redacted>")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PressureCause {
    Admission,
    Delivery,
    Sequence,
    Detached,
}

#[derive(Clone, Eq, PartialEq)]
struct UnresolvedPressure {
    membership: PressureMembership,
    cause: PressureCause,
    high_water: StreamPosition,
    recovered_through: Option<StreamPosition>,
}

#[derive(Clone, Default)]
struct PressureTracker {
    state: Arc<Mutex<PressureState>>,
}

#[derive(Default)]
struct PressureState {
    unresolved: Vec<UnresolvedPressure>,
    saturated: bool,
}

#[derive(Clone, Default)]
struct DocumentDeliveryActivity(Arc<AtomicUsize>);

impl DocumentDeliveryActivity {
    fn start(&self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }

    fn finish(&self) {
        let previous = self.0.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "document delivery activity cannot underflow");
    }

    fn active(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }
}

impl PressureTracker {
    fn lock(&self) -> MutexGuard<'_, PressureState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn record(
        &self,
        membership: PressureMembership,
        cause: PressureCause,
        high_water: StreamPosition,
    ) {
        let mut state = self.lock();
        if let Some(current) = state
            .unresolved
            .iter_mut()
            .find(|current| current.membership == membership && current.cause == cause)
        {
            if pressure_position_precedes(current.high_water, high_water) {
                current.high_water = high_water;
            }
            return;
        }
        if state.unresolved.len() >= MAX_TRACKED_PRESSURE_CAUSES {
            state.saturated = true;
            return;
        }
        state.unresolved.push(UnresolvedPressure {
            membership,
            cause,
            high_water,
            recovered_through: None,
        });
    }

    fn record_recovery(&self, membership: &PressureMembership, through: StreamPosition) {
        for unresolved in self
            .lock()
            .unresolved
            .iter_mut()
            .filter(|unresolved| unresolved.membership == *membership)
        {
            if pressure_position_covers(through, unresolved.high_water)
                && unresolved
                    .recovered_through
                    .is_none_or(|current| pressure_position_precedes(current, through))
            {
                unresolved.recovered_through = Some(through);
            }
        }
    }

    fn commit_recoveries(&self) {
        self.lock().unresolved.retain(|unresolved| {
            !unresolved
                .recovered_through
                .is_some_and(|through| pressure_position_covers(through, unresolved.high_water))
        });
    }

    fn required_high_water(&self, membership: &PressureMembership) -> Option<StreamPosition> {
        self.lock()
            .unresolved
            .iter()
            .filter(|unresolved| unresolved.membership == *membership)
            .map(|unresolved| unresolved.high_water)
            .max_by_key(|position| (position.epoch().get(), position.sequence().get()))
    }

    fn retire_membership(&self, subscription: &SubscriptionId, binding: &SubscriptionBinding) {
        self.lock().unresolved.retain(|unresolved| {
            &unresolved.membership.subscription != subscription
                || &unresolved.membership.binding != binding
        });
    }

    fn clear(&self) {
        let mut state = self.lock();
        state.unresolved.clear();
        state.saturated = false;
    }

    fn is_degraded(&self) -> bool {
        let state = self.lock();
        state.saturated || !state.unresolved.is_empty()
    }

    fn cause_count(&self) -> usize {
        self.lock().unresolved.len()
    }
}

/// Sealed current-authority proof accepted by one document delivery queue.
pub(crate) struct AuthorizedAsyncBufferEntry {
    membership: OwnedActiveAsyncMembershipGuard,
    binding: SubscriptionBinding,
    document_scope: DocumentAuthorizationScope,
    component_memo: AuthorizationMemo,
    document_generation: u64,
    resolved_fanout: usize,
    _resolved_target_scope: Option<ContentDigest>,
    terminal: bool,
}

impl AuthorizedAsyncBufferEntry {
    pub(crate) fn new(
        membership: OwnedActiveAsyncMembershipGuard,
        binding: SubscriptionBinding,
        document_scope: DocumentAuthorizationScope,
        component_memo: AuthorizationMemo,
        document_generation: u64,
        resolved_fanout: usize,
        terminal: bool,
    ) -> Self {
        let resolved_target_scope = membership
            .resolved_event()
            .map(|resolved| resolved.target_scope().clone());
        Self {
            membership,
            binding,
            document_scope,
            component_memo,
            document_generation,
            resolved_fanout,
            _resolved_target_scope: resolved_target_scope,
            terminal,
        }
    }

    pub(crate) const fn envelope(&self) -> &AsyncEnvelope {
        self.membership.envelope()
    }

    pub(crate) const fn binding(&self) -> &SubscriptionBinding {
        &self.binding
    }

    pub(crate) const fn terminal(&self) -> bool {
        self.terminal
    }

    pub(crate) const fn document_generation(&self) -> u64 {
        self.document_generation
    }

    pub(crate) fn is_current_at(&self, now: crate::identity::UnixMillis) -> bool {
        self.membership.as_active().is_current_at(now)
    }

    pub(crate) fn matches_authorization(
        &self,
        binding: &SubscriptionBinding,
        document_scope: &DocumentAuthorizationScope,
        component_memo: &AuthorizationMemo,
        context: &super::AsyncEnvelopeContext,
    ) -> bool {
        &self.binding == binding
            && &self.document_scope == document_scope
            && &self.component_memo == component_memo
            && self.membership.as_active().context() == context
    }

    pub(crate) fn replace_membership(&mut self, membership: OwnedActiveAsyncMembershipGuard) {
        self.resolved_fanout = membership
            .resolved_event()
            .map_or(1, |resolved| usize::from(resolved.recipients().get()));
        self._resolved_target_scope = membership
            .resolved_event()
            .map(|resolved| resolved.target_scope().clone());
        self.membership = membership;
    }

    pub(crate) const fn replace_document_generation(&mut self, generation: u64) {
        self.document_generation = generation;
    }

    pub(crate) fn current_resolution_matches(
        &self,
        membership: &OwnedActiveAsyncMembershipGuard,
    ) -> bool {
        let resolved_fanout = membership
            .resolved_event()
            .map_or(1, |resolved| usize::from(resolved.recipients().get()));
        let resolved_target_scope = membership
            .resolved_event()
            .map(|resolved| resolved.target_scope());
        self.resolved_fanout == resolved_fanout
            && self._resolved_target_scope.as_ref() == resolved_target_scope
    }

    pub(crate) fn resolved_delivery(
        &self,
        deployment_fanout_limit: usize,
    ) -> ResolvedAsyncDelivery<'_> {
        ResolvedAsyncDelivery::new(
            self.membership.as_active(),
            self.membership.resolved_event(),
            deployment_fanout_limit,
        )
    }
}

impl fmt::Debug for AuthorizedAsyncBufferEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<AuthorizedAsyncBufferEntry:redacted>")
    }
}

/// Safe internal failure that contains no envelope, subscription, or payload data.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AsyncBackpressureError {
    close_code: AsyncCloseCode,
}

impl AsyncBackpressureError {
    /// Returns the closed safe failure category.
    #[must_use]
    pub const fn close_code(self) -> AsyncCloseCode {
        self.close_code
    }
}

impl fmt::Display for AsyncBackpressureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.close_code.as_str())
    }
}

impl fmt::Debug for AsyncBackpressureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for AsyncBackpressureError {}

/// One dequeued envelope holding exactly one shared active-delivery permit.
pub(crate) struct AsyncDeliveryLease {
    entries: Vec<AsyncBufferEntry>,
    permit: Option<Permit>,
    queue: ResourceQueue<AsyncBufferEntry>,
    activity: DocumentDeliveryActivity,
    cancellation: crate::resource::CancellationFlag,
    pressure: PressureTracker,
    membership: PressureMembership,
    high_water: StreamPosition,
    deployment_fanout_limit: usize,
    resolved: bool,
}

pub(crate) struct PulledCandidateLossGuard {
    pressure: PressureTracker,
    membership: Option<PressureMembership>,
    high_water: StreamPosition,
}

impl PulledCandidateLossGuard {
    pub(crate) fn disarm(&mut self) {
        self.membership = None;
    }
}

impl Drop for PulledCandidateLossGuard {
    fn drop(&mut self) {
        if let Some(membership) = self.membership.take() {
            self.pressure
                .record(membership, PressureCause::Delivery, self.high_water);
        }
    }
}

pub(crate) enum LeaseDispatchError {
    Retired,
    AuthorizationLost,
    MembershipExpired,
    ReplayRetired(ReplayDispatchError),
    ReplayAuthorizationLost(ReplayDispatchError),
    ReplayMembershipExpired(ReplayDispatchError),
    Sequence(SequenceError),
    Replay(ReplayDispatchError),
}

impl AsyncDeliveryLease {
    /// Returns the registered envelope while this bounded delivery owns its permit.
    #[must_use]
    pub(crate) fn envelope(&self) -> &AsyncEnvelope {
        self.entries
            .first()
            .expect("delivery leases always own one bounded queue group")
            .authorized
            .envelope()
    }

    pub(crate) fn binding(&self) -> &SubscriptionBinding {
        self.entries
            .first()
            .expect("delivery leases always own one bounded queue group")
            .authorized
            .binding()
    }

    pub(crate) const fn pressure_membership(&self) -> &PressureMembership {
        &self.membership
    }

    pub(crate) fn authorized_entries_mut(
        &mut self,
    ) -> impl ExactSizeIterator<Item = &mut AuthorizedAsyncBufferEntry> {
        self.entries.iter_mut().map(|entry| &mut entry.authorized)
    }

    pub(crate) fn is_replay(&self) -> bool {
        self.entries
            .first()
            .is_some_and(|entry| matches!(entry.group, AsyncBufferGroup::Replay { .. }))
    }

    pub(crate) fn is_canceled(&self) -> bool {
        self.cancellation.is_canceled()
    }

    /// Consumes this exact admitted proof through the document-owned sequence authority.
    pub(crate) fn dispatch(
        mut self,
        sequence: &mut SequenceMachine,
        now: crate::identity::UnixMillis,
        dispatcher: &mut dyn AsyncEnvelopeDispatchPort,
    ) -> Result<SequenceDisposition, LeaseDispatchError> {
        if self.cancellation.is_canceled() {
            return Err(LeaseDispatchError::Retired);
        }
        let entry = self
            .entries
            .first()
            .expect("single delivery lease owns one entry");
        debug_assert_eq!(self.entries.len(), 1);
        debug_assert_eq!(entry.group, AsyncBufferGroup::Single);
        let outcome = sequence.dispatch(
            entry
                .authorized
                .resolved_delivery(self.deployment_fanout_limit),
            now,
            dispatcher,
        );
        match outcome {
            Ok(
                disposition @ (SequenceDisposition::Apply
                | SequenceDisposition::IgnoreDuplicate
                | SequenceDisposition::IgnoreStaleEpoch),
            ) => {
                self.resolved = true;
                Ok(disposition)
            }
            Ok(disposition) => {
                self.pressure.record(
                    self.membership.clone(),
                    PressureCause::Sequence,
                    self.high_water,
                );
                self.resolved = true;
                Ok(disposition)
            }
            Err(error) => {
                self.pressure.record(
                    self.membership.clone(),
                    PressureCause::Delivery,
                    self.high_water,
                );
                self.resolved = true;
                Err(LeaseDispatchError::Sequence(error))
            }
        }
    }

    /// Consumes one atomically admitted transcript through Task 3 recovery.
    pub(crate) fn dispatch_replay_with<F>(
        mut self,
        sequence: &mut SequenceMachine,
        dispatcher: &mut dyn AsyncEnvelopeDispatchPort,
        mut validate_current: F,
    ) -> Result<ReplayDispatchOutcome, LeaseDispatchError>
    where
        F: FnMut(
            &mut AuthorizedAsyncBufferEntry,
        ) -> Result<crate::identity::UnixMillis, LeaseDispatchError>,
    {
        if self.cancellation.is_canceled() {
            return Err(LeaseDispatchError::Retired);
        }
        debug_assert!(self.is_replay());
        let envelopes = self
            .entries
            .iter()
            .map(|entry| entry.authorized.envelope())
            .collect::<Vec<_>>();
        let required_high_water = self
            .pressure
            .required_high_water(&self.membership)
            .filter(|_| sequence.state() == super::SequenceState::Current);
        let mut recovery = match sequence.prepare_replay(&envelopes, required_high_water) {
            Ok(recovery) => recovery,
            Err(error) => {
                self.pressure.record(
                    self.membership.clone(),
                    PressureCause::Delivery,
                    self.high_water,
                );
                self.resolved = true;
                return Err(LeaseDispatchError::Replay(error));
            }
        };
        for entry in &mut self.entries {
            if self.cancellation.is_canceled() {
                self.pressure.record(
                    self.membership.clone(),
                    PressureCause::Delivery,
                    self.high_water,
                );
                self.resolved = true;
                return Err(LeaseDispatchError::ReplayRetired(
                    sequence.interrupt_replay(&recovery, super::SequenceErrorKind::DeliveryRetired),
                ));
            }
            let now = match validate_current(&mut entry.authorized) {
                Ok(now) => now,
                Err(error) => {
                    self.pressure.record(
                        self.membership.clone(),
                        PressureCause::Delivery,
                        self.high_water,
                    );
                    self.resolved = true;
                    return Err(match error {
                        LeaseDispatchError::Retired => {
                            LeaseDispatchError::ReplayRetired(sequence.interrupt_replay(
                                &recovery,
                                super::SequenceErrorKind::DeliveryRetired,
                            ))
                        }
                        LeaseDispatchError::AuthorizationLost => {
                            LeaseDispatchError::ReplayAuthorizationLost(sequence.interrupt_replay(
                                &recovery,
                                super::SequenceErrorKind::AuthorizationLost,
                            ))
                        }
                        LeaseDispatchError::MembershipExpired => {
                            LeaseDispatchError::ReplayMembershipExpired(sequence.interrupt_replay(
                                &recovery,
                                super::SequenceErrorKind::MembershipExpired,
                            ))
                        }
                        other => other,
                    });
                }
            };
            let delivery = entry
                .authorized
                .resolved_delivery(self.deployment_fanout_limit);
            if let Err(error) =
                sequence.dispatch_replay_entry(&mut recovery, delivery, now, dispatcher)
            {
                self.pressure.record(
                    self.membership.clone(),
                    PressureCause::Delivery,
                    self.high_water,
                );
                self.resolved = true;
                return Err(LeaseDispatchError::Replay(error));
            }
        }
        let outcome = sequence.finish_replay(recovery);
        match outcome {
            Ok(outcome) => {
                self.resolved = true;
                Ok(outcome)
            }
            Err(error) => {
                self.pressure.record(
                    self.membership.clone(),
                    PressureCause::Delivery,
                    self.high_water,
                );
                self.resolved = true;
                Err(LeaseDispatchError::Replay(error))
            }
        }
    }
}

impl Drop for AsyncDeliveryLease {
    fn drop(&mut self) {
        if !self.resolved {
            self.pressure.record(
                self.membership.clone(),
                PressureCause::Delivery,
                self.high_water,
            );
        }
        // Release the in-flight reservation before evaluating the aggregate
        // idle boundary. A dequeued sibling is still retained work until its
        // delivery lease resolves or is abandoned.
        drop(self.permit.take());
        self.activity.finish();
        commit_recoveries_if_idle(&self.queue, &self.activity, &self.pressure);
    }
}

fn commit_recoveries_if_idle(
    queue: &ResourceQueue<AsyncBufferEntry>,
    activity: &DocumentDeliveryActivity,
    pressure: &PressureTracker,
) {
    if queue.is_empty() && activity.active() == 0 {
        pressure.commit_recoveries();
    }
}

impl fmt::Debug for AsyncDeliveryLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<AsyncDeliveryLease:redacted>")
    }
}

/// Policy wrapper over the shared owner, queue, permit pool, and cancellation flag.
pub(crate) struct AsyncBackpressure {
    owner: ResourceOwner<AsyncBufferEntry>,
    permits: PermitPool,
    activity: DocumentDeliveryActivity,
    policy: AsyncPolicy,
    pressure: PressureTracker,
    closed: Option<AsyncCloseCode>,
    telemetry: AsyncTelemetry,
}

impl fmt::Debug for AsyncBackpressure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AsyncBackpressure")
            .field("retained_events", &self.retained_events())
            .field("retained_bytes", &self.retained_bytes())
            .field("active_permits", &self.active_permits())
            .field("degraded", &self.is_degraded())
            .field("closed", &self.closed)
            .finish()
    }
}

impl AsyncBackpressure {
    /// Creates one bounded delivery scope without allocating a second queue.
    pub(crate) fn new(
        owner: ResourceOwner<AsyncBufferEntry>,
        permits: PermitPool,
        policy: AsyncPolicy,
    ) -> Result<Self, AsyncBackpressureError> {
        let bounds = owner.queue().bounds();
        if bounds.max_items() > MAX_ASYNC_BUFFER_EVENTS
            || bounds.max_bytes() > MAX_ASYNC_BUFFER_BYTES
            || policy.max_payload_bytes.get() > MAX_ASYNC_PAYLOAD_BYTES
            || policy.max_replay_events.get() > MAX_REPLAY_TRANSCRIPT_ENVELOPES
            || policy.max_fanout.get() > usize::from(MAX_EVENT_FANOUT)
        {
            return Err(AsyncBackpressureError {
                close_code: AsyncCloseCode::InvalidPolicy,
            });
        }
        Ok(Self {
            owner,
            permits,
            activity: DocumentDeliveryActivity::default(),
            policy,
            pressure: PressureTracker::default(),
            closed: None,
            telemetry: AsyncTelemetry::default(),
        })
    }

    /// Offers one entry sealed by current document and membership authority.
    pub(crate) fn offer(
        &mut self,
        authorized: AuthorizedAsyncBufferEntry,
    ) -> Result<BufferDisposition, AsyncBackpressureError> {
        if let Some(code) = self.closed {
            return Ok(BufferDisposition::Closed(code));
        }
        if self.owner.cancellation().is_canceled() {
            return Ok(self.finish(BufferDisposition::Closed(AsyncCloseCode::Retired)));
        }
        if !fanout_is_authorized(
            authorized.envelope(),
            authorized.resolved_fanout,
            self.policy,
        ) {
            return Ok(self.finish(BufferDisposition::Closed(AsyncCloseCode::FanoutExceeded)));
        }
        let encoded = match encode_async_envelope(authorized.envelope(), &AsyncCodecLimits::v1()) {
            Ok(encoded) => encoded,
            Err(_) => {
                return Ok(self.finish(BufferDisposition::Closed(AsyncCloseCode::InvalidEnvelope)));
            }
        };
        let payload_bytes = match canonical_async_payload_len(authorized.envelope()) {
            Ok(payload_bytes) => payload_bytes,
            Err(_) => {
                return Ok(self.finish(BufferDisposition::Closed(AsyncCloseCode::InvalidEnvelope)));
            }
        };
        if payload_bytes > self.policy.max_payload_bytes.get() {
            return Ok(self.finish(BufferDisposition::Closed(AsyncCloseCode::PayloadTooLarge)));
        }

        let key = coalescing_key(&authorized);
        let pressure_membership = PressureMembership::from_authorized(&authorized);
        let position = authorized.envelope().position();
        let admitted = self.owner.queue().try_admit_tail_with(
            encoded.len(),
            AsyncBufferEntry::single(authorized),
            |tail| classify_tail(tail, key.as_ref(), position),
        );
        match admitted {
            Ok(TailAdmissionOutcome::Appended) => Ok(self.finish(BufferDisposition::Queued)),
            Ok(TailAdmissionOutcome::Replaced) => {
                self.pressure
                    .record(pressure_membership, PressureCause::Admission, position);
                Ok(self.finish(BufferDisposition::Coalesced))
            }
            Ok(TailAdmissionOutcome::Retained) => Ok(self.finish(BufferDisposition::Coalesced)),
            Ok(TailAdmissionOutcome::Rejected) => {
                self.pressure
                    .record(pressure_membership, PressureCause::Admission, position);
                Ok(self.finish(BufferDisposition::Degraded))
            }
            Err(ResourceError::Retired) => {
                Ok(self.finish(BufferDisposition::Closed(AsyncCloseCode::Retired)))
            }
            Err(
                ResourceError::ItemsExceeded
                | ResourceError::BytesExceeded
                | ResourceError::PermitsExceeded,
            ) => {
                self.pressure
                    .record(pressure_membership, PressureCause::Admission, position);
                Ok(self.finish(BufferDisposition::Degraded))
            }
        }
    }

    /// Atomically preflights and admits one complete replay transcript.
    ///
    /// Replay entries never coalesce because Task 3 requires their exact
    /// ordered sequence evidence. Structural/resource rejection is typed as an
    /// invalid replay rather than manufacturing a new pressure obligation.
    pub(crate) fn offer_replay(
        &mut self,
        transcript: Vec<AuthorizedAsyncBufferEntry>,
    ) -> Result<BufferDisposition, AsyncBackpressureError> {
        if let Some(code) = self.closed {
            return Ok(BufferDisposition::Closed(code));
        }
        if self.owner.cancellation().is_canceled() {
            return Ok(self.finish(BufferDisposition::Closed(AsyncCloseCode::Retired)));
        }
        if transcript.is_empty()
            || transcript.len() > MAX_REPLAY_TRANSCRIPT_ENVELOPES
            || transcript.len() > self.policy.max_replay_events.get()
        {
            return Err(self.invalid_replay());
        }
        if transcript.windows(2).any(|pair| {
            let [previous, next] = pair else {
                unreachable!("two-entry replay window")
            };
            !same_replay_scope(previous, next)
                || previous
                    .envelope()
                    .position()
                    .sequence()
                    .get()
                    .checked_add(1)
                    != Some(next.envelope().position().sequence().get())
        }) {
            return Err(self.invalid_replay());
        }
        let bounds = self.owner.queue().bounds();

        let mut total_bytes = 0usize;
        let mut prepared = Vec::with_capacity(transcript.len());
        let transcript_len = transcript.len();
        for (index, authorized) in transcript.into_iter().enumerate() {
            if !fanout_is_authorized(
                authorized.envelope(),
                authorized.resolved_fanout,
                self.policy,
            ) {
                return Ok(self.finish(BufferDisposition::Closed(AsyncCloseCode::FanoutExceeded)));
            }
            let encoded =
                match encode_async_envelope(authorized.envelope(), &AsyncCodecLimits::v1()) {
                    Ok(encoded) => encoded,
                    Err(_) => {
                        return Err(self.invalid_replay());
                    }
                };
            let payload_bytes = match canonical_async_payload_len(authorized.envelope()) {
                Ok(payload_bytes) => payload_bytes,
                Err(_) => {
                    return Err(self.invalid_replay());
                }
            };
            if payload_bytes > self.policy.max_payload_bytes.get() {
                return Err(self.invalid_replay());
            }
            total_bytes = match total_bytes.checked_add(encoded.len()) {
                Some(total) => total,
                None => {
                    return Err(self.invalid_replay());
                }
            };
            prepared.push((
                encoded.len(),
                AsyncBufferEntry::replay(authorized, index, transcript_len),
            ));
        }
        let available_bytes = bounds
            .max_bytes()
            .saturating_sub(self.owner.queue().retained_bytes());
        if total_bytes > available_bytes {
            return Err(self.invalid_replay());
        }

        match self.owner.queue().try_push_batch(prepared) {
            Ok(()) => {}
            Err(ResourceError::Retired) => {
                return Ok(self.finish(BufferDisposition::Closed(AsyncCloseCode::Retired)));
            }
            Err(
                ResourceError::ItemsExceeded
                | ResourceError::BytesExceeded
                | ResourceError::PermitsExceeded,
            ) => {
                return Err(self.invalid_replay());
            }
        }
        Ok(self.finish(BufferDisposition::Queued))
    }

    /// Validates bounded replay resource semantics before host callbacks.
    pub(crate) fn preflight_replay(&mut self, transcript: &[AsyncEnvelope]) -> ReplayPreflight {
        if let Some(code) = self.closed {
            return ReplayPreflight::Closed(code);
        }
        if self.owner.cancellation().is_canceled() {
            return ReplayPreflight::Closed(
                match self.finish(BufferDisposition::Closed(AsyncCloseCode::Retired)) {
                    BufferDisposition::Closed(code) => code,
                    _ => unreachable!("retired replay preflight remains closed"),
                },
            );
        }
        let count = transcript.len();
        let available_items = self
            .owner
            .queue()
            .bounds()
            .max_items()
            .saturating_sub(self.owner.queue().len());
        if count == 0
            || count > MAX_REPLAY_TRANSCRIPT_ENVELOPES
            || count > self.policy.max_replay_events.get()
            || count > available_items
        {
            return ReplayPreflight::Invalid;
        }
        let available_bytes = self
            .owner
            .queue()
            .bounds()
            .max_bytes()
            .saturating_sub(self.owner.queue().retained_bytes());
        let mut total_bytes = 0usize;
        for envelope in transcript {
            let Ok(encoded) = encode_async_envelope(envelope, &AsyncCodecLimits::v1()) else {
                return ReplayPreflight::Invalid;
            };
            let Ok(payload_bytes) = canonical_async_payload_len(envelope) else {
                return ReplayPreflight::Invalid;
            };
            if payload_bytes > self.policy.max_payload_bytes.get() {
                return ReplayPreflight::Invalid;
            }
            let Some(next_total) = total_bytes.checked_add(encoded.len()) else {
                return ReplayPreflight::Invalid;
            };
            total_bytes = next_total;
            if total_bytes > available_bytes {
                return ReplayPreflight::Invalid;
            }
        }
        ReplayPreflight::Ready
    }

    fn invalid_replay(&mut self) -> AsyncBackpressureError {
        AsyncBackpressureError {
            close_code: AsyncCloseCode::InvalidEnvelope,
        }
    }

    pub(crate) fn record_replay_rejection(&mut self) {
        self.telemetry.increment(AsyncTelemetryCounter::Rejected);
    }

    /// Returns the exact number of retained queue entries.
    #[must_use]
    pub(crate) fn retained_events(&self) -> usize {
        self.owner.queue().len()
    }

    /// Returns the exact canonical envelope bytes retained by the shared queue.
    #[must_use]
    pub(crate) fn retained_bytes(&self) -> usize {
        self.owner.queue().retained_bytes()
    }

    /// Returns the number of delivery-work permits currently held.
    #[must_use]
    pub(crate) fn active_permits(&self) -> usize {
        self.activity.active()
    }

    pub(crate) const fn closed_code(&self) -> Option<AsyncCloseCode> {
        self.closed
    }

    /// Returns whether pressure made exact sequence continuity uncertain.
    #[must_use]
    pub(crate) fn is_degraded(&self) -> bool {
        self.pressure.is_degraded()
    }

    pub(crate) fn unresolved_pressure_cause_count(&self) -> usize {
        self.pressure.cause_count()
    }

    pub(crate) fn record_delivery_loss(
        &mut self,
        membership: PressureMembership,
        high_water: StreamPosition,
    ) {
        self.pressure
            .record(membership, PressureCause::Delivery, high_water);
        self.finish(BufferDisposition::Degraded);
    }

    pub(crate) fn track_pulled_candidate(
        &self,
        membership: PressureMembership,
        high_water: StreamPosition,
    ) -> PulledCandidateLossGuard {
        PulledCandidateLossGuard {
            pressure: self.pressure.clone(),
            membership: Some(membership),
            high_water,
        }
    }

    pub(crate) fn record_replay_recovery(
        &self,
        membership: &PressureMembership,
        through: StreamPosition,
    ) {
        self.pressure.record_recovery(membership, through);
        self.commit_recoveries_if_drained();
    }

    pub(crate) fn required_high_water(
        &self,
        membership: &PressureMembership,
    ) -> Option<StreamPosition> {
        self.pressure.required_high_water(membership)
    }

    pub(crate) fn commit_recoveries_if_drained(&self) {
        commit_recoveries_if_idle(self.owner.queue(), &self.activity, &self.pressure);
    }

    /// Returns one bounded low-cardinality telemetry snapshot.
    #[must_use]
    pub(crate) const fn telemetry_snapshot(&self) -> AsyncTelemetrySnapshot {
        self.telemetry.snapshot()
    }

    /// Starts one bounded delivery or leaves the queue untouched when saturated.
    pub(crate) fn try_start_delivery(&mut self) -> Option<AsyncDeliveryLease> {
        if self.closed.is_some() || self.owner.cancellation().is_canceled() {
            if self.closed.is_none() {
                self.finish(BufferDisposition::Closed(AsyncCloseCode::Retired));
            }
            return None;
        }
        let permit = self.permits.try_acquire().ok()?;
        let Some(entries) = self
            .owner
            .queue()
            .pop_batch_with(|index, entry| entry.dequeue_group_len(index))
        else {
            drop(permit);
            return None;
        };
        let first = entries
            .first()
            .expect("a dequeued resource batch is never empty");
        let membership = PressureMembership::from_authorized(&first.authorized);
        let high_water = entries
            .iter()
            .map(|entry| entry.authorized.envelope().position())
            .max_by_key(|position| (position.epoch().get(), position.sequence().get()))
            .expect("a dequeued resource batch is never empty");
        self.activity.start();
        Some(AsyncDeliveryLease {
            entries,
            permit: Some(permit),
            queue: self.owner.queue().clone(),
            activity: self.activity.clone(),
            cancellation: self.owner.cancellation(),
            pressure: self.pressure.clone(),
            membership,
            high_water,
            deployment_fanout_limit: self.policy.max_fanout.get(),
            resolved: false,
        })
    }

    /// Cancels this scope and drains every retained envelope exactly once.
    pub(crate) fn retire(&mut self) -> Retirement {
        let first_retirement = self.closed.is_none();
        if first_retirement {
            self.closed = Some(AsyncCloseCode::Retired);
            self.pressure.clear();
        }
        let retirement = self.owner.retire();
        if first_retirement {
            self.telemetry.increment(AsyncTelemetryCounter::Cleanup);
        }
        retirement
    }

    pub(crate) fn retire_membership(
        &mut self,
        subscription: &SubscriptionId,
        binding: &SubscriptionBinding,
    ) -> (usize, usize) {
        let removed = self.owner.queue().remove_if(|entry| {
            entry.authorized.envelope().subscription() == subscription
                && entry.authorized.binding() == binding
        });
        if removed.0 > 0 {
            self.telemetry.increment(AsyncTelemetryCounter::Cleanup);
        }
        self.pressure.retire_membership(subscription, binding);
        self.commit_recoveries_if_drained();
        removed
    }

    pub(crate) fn retain_current_memberships<F>(&mut self, mut is_current: F) -> (usize, usize)
    where
        F: FnMut(&SubscriptionId, &SubscriptionBinding) -> bool,
    {
        let mut detached = Vec::new();
        let removed = self.owner.queue().remove_if(|entry| {
            let remove = !entry.authorized.terminal()
                && !is_current(
                    entry.authorized.envelope().subscription(),
                    entry.authorized.binding(),
                );
            if remove {
                detached.push((
                    PressureMembership::from_authorized(&entry.authorized),
                    entry.authorized.envelope().position(),
                ));
            }
            remove
        });
        if removed.0 > 0 {
            for (membership, high_water) in detached {
                self.pressure
                    .record(membership, PressureCause::Detached, high_water);
            }
            self.telemetry.increment(AsyncTelemetryCounter::Cleanup);
        }
        self.commit_recoveries_if_drained();
        removed
    }

    pub(crate) fn has_membership_entries(
        &self,
        subscription: &SubscriptionId,
        binding: &SubscriptionBinding,
    ) -> bool {
        self.owner.queue().any(|entry| {
            entry.authorized.envelope().subscription() == subscription
                && entry.authorized.binding() == binding
        })
    }

    fn finish(&mut self, disposition: BufferDisposition) -> BufferDisposition {
        let counter = match disposition {
            BufferDisposition::Queued => AsyncTelemetryCounter::Queued,
            BufferDisposition::Coalesced => AsyncTelemetryCounter::Coalesced,
            BufferDisposition::Degraded => AsyncTelemetryCounter::Degraded,
            BufferDisposition::Closed(code) => {
                if self.closed.is_none() {
                    self.closed = Some(code);
                    if matches!(
                        code,
                        AsyncCloseCode::InvalidPolicy
                            | AsyncCloseCode::InvalidEnvelope
                            | AsyncCloseCode::PayloadTooLarge
                            | AsyncCloseCode::FanoutExceeded
                    ) {
                        self.telemetry.increment(AsyncTelemetryCounter::Rejected);
                    }
                    self.owner.retire();
                    self.telemetry.increment(AsyncTelemetryCounter::Cleanup);
                    AsyncTelemetryCounter::Closed
                } else {
                    return disposition;
                }
            }
        };
        self.telemetry.increment(counter);
        disposition
    }
}

fn pressure_position_precedes(left: StreamPosition, right: StreamPosition) -> bool {
    left.epoch() < right.epoch()
        || (left.epoch() == right.epoch() && left.sequence() < right.sequence())
}

fn pressure_position_covers(through: StreamPosition, required: StreamPosition) -> bool {
    !pressure_position_precedes(through, required)
}

fn coalescing_key(authorized: &AuthorizedAsyncBufferEntry) -> Option<CoalescingKey> {
    let envelope = authorized.envelope();
    let subscription = envelope.subscription().clone();
    let stream = envelope.stream().clone();
    let epoch = envelope.position().epoch();
    match envelope.payload() {
        AsyncPayload::Refresh(_) => Some(CoalescingKey::Refresh {
            binding: authorized.binding.clone(),
            document_scope: authorized.document_scope.clone(),
            component_memo: authorized.component_memo.clone(),
            subscription,
            stream,
            epoch,
        }),
        AsyncPayload::PresentationSignal(signal) => Some(CoalescingKey::PresentationSignal {
            binding: authorized.binding.clone(),
            document_scope: authorized.document_scope.clone(),
            component_memo: authorized.component_memo.clone(),
            subscription,
            stream,
            epoch,
            signal: signal.name().clone(),
            schema: signal.schema(),
        }),
        AsyncPayload::BrowserEvent(_)
        | AsyncPayload::Heartbeat(_)
        | AsyncPayload::Complete(_)
        | AsyncPayload::Error(_) => None,
    }
}

fn classify_tail(
    tail: Option<&AsyncBufferEntry>,
    key: Option<&CoalescingKey>,
    position: StreamPosition,
) -> TailAdmission {
    let Some(key) = key else {
        return TailAdmission::Append;
    };
    let Some(tail) = tail else {
        return TailAdmission::Append;
    };
    if tail.group != AsyncBufferGroup::Single {
        return TailAdmission::Append;
    }
    let tail_key = coalescing_key(&tail.authorized);
    if tail_key.as_ref() != Some(key) {
        return TailAdmission::Append;
    }
    if position.sequence() <= tail.authorized.envelope().position().sequence() {
        return TailAdmission::Retain;
    }
    let Some(expected) = tail
        .authorized
        .envelope()
        .position()
        .sequence()
        .get()
        .checked_add(1)
    else {
        return TailAdmission::Reject;
    };
    if position.sequence().get() == expected {
        TailAdmission::Replace
    } else {
        TailAdmission::Reject
    }
}

fn fanout_is_authorized(envelope: &AsyncEnvelope, fanout: usize, policy: AsyncPolicy) -> bool {
    if fanout == 0 || fanout > policy.max_fanout.get() {
        return false;
    }
    match envelope.payload() {
        AsyncPayload::BrowserEvent(event) => fanout <= usize::from(event.maximum_fanout().get()),
        AsyncPayload::Refresh(_)
        | AsyncPayload::PresentationSignal(_)
        | AsyncPayload::Heartbeat(_)
        | AsyncPayload::Complete(_)
        | AsyncPayload::Error(_) => fanout == 1,
    }
}

fn same_replay_scope(
    previous: &AuthorizedAsyncBufferEntry,
    next: &AuthorizedAsyncBufferEntry,
) -> bool {
    previous.binding == next.binding
        && previous.document_scope == next.document_scope
        && previous.component_memo == next.component_memo
        && previous.envelope().subscription() == next.envelope().subscription()
        && previous.envelope().stream() == next.envelope().stream()
        && previous.envelope().position().epoch() == next.envelope().position().epoch()
}
