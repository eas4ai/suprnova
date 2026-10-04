//! `BroadcastingWsHandler` - wires the JSON-envelope subscribe
//! protocol against a `BroadcastHub` + `ChannelRegistry`.
//!
//! Drop into `ws!()` with the resolved hub and registry:
//!
//! ```rust,no_run
//! # use std::sync::Arc;
//! # use suprnova::{ws, async_trait, Middleware, Next, Request, Response};
//! # use suprnova::{BroadcastingWsHandler, BroadcastHub, InMemoryBroadcastHub};
//! # use suprnova::broadcasting::ChannelRegistry;
//! # struct SessionMiddleware;
//! # impl SessionMiddleware { fn new() -> Self { SessionMiddleware } }
//! # #[async_trait]
//! # impl Middleware for SessionMiddleware {
//! #     async fn handle(&self, request: Request, next: Next) -> Response { next(request).await }
//! # }
//! # let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
//! # let registry = Arc::new(ChannelRegistry::new());
//! ws!("/ws/broadcast", BroadcastingWsHandler::new(hub, registry))
//!     .middleware(SessionMiddleware::new());
//! ```
//!
//! # Security note
//!
//! Client-initiated `Publish` frames are gated by **two** checks:
//!
//! 1. The connection MUST hold an authorized subscription to the
//!    target channel (i.e. an entry in the per-connection forwarders
//!    map placed there by a successful `Subscribe`). Publishes from
//!    connections that never subscribed - or whose subscription was
//!    rejected - are refused even if `authorize_publish` would have
//!    returned `true`. This mirrors the Pusher client-event contract
//!    where client events require an established private/presence
//!    subscription.
//! 2. `Channel::authorize_publish` is then consulted on the resolved
//!    channel. The default implementation returns `false` (deny), so
//!    only channels that explicitly override the hook accept client
//!    publishes.
//!
//! Unknown channels always reject. Server-side `hub.publish()` calls
//! bypass both gates entirely (server is already trusted).

use crate::FrameworkError;
use crate::broadcasting::channel::ChannelRegistry;
use crate::broadcasting::hub::{BroadcastEnvelope, BroadcastHub};
use crate::broadcasting::protocol::{ClientFrame, ServerFrame};
use crate::http::Request;
use crate::ws::{WebSocketHandler, WsSocket};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use tokio::task::JoinHandle;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Presence state carried alongside each forwarder.
// ---------------------------------------------------------------------------

/// A forwarder task that is aborted when its handle is dropped.
///
/// A plain `JoinHandle` detaches on drop. A forwarder waits on
/// `rx.recv()` while the hub keeps the channel's sender alive, so a
/// detached one lives until the next event on its channel, or forever.
/// Every path that loses the handle without the explicit teardown must
/// still stop the task: the connection future cancelled when the server
/// aborts WebSocket tasks at the end of its shutdown drain, or a
/// re-subscribe cancelled during its cleanup.
struct ForwarderTask(JoinHandle<()>);

impl Drop for ForwarderTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// A presence member this connection tracked on the hub. When the
/// subscription ends, the member is untracked and announced as
/// `presence.left`.
///
/// [`Self::leave`] does that on the normal paths, and the caller awaits
/// it. When the membership is dropped first instead - the connection
/// future is cancelled, or a `leave` in progress is cancelled - `Drop`
/// hands the same two steps to a spawned task. The in-memory hub has no
/// presence TTL, so a member nobody untracks stays visible for the life
/// of the process.
///
/// The cleanup runs at least once. A `leave` cancelled after its untrack
/// repeats it from `Drop`; untracking an absent member is a no-op, and
/// rosters are keyed by member id, so a repeated `presence.left` is too.
struct PresenceMembership {
    hub: Arc<dyn BroadcastHub>,
    channel: String,
    member_id: String,
    info: Value,
    /// Set once `leave` has finished; `Drop` then has nothing to do.
    left: bool,
}

