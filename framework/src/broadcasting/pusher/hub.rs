//! The Pusher-protocol broadcast hub.

use super::auth::PusherAuth;
use super::config::PusherConfig;
use super::{encryption, names, signing};
use crate::FrameworkError;
use crate::broadcasting::hub::reject_reserved_channel;
use crate::broadcasting::{BroadcastEnvelope, BroadcastHub, ChannelRegistry, InMemoryBroadcastHub};
use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

/// A [`BroadcastHub`] that publishes through a Pusher-protocol service:
/// Pusher Channels, Soketi or Laravel Reverb.
///
/// Every publish goes to an in-process [`InMemoryBroadcastHub`] first,
/// so a `ws!` endpoint served by this process keeps receiving events,
/// then to the service's REST API. The [`ChannelRegistry`] decides each
/// channel's wire name (see
/// [`Channel::visibility`](crate::broadcasting::Channel::visibility)),
/// the same registry the authorization endpoints use.
///
/// ```rust,no_run
/// use std::sync::Arc;
/// use suprnova::broadcasting::{BroadcastHub, ChannelRegistry};
/// use suprnova::{App, PusherBroadcastHub, PusherConfig};
/// # fn ex() -> Result<(), suprnova::FrameworkError> {
/// let registry = Arc::new(ChannelRegistry::new());
/// let hub = PusherBroadcastHub::new(PusherConfig::from_env()?, Arc::clone(&registry))?;
/// App::singleton(hub.auth());
/// App::bind::<dyn BroadcastHub>(Arc::new(hub));
/// # Ok(()) }
/// ```
pub struct PusherBroadcastHub {
    config: Arc<PusherConfig>,
    registry: Arc<ChannelRegistry>,
    client: reqwest::Client,
    local: InMemoryBroadcastHub,
}

impl PusherBroadcastHub {
    /// A hub for `config`, naming channels through `registry`.
    ///
    /// # Errors
    ///
    /// Fails when a config value is invalid (see [`PusherConfig`]),
    /// with an error that names the field and never quotes the secret,
    /// or when the HTTP client cannot be built (for example, when the
    /// TLS backend cannot load).
    pub fn new(
        config: PusherConfig,
        registry: Arc<ChannelRegistry>,
    ) -> Result<Self, FrameworkError> {
        config.validate()?;
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| {
                FrameworkError::internal(format!(
                    "building the Pusher HTTP client failed: {}",
                    e.without_url()
                ))
            })?;
        Ok(Self {
            config: Arc::new(config),
            registry,
            client,
            local: InMemoryBroadcastHub::new(),
        })
    }

    /// The authorization state for
    /// [`pusher_channel_auth`](crate::broadcasting::pusher_channel_auth)
    /// and [`pusher_user_auth`](crate::broadcasting::pusher_user_auth).
    /// Bind it with `App::singleton(hub.auth())`.
    pub fn auth(&self) -> PusherAuth {
        PusherAuth::new(Arc::clone(&self.config), Arc::clone(&self.registry))
    }

    /// POST one envelope to `/apps/{app_id}/events`. An envelope carries
    /// one channel, so one envelope is one request.
    async fn publish_remote(&self, envelope: &BroadcastEnvelope) -> Result<(), FrameworkError> {
        let channel = names::wire_name(&self.registry, &envelope.channel)?;
        let data = self.event_data(&channel, &envelope.data)?;
        let body = serde_json::to_vec(&EventRequest {
            name: &envelope.event,
            channels: [&channel],
            data: &data,
            socket_id: envelope.except.as_deref(),
        })
        .map_err(|e| FrameworkError::from_external_with("serializing a Pusher event failed", e))?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| {
                FrameworkError::internal(
                    "the system clock reads before 1970, so a Pusher request cannot be timestamped",
                )
            })?
            .as_secs();
        let secret = self.config.secret();
        let signed = signing::events_query(
            &self.config.key,
            secret,
            &self.config.app_id,
            timestamp,
            &body,
        )?;
        let url = format!(
            "{}/apps/{}/events?{}",
            self.config.base_url(),
            self.config.app_id,
            signed.to_query()
        );
        let scrub = [secret, signed.signature.as_str()];

        let response = self
            .client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| {
                FrameworkError::internal(format!(
                    "Pusher publish to '{channel}' failed: {}",
                    describe_transport_error(e, &scrub)
                ))
            })?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let answer = match response.bytes().await {
            Ok(bytes) => excerpt(&String::from_utf8_lossy(&bytes), &scrub),
            Err(_) => "(the response body could not be read)".to_string(),
        };
        Err(FrameworkError::internal(format!(
            "Pusher publish to '{channel}' failed with HTTP {}: {answer}",
            status.as_u16()
        )))
    }

    /// The `data` field for `channel`: the event data as a JSON string,
    /// or that string encrypted when the wire name is
    /// `private-encrypted-`.
    ///
    /// The wire name decides, not the channel's visibility, because
    /// Pusher clients decide by the name too: whatever arrives on a
    /// `private-encrypted-` channel is decrypted, so it must never be
    /// plaintext.
    fn event_data(&self, channel: &str, data: &Value) -> Result<String, FrameworkError> {
        let serialize_failed =
            |e| FrameworkError::from_external_with("serializing Pusher event data failed", e);
        if !channel.starts_with(ENCRYPTED_PREFIX) {
            return serde_json::to_string(data).map_err(serialize_failed);
        }
        let master_key = self.config.encryption_master_key().ok_or_else(|| {
            FrameworkError::internal(format!(
                "Pusher publish to encrypted channel '{channel}' refused: no encryption \
                 master key is configured (set PUSHER_ENCRYPTION_MASTER_KEY_BASE64 or call \
                 PusherConfig::encryption_master_key_base64). The event is never sent as \
                 plaintext."
            ))
        })?;
        let plaintext = serde_json::to_vec(data).map_err(serialize_failed)?;
        encryption::encrypt(channel, master_key, &plaintext)
    }
}

