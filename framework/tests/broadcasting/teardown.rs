//! IDENTITY-018: a broadcasting connection whose handler future is
//! cancelled - the server aborts WebSocket tasks at the end of its
//! shutdown drain - must still untrack its presence members, announce
//! `presence.left`, and stop its forwarder tasks. The in-memory hub has no
//! presence TTL, so a member that is never untracked stays visible for the
//! life of the process.
//!
//! `CancellableHandler` wraps the real `BroadcastingWsHandler` and drops
//! its `handle` future when the test says so, which is what a task abort
//! does to it.

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use suprnova::FrameworkError;
use suprnova::broadcasting::{
    BroadcastEnvelope, BroadcastHub, BroadcastingWsHandler, Channel, ChannelParams,
    ChannelRegistry, InMemoryBroadcastHub, PresenceChannel,
};
use suprnova::http::Request;
use suprnova::routing::Router;
use suprnova::ws::{OriginPolicy, WebSocketHandler, WsConfig, WsSocket};
use tokio::net::TcpListener;
use tokio::sync::{Notify, broadcast};
use tokio_tungstenite::tungstenite::Message;

const CHANNEL: &str = "presence.teardown";

struct PresenceRoom;

#[async_trait]
impl Channel for PresenceRoom {
    fn name(&self) -> &'static str {
        CHANNEL
    }
    fn presence_info(&self) -> Option<&dyn PresenceChannel> {
        Some(self)
    }
}

#[async_trait]
impl PresenceChannel for PresenceRoom {
    async fn member_info(
        &self,
        _req: &Request,
        _params: &ChannelParams,
    ) -> Result<Value, FrameworkError> {
        Ok(json!({ "user_id": 7 }))
    }
}

/// Runs the broadcasting handler until `cancel` is notified, then drops
/// its future mid-flight.
struct CancellableHandler {
    inner: BroadcastingWsHandler,
    cancel: Arc<Notify>,
}

#[async_trait]
impl WebSocketHandler for CancellableHandler {
    async fn handle(&self, socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
        tokio::select! {
            result = self.inner.handle(socket, req) => result,
            () = self.cancel.notified() => Ok(()),
        }
    }
}

/// An in-memory hub whose first `untrack_member` call never finishes: a
/// slow presence backend, so a test can cancel the connection while a
/// re-subscribe is cleaning up the member it replaces. Later calls pass
/// through.
struct StallFirstUntrackHub {
    inner: InMemoryBroadcastHub,
    stalled: AtomicBool,
    entered: Arc<Notify>,
}

#[async_trait]
impl BroadcastHub for StallFirstUntrackHub {
    fn subscribe(&self, channel: &str) -> broadcast::Receiver<BroadcastEnvelope> {
        self.inner.subscribe(channel)
    }

    async fn publish(&self, envelope: BroadcastEnvelope) -> Result<(), FrameworkError> {
        self.inner.publish(envelope).await
    }

    fn subscriber_count(&self, channel: &str) -> usize {
        self.inner.subscriber_count(channel)
    }

    async fn track_member(
        &self,
        channel: &str,
        member_id: &str,
        info: Value,
    ) -> Result<(), FrameworkError> {
        self.inner.track_member(channel, member_id, info).await
    }

    async fn untrack_member(&self, channel: &str, member_id: &str) -> Result<(), FrameworkError> {
        if !self.stalled.swap(true, Ordering::SeqCst) {
            self.entered.notify_one();
            std::future::pending::<()>().await;
        }
        self.inner.untrack_member(channel, member_id).await
    }

    async fn list_members(&self, channel: &str) -> Vec<Value> {
        self.inner.list_members(channel).await
    }
}

/// Serve the cancellable broadcasting handler over `hub` on a free port.
async fn spawn_server(hub: Arc<dyn BroadcastHub>, cancel: Arc<Notify>) -> u16 {
    let mut registry = ChannelRegistry::new();
    registry.register(PresenceRoom);
    let handler = CancellableHandler {
        inner: BroadcastingWsHandler::new(hub, Arc::new(registry)),
        cancel,
    };
    let router = Arc::new(Router::new().ws_with_config(
        "/ws/teardown",
        handler,
        WsConfig {
            origin_policy: OriginPolicy::AllowAny,
            ..Default::default()
        },
    ));
    let middleware = Arc::new(suprnova::middleware::MiddlewareRegistry::new());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let io = hyper_util::rt::TokioIo::new(stream);
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let service = hyper::service::service_fn(move |req| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move {
                        Ok::<_, std::convert::Infallible>(
                            suprnova::server::handle_request(router, middleware, req).await,
                        )
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .with_upgrades()
                    .await;
            });
        }
    });
    port
}

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn next_frame(ws: &mut WsStream) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(2), ws.next())
            .await
            .expect("frame in time")
            .expect("stream open")
            .expect("valid frame");
        match msg {
            Message::Text(t) => return serde_json::from_str(&t).unwrap(),
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("unexpected WS message: {other:?}"),
        }
    }
}