impl PresenceMembership {
    /// Untrack the member and announce `presence.left`. `during` names the
    /// path that ended the subscription, for the logs.
    async fn leave(mut self, during: &'static str) {
        untrack_and_announce(
            &self.hub,
            &self.channel,
            &self.member_id,
            self.info.clone(),
            during,
        )
        .await;
        self.left = true;
    }
}

impl Drop for PresenceMembership {
    fn drop(&mut self) {
        if self.left {
            return;
        }
        let hub = Arc::clone(&self.hub);
        let channel = std::mem::take(&mut self.channel);
        let member_id = std::mem::take(&mut self.member_id);
        let info = self.info.take();
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn(async move {
                    untrack_and_announce(&hub, &channel, &member_id, info, "cancelled connection")
                        .await;
                });
            }
            Err(_) => tracing::warn!(
                channel = %channel,
                member_id = %member_id,
                "presence member left tracked: no Tokio runtime to run its cleanup on"
            ),
        }
    }
}

/// Untrack `member_id` on `channel` and publish its `presence.left`.
///
/// Every caller is a cleanup path whose subscription is already gone, so
/// a hub failure is logged, not returned: there is no client request left
/// to fail, and the publish still runs when the untrack fails.
async fn untrack_and_announce(
    hub: &Arc<dyn BroadcastHub>,
    channel: &str,
    member_id: &str,
    info: Value,
    during: &'static str,
) {
    if let Err(e) = hub.untrack_member(channel, member_id).await {
        tracing::warn!(
            error = %e,
            channel = %channel,
            member_id = %member_id,
            during,
            "presence untrack failed"
        );
    }
    if let Err(e) = hub
        .publish(BroadcastEnvelope::new(
            channel.to_string(),
            "presence.left",
            info,
        ))
        .await
    {
        tracing::warn!(
            channel = %channel,
            error = %e,
            during,
            "broadcasting handler: presence.left publish failed"
        );
    }
}

/// One subscription in the per-connection map. Dropping it stops the
/// forwarder and, if the subscription joined a presence channel, cleans
/// up the member; see [`ForwarderTask`] and [`PresenceMembership`].
struct ForwarderEntry {
    forwarder: ForwarderTask,
    presence: Option<PresenceMembership>,
}

