//! Canonical event-routing and stream-subscription declarations.

use std::fmt;
use std::num::NonZeroU8;

use crate::identity::{BrowserOperationName, IslandSlot};
use crate::metadata::{MetadataError, MetadataErrorKind};

/// Maximum registered target scopes for one typed event.
pub const MAX_EVENT_TARGETS: usize = 16;
/// Maximum delivery fanout declared by one typed event.
pub const MAX_EVENT_FANOUT: u16 = 1_024;
/// Maximum stream subscriptions registered by one component.
pub const MAX_SUBSCRIPTIONS: usize = 32;
/// Maximum topic scopes registered by one subscription.
pub const MAX_SUBSCRIPTION_TOPICS: usize = 32;
/// Maximum typed event names registered by one subscription.
pub const MAX_SUBSCRIPTION_EVENTS: usize = 64;
/// Number of closed physical subscription transports.
pub const MAX_SUBSCRIPTION_MODES: usize = 2;

const MAX_STREAM_NAME_BYTES: usize = 128;
const MAX_TOPIC_NAME_BYTES: usize = 256;

/// Closed root schema understood by the browser event validator.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum BrowserPayloadSchema {
    /// Any bounded structured JSON value.
    Json,
    /// The JSON `null` value.
    Null,
    /// A JSON boolean.
    Boolean,
    /// A signed integer encoded as a JSON number.
    I64,
    /// An unsigned integer encoded as a JSON number.
    U64,
    /// A finite JSON number.
    F64,
    /// A JSON string.
    String,
}

/// Trusted origin of one registered event contract.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EventSource {
    /// An application component authored the event.
    Component,
    /// An authorized typed stream authored the event.
    Stream,
}

/// Closed delivery target together with any target-specific propagation scope.
///
/// Named-island and browser-listener variants carry their registered identity,
/// so choosing a target kind never grants an arbitrary document-global scope.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EventTarget {
    /// The island that authored or owns delivery.
    SelfIsland,
    /// The direct owning parent island.
    Parent,
    /// A direct owned child island.
    Child,
    /// One exact registered island slot in the current document.
    NamedIsland(IslandSlot),
    /// The current validated document.
    Document,
    /// One exact approved browser listener.
    Browser(BrowserOperationName),
}

/// Canonically sorted, duplicate-free event target scopes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedTargets(Vec<EventTarget>);

impl BoundedTargets {
    /// Sorts and validates a nonempty bounded target declaration.
    pub fn new(mut targets: Vec<EventTarget>) -> Result<Self, MetadataError> {
        if targets.is_empty() {
            return Err(MetadataError::new(MetadataErrorKind::InvalidEventTarget));
        }
        if targets.len() > MAX_EVENT_TARGETS {
            return Err(MetadataError::new(MetadataErrorKind::TooManyEventTargets));
        }
        targets.sort();
        if targets.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(MetadataError::new(MetadataErrorKind::DuplicateEventTarget));
        }
        Ok(Self(targets))
    }

    /// Returns target scopes in canonical order.
    #[must_use]
    pub fn as_slice(&self) -> &[EventTarget] {
        &self.0
    }
}

/// Required ordering contract for typed event delivery.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EventOrder {
    /// Preserve the sequence established independently by each source.
    PerSourceSequence,
}

/// Closed cycle-prevention contract for event propagation.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EventCyclePolicy {
    /// Never deliver the same event through an island twice.
    ForbidRepeatedIsland,
    /// Stop delivery after the declared nonzero number of hops.
    MaximumHops(NonZeroU8),
}

/// Stable registered identity for one asynchronous stream.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct StreamName(String);

impl StreamName {
    /// Parses the bounded registered stream-name grammar.
    pub fn parse(value: &str) -> Result<Self, MetadataError> {
        parse_contract_name(value, MAX_STREAM_NAME_BYTES).map(Self)
    }

    /// Returns the validated stream identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for StreamName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<StreamName>")
    }
}

/// Stable trusted topic scope for one asynchronous subscription.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct TopicName(String);

impl TopicName {
    /// Parses the bounded registered topic-name grammar.
    pub fn parse(value: &str) -> Result<Self, MetadataError> {
        parse_contract_name(value, MAX_TOPIC_NAME_BYTES).map(Self)
    }

    /// Returns the validated topic identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for TopicName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<TopicName>")
    }
}

/// Canonically sorted, duplicate-free subscription topics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedTopics(Vec<TopicName>);