/// The wire-name prefix of an end-to-end encrypted channel.
const ENCRYPTED_PREFIX: &str = "private-encrypted-";

/// The JSON body of one `POST /apps/{app_id}/events` call. A struct
/// rather than `json!` so the body has one fixed shape, and `socket_id`
/// is left out entirely when no connection is excluded.
#[derive(Serialize)]
struct EventRequest<'a> {
    name: &'a str,
    channels: [&'a str; 1],
    data: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    socket_id: Option<&'a str>,
}

/// At most this many bytes of an error answer go into the error, so a
/// large error page cannot flood the log.
const ERROR_EXCERPT_BYTES: usize = 200;

/// At most [`ERROR_EXCERPT_BYTES`] of `text`, with each of `secrets`
/// replaced by `"[redacted]"` first.
///
/// Scrubbing comes before truncating so a cut can never leave a whole
/// secret behind. A server that echoes the request back (some error
/// pages quote the string they expected to be signed) would otherwise
/// put the signature into a log line.
fn excerpt(text: &str, secrets: &[&str]) -> String {
    let mut text = text.to_string();
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        text = text.replace(secret, "[redacted]");
    }
    let mut end = text.len().min(ERROR_EXCERPT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}

/// Describe a transport error without the request URL, whose query
/// carries the signature: reqwest's own message quotes the URL.
fn describe_transport_error(error: reqwest::Error, secrets: &[&str]) -> String {
    let error = error.without_url();
    let mut text = error.to_string();
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    excerpt(&text, secrets)
}

#[async_trait]
impl BroadcastHub for PusherBroadcastHub {
    fn subscribe(&self, channel: &str) -> broadcast::Receiver<BroadcastEnvelope> {
        self.local.subscribe(channel)
    }

    /// Publish locally, then to the service.
    async fn publish(&self, envelope: BroadcastEnvelope) -> Result<(), FrameworkError> {
        reject_reserved_channel(&envelope.channel)?;
        self.local.publish(envelope.clone()).await?;
        self.publish_remote(&envelope).await
    }

    fn subscriber_count(&self, channel: &str) -> usize {
        self.local.subscriber_count(channel)
    }

    async fn track_member(
        &self,
        channel: &str,
        member_id: &str,
        info: Value,
    ) -> Result<(), FrameworkError> {
        self.local.track_member(channel, member_id, info).await
    }