impl ForwarderEntry {
    /// End the subscription: stop the forwarder, then untrack and announce
    /// the presence member, if any. The forwarder goes first because the
    /// subscription is over; anything it forwarded now would be stale.
    async fn end(self, during: &'static str) {
        let Self {
            forwarder,
            presence,
        } = self;
        drop(forwarder);
        if let Some(membership) = presence {
            membership.leave(during).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// The framework's reusable WS handler that implements the
/// broadcasting subscribe/unsubscribe/publish protocol over the
/// JSON envelope wire format defined in `protocol.rs`.
///
/// Construct with `BroadcastingWsHandler::new(hub, registry)` and
/// register with `Router::ws`:
///
/// ```rust,no_run
/// # use std::sync::Arc;
/// # use suprnova::{Router, BroadcastingWsHandler, BroadcastHub, InMemoryBroadcastHub};
/// # use suprnova::broadcasting::ChannelRegistry;
/// # let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
/// # let registry = Arc::new(ChannelRegistry::new());
/// let handler = BroadcastingWsHandler::new(hub.clone(), registry.clone());
/// let router = Router::new().ws("/ws/broadcast", handler);
/// ```
/// Default per-connection cap on distinct channel subscriptions. A
/// well-behaved client subscribes to a handful of channels; the cap
/// only matters as a guardrail against a malicious or buggy client
/// minting thousands of `orders.{id}` permutations on one connection
/// to fill the per-connection forwarder map and tie up tasks.
pub const DEFAULT_MAX_SUBSCRIPTIONS_PER_CONNECTION: usize = 100;

/// WebSocket handler that bridges the JSON-envelope wire protocol
/// (`ClientFrame` / `ServerFrame`) to a [`BroadcastHub`]. Subscribers
/// connect, the handler authorizes channel joins via the
/// [`ChannelRegistry`], forwards published frames, and applies the
/// per-connection subscription cap.
pub struct BroadcastingWsHandler {
    hub: Arc<dyn BroadcastHub>,
    registry: Arc<ChannelRegistry>,
    /// Per-connection cap on the count of distinct channel keys held
    /// in the `forwarders` map. Re-subscribes to an already-present
    /// channel are not counted (idempotent), so a client can refresh
    /// an existing subscription regardless of the cap. See
    /// [`Self::with_max_subscriptions`].
    max_subscriptions: usize,
}

impl BroadcastingWsHandler {
    /// Create a new handler backed by the given hub and channel registry.
    ///
    /// `hub` accepts any `Arc<H>` where `H: BroadcastHub`; the
    /// coercion to `Arc<dyn BroadcastHub>` happens at the call site.
    pub fn new(hub: Arc<dyn BroadcastHub>, registry: Arc<ChannelRegistry>) -> Self {
        Self {
            hub,
            registry,
            max_subscriptions: DEFAULT_MAX_SUBSCRIPTIONS_PER_CONNECTION,
        }
    }

    /// Override the per-connection subscription cap. Once a connection
    /// holds `max` distinct channel keys in its forwarder map, further
    /// `Subscribe` frames for *new* channel names are rejected with a
    /// `ServerFrame::Error { reason: "subscription limit reached" }` -
    /// re-subscribes to an already-active channel are still allowed
    /// (they replace the forwarder in place and don't grow the map).
    ///
    /// The default is [`DEFAULT_MAX_SUBSCRIPTIONS_PER_CONNECTION`]
    /// (`100`). Lower it on memory-constrained deployments or when an
    /// app declaratively bounds the channel surface; raise it for a
    /// handful of trusted-internal clients that legitimately fan out
    /// many channels per socket.
    pub fn with_max_subscriptions(mut self, max: usize) -> Self {
        self.max_subscriptions = max;
        self
    }
}

#[async_trait]
impl WebSocketHandler for BroadcastingWsHandler {
    async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
        // Per-channel forwarder entries. Ended on unsubscribe or when
        // the connection ends; dropping one ends it too.
        let forwarders: Arc<Mutex<HashMap<String, ForwarderEntry>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Outbound mpsc: forwarders push serialised ServerFrame::Event
        // strings here; the select! arm below drains them to the socket.
        // A String channel, drained by this handler, keeps serialisation
        // and the order of frames inside this module. A `WsSender` in each
        // forwarder would send on the socket's queue directly.
        let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::channel::<String>(64);

        // Assign this connection a socket id and announce it first, so the
        // client can echo it as `X-Socket-ID` and a server-side
        // `broadcast_to_others` can exclude this connection. Mirrors Pusher's
        // `connection_established`.
        let socket_id = Uuid::new_v4().to_string();
        socket
            .send_text(
                serde_json::to_string(&ServerFrame::Connected {
                    socket_id: socket_id.clone(),
                })
                .unwrap_or_default(),
            )
            .await?;

        // Inner-async-block pattern: every exit out of the loop body
        // (clean break on `Ok(None)`, `?` on outbound/inbound IO, `?` from
        // helper functions) lands here in `result`, after which the
        // teardown loop below runs unconditionally. Without this wrapping
        // the typical browser disconnect - tab close, network drop, OS
        // RST - would skip teardown entirely: presence members would leak
        // forever, forwarder tasks would detach blocked on `rx.recv()`,
        // and the hub channel would stay pinned by their receiver count.
        let result: Result<(), FrameworkError> = async {
            loop {
                tokio::select! {
                    // Outbound arm: a forwarder pushed an event.
                    Some(text) = outbound_rx.recv() => {
                        socket.send_text(text).await?;
                    }
                    // Inbound arm: client sent a frame.
                    inbound = socket.recv_text() => {
                        let text = match inbound? {
                            Some(t) => t,
                            None => break, // connection closed cleanly
                        };

                        match serde_json::from_str::<ClientFrame>(&text) {
                            Ok(ClientFrame::Subscribe { channel, data }) => {
                                handle_subscribe(
                                    &channel,
                                    &data,
                                    &req,
                                    &self.hub,
                                    &self.registry,
                                    &forwarders,
                                    &outbound_tx,
                                    &socket_id,
                                    self.max_subscriptions,
                                    &mut socket,
                                )
                                .await?;
                            }
                            Ok(ClientFrame::Unsubscribe { channel }) => {
                                handle_unsubscribe(
                                    &channel,
                                    &forwarders,
                                    &mut socket,
                                )
                                .await?;
                            }
                            Ok(ClientFrame::Publish { channel, event, data }) => {
                                // Two-stage publish authorization. Fail closed on:
                                //   - Connection never subscribed: no entry in
                                //     `forwarders` → reject (Pusher client-event
                                //     contract requires an established subscription)
                                //   - Unknown channel: no impl to consult → reject
                                //   - Channel says no: reject with Error frame
                                //   - Channel says yes: proceed to hub.publish
                                let is_subscribed = {
                                    let map = forwarders.lock().await;
                                    map.contains_key(&channel)
                                };

                                let allowed = if !is_subscribed {
                                    false
                                } else {
                                    match self.registry.resolve(&channel) {
                                        Some((ch, params)) => {
                                            ch.authorize_publish(&req, &params, &event, &data).await
                                        }
                                        None => false,
                                    }
                                };

                                if !allowed {
                                    let err = ServerFrame::Error {
                                        channel: Some(channel.clone()),
                                        reason: "publish unauthorized".into(),
                                    };
                                    socket
                                        .send_text(
                                            serde_json::to_string(&err).unwrap_or_default(),
                                        )
                                        .await?;
                                } else {
                                    // Client publishes are not socket-excluded - the
                                    // publisher receives its own event like any other
                                    // subscriber (see broadcasting docs).
                                    let chan_for_err = channel.clone();
                                    if let Err(e) = self
                                        .hub
                                        .publish(BroadcastEnvelope::new(channel, event, data))
                                        .await
                                    {
                                        // Surface broker / fanout failures back to
                                        // the originating client so it knows the
                                        // publish didn't reach other processes.
                                        let err = ServerFrame::Error {
                                            channel: Some(chan_for_err),
                                            reason: format!("publish failed: {e}"),
                                        };
                                        socket
                                            .send_text(
                                                serde_json::to_string(&err).unwrap_or_default(),
                                            )
                                            .await?;
                                    }
                                }
                            }
                            Err(e) => {
                                let err = ServerFrame::Error {
                                    channel: None,
                                    reason: format!("malformed envelope: {e}"),
                                };
                                socket
                                    .send_text(serde_json::to_string(&err).unwrap_or_default())
                                    .await?;
                            }
                        }
                    }
                }
            }
            Ok(())
        }
        .await;

        // Teardown runs on every exit path out of the loop, not just the
        // clean `Ok(None)` break above: stop each forwarder, then untrack
        // and announce each presence member. It is awaited here so a
        // returning handler has finished its cleanup. A hub failure is
        // logged and does not replace the exit reason in `result`.
        //
        // The one exit this code cannot see is the future itself being
        // dropped - the server aborts WebSocket tasks when its shutdown
        // drain times out. That path drops `forwarders` and every entry
        // in it, and the entries' own `Drop` does the same cleanup.
        let entries: Vec<ForwarderEntry> = {
            let mut map = forwarders.lock().await;
            map.drain().map(|(_, entry)| entry).collect()
        };
        for entry in entries {
            entry.end("connection close").await;
        }

        // Re-raise the inner loop's exit reason after teardown ran.
        result
    }
}

// ---------------------------------------------------------------------------
// Internal helpers (free functions to avoid the borrow-checker difficulties
// of `&self` methods that also mutably borrow `socket`).
// ---------------------------------------------------------------------------

/// Subscribe to `channel`, **then** snapshot its presence roster.
///
/// The order is the entire content of this function, which is why it is a
/// function at all rather than two lines inline.
///
/// It used to run the other way round: `list_members` first, then
/// `subscribe`. Anyone who joined in the gap between those two calls
/// appeared in *neither* - not in the snapshot, because they had not
/// joined when it was taken; and not in the event stream, because the
/// subscription did not exist yet when their `presence.joined` was
/// published. The new subscriber's roster was then permanently short, with
/// no error and no way to notice short of comparing rosters between
/// clients. Only a re-subscribe would repair it.
///
/// Subscribing first cannot lose anyone. A member who joins in the window
/// is published into the now-live receiver, and may *also* appear in the
/// snapshot taken a moment later - so the failure mode inverts from a
/// silent omission to an at-most-once duplicate join for a member already
/// in the roster. Presence rosters are keyed by member id, so that
/// duplicate is idempotent. Trading a permanent omission for an idempotent
/// repeat is the whole trade, and it is not a close call.
///
/// `want_roster` keeps a non-presence channel from paying for a roster
/// read it will discard.
async fn subscribe_then_snapshot(
    hub: &Arc<dyn BroadcastHub>,
    channel: &str,
    want_roster: bool,
) -> (broadcast::Receiver<BroadcastEnvelope>, Option<Vec<Value>>) {
    let rx = hub.subscribe(channel);
    let roster = if want_roster {
        Some(hub.list_members(channel).await)
    } else {
        None
    };
    (rx, roster)
}

// The subscribe path needs all these parameters; a struct would require
// explicit lifetime annotations that add more noise than the lint saves.
#[allow(clippy::too_many_arguments)]
async fn handle_subscribe(
    channel: &str,
    data: &serde_json::Value,
    req: &Request,
    hub: &Arc<dyn BroadcastHub>,
    registry: &Arc<ChannelRegistry>,
    forwarders: &Arc<Mutex<HashMap<String, ForwarderEntry>>>,
    outbound_tx: &tokio::sync::mpsc::Sender<String>,
    socket_id: &str,
    max_subscriptions: usize,
    socket: &mut WsSocket,
) -> Result<(), FrameworkError> {
    // Per-connection subscription cap. Re-subscribes to an existing
    // channel are exempt (they REPLACE the forwarder in place - see
    // the `map.insert` below - so the map size doesn't grow);
    // first-time subscribes to a brand-new channel name count against
    // the cap. Without this gate a malicious client could subscribe
    // to `orders.{id}` with thousands of distinct ids on one socket
    // and inflate the per-connection forwarder map to exhaust memory
    // and tokio task slots. Check this BEFORE `hub.subscribe` and the
    // `tokio::spawn` so we never spawn a forwarder we'd refuse to
    // register - frames on a single connection are processed
    // sequentially in the `select!` loop, so reading the map here and
    // inserting later is race-free per connection.
    {
        let map = forwarders.lock().await;
        if !map.contains_key(channel) && map.len() >= max_subscriptions {
            let err = ServerFrame::Error {
                channel: Some(channel.to_string()),
                reason: "subscription limit reached".into(),
            };
            drop(map);
            socket
                .send_text(serde_json::to_string(&err).unwrap_or_default())
                .await?;
            return Ok(());
        }
    }

    // Resolve the channel from the registry, capturing any params bound from a
    // parameterized name (e.g. `{id}` for `orders.{id}` subscribed as `orders.42`).
    let Some((ch, params)) = registry.resolve(channel) else {
        let err = ServerFrame::Error {
            channel: Some(channel.to_string()),
            reason: "no such channel".into(),
        };
        socket
            .send_text(serde_json::to_string(&err).unwrap_or_default())
            .await?;
        return Ok(());
    };

    // Authorize the subscription.
    if !ch.authorize(req, &params, data).await {
        let err = ServerFrame::Error {
            channel: Some(channel.to_string()),
            reason: "unauthorized".into(),
        };
        socket
            .send_text(serde_json::to_string(&err).unwrap_or_default())
            .await?;
        return Ok(());
    }

    // This subscriber's own presence identity. Derived from the request,
    // not from hub state, so it can be built before subscribing.
    let presence_identity: Option<(String, Value)> = if let Some(pc) = ch.presence_info() {
        let info = pc.member_info(req, &params).await?;
        Some((Uuid::new_v4().to_string(), info))
    } else {
        None
    };

    // Subscribe to the hub, then snapshot the roster - in that order, and
    // the order is the whole point. See `subscribe_then_snapshot`.
    let (mut rx, roster) = subscribe_then_snapshot(hub, channel, presence_identity.is_some()).await;

    let presence_bootstrap: Option<(Vec<Value>, String, Value)> =
        presence_identity.map(|(member_id, info)| (roster.unwrap_or_default(), member_id, info));

    // Spawn a forwarder.
    let tx = outbound_tx.clone();
    let self_socket = socket_id.to_string();
    // Capture the channel name so the forwarder can name the channel
    // when it emits a Lagged frame after a `broadcast::RecvError::Lagged(_)`.
    let forwarder_channel = channel.to_string();
    let forwarder = ForwarderTask(tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(envelope) => {
                    // Skip the connection this broadcast excludes
                    // (`broadcast_to_others` / per-dispatch `except`); every
                    // other subscriber still receives it.
                    if envelope.except.as_deref() == Some(self_socket.as_str()) {
                        continue;
                    }
                    let frame = ServerFrame::Event {
                        channel: envelope.channel,
                        event: envelope.event,
                        data: envelope.data,
                    };
                    let text = match serde_json::to_string(&frame) {
                        Ok(t) => t,
                        Err(_) => continue,
                    };
                    if tx.send(text).await.is_err() {
                        return; // outbound channel closed - connection gone
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    // The subscriber fell behind the per-channel ring
                    // buffer; `skipped` envelopes were dropped on this
                    // connection. Surface this so the client knows its
                    // local state is stale and must refetch - silently
                    // skipping events would let bugs hide as "we lost a
                    // tick" rather than "the client's state diverged
                    // from the server's".
                    let frame = ServerFrame::Lagged {
                        channel: forwarder_channel.clone(),
                        skipped,
                    };
                    if let Ok(text) = serde_json::to_string(&frame)
                        && tx.send(text).await.is_err()
                    {
                        return; // outbound closed mid-Lagged send
                    }
                    continue;
                }
            }
        }
    }));

    // Destructure bootstrap data - used after the forwarder is inserted.
    let (presence_here_members, presence_member_id, presence_info) =
        if let Some((existing, mid, info)) = presence_bootstrap {
            (Some(existing), Some(mid), Some(info))
        } else {
            (None, None, None)
        };

    // Register the new subscription in place of any existing one for this
    // channel (idempotent re-subscribe), in one step under the lock. From
    // here on the map owns the new forwarder and presence member, so a
    // connection cancelled at any later await still cleans them up. The
    // membership is built under the lock, with no await before the insert,
    // so a cancellation cannot drop one that was never registered.
    let replaced = {
        let mut map = forwarders.lock().await;
        let presence = match (presence_member_id.as_deref(), presence_info.as_ref()) {
            (Some(mid), Some(info)) => Some(PresenceMembership {
                hub: Arc::clone(hub),
                channel: channel.to_string(),
                member_id: mid.to_string(),
                info: info.clone(),
                left: false,
            }),
            _ => None,
        };
        map.insert(
            channel.to_string(),
            ForwarderEntry {
                forwarder,
                presence,
            },
        )
    };

    // End the subscription this one replaced, outside the lock. A hub
    // failure is logged, not returned: the client just re-subscribed, and
    // the new subscription should not fail because the old member's
    // `presence.left` could not be forwarded cross-process.
    if let Some(old) = replaced {
        old.end("re-subscribe").await;
    }

    // Send Subscribed ack first.
    let ack = ServerFrame::Subscribed {
        channel: channel.to_string(),
    };
    socket
        .send_text(serde_json::to_string(&ack).unwrap_or_default())
        .await?;

    // Presence post-subscribe steps - forwarder is now live so
    // hub.subscribe() receiver is already active.
    if let (Some(existing), Some(mid), Some(info)) =
        (presence_here_members, presence_member_id, presence_info)
    {
        // Track member AFTER taking the snapshot so self is absent from
        // the presence.here payload (standard Pusher behaviour). A
        // producer-down failure here surfaces to the WS handler so the
        // client sees a real error instead of joining a presence
        // channel that peer instances will never learn about.
        hub.track_member(channel, &mid, info.clone()).await?;

        // presence.here - sent directly to this socket only (not via hub).
        let here = ServerFrame::Event {
            channel: channel.to_string(),
            event: "presence.here".into(),
            data: json!({ "members": existing }),
        };
        socket
            .send_text(serde_json::to_string(&here).unwrap_or_default())
            .await?;

        // presence.joined - published via hub so all subscribers receive it
        // (including the new subscriber via their forwarder - that's the
        // standard Pusher self-join behaviour; clients filter by member_id).
        // A hub failure here is the subscriber being announced; surface
        // via an Error frame on this socket. The local member entry
        // already exists, so cross-process fanout is the only thing
        // that could have dropped.
        if let Err(e) = hub
            .publish(BroadcastEnvelope::new(
                channel.to_string(),
                "presence.joined",
                info,
            ))
            .await
        {
            let err = ServerFrame::Error {
                channel: Some(channel.to_string()),
                reason: format!("presence.joined publish failed: {e}"),
            };
            socket
                .send_text(serde_json::to_string(&err).unwrap_or_default())
                .await?;
        }
    }

    Ok(())
}

