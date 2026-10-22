//! `BroadcastHub` + the default in-process implementation backed by
//! `tokio::sync::broadcast` per channel.
//!
//! Channels are created lazily on first subscribe or publish.
//! Subscriber drops are reflected in `subscriber_count` after the
//! next publish operation, per `tokio::sync::broadcast` semantics.
//!
//! Long-running processes that publish to a churn of distinct channel
//! names (e.g. `user.{id}.orders`) would otherwise leak a sender per
//! channel even after every subscriber detached. The in-memory hub
//! piggy-backs an opportunistic sweep onto every new-channel creation
//! to evict senders whose `receiver_count() == 0`. This bounds growth
//! to the live working set without a background task.

use crate::FrameworkError;
use crate::broadcasting::live::{
    AcceptedLiveTopicPublication, LiveTopicExclusion, LiveTopicRuntime, LiveTopicSignal,
    live_topic_channel, live_topic_envelope,
};
use crate::lock;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::RwLock as AsyncRwLock;
use tokio::sync::broadcast;

/// Per-channel broadcast buffer capacity. Subscribers that fall this
/// far behind get a `RecvError::Lagged(n)` on the next recv; the
/// handler decides how to surface it. 256 is comfortable for typical
/// chat/presence workloads.
const CHANNEL_CAPACITY: usize = 256;

/// Reject publishes to channels whose names begin with `"__"`. The
/// double-underscore prefix is reserved for framework meta-channels -
/// `__presence__` carries cross-process presence replication and is
/// fed by the hub itself via a dedicated producer path, never through
/// `publish`. Letting application code (or a compromised handler) call
/// `hub.publish` with a reserved channel would inject phantom presence
/// events into every peer's `cross_process_view`.
///
/// The `ChannelRegistry` already blocks registration of `__`-prefixed
/// names; this is the matching guard at the publish boundary so the
/// invariant holds even for code that publishes without a registered
/// channel.
pub(crate) fn reject_reserved_channel(channel: &str) -> Result<(), FrameworkError> {
    if channel.starts_with("__") {
        return Err(FrameworkError::internal(format!(
            "BroadcastHub::publish: channel name '{channel}' starts with reserved \
             prefix '__'; this prefix is reserved for framework meta-channels \
             (e.g. __presence__ for cross-process presence replication)"
        )));
    }
    Ok(())
}

/// One published event on a channel.
///
/// Named `BroadcastEnvelope` (rather than `Envelope`) to avoid a
/// naming conflict with `queue::Envelope`, which is also re-exported
/// from the `suprnova` crate root.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BroadcastEnvelope {
    /// Channel name (e.g. "chat.42", "presence.lobby").
    pub channel: String,
    /// Event name (e.g. "MessagePosted", "MemberJoined").
    pub event: String,
    /// Event payload, opaque to the hub.
    pub data: Value,
    /// Optional connection `socket_id` to exclude from delivery - the basis
    /// for [`Broadcastable::broadcast_to_others`](crate::broadcasting::Broadcastable::broadcast_to_others).
    /// The forwarder for the matching connection skips this envelope; every
    /// other subscriber still receives it. `None` (the default) delivers to
    /// all. It rides the cross-process fanout so exclusion holds there too, but
    /// is never forwarded to clients - it is a server-side routing concern.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub except: Option<String>,
}

impl BroadcastEnvelope {
    /// A new envelope delivered to every subscriber (no exclusion).
    pub fn new(channel: impl Into<String>, event: impl Into<String>, data: Value) -> Self {
        Self {
            channel: channel.into(),
            event: event.into(),
            data,
            except: None,
        }
    }

    /// Exclude one connection (by its `socket_id`) from this broadcast. Used by
    /// `broadcast_to_others`, and the per-dispatch escape hatch for code that
    /// publishes directly: `hub.publish(BroadcastEnvelope::new(c, e, d).with_except(id))`.
    pub fn with_except(mut self, socket_id: impl Into<String>) -> Self {
        self.except = Some(socket_id.into());
        self
    }
}

