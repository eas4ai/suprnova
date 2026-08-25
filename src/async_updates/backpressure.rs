//! Bounded server-side admission for typed asynchronous delivery.

use std::error::Error;
use std::fmt;
use std::num::NonZeroUsize;

use crate::identity::BrowserOperationName;
use crate::resource::{Permit, PermitPool, ResourceError, ResourceOwner, Retirement};

use super::envelope::canonical_async_payload_len;
use super::telemetry::AsyncTelemetry;
use super::{
    AsyncCodecLimits, AsyncEnvelope, AsyncPayload, AsyncTelemetryCounter, AsyncTelemetrySnapshot,
    BrowserPayloadSchema, MAX_EVENT_FANOUT, MAX_REPLAY_TRANSCRIPT_ENVELOPES, StreamEpoch,
    StreamName, StreamPosition, SubscriptionId, encode_async_envelope,
};

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
    /// A contiguous replaceable tail message was superseded in place.
    Coalesced,
    /// Continuity is no longer provable and authoritative recovery is required.
    Degraded,
    /// The delivery scope is permanently closed with a safe typed reason.
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
pub struct AsyncBufferEntry {
    envelope: AsyncEnvelope,
}

impl fmt::Debug for AsyncBufferEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<AsyncBufferEntry:redacted>")
    }
}

#[derive(Clone, Eq, PartialEq)]
enum CoalescingKey {
    Refresh {
        subscription: SubscriptionId,
        stream: StreamName,
        epoch: StreamEpoch,
    },
    PresentationSignal {
        subscription: SubscriptionId,
        stream: StreamName,
        epoch: StreamEpoch,
        signal: BrowserOperationName,
        schema: BrowserPayloadSchema,
    },
}

#[derive(Clone)]
struct TailMetadata {
    key: Option<CoalescingKey>,
    through: StreamPosition,
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
pub struct AsyncDelivery {
    entry: AsyncBufferEntry,
    _permit: Permit,
}

impl AsyncDelivery {
    /// Returns the registered envelope while this bounded delivery owns its permit.
    #[must_use]
    pub const fn envelope(&self) -> &AsyncEnvelope {
        &self.entry.envelope
    }
}

impl fmt::Debug for AsyncDelivery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<AsyncDelivery:redacted>")
    }
}

/// Policy wrapper over the shared owner, queue, permit pool, and cancellation flag.
pub struct AsyncBackpressure {
    owner: ResourceOwner<AsyncBufferEntry>,
    permits: PermitPool,
    policy: AsyncPolicy,
    tail: Option<TailMetadata>,
    degraded: bool,
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
            .field("degraded", &self.degraded)
            .field("closed", &self.closed)
            .finish()
    }
}