async fn handle_unsubscribe(
    channel: &str,
    forwarders: &Arc<Mutex<HashMap<String, ForwarderEntry>>>,
    socket: &mut WsSocket,
) -> Result<(), FrameworkError> {
    let entry = {
        let mut map = forwarders.lock().await;
        map.remove(channel)
    };

    // A hub failure while ending the subscription is logged, not returned:
    // the client still gets its Unsubscribed ack below.
    if let Some(entry) = entry {
        entry.end("unsubscribe").await;
    }

    let ack = ServerFrame::Unsubscribed {
        channel: channel.to_string(),
    };
    socket
        .send_text(serde_json::to_string(&ack).unwrap_or_default())
        .await?;
    Ok(())
}

#[cfg(test)]
mod presence_ordering_tests {
    //! P2-08 - a member joining between the roster snapshot and the
    //! subscription used to vanish from the new subscriber's roster
    //! permanently.
    //!
    //! The window is a genuine race, so these do not try to hit it by
    //! timing. `JoinDuringSnapshotHub` wraps a real hub and performs the
    //! interleaving join *itself*, immediately after delegating
    //! `list_members` - modelling "somebody joined the instant after the
    //! snapshot was taken" exactly, on every run. The same decorator trick
    //! as `queue_fault_injection.rs`: make the race a scripted step rather
    //! than a sleep and a prayer.