/// The broadcasting primitive. Apps depend on `Arc<dyn BroadcastHub>`
/// resolved from the container so the in-process default can be
/// swapped for a multi-process implementation without touching the
/// publish/subscribe call sites.
#[async_trait]
pub trait BroadcastHub: Send + Sync + 'static {
    /// Subscribe to a channel; the returned receiver yields each
    /// published envelope for as long as it stays in scope. Dropping
    /// the receiver detaches; the slot is reclaimed on the next
    /// publish to that channel.
    fn subscribe(&self, channel: &str) -> broadcast::Receiver<BroadcastEnvelope>;

    /// Publish an envelope to all subscribers of `envelope.channel`.
    ///
    /// Returning `Ok(())` with zero local subscribers is normal - channels
    /// are created on demand. `Err(_)` signals that the publish could not
    /// be reliably delivered: for in-process hubs that's effectively never
    /// (`Ok(())` always), but for cross-process implementations it covers
    /// fanout backend failures (broker disconnect, stream closed) so the
    /// caller - typically `Event::dispatch` via
    /// [`BroadcastListener`](crate::broadcasting::BroadcastListener) - can
    /// surface the loss instead of swallowing it. Local fanout, when it
    /// happens, runs before the error is returned and is unaffected.
    async fn publish(&self, envelope: BroadcastEnvelope) -> Result<(), FrameworkError>;

    /// Subscribe to the reserved signal channel for an Inertia Live topic.
    ///
    /// Custom hubs must implement this with the same authority as
    /// [`Self::publish_live_topic`]. The default returns an explicit error so
    /// a configured hub cannot silently degrade to process-local delivery.
    fn subscribe_live_topic(
        &self,
        _topic: &str,
    ) -> Result<broadcast::Receiver<BroadcastEnvelope>, FrameworkError> {
        Err(live_topics_unsupported())
    }

    /// Registers a boot-declared family policy without storing any instance names.
    fn configure_live_topic_family(
        &self,
        _family: &str,
        _retention: Duration,
    ) -> Result<(), FrameworkError> {
        Err(live_topics_unsupported())
    }

    /// Records listener ownership so in-memory instances stay pinned while memberships exist.
    /// Redis listeners renew their instance TTL through baseline heartbeats.
    fn live_topic_membership(&self, _topic: &str, _active: bool) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// Retires idle family instances during Live's existing membership prune pass.
    /// Returns the number of in-memory instances retired; Redis expires its own keys.
    fn prune_live_topics(&self) -> Result<usize, FrameworkError> {
        Ok(0)
    }

    /// Read the last publication cursor accepted for an Inertia Live topic.
    async fn live_topic_cursor(&self, _topic: &str) -> Result<u64, FrameworkError> {
        Err(live_topics_unsupported())
    }

    /// Reads one consistent baseline for all bindings before an Inertia loader starts.
    async fn live_topic_cursors(&self, _topics: &[String]) -> Result<Vec<u64>, FrameworkError> {
        Err(live_topics_unsupported())
    }

    /// Publish a data-free Inertia Live topic signal.
    ///
    /// Calls inside `interval` are coalesced into one signal containing the
    /// union of their props. Every accepted call still advances the cursor.
    async fn publish_live_topic(
        &self,
        _topic: &str,
        _props: &[String],
        _except: Option<LiveTopicExclusion>,
        _interval: Duration,
    ) -> Result<(), FrameworkError> {
        Err(live_topics_unsupported())
    }

    /// Subscriber count for a channel. `0` if no one is subscribed
    /// or the channel hasn't been created yet.
    fn subscriber_count(&self, _channel: &str) -> usize {
        0
    }

    /// Track a presence member on a channel. The `member_id` is a
    /// per-connection UUID assigned by the handler; `info` is the
    /// public payload returned by `PresenceChannel::member_info`.
    ///
    /// `Ok(())` on success. `Err(_)` on a fanout-backend failure
    /// (broker disconnect, stream closed) so the WS handler can
    /// surface the loss to the client instead of silently joining a
    /// presence channel that peer instances will never learn about.
    /// In-process hubs always return `Ok(())`.
    ///
    /// Default: no-op, so future fanout implementations can implement
    /// member tracking lazily.
    async fn track_member(
        &self,
        _channel: &str,
        _member_id: &str,
        _info: Value,
    ) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// Remove a previously tracked member. Called on unsubscribe or
    /// connection close. Same `Result` contract as
    /// [`Self::track_member`]: in-process hubs always succeed, fanout
    /// hubs surface backend failures.
    async fn untrack_member(&self, _channel: &str, _member_id: &str) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// Return the current member list for a channel - each element is
    /// the `info` value passed to [`Self::track_member`] for a live member.
    /// Default: empty vec.
    async fn list_members(&self, _channel: &str) -> Vec<Value> {
        Vec::new()
    }
}

