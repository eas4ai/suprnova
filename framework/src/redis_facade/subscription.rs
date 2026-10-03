//! Subscriptions, each on a connection of its own.

use futures::StreamExt;

/// A message a subscription received.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RedisMessage {
    /// The channel it was published to.
    pub channel: String,
    /// The pattern it matched, for a
    /// [`psubscribe`](super::RedisConnection::psubscribe) subscription.
    pub pattern: Option<String>,
    /// What was published.
    pub payload: Vec<u8>,
}

impl RedisMessage {
    /// The payload as text, when it is UTF-8.
    pub fn payload_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.payload).ok()
    }
}

/// An open subscription. It holds a connection of its own, which closes
/// when the subscription is dropped.
pub struct RedisSubscription {
    pub(crate) stream: redis::aio::PubSubStream,
}

impl RedisSubscription {
    /// The next message, waiting for one; `None` once the connection is
    /// gone.
    pub async fn next(&mut self) -> Option<RedisMessage> {
        let message = self.stream.next().await?;
        let text = |value: redis::RedisResult<Vec<u8>>| {
            value.map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        };
        Some(RedisMessage {
            channel: text(message.get_channel::<Vec<u8>>()).unwrap_or_default(),
            pattern: if message.from_pattern() {
                text(message.get_pattern::<Vec<u8>>()).ok()
            } else {
                None
            },
            payload: message.get_payload_bytes().to_vec(),
        })
    }
}

impl std::fmt::Debug for RedisSubscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisSubscription").finish_non_exhaustive()
    }
}
