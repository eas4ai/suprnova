//! Subscriptions, each on a connection of its own.

use futures::StreamExt;
use std::time::Duration;

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

/// The longest wait between two attempts to subscribe again.
const MOST_BACKOFF: Duration = Duration::from_secs(2);

/// Open a connection of its own and subscribe it to `names`, as channels or
/// as patterns.
pub(crate) async fn open(
    client: &redis::Client,
    names: &[String],
    patterns: bool,
) -> redis::RedisResult<redis::aio::PubSubStream> {
    let mut pubsub = client.get_async_pubsub().await?;
    for name in names {
        if patterns {
            pubsub.psubscribe(name).await?;
        } else {
            pubsub.subscribe(name).await?;
        }
    }
    Ok(pubsub.into_on_message())
}

/// An open subscription. It holds a connection of its own, which closes
/// when the subscription is dropped. When the server closes it, on a
/// restart, a `CLIENT KILL`, or a full output buffer, the subscription
/// opens a new one and subscribes again; the messages published in between
/// are not delivered, as Pub/Sub keeps none.
pub struct RedisSubscription {
    pub(crate) stream: redis::aio::PubSubStream,
    pub(crate) client: redis::Client,
    pub(crate) names: Vec<String>,
    pub(crate) patterns: bool,
}

impl RedisSubscription {
    /// The next message, waiting for one, and subscribing again on a new
    /// connection whenever the server closes this one. `None` only when
    /// subscribing again fails for a reason other than a connection that
    /// cannot be made, such as a server that refuses the subscription.
    pub async fn next(&mut self) -> Option<RedisMessage> {
        let message = loop {
            if let Some(message) = self.stream.next().await {
                break message;
            }
            let mut backoff = Duration::from_millis(50);
            loop {
                match open(&self.client, &self.names, self.patterns).await {
                    Ok(stream) => {
                        self.stream = stream;
                        break;
                    }
                    Err(error) if crate::redis_retry::is_transient(&error) => {
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(MOST_BACKOFF);
                    }
                    Err(_) => return None,
                }
            }
        };
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