fn live_topics_unsupported() -> FrameworkError {
    FrameworkError::internal("configured BroadcastHub does not support Inertia Live topic signals")
}

/// In-process broadcast hub. Default for single-process apps.
pub struct InMemoryBroadcastHub {
    channels: Arc<RwLock<HashMap<String, broadcast::Sender<BroadcastEnvelope>>>>,
    /// Per-channel presence members: channel → (member_id → info).
    members: Arc<AsyncRwLock<HashMap<String, HashMap<String, Value>>>>,
    live_topics: LiveTopicRuntime,
}

impl InMemoryBroadcastHub {
    /// Construct an empty in-process broadcast hub.
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            members: Arc::new(AsyncRwLock::new(HashMap::new())),
            live_topics: LiveTopicRuntime::default(),
        }
    }

    /// Subscribe to `channel`, evicting any zero-receiver senders we
    /// pass through on the way. The returned receiver is created **before**
    /// any map lock is released, so a concurrent eviction sweep can never
    /// observe a receiver_count == 0 in a race window between sender clone
    /// and the caller calling `.subscribe()` on it. New-channel creation
    /// uses the write lock the prune path also needs, making this
    /// atomic w.r.t. eviction.
    fn subscribe_for(&self, channel: &str) -> broadcast::Receiver<BroadcastEnvelope> {
        // Fast path - read lock, sender already exists. Subscribe under
        // the read guard so the sender can't be swept between clone and
        // subscribe.
        if let Ok(map) = self.channels.read()
            && let Some(tx) = map.get(channel)
        {
            return tx.subscribe();
        }
        self.live_topics.prune();
        // Slow path - create the channel under a write lock and sweep
        // any dead siblings while we hold it.
        match lock::write(&self.channels, "broadcast hub channels") {
            Ok(mut map) => {
                let tx = map
                    .entry(channel.to_string())
                    .or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0);
                let rx = tx.subscribe();
                Self::sweep_dead_channels(&mut map, channel);
                rx
            }
            Err(_) => {
                tracing::error!(
                    channel = %channel,
                    "BroadcastHub channels RwLock poisoned; returning orphan \
                     receiver for this channel. Messages on this channel will \
                     never arrive until process restart."
                );
                broadcast::channel(CHANNEL_CAPACITY).0.subscribe()
            }
        }
    }

    /// Resolve the sender for a publish. Unlike `subscribe_for`, this
    /// path never creates the channel - publishing to a channel with no
    /// subscribers is a deliberate no-op (the alternative - creating an
    /// orphan sender per publish - is the leak we're trying to avoid).
    fn sender_for_publish(&self, channel: &str) -> Option<broadcast::Sender<BroadcastEnvelope>> {
        self.channels.read().ok()?.get(channel).cloned()
    }

    /// Publish framework-owned traffic after its caller has selected a
    /// reserved channel through a typed API.
    pub(crate) fn publish_framework(&self, envelope: BroadcastEnvelope) {
        if let Some(sender) = self.sender_for_publish(&envelope.channel) {
            let _ = sender.send(envelope);
        }
    }

    /// Evict zero-receiver senders. Called under the write lock so the
    /// observation is consistent with subscribe (no receiver can appear
    /// between `receiver_count() == 0` and the removal). The channel
    /// being created right now is preserved unconditionally: it currently
    /// has the receiver we just minted but the caller hasn't observed
    /// it yet, so its `receiver_count` could read as 0 transiently.
    fn sweep_dead_channels(
        map: &mut HashMap<String, broadcast::Sender<BroadcastEnvelope>>,
        keep: &str,
    ) {
        map.retain(|k, tx| k == keep || tx.receiver_count() > 0);
    }

    /// Number of distinct channels currently held in the map - both live
    /// and (until the next sweep) idle. Exposed for tests that exercise
    /// the eviction policy; `subscriber_count` can't distinguish "no
    /// channel" from "channel exists with 0 receivers".
    #[cfg(test)]
    pub(crate) fn channel_count(&self) -> usize {
        self.channels.read().map(|m| m.len()).unwrap_or(0)
    }

    /// Reports shared sweep executions and visited expiry candidates for deterministic tests.
    #[doc(hidden)]
    #[must_use]
    pub fn live_topic_sweep_stats_for_test(&self) -> (u64, u64) {
        self.live_topics.sweep_stats()
    }

    /// Number of channels the presence member map holds. Exposed for tests
    /// of the empty-channel removal; `list_members` cannot distinguish "no
    /// entry" from "an empty entry".
    #[cfg(test)]
    pub(crate) async fn presence_channel_count(&self) -> usize {
        self.members.read().await.len()
    }
}