async fn subscribe(ws: &mut WsStream) {
    let frame = json!({ "action": "subscribe", "channel": CHANNEL });
    ws.send(Message::text(frame.to_string())).await.unwrap();
}

/// Connect and join the presence channel, consuming the frames the join
/// produces (`connected`, `subscribed`, `presence.here`, own
/// `presence.joined`).
async fn join(port: u16) -> WsStream {
    let url = format!("ws://127.0.0.1:{port}/ws/teardown");
    let mut ws = tokio_tungstenite::connect_async(&url).await.unwrap().0;
    assert_eq!(next_frame(&mut ws).await["action"], "connected");
    subscribe(&mut ws).await;
    assert_eq!(next_frame(&mut ws).await["action"], "subscribed");
    assert_eq!(next_frame(&mut ws).await["event"], "presence.here");
    assert_eq!(next_frame(&mut ws).await["event"], "presence.joined");
    ws
}

/// Poll until `check` holds or two seconds pass; returns the last result.
async fn eventually<F, Fut>(mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if check().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Wait for a `presence.left` envelope on `observer`.
async fn saw_presence_left(observer: &mut broadcast::Receiver<BroadcastEnvelope>) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, observer.recv()).await {
            Ok(Ok(envelope)) if envelope.event == "presence.left" => return true,
            Ok(Ok(_)) => continue,
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => return false,
        }
    }
}

/// The server-shutdown shape: the connection's handler future is dropped
/// while it waits for the next frame. Its member must be untracked and
/// announced, and its forwarder must stop holding the channel.
#[tokio::test]
async fn cancelled_connection_untracks_presence_and_stops_its_forwarder() {
    let hub = Arc::new(InMemoryBroadcastHub::new());
    let cancel = Arc::new(Notify::new());
    let port = spawn_server(hub.clone(), cancel.clone()).await;
    let mut observer = hub.subscribe(CHANNEL);

    let _ws = join(port).await;
    assert_eq!(hub.list_members(CHANNEL).await.len(), 1);
    assert_eq!(
        hub.subscriber_count(CHANNEL),
        2,
        "the observer and the connection's forwarder"
    );

    cancel.notify_one();

    assert!(
        eventually(|| async { hub.list_members(CHANNEL).await.is_empty() }).await,
        "a cancelled connection left its presence member tracked: {:?}",
        hub.list_members(CHANNEL).await
    );
    assert!(
        saw_presence_left(&mut observer).await,
        "a cancelled connection must still announce presence.left"
    );
    assert!(
        eventually(|| async { hub.subscriber_count(CHANNEL) == 1 }).await,
        "the cancelled connection's forwarder still holds a receiver: {} receivers",
        hub.subscriber_count(CHANNEL)
    );
}

/// The re-subscribe shape: the connection is cancelled while a
/// re-subscribe is still untracking the member it replaces. Neither the
/// replaced member nor any forwarder may survive.
#[tokio::test]
async fn connection_cancelled_during_resubscribe_cleanup_leaves_nothing_behind() {
    let entered = Arc::new(Notify::new());
    let hub = Arc::new(StallFirstUntrackHub {
        inner: InMemoryBroadcastHub::new(),
        stalled: AtomicBool::new(false),
        entered: entered.clone(),
    });
    let cancel = Arc::new(Notify::new());
    let port = spawn_server(hub.clone(), cancel.clone()).await;
    let mut observer = hub.subscribe(CHANNEL);

    let mut ws = join(port).await;
    assert_eq!(hub.list_members(CHANNEL).await.len(), 1);

    // Re-subscribe: the handler replaces the subscription and starts
    // untracking the old member, which stalls.
    subscribe(&mut ws).await;
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .expect("the re-subscribe reached the old member's untrack");

    cancel.notify_one();

    assert!(
        eventually(|| async { hub.list_members(CHANNEL).await.is_empty() }).await,
        "the member the re-subscribe was replacing stayed tracked: {:?}",
        hub.list_members(CHANNEL).await
    );
    assert!(
        saw_presence_left(&mut observer).await,
        "the replaced member must still be announced as left"
    );
    assert!(
        eventually(|| async { hub.subscriber_count(CHANNEL) == 1 }).await,
        "a forwarder survived the cancelled re-subscribe: {} receivers",
        hub.subscriber_count(CHANNEL)
    );
}
