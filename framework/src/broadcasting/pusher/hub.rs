//! The Pusher-protocol broadcast hub.

use super::auth::PusherAuth;
use super::client::PusherClient;
use super::config::PusherConfig;
use crate::FrameworkError;
use crate::broadcasting::hub::reject_reserved_channel;
use crate::broadcasting::{BroadcastEnvelope, BroadcastHub, ChannelRegistry, InMemoryBroadcastHub};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::broadcast;

/// A [`BroadcastHub`] that publishes through a Pusher-protocol service:
/// Pusher Channels, Soketi or Laravel Reverb.
///
/// Every publish goes to an in-process [`InMemoryBroadcastHub`] first,
/// so a `ws!` endpoint served by this process keeps receiving events,
/// then to the service's REST API through the hub's [`PusherClient`]. The
/// [`ChannelRegistry`] decides each channel's wire name (see
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
    client: PusherClient,
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
        Ok(Self {
            client: PusherClient::new(config, registry)?,
            local: InMemoryBroadcastHub::new(),
        })
    }

    /// The authorization state for
    /// [`pusher_channel_auth`](crate::broadcasting::pusher_channel_auth)
    /// and [`pusher_user_auth`](crate::broadcasting::pusher_user_auth).
    /// Bind it with `App::singleton(hub.auth())`.
    pub fn auth(&self) -> PusherAuth {
        PusherAuth::new(
            Arc::clone(self.client.config()),
            Arc::clone(self.client.registry()),
        )
    }

    /// The client this hub publishes through, with the hub's config and
    /// registry. Mirrors Laravel's `PusherBroadcaster::getPusher`.
    ///
    /// Use it to ask the service what the in-process hub cannot know, such
    /// as the users on a presence channel across every process; clone it
    /// to keep it beyond the hub's borrow.
    pub fn client(&self) -> &PusherClient {
        &self.client
    }
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
        self.client.trigger(&envelope).await
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

    /// The members this process tracks. The service's own list, which
    /// sees every connection, is
    /// [`PusherClient::presence_users`] through [`PusherBroadcastHub::client`].
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
}