impl Default for InMemoryBroadcastHub {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BroadcastHub for InMemoryBroadcastHub {
    fn subscribe(&self, channel: &str) -> broadcast::Receiver<BroadcastEnvelope> {
        self.subscribe_for(channel)
    }

    async fn publish(&self, envelope: BroadcastEnvelope) -> Result<(), FrameworkError> {
        reject_reserved_channel(&envelope.channel)?;
        self.publish_framework(envelope);
        // No-subscriber publishes are normal: in-process delivery has
        // nowhere to go and that isn't a failure.
        Ok(())
    }

    fn subscribe_live_topic(
        &self,
        topic: &str,
    ) -> Result<broadcast::Receiver<BroadcastEnvelope>, FrameworkError> {
        Ok(self.subscribe_for(&live_topic_channel(topic)))
    }

    fn configure_live_topic_family(
        &self,
        family: &str,
        retention: Duration,
    ) -> Result<(), FrameworkError> {
        self.live_topics.configure_family(family, retention)
    }

    fn live_topic_membership(&self, topic: &str, active: bool) -> Result<(), FrameworkError> {
        self.live_topics.membership(topic, active);
        Ok(())
    }

    fn prune_live_topics(&self) -> Result<usize, FrameworkError> {
        let Some(retired) = self.live_topics.sweep() else {
            return Ok(0);
        };
        let mut channels = self
            .channels
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        channels.retain(|_, sender| sender.receiver_count() > 0);
        Ok(retired)
    }

    async fn live_topic_cursor(&self, topic: &str) -> Result<u64, FrameworkError> {
        Ok(self.live_topics.cursor(topic).await)
    }

    async fn live_topic_cursors(&self, topics: &[String]) -> Result<Vec<u64>, FrameworkError> {
        Ok(self.live_topics.cursors(topics).await)
    }

    async fn publish_live_topic(
        &self,
        topic: &str,
        props: &[String],
        except: Option<LiveTopicExclusion>,
        interval: Duration,
    ) -> Result<(), FrameworkError> {
        match self
            .live_topics
            .accept(topic, props, except, interval)
            .await?
        {
            AcceptedLiveTopicPublication::Immediate { signal, except } => {
                self.publish_live_signal(signal, except);
            }
            AcceptedLiveTopicPublication::Pending { deadline } => {
                let state = self.live_topics.clone();
                let channels = Arc::clone(&self.channels);
                let topic = topic.to_owned();
                tokio::spawn(async move {
                    tokio::time::sleep_until(deadline).await;
                    if let Some((signal, except)) = state.take_pending(&topic).await {
                        let envelope = live_topic_envelope(signal, except);
                        if let Some(sender) = channels
                            .read()
                            .ok()
                            .and_then(|map| map.get(&envelope.channel).cloned())
                        {
                            let _ = sender.send(envelope);
                        }
                    }
                });
            }
            AcceptedLiveTopicPublication::Coalesced => {}
        }
        Ok(())
    }

