//! Typed, bounded declarations for authorized asynchronous updates.

mod authorization;
mod backpressure;
mod envelope;
mod metadata;
mod sequence;
mod sse;
mod subscription;
mod telemetry;
mod transport;
mod websocket;

pub use authorization::{
    AuthoritativeStreamPosition, AuthorizedSubscription, CurrentSubscriptionRegistration,
    IssuedSubscription, SubscriptionAuthorizationDecision, SubscriptionAuthorizationOperation,
    SubscriptionAuthorizationPort, SubscriptionAuthorizationRequest, SubscriptionBaselineRequest,
    SubscriptionBinding, SubscriptionContinuityPort, SubscriptionCredentialPort,
    SubscriptionCredentialRequest, SubscriptionCredentialRotationOutcome,
    SubscriptionCredentialRotationRequest, SubscriptionCredentialScope, SubscriptionFuture,
    SubscriptionIssueRequest, SubscriptionRegistryPort, SubscriptionRegistryRequest,
    SubscriptionService, TrustedMountParameters,
};
pub use backpressure::{
    AsyncBackpressure, AsyncBackpressureError, AsyncBufferEntry, AsyncCloseCode, AsyncDelivery,
    AsyncPolicy, BufferDisposition, MAX_ASYNC_BUFFER_BYTES, MAX_ASYNC_BUFFER_EVENTS,
    MAX_ASYNC_PAYLOAD_BYTES,
};
pub use envelope::{
    ActiveAsyncMembershipGuard, AsyncCodecLimits, AsyncEnvelope, AsyncEnvelopeContext,
    AsyncEnvelopeError, AsyncEnvelopeErrorKind, AsyncMembershipRegistryPort,
    AsyncMembershipRequest, AsyncMembershipValidation, AsyncPayload,
    BoundedPresentationSignalContracts, CompletionReason, Heartbeat, MAX_ASYNC_ENVELOPE_ENTRIES,
    PresentationSignalContract, RegisteredBrowserEvent, RegisteredPresentationSignal,
    RegisteredRefresh, SUPPORTED_ASYNC_PROTOCOL_VERSIONS, StreamErrorCode, SubscriptionId,
    decode_async_envelope, encode_async_envelope,
};

pub use metadata::{
    BoundedEventNames, BoundedTargets, BoundedTopics, BrowserPayloadSchema, EventCyclePolicy,
    EventOrder, EventSource, EventTarget, MAX_EVENT_FANOUT, MAX_EVENT_TARGETS,
    MAX_SUBSCRIPTION_EVENTS, MAX_SUBSCRIPTION_MODES, MAX_SUBSCRIPTION_TOPICS, MAX_SUBSCRIPTIONS,
    ReconnectPolicy, StreamName, SubscriptionMetadata, SubscriptionMode, SubscriptionModes,
    TopicName,
};
pub use sequence::{
    AsyncContinuityAuthorityPort, AsyncContinuityRequest, AsyncDispatchError,
    AsyncDispatchErrorKind, AsyncEnvelopeDispatchPort, BaselineDisposition,
    MAX_REPLAY_TRANSCRIPT_ENVELOPES, ReplayDispatchError, ReplayDispatchOutcome,
    SequenceDegradation, SequenceDisposition, SequenceError, SequenceErrorKind, SequenceMachine,
    SequenceState,
};
pub use sse::{SseEncoder, SseEvent, SseMembershipControl, SseResponseContract};
pub use subscription::{
    ASYNC_SUBSCRIPTION_PROTOCOL_V1, AuthorizationMemo, BoundedEventContracts, CapabilityVersion,
    MAX_CANONICAL_SUBSCRIPTION_CLAIMS_BYTES, MAX_POLL_INTERVAL_MS, MAX_POLL_JITTER_BASIS_POINTS,
    MAX_RECONNECT_ATTEMPTS, MAX_SUBSCRIPTION_DESCRIPTOR_BYTES, MAX_SUBSCRIPTION_LIFETIME_MS,
    MIN_POLL_INTERVAL_MS, PollFallbackPolicy, PollInitialBehavior, PollVisibilityPolicy,
    StreamEpoch, StreamPosition, StreamSequence, SubscriptionClaims, SubscriptionDescriptor,
    SubscriptionDescriptorCodec, SubscriptionError, SubscriptionErrorKind,
    SubscriptionEventContract, TransportCredential, VerifiedSubscriptionDescriptor,
};
pub use telemetry::{AsyncTelemetryCounter, AsyncTelemetrySnapshot};
pub use transport::{
    AsyncEventSession, AsyncEventSource, AsyncTransportAuthorityPort,
    AsyncTransportAuthorityRequest, AsyncTransportAuthorityValidation, AsyncTransportError,
    AsyncTransportErrorKind, AsyncTransportFuture, AuthorizedTransportAdd,
    AuthorizedTransportSubscription, CloseDisposition, DocumentAuthorizationScope,
    DocumentTransportHandle, DocumentTransportKind, DocumentTransportLimits,
    DocumentTransportSession, EstablishingTransportAdd, MAX_DOCUMENT_TRANSPORT_MEMBERSHIPS,
    PendingTransportAdd, PendingTransportRemove, ReadyTransportAdd, ReadyTransportRemove,
    TransportMembershipOperation, VerifiedOrigin,
};
pub use websocket::{
    AuthorizedWebSocketUpgrade, WebSocketAuthentication, WebSocketCodec, WebSocketControlRecord,
    WebSocketFrame, WebSocketMembershipControl, WebSocketOriginPolicy,
};