impl AsyncBackpressure {
    /// Creates one bounded delivery scope without allocating a second queue.
    pub fn new(
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
            policy,
            tail: None,
            degraded: false,
            closed: None,
            telemetry: AsyncTelemetry::default(),
        })
    }

    /// Offers one already-registered envelope using trusted descriptor fanout.
    pub fn offer(
        &mut self,
        envelope: AsyncEnvelope,
        fanout: usize,
    ) -> Result<BufferDisposition, AsyncBackpressureError> {
        if let Some(code) = self.closed {
            return Ok(BufferDisposition::Closed(code));
        }
        if self.owner.cancellation().is_canceled() {
            return Ok(self.finish(BufferDisposition::Closed(AsyncCloseCode::Retired)));
        }
        if !fanout_is_authorized(&envelope, fanout, self.policy) {
            return Ok(self.finish(BufferDisposition::Closed(AsyncCloseCode::FanoutExceeded)));
        }
        let encoded = match encode_async_envelope(&envelope, &AsyncCodecLimits::v1()) {
            Ok(encoded) => encoded,
            Err(_) => {
                self.telemetry.increment(AsyncTelemetryCounter::Rejected);
                return Err(AsyncBackpressureError {
                    close_code: AsyncCloseCode::InvalidEnvelope,
                });
            }
        };
        let payload_bytes = match canonical_async_payload_len(&envelope) {
            Ok(payload_bytes) => payload_bytes,
            Err(_) => {
                self.telemetry.increment(AsyncTelemetryCounter::Rejected);
                return Err(AsyncBackpressureError {
                    close_code: AsyncCloseCode::InvalidEnvelope,
                });
            }
        };
        if payload_bytes > self.policy.max_payload_bytes.get() {
            return Ok(self.finish(BufferDisposition::Closed(AsyncCloseCode::PayloadTooLarge)));
        }

        let key = coalescing_key(&envelope);
        if let (Some(key), Some(tail)) = (&key, &self.tail)
            && tail.key.as_ref() == Some(key)
        {
            let Some(expected) = tail.through.sequence().get().checked_add(1) else {
                self.degraded = true;
                return Ok(self.finish(BufferDisposition::Degraded));
            };
            if envelope.position().sequence().get() != expected {
                self.degraded = true;
                return Ok(self.finish(BufferDisposition::Degraded));
            }
            let position = envelope.position();
            let replaced = self
                .owner
                .queue()
                .try_replace_back(encoded.len(), AsyncBufferEntry { envelope });
            return match replaced {
                Ok(true) => {
                    self.tail = Some(TailMetadata {
                        key: Some(key.clone()),
                        through: position,
                    });
                    self.degraded = true;
                    Ok(self.finish(BufferDisposition::Coalesced))
                }
                Ok(false) => {
                    self.tail = None;
                    self.degraded = true;
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
                    self.degraded = true;
                    Ok(self.finish(BufferDisposition::Degraded))
                }
            };
        }

        let position = envelope.position();
        let queued = self
            .owner
            .queue()
            .try_push(encoded.len(), AsyncBufferEntry { envelope });
        match queued {
            Ok(()) => {
                self.tail = Some(TailMetadata {
                    key,
                    through: position,
                });
                Ok(self.finish(BufferDisposition::Queued))
            }
            Err(ResourceError::Retired) => {
                Ok(self.finish(BufferDisposition::Closed(AsyncCloseCode::Retired)))
            }
            Err(
                ResourceError::ItemsExceeded
                | ResourceError::BytesExceeded
                | ResourceError::PermitsExceeded,
            ) => {
                self.degraded = true;
                Ok(self.finish(BufferDisposition::Degraded))
            }
        }
    }

    /// Atomically preflights and admits one complete replay transcript.
    ///
    /// Empty, over-count, or aggregate-overflow transcripts degrade without
    /// partially changing the queue. Replay entries never coalesce because
    /// Task 3 requires their exact ordered sequence evidence.
    pub fn offer_replay(
        &mut self,
        transcript: Vec<(AsyncEnvelope, usize)>,
    ) -> Result<BufferDisposition, AsyncBackpressureError> {
        if let Some(code) = self.closed {
            return Ok(BufferDisposition::Closed(code));
        }
        if self.owner.cancellation().is_canceled() {
            return Ok(self.finish(BufferDisposition::Closed(AsyncCloseCode::Retired)));
        }
        if transcript.is_empty() || transcript.len() > self.policy.max_replay_events.get() {
            self.degraded = true;
            return Ok(self.finish(BufferDisposition::Degraded));
        }
        let bounds = self.owner.queue().bounds();
        let available_items = bounds.max_items().saturating_sub(self.owner.queue().len());
        if transcript.len() > available_items {
            self.degraded = true;
            return Ok(self.finish(BufferDisposition::Degraded));
        }

        let mut total_bytes = 0usize;
        let mut prepared = Vec::with_capacity(transcript.len());
        for (envelope, fanout) in transcript {
            if !fanout_is_authorized(&envelope, fanout, self.policy) {
                return Ok(self.finish(BufferDisposition::Closed(AsyncCloseCode::FanoutExceeded)));
            }
            let encoded = match encode_async_envelope(&envelope, &AsyncCodecLimits::v1()) {
                Ok(encoded) => encoded,
                Err(_) => {
                    self.telemetry.increment(AsyncTelemetryCounter::Rejected);
                    return Err(AsyncBackpressureError {
                        close_code: AsyncCloseCode::InvalidEnvelope,
                    });
                }
            };
            let payload_bytes = canonical_async_payload_len(&envelope).map_err(|_| {
                self.telemetry.increment(AsyncTelemetryCounter::Rejected);
                AsyncBackpressureError {
                    close_code: AsyncCloseCode::InvalidEnvelope,
                }
            })?;
            if payload_bytes > self.policy.max_payload_bytes.get() {
                return Ok(self.finish(BufferDisposition::Closed(AsyncCloseCode::PayloadTooLarge)));
            }
            total_bytes = match total_bytes.checked_add(encoded.len()) {
                Some(total) => total,
                None => {
                    self.degraded = true;
                    return Ok(self.finish(BufferDisposition::Degraded));
                }
            };
            prepared.push((envelope, encoded.len()));
        }
        let available_bytes = bounds
            .max_bytes()
            .saturating_sub(self.owner.queue().retained_bytes());
        if total_bytes > available_bytes {
            self.degraded = true;
            return Ok(self.finish(BufferDisposition::Degraded));
        }

        for (envelope, encoded_bytes) in prepared {
            let position = envelope.position();
            let key = coalescing_key(&envelope);
            match self
                .owner
                .queue()
                .try_push(encoded_bytes, AsyncBufferEntry { envelope })
            {
                Ok(()) => {
                    self.tail = Some(TailMetadata {
                        key,
                        through: position,
                    });
                }
                Err(ResourceError::Retired) => {
                    return Ok(self.finish(BufferDisposition::Closed(AsyncCloseCode::Retired)));
                }
                Err(
                    ResourceError::ItemsExceeded
                    | ResourceError::BytesExceeded
                    | ResourceError::PermitsExceeded,
                ) => {
                    self.degraded = true;
                    return Ok(self.finish(BufferDisposition::Degraded));
                }
            }
        }
        Ok(self.finish(BufferDisposition::Queued))
    }

    /// Returns the exact number of retained queue entries.
    #[must_use]
    pub fn retained_events(&self) -> usize {
        self.owner.queue().len()
    }

    /// Returns the exact canonical envelope bytes retained by the shared queue.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.owner.queue().retained_bytes()
    }

    /// Returns the number of delivery-work permits currently held.
    #[must_use]
    pub fn active_permits(&self) -> usize {
        self.permits.active()
    }

    /// Returns whether pressure made exact sequence continuity uncertain.
    #[must_use]
    pub const fn is_degraded(&self) -> bool {
        self.degraded
    }

    /// Returns one bounded low-cardinality telemetry snapshot.
    #[must_use]
    pub const fn telemetry_snapshot(&self) -> AsyncTelemetrySnapshot {
        self.telemetry.snapshot()
    }

    /// Starts one bounded delivery or leaves the queue untouched when saturated.
    pub fn try_start_delivery(&mut self) -> Option<AsyncDelivery> {
        if self.closed.is_some() || self.owner.cancellation().is_canceled() {
            if self.closed.is_none() {
                self.finish(BufferDisposition::Closed(AsyncCloseCode::Retired));
            }
            return None;
        }
        let permit = self.permits.try_acquire().ok()?;
        let Some(entry) = self.owner.queue().pop() else {
            drop(permit);
            return None;
        };
        if self.owner.queue().is_empty() {
            self.tail = None;
        }
        Some(AsyncDelivery {
            entry,
            _permit: permit,
        })
    }

    /// Cancels this scope and drains every retained envelope exactly once.
    pub fn retire(&mut self) -> Retirement {
        self.tail = None;
        let first_retirement = self.closed.is_none();
        if first_retirement {
            self.closed = Some(AsyncCloseCode::Retired);
        }
        let retirement = self.owner.retire();
        if first_retirement {
            self.telemetry.increment(AsyncTelemetryCounter::Cleanup);
        }
        retirement
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
                    self.tail = None;
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

fn coalescing_key(envelope: &AsyncEnvelope) -> Option<CoalescingKey> {
    let subscription = envelope.subscription().clone();
    let stream = envelope.stream().clone();
    let epoch = envelope.position().epoch();
    match envelope.payload() {
        AsyncPayload::Refresh(_) => Some(CoalescingKey::Refresh {
            subscription,
            stream,
            epoch,
        }),
        AsyncPayload::PresentationSignal(signal) => Some(CoalescingKey::PresentationSignal {
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