    use super::*;
    use crate::broadcasting::hub::InMemoryBroadcastHub;

    const CHANNEL: &str = "presence-room";
    const LATE_JOINER: &str = "late-joiner";

    /// Delegates everything, but stages a join right after the roster is
    /// read - inside the window the old ordering left open.
    struct JoinDuringSnapshotHub {
        inner: InMemoryBroadcastHub,
    }

    #[async_trait]
    impl BroadcastHub for JoinDuringSnapshotHub {
        fn subscribe(&self, channel: &str) -> broadcast::Receiver<BroadcastEnvelope> {
            self.inner.subscribe(channel)
        }

        async fn publish(&self, envelope: BroadcastEnvelope) -> Result<(), FrameworkError> {
            self.inner.publish(envelope).await
        }

        async fn track_member(
            &self,
            channel: &str,
            member_id: &str,
            info: Value,
        ) -> Result<(), FrameworkError> {
            self.inner.track_member(channel, member_id, info).await
        }

        async fn list_members(&self, channel: &str) -> Vec<Value> {
            // Read the roster first, so the joiner is genuinely absent
            // from the snapshot the caller receives...
            let snapshot = self.inner.list_members(channel).await;

            // ...then join, and announce it. Whether the caller ever learns
            // about this member is decided entirely by whether it had
            // already subscribed before calling us.
            self.inner
                .track_member(channel, LATE_JOINER, json!({"id": LATE_JOINER}))
                .await
                .expect("in-memory track_member cannot fail");
            self.inner
                .publish(BroadcastEnvelope::new(
                    channel.to_string(),
                    "presence.joined",
                    json!({"id": LATE_JOINER}),
                ))
                .await
                .expect("in-memory publish cannot fail");

            snapshot
        }
    }