    fn subscriber_count(&self, channel: &str) -> usize {
        self.channels
            .read()
            .ok()
            .and_then(|m| m.get(channel).map(|tx| tx.receiver_count()))
            .unwrap_or(0)
    }

    async fn track_member(
        &self,
        channel: &str,
        member_id: &str,
        info: Value,
    ) -> Result<(), FrameworkError> {
        let mut map = self.members.write().await;
        map.entry(channel.to_string())
            .or_default()
            .insert(member_id.to_string(), info);
        Ok(())
    }

    async fn untrack_member(&self, channel: &str, member_id: &str) -> Result<(), FrameworkError> {
        let mut map = self.members.write().await;
        if let Some(ch_members) = map.get_mut(channel) {
            ch_members.remove(member_id);
            // Forget a channel whose last member left. A parameterized
            // presence channel (`presence.room.{id}`) churns through
            // distinct names, and an empty map kept per name would grow
            // with history instead of with the live members. The Pusher
            // and fanout hubs keep their members here too.
            if ch_members.is_empty() {
                map.remove(channel);
            }
        }
        Ok(())
    }

    async fn list_members(&self, channel: &str) -> Vec<Value> {
        let map = self.members.read().await;
        map.get(channel)
            .map(|ch| ch.values().cloned().collect())
            .unwrap_or_default()
    }
}

impl InMemoryBroadcastHub {
    fn publish_live_signal(&self, signal: LiveTopicSignal, except: Option<LiveTopicExclusion>) {
        self.publish_framework(live_topic_envelope(signal, except));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn publish_with_no_subscribers_is_silent() {
        let hub = InMemoryBroadcastHub::new();
        // Should not panic, should not log error, and returns Ok.
        hub.publish(BroadcastEnvelope::new("lonely", "Tick", json!({})))
            .await
            .expect("in-memory publish is infallible");
    }

    #[tokio::test]
    async fn publish_without_subscribers_does_not_create_channel() {
        // Publishes to never-subscribed names must not park senders
        // forever - that was the original growth vector (`user.{id}` etc.).
        let hub = InMemoryBroadcastHub::new();
        for i in 0..100 {
            hub.publish(BroadcastEnvelope::new(
                format!("user.{i}.orders"),
                "Tick",
                json!({}),
            ))
            .await
            .unwrap();
        }
        assert_eq!(hub.channel_count(), 0);
    }

    #[tokio::test]
    async fn new_channel_creation_sweeps_dead_siblings() {
        let hub = InMemoryBroadcastHub::new();
        // Spin up two live channels.
        let _a = hub.subscribe("user.1.orders");
        let _b = hub.subscribe("user.2.orders");
        // And a transient one that loses its only receiver.
        {
            let _gone = hub.subscribe("user.3.orders");
        }
        assert_eq!(hub.channel_count(), 3);

        // Force the slow path (write-lock + sweep) by subscribing to a
        // new channel.
        let _c = hub.subscribe("user.4.orders");

        // Three live (1, 2, 4); user.3 evicted.
        assert_eq!(hub.channel_count(), 3);
    }

    /// IDENTITY-016: a presence channel whose last member leaves must be
    /// forgotten. A parameterized presence channel (`presence.room.{id}`)
    /// churns through distinct names; keeping an empty member map per name
    /// grows memory with history, not with the live working set.
    #[tokio::test]
    async fn untracking_the_last_member_forgets_the_presence_channel() {
        let hub = InMemoryBroadcastHub::new();
        for i in 0..50 {
            let channel = format!("presence.room.{i}");
            hub.track_member(&channel, "member", json!({ "id": i }))
                .await
                .unwrap();
            hub.untrack_member(&channel, "member").await.unwrap();
        }
        assert_eq!(
            hub.presence_channel_count().await,
            0,
            "every channel lost its only member, so none may stay resident"
        );

        // A channel that still has a member is kept, with that member.
        hub.track_member("presence.keep", "a", json!({ "id": "a" }))
            .await
            .unwrap();
        hub.track_member("presence.keep", "b", json!({ "id": "b" }))
            .await
            .unwrap();
        hub.untrack_member("presence.keep", "a").await.unwrap();
        assert_eq!(hub.presence_channel_count().await, 1);
        assert_eq!(
            hub.list_members("presence.keep").await,
            vec![json!({ "id": "b" })]
        );

        // Untracking from a channel that was never tracked creates nothing.
        hub.untrack_member("presence.never", "x").await.unwrap();
        assert_eq!(hub.presence_channel_count().await, 1);
    }

    #[tokio::test]
    async fn publish_rejects_reserved_double_underscore_prefix() {
        // The double-underscore prefix is reserved for framework meta-channels
        // such as __presence__. The ChannelRegistry blocks `__`-prefixed names
        // at registration; the matching guard at the publish boundary keeps a
        // server-side caller - or a compromised handler - from injecting
        // phantom envelopes onto a reserved meta-channel.
        let hub = InMemoryBroadcastHub::new();

        let err = hub
            .publish(BroadcastEnvelope::new(
                "__presence__",
                "member_added",
                json!({ "spoof": true }),
            ))
            .await
            .expect_err("publish to __presence__ must be rejected");

        let msg = err.to_string();
        assert!(
            msg.contains("reserved prefix '__'"),
            "error must name the reserved prefix; got: {msg}"
        );

        let err2 = hub
            .publish(BroadcastEnvelope::new("__rpc__", "Tick", json!({})))
            .await
            .expect_err("any __-prefixed channel must be rejected, not just __presence__");
        assert!(err2.to_string().contains("reserved prefix '__'"));

        // Non-reserved publishes still succeed.
        hub.publish(BroadcastEnvelope::new("chat.42", "Tick", json!({})))
            .await
            .expect("non-reserved channel still publishes");
    }

    #[tokio::test]
    async fn live_subscriber_survives_concurrent_eviction_sweep() {
        // Regression guard: a fast-path subscribe must hand back a
        // Receiver minted **before** the read lock is released. If the
        // sender were cloned and the lock dropped, a concurrent sweep
        // could see receiver_count == 0 and evict before .subscribe()
        // runs - the new subscriber would silently miss every event.
        let hub = Arc::new(InMemoryBroadcastHub::new());

        let live = hub.subscribe("hot.chan");
        // `live` is a Receiver, sender stays in the map. Now exercise
        // the slow-path sweep over and over via a churn channel while
        // a second subscriber races in on the hot channel.
        let racer = Arc::clone(&hub);
        let race = tokio::spawn(async move {
            // Subscribe under the read-lock fast path.
            let mut rx = racer.subscribe("hot.chan");
            // Publish from the same task - the receiver was created
            // under the read guard so it MUST see the event.
            racer
                .publish(BroadcastEnvelope::new("hot.chan", "Ping", json!({})))
                .await
                .unwrap();
            rx.recv().await.expect("racer receives despite sweep")
        });

        // Generate sweep pressure by creating-and-dropping siblings.
        for i in 0..16 {
            let _ = hub.subscribe(&format!("sweep.{i}"));
        }
        let got = race.await.unwrap();
        assert_eq!(got.event, "Ping");
        drop(live);
    }

    #[tokio::test]
    async fn signal_005_in_memory_cursor_and_payload_are_data_free() {
        let hub = InMemoryBroadcastHub::new();
        let mut receiver = hub
            .subscribe_live_topic("orders.42")
            .expect("in-memory hub supports live topics");

        hub.publish_live_topic(
            "orders.42",
            &["summary".to_owned(), "summary".to_owned()],
            Some(crate::broadcasting::LiveTopicExclusion {
                socket: "socket-a".to_owned(),
                sender: crate::broadcasting::LiveTopicSender::Principal("alice".to_owned()),
            }),
            Duration::from_millis(100),
        )
        .await
        .expect("first signal publishes");

        let envelope = receiver.recv().await.expect("subscriber receives signal");
        assert_eq!(envelope.channel, "__live.topic.orders.42");
        assert_eq!(envelope.event, "inertia.live");
        assert_eq!(envelope.except.as_deref(), Some("socket-a"));
        assert_eq!(
            envelope.data,
            json!({"topic": "orders.42", "cursor": 1, "props": ["summary"], "exclusion": {"socket": "socket-a", "sender": {"kind": "principal", "id": "alice"}}})
        );
        assert_eq!(hub.live_topic_cursor("orders.42").await.unwrap(), 1);
    }

    #[tokio::test]
    async fn signal_005_in_memory_coalesces_props_and_mixed_exclusions() {
        let hub = InMemoryBroadcastHub::new();
        let mut receiver = hub
            .subscribe_live_topic("dashboard")
            .expect("in-memory hub supports live topics");
        let interval = Duration::from_millis(80);

        hub.publish_live_topic("dashboard", &["seed".to_owned()], None, interval)
            .await
            .unwrap();
        receiver.recv().await.unwrap();

        hub.publish_live_topic(
            "dashboard",
            &["stats".to_owned()],
            Some(crate::broadcasting::LiveTopicExclusion {
                socket: "socket-a".to_owned(),
                sender: crate::broadcasting::LiveTopicSender::Principal("alice".to_owned()),
            }),
            interval,
        )
        .await
        .unwrap();
        hub.publish_live_topic(
            "dashboard",
            &["activity".to_owned(), "stats".to_owned()],
            Some(crate::broadcasting::LiveTopicExclusion {
                socket: "socket-b".to_owned(),
                sender: crate::broadcasting::LiveTopicSender::Principal("alice".to_owned()),
            }),
            interval,
        )
        .await
        .unwrap();

        let envelope = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("coalesced signal is flushed")
            .expect("signal channel remains open");
        assert_eq!(envelope.except, None);
        assert_eq!(
            envelope.data,
            json!({
                "topic": "dashboard",
                "cursor": 3,
                "props": ["activity", "stats"]
            })
        );
        assert_eq!(hub.live_topic_cursor("dashboard").await.unwrap(), 3);
    }

    #[tokio::test]
    async fn signal_005_empty_props_dominate_a_coalesced_union() {
        let hub = InMemoryBroadcastHub::new();
        let mut receiver = hub.subscribe_live_topic("dashboard").unwrap();
        let interval = Duration::from_millis(80);

        hub.publish_live_topic("dashboard", &["seed".to_owned()], None, interval)
            .await
            .unwrap();
        receiver.recv().await.unwrap();
        hub.publish_live_topic("dashboard", &["stats".to_owned()], None, interval)
            .await
            .unwrap();
        hub.publish_live_topic("dashboard", &[], None, interval)
            .await
            .unwrap();

        let envelope = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(envelope.data["props"], json!([]));
        assert_eq!(envelope.data["cursor"], 3);
    }

    struct Signal005UnsupportedHub;

    #[async_trait]
    impl BroadcastHub for Signal005UnsupportedHub {
        fn subscribe(&self, _channel: &str) -> broadcast::Receiver<BroadcastEnvelope> {
            broadcast::channel(1).1
        }

        async fn publish(&self, _envelope: BroadcastEnvelope) -> Result<(), FrameworkError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn signal_005_custom_hubs_fail_explicitly_until_supported() {
        let hub = Signal005UnsupportedHub;
        assert!(hub.subscribe_live_topic("orders").is_err());
        assert!(hub.live_topic_cursor("orders").await.is_err());
        assert!(
            hub.publish_live_topic("orders", &[], None, Duration::from_millis(100))
                .await
                .is_err()
        );
    }
}
