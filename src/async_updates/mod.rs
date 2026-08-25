//! Typed, bounded declarations for authorized asynchronous updates.

mod authorization;
mod envelope;
mod metadata;
mod sequence;
mod subscription;

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
pub use envelope::{
    AsyncCodecLimits, AsyncEnvelope, AsyncEnvelopeContext, AsyncEnvelopeError,
    AsyncEnvelopeErrorKind, AsyncMembershipRegistryPort, AsyncMembershipRequest,
    AsyncMembershipValidation, AsyncPayload, BoundedPresentationSignalContracts, CompletionReason,
    Heartbeat, PresentationSignalContract, RegisteredBrowserEvent, RegisteredPresentationSignal,
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
    AsyncContinuityAuthorityPort, AsyncContinuityRequest, BaselineDisposition,
    MAX_REPLAY_TRANSCRIPT_ENVELOPES, SequenceDegradation, SequenceDisposition, SequenceError,
    SequenceErrorKind, SequenceMachine, SequenceState,
};
pub use subscription::{
    ASYNC_SUBSCRIPTION_PROTOCOL_V1, AuthorizationMemo, BoundedEventContracts, CapabilityVersion,
    MAX_CANONICAL_SUBSCRIPTION_CLAIMS_BYTES, MAX_POLL_INTERVAL_MS, MAX_POLL_JITTER_BASIS_POINTS,
    MAX_RECONNECT_ATTEMPTS, MAX_SUBSCRIPTION_DESCRIPTOR_BYTES, MAX_SUBSCRIPTION_LIFETIME_MS,
    MIN_POLL_INTERVAL_MS, PollFallbackPolicy, PollInitialBehavior, PollVisibilityPolicy,
    StreamEpoch, StreamPosition, StreamSequence, SubscriptionClaims, SubscriptionDescriptor,
    SubscriptionDescriptorCodec, SubscriptionError, SubscriptionErrorKind,
    SubscriptionEventContract, TransportCredential, VerifiedSubscriptionDescriptor,
};
