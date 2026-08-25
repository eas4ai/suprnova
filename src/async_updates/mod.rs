//! Typed, bounded declarations for authorized asynchronous updates.

mod authorization;
mod metadata;
mod subscription;

pub use authorization::{
    AuthorizedSubscription, IssuedSubscription, SubscriptionAuthorizationDecision,
    SubscriptionAuthorizationOperation, SubscriptionAuthorizationPort,
    SubscriptionAuthorizationRequest, SubscriptionBinding, SubscriptionCredentialDecision,
    SubscriptionCredentialPort, SubscriptionCredentialRequest, SubscriptionFuture,
    SubscriptionIssueRequest, SubscriptionService,
};

pub use metadata::{
    BoundedEventNames, BoundedTargets, BoundedTopics, BrowserPayloadSchema, EventCyclePolicy,
    EventOrder, EventSource, EventTarget, MAX_EVENT_FANOUT, MAX_EVENT_TARGETS,
    MAX_SUBSCRIPTION_EVENTS, MAX_SUBSCRIPTION_MODES, MAX_SUBSCRIPTION_TOPICS, MAX_SUBSCRIPTIONS,
    ReconnectPolicy, StreamName, SubscriptionMetadata, SubscriptionMode, SubscriptionModes,
    TopicName,
};
pub use subscription::{
    ASYNC_SUBSCRIPTION_PROTOCOL_V1, AuthorizationMemo, CapabilityVersion, MAX_POLL_INTERVAL_MS,
    MAX_POLL_JITTER_BASIS_POINTS, MAX_RECONNECT_ATTEMPTS, MAX_SUBSCRIPTION_LIFETIME_MS,
    MIN_POLL_INTERVAL_MS, PollFallbackPolicy, PollInitialBehavior, PollVisibilityPolicy,
    StreamEpoch, StreamPosition, StreamSequence, SubscriptionClaims, SubscriptionDescriptor,
    SubscriptionDescriptorCodec, SubscriptionError, SubscriptionErrorKind, TransportCredential,
    VerifiedSubscriptionDescriptor,
};

/// Registered typed event contracts allowed by one subscription descriptor.
pub type BoundedEventContracts = BoundedEventNames;
