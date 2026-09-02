//! Application-facing publication of authorized asynchronous updates.

use std::fmt;
use std::sync::Arc;

use suprnova_live::canonical::CanonicalValue;

use super::LiveRuntime;
use super::async_updates::{AsyncState, PublishError, StreamPayloadSpec};
use super::metadata::EventPayloadMetadata;

/// Where one published browser event is delivered inside each subscribing document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LiveEventTarget {
    /// The subscribing island itself.
    Island,
    /// The subscribing island's parent island.
    Parent,
    /// The subscribing island's child islands.
    Child,
    /// Every island of the subscribing document.
    Document,
    /// One named island slot of the subscribing document.
    NamedIsland(String),
    /// One registered browser listener.
    Browser(String),
}

/// Closed reasons a publication was not accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveStreamErrorKind {
    /// No Live runtime is bound in this process.
    RuntimeUnavailable,
    /// The topic is not a valid stream topic.
    InvalidTopic,
    /// The payload or target violates the declared event contract.
    InvalidPayload,
}

/// Failure to publish one asynchronous update.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveStreamError {
    kind: LiveStreamErrorKind,
}

impl LiveStreamError {
    const fn new(kind: LiveStreamErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed failure class.
    #[must_use]
    pub const fn kind(&self) -> LiveStreamErrorKind {
        self.kind
    }
}

impl fmt::Display for LiveStreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            LiveStreamErrorKind::RuntimeUnavailable => {
                formatter.write_str("Live runtime is not bound")
            }
            LiveStreamErrorKind::InvalidTopic => formatter.write_str("invalid Live stream topic"),
            LiveStreamErrorKind::InvalidPayload => {
                formatter.write_str("invalid Live stream payload")
            }
        }
    }
}

impl std::error::Error for LiveStreamError {}

/// Publisher for the registered stream topics of the bound Live runtime.
///
/// Publishing never creates authority: only subscriptions the runtime issued
/// to authorized documents receive the typed envelopes.
#[derive(Clone)]
pub struct LiveStreams {
    state: Arc<AsyncState>,
}

impl LiveStreams {
    /// Resolves the publisher of the process-wide Live runtime.
    pub fn resolve() -> Result<Self, LiveStreamError> {
        LiveRuntime::bind()
            .map(|runtime| Self::from_runtime(&runtime))
            .map_err(|_| LiveStreamError::new(LiveStreamErrorKind::RuntimeUnavailable))
    }

    /// Returns the publisher of one explicit runtime.
    #[must_use]
    pub fn from_runtime(runtime: &LiveRuntime) -> Self {
        Self {
            state: Arc::clone(runtime.async_state()),
        }
    }

    /// Tells every subscriber of `topic` to refresh from the server.
    #[allow(
        clippy::unused_async,
        reason = "publication is asynchronous in the public contract so durable fan-out can join later"
    )]
    pub async fn refresh(&self, topic: &str) -> Result<(), LiveStreamError> {
        self.state
            .publish(topic, &StreamPayloadSpec::Refresh)
            .map_err(publish_error)
    }

    /// Delivers one typed browser event to every subscriber of `topic`.
    #[allow(
        clippy::unused_async,
        reason = "publication is asynchronous in the public contract so durable fan-out can join later"
    )]
    pub async fn event<T: EventPayloadMetadata>(
        &self,
        topic: &str,
        target: LiveEventTarget,
        payload: CanonicalValue,
    ) -> Result<(), LiveStreamError> {
        self.state
            .publish(
                topic,
                &StreamPayloadSpec::BrowserEvent {
                    name: T::NAME.to_owned(),
                    version: T::VERSION,
                    target,
                    payload,
                },
            )
            .map_err(publish_error)
    }
}

impl fmt::Debug for LiveStreams {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<LiveStreams:redacted>")
    }
}

const fn publish_error(error: PublishError) -> LiveStreamError {
    match error {
        PublishError::InvalidTopic => LiveStreamError::new(LiveStreamErrorKind::InvalidTopic),
        PublishError::InvalidPayload => LiveStreamError::new(LiveStreamErrorKind::InvalidPayload),
    }
}