    fn hub() -> Arc<dyn BroadcastHub> {
        Arc::new(JoinDuringSnapshotHub {
            inner: InMemoryBroadcastHub::new(),
        })
    }

    /// The regression. Somebody joins in the window; the new subscriber
    /// must learn about them one way or the other.
    ///
    /// It does not matter *which* way - a roster is a set, and a member
    /// present in the snapshot or announced on the stream ends up in the
    /// same place. What matters is that "neither" is impossible.
    #[tokio::test]
    async fn a_member_joining_during_the_snapshot_is_not_lost() {
        let hub = hub();

        let (mut rx, roster) = subscribe_then_snapshot(&hub, CHANNEL, true).await;
        let roster = roster.expect("a presence channel asked for its roster");

        let in_snapshot = roster
            .iter()
            .any(|m| m.get("id").and_then(Value::as_str) == Some(LATE_JOINER));

        let in_stream = match rx.try_recv() {
            Ok(envelope) => {
                envelope.event == "presence.joined"
                    && envelope.data.get("id").and_then(Value::as_str) == Some(LATE_JOINER)
            }
            Err(_) => false,
        };

        assert!(
            in_snapshot || in_stream,
            "a member who joined between the subscribe and the snapshot \
             appeared in neither. Their join is gone for good: the roster \
             is permanently short and only a re-subscribe repairs it. This \
             is the defect - `list_members` ran before `subscribe`."
        );
    }