impl BoundedTopics {
    /// Sorts and validates a nonempty bounded topic declaration.
    pub fn new(mut topics: Vec<TopicName>) -> Result<Self, MetadataError> {
        if topics.is_empty() {
            return Err(MetadataError::new(
                MetadataErrorKind::InvalidSubscriptionMetadata,
            ));
        }
        if topics.len() > MAX_SUBSCRIPTION_TOPICS {
            return Err(MetadataError::new(
                MetadataErrorKind::TooManySubscriptionTopics,
            ));
        }
        topics.sort();
        if topics.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(MetadataError::new(
                MetadataErrorKind::DuplicateSubscriptionTopic,
            ));
        }
        Ok(Self(topics))
    }

    /// Returns topic scopes in canonical order.
    #[must_use]
    pub fn as_slice(&self) -> &[TopicName] {
        &self.0
    }
}

/// Canonically sorted, duplicate-free registered stream event names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedEventNames(Vec<BrowserOperationName>);

impl BoundedEventNames {
    /// Sorts and validates a nonempty bounded event-name declaration.
    pub fn new(mut events: Vec<BrowserOperationName>) -> Result<Self, MetadataError> {
        if events.is_empty() {
            return Err(MetadataError::new(
                MetadataErrorKind::InvalidSubscriptionMetadata,
            ));
        }
        if events.len() > MAX_SUBSCRIPTION_EVENTS {
            return Err(MetadataError::new(
                MetadataErrorKind::TooManySubscriptionEvents,
            ));
        }
        events.sort();
        if events.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(MetadataError::new(
                MetadataErrorKind::DuplicateSubscriptionEvent,
            ));
        }
        Ok(Self(events))
    }

    /// Returns registered stream event names in canonical order.
    #[must_use]
    pub fn as_slice(&self) -> &[BrowserOperationName] {
        &self.0
    }
}

/// Approved physical transports for one logical subscription.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SubscriptionMode {
    /// Same-origin server-sent event delivery.
    ServerSentEvents,
    /// Origin-validated WebSocket delivery.
    WebSocket,
}

/// Canonically sorted, duplicate-free approved subscription transports.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionModes(Vec<SubscriptionMode>);

impl SubscriptionModes {
    /// Sorts and validates a nonempty closed transport declaration.
    pub fn new(mut modes: Vec<SubscriptionMode>) -> Result<Self, MetadataError> {
        if modes.is_empty() {
            return Err(MetadataError::new(
                MetadataErrorKind::InvalidSubscriptionMetadata,
            ));
        }
        if modes.len() > MAX_SUBSCRIPTION_MODES {
            return Err(MetadataError::new(
                MetadataErrorKind::TooManySubscriptionModes,
            ));
        }
        modes.sort();
        if modes.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(MetadataError::new(
                MetadataErrorKind::DuplicateSubscriptionMode,
            ));
        }
        Ok(Self(modes))
    }

    /// Returns approved transport modes in canonical order.
    #[must_use]
    pub fn as_slice(&self) -> &[SubscriptionMode] {
        &self.0
    }
}

/// Closed reconnect behavior declared for one subscription.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ReconnectPolicy {
    /// Establish a new transport and obtain an authoritative refresh.
    RefreshOnReconnect,
    /// Attempt bounded continuity resume and refresh when proof is unavailable.
    ResumeOrRefresh {
        /// Maximum resume attempts before authoritative refresh.
        maximum_attempts: NonZeroU8,
    },
}

/// Canonical registered declaration for one authorized asynchronous stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionMetadata {
    stream: StreamName,
    topics: BoundedTopics,
    events: BoundedEventNames,
    modes: SubscriptionModes,
    reconnect: ReconnectPolicy,
}

impl SubscriptionMetadata {
    /// Creates one fully bounded registered subscription declaration.
    #[must_use]
    pub const fn new(
        stream: StreamName,
        topics: BoundedTopics,
        events: BoundedEventNames,
        modes: SubscriptionModes,
        reconnect: ReconnectPolicy,
    ) -> Self {
        Self {
            stream,
            topics,
            events,
            modes,
            reconnect,
        }
    }

    /// Returns the registered stream identity.
    #[must_use]
    pub const fn stream(&self) -> &StreamName {
        &self.stream
    }

    /// Returns trusted topic scopes in canonical order.
    #[must_use]
    pub const fn topics(&self) -> &BoundedTopics {
        &self.topics
    }

    /// Returns allowed registered typed events in canonical order.
    #[must_use]
    pub const fn events(&self) -> &BoundedEventNames {
        &self.events
    }

    /// Returns approved physical transport modes.
    #[must_use]
    pub const fn modes(&self) -> &SubscriptionModes {
        &self.modes
    }

    /// Returns the reconnect contract.
    #[must_use]
    pub const fn reconnect(&self) -> ReconnectPolicy {
        self.reconnect
    }
}

fn parse_contract_name(value: &str, maximum_bytes: usize) -> Result<String, MetadataError> {
    let valid = !value.is_empty()
        && value.len() <= maximum_bytes
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        });
    if !valid {
        return Err(MetadataError::new(MetadataErrorKind::InvalidIdentity));
    }
    Ok(value.to_owned())
}