    async fn untrack_member(&self, channel: &str, member_id: &str) -> Result<(), FrameworkError> {
        self.local.untrack_member(channel, member_id).await
    }

    async fn list_members(&self, channel: &str) -> Vec<Value> {
        self.local.list_members(channel).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broadcasting::{Channel, ChannelVisibility};
    use serde_json::json;

    struct Vault;
    #[async_trait]
    impl Channel for Vault {
        fn name(&self) -> &'static str {
            "vault.{id}"
        }
        fn visibility(&self) -> ChannelVisibility {
            ChannelVisibility::Encrypted
        }
    }

    fn hub_without_master_key() -> PusherBroadcastHub {
        let mut registry = ChannelRegistry::new();
        registry.register(Vault);
        // Port 9 (discard): if the hub ever tried to send, the error
        // would be a transport error, which the test tells apart.
        let config = PusherConfig::new("3", "app-key", "app-secret")
            .host("127.0.0.1")
            .port(9)
            .scheme(crate::broadcasting::PusherScheme::Http);
        PusherBroadcastHub::new(config, Arc::new(registry)).expect("client builds")
    }

    #[tokio::test]
    async fn pusher_publish_to_encrypted_channel_without_master_key_is_err() {
        let hub = hub_without_master_key();
        let mut local = hub.subscribe("vault.1");
        let err = hub
            .publish(BroadcastEnvelope::new(
                "vault.1",
                "Opened",
                json!({"pin": 1234}),
            ))
            .await
            .expect_err("an encrypted channel without a master key must not publish");
        let message = err.to_string();
        assert!(
            message.contains("master key"),
            "the error says why, before any request is sent: {message}"
        );
        assert!(
            !message.contains("app-secret"),
            "never the secret: {message}"
        );
        // The in-process subscriber is unaffected.
        let got = local
            .try_recv()
            .expect("local subscriber received the envelope");
        assert_eq!(got.event, "Opened");
    }

    #[tokio::test]
    async fn pusher_publish_rejects_reserved_channel_before_anything_else() {
        let hub = hub_without_master_key();
        let err = hub
            .publish(BroadcastEnvelope::new("__presence__", "Spoof", json!({})))
            .await
            .expect_err("reserved names are refused");
        assert!(err.to_string().contains("reserved prefix '__'"));
    }

    #[test]
    fn pusher_hub_new_refuses_an_invalid_config() {
        let registry = Arc::new(ChannelRegistry::new());
        let bad_host = PusherConfig::new("3", "app-key", "app-secret").host("evil.example/x?");
        let err = PusherBroadcastHub::new(bad_host, Arc::clone(&registry))
            .err()
            .map(|e| e.to_string())
            .expect("a bad host fails at boot");
        assert!(err.contains("host"), "{err}");
        assert!(!err.contains("app-secret"), "never the secret: {err}");

        let zero_timeout =
            PusherConfig::new("3", "app-key", "app-secret").timeout(std::time::Duration::ZERO);
        let err = PusherBroadcastHub::new(zero_timeout, registry)
            .err()
            .map(|e| e.to_string())
            .expect("a zero timeout fails at boot");
        assert!(err.contains("timeout"), "{err}");
    }

    #[test]
    fn pusher_error_excerpt_is_capped_and_scrubbed() {
        let signature = "927d37a56401cbf139d27b5fbfea241b0b3edd53d143efb127106ba258b697c5";
        let body = format!("bad signature {signature} for secret app-secret");
        let text = excerpt(&body, &["app-secret", signature]);
        assert!(!text.contains(signature), "{text}");
        assert!(!text.contains("app-secret"), "{text}");
        assert!(text.contains("[redacted]"), "{text}");

        // Capped at 200 bytes, on a character boundary.
        let long = "é".repeat(150); // 300 bytes
        let capped = excerpt(&long, &[]);
        assert!(capped.len() <= 200, "{} bytes", capped.len());
        assert_eq!(capped, "é".repeat(100));

        // An empty secret never redacts every gap between characters.
        assert_eq!(excerpt("plain", &[""]), "plain");
    }
}