    /// Pins the ordering itself, so a refactor that reverts it fails here
    /// even if the assertion above were somehow satisfied another way. The
    /// joiner must arrive on the *stream*, which is only possible if the
    /// subscription already existed when `list_members` published it.
    #[tokio::test]
    async fn the_subscription_is_live_before_the_roster_is_read() {
        let hub = hub();

        let (mut rx, _roster) = subscribe_then_snapshot(&hub, CHANNEL, true).await;

        let envelope = rx.try_recv().expect(
            "the join published during `list_members` must have landed in \
             the receiver, which is only true if `subscribe` ran first. \
             Nothing received means the roster was snapshotted before the \
             subscription existed.",
        );
        assert_eq!(envelope.event, "presence.joined");
        assert_eq!(
            envelope.data.get("id").and_then(Value::as_str),
            Some(LATE_JOINER)
        );
    }

    /// A non-presence channel must not pay for a roster read it discards.
    #[tokio::test]
    async fn a_non_presence_channel_reads_no_roster() {
        let hub = hub();

        let (mut rx, roster) = subscribe_then_snapshot(&hub, CHANNEL, false).await;

        assert!(roster.is_none(), "no roster was asked for");
        assert!(
            rx.try_recv().is_err(),
            "`list_members` must not have been called at all - the \
             decorator's staged join is the proof it was"
        );
    }
}
