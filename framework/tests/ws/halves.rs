//! The two halves of a `WsSocket`: a `WsSender` that sends from any task
//! and a `WsReceiver` that reads.
//!
//! The first group drives a socket over an in-memory pipe. The last test
//! goes through the server's upgrade path, because what it asserts is the
//! server's: the connection ends with the handler, and a sender that the
//! handler gave away does not hold it open.

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use suprnova::FrameworkError;
use suprnova::http::Request;
use suprnova::middleware::MiddlewareRegistry;
use suprnova::routing::Router;
use suprnova::ws::{OriginPolicy, WebSocketHandler, WsConfig, WsSender, WsSocket};
use tokio::io::{DuplexStream, duplex};
use tokio::net::TcpListener;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::Role;

/// Longer than any step here takes, and short enough that a step which
/// never finishes fails the test and does not hang the run.
const DEADLINE: Duration = Duration::from_secs(5);

async fn within<F: Future>(step: F) -> F::Output {
    tokio::time::timeout(DEADLINE, step)
        .await
        .expect("the step finished before the deadline")
}

/// The next frame the client reads.
async fn next_frame<S>(client: &mut WebSocketStream<S>) -> Message
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    within(client.next())
        .await
        .expect("the connection is open")
        .expect("the frame is readable")
}

/// A server-side socket, the client that talks to it, and the count of
/// unanswered pings that the socket shares with a heartbeat.
async fn pair() -> (WsSocket, WebSocketStream<DuplexStream>, Arc<AtomicUsize>) {
    let (client_io, server_io) = duplex(64 * 1024);
    let server = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
    let client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
    let missed_pings = Arc::new(AtomicUsize::new(0));
    let socket = WsSocket::from_stream_with_heartbeat(server, missed_pings.clone());
    (socket, client, missed_pings)
}

/// The regression the issue reports: nothing could send while the handler
/// waited for the peer.
#[tokio::test]
async fn a_sender_sends_while_the_handler_waits_in_recv() {
    let (mut socket, mut client, _) = pair().await;
    let sender = socket.sender();

    let (at_the_read, handler_is_at_the_read) = tokio::sync::oneshot::channel();
    let handler = tokio::spawn(async move {
        let _ = at_the_read.send(());
        socket.recv_text().await
    });
    // The handler runs up to its read, and one turn of the scheduler
    // later it is in it: the client has sent nothing, so the read has
    // nothing to return and the task waits there.
    within(handler_is_at_the_read)
        .await
        .expect("the handler task started");
    tokio::task::yield_now().await;
    assert!(!handler.is_finished(), "the handler waits for the peer");

    sender
        .send_text("pushed")
        .await
        .expect("the queue accepts the frame");
    let frame = next_frame(&mut client).await;
    assert_eq!(frame.to_text().expect("a text frame"), "pushed");
    assert!(
        !handler.is_finished(),
        "the handler still waits for the peer"
    );

    client
        .send(Message::text("done"))
        .await
        .expect("the client sends");
    let heard = within(handler)
        .await
        .expect("the handler task joins")
        .expect("the read succeeds");
    assert_eq!(heard.as_deref(), Some("done"));
}

#[tokio::test]
async fn the_halves_work_in_two_tasks() {
    let (socket, mut client, _) = pair().await;
    let (sender, mut receiver) = socket.split();

    let echo = sender.clone();
    let reader = tokio::spawn(async move {
        while let Some(text) = receiver.recv_text().await? {
            echo.send_text(format!("echo: {text}")).await?;
        }
        Ok::<(), FrameworkError>(())
    });

    sender
        .send_binary(vec![1u8, 2, 3])
        .await
        .expect("the queue accepts the frame");
    assert_eq!(
        next_frame(&mut client).await,
        Message::binary(vec![1u8, 2, 3])
    );

    client
        .send(Message::text("hello"))
        .await
        .expect("the client sends");
    let frame = next_frame(&mut client).await;
    assert_eq!(frame.to_text().expect("a text frame"), "echo: hello");

    client.close(None).await.expect("the client closes");
    within(reader)
        .await
        .expect("the reader task joins")
        .expect("the reader ends without an error when the peer closes");
}

#[tokio::test]
async fn the_receiver_is_a_stream_of_every_message() {
    let (socket, mut client, missed_pings) = pair().await;
    let (_sender, mut receiver) = socket.split();
    // Two pings are out and neither has an answer yet.
    missed_pings.store(2, Ordering::Release);

    client
        .send(Message::text("one"))
        .await
        .expect("the client sends text");
    client
        .send(Message::binary(vec![2u8]))
        .await
        .expect("the client sends bytes");
    client
        .send(Message::Pong(Default::default()))
        .await
        .expect("the client sends a pong");

    let first = within(receiver.next())
        .await
        .expect("a message")
        .expect("no error");
    assert_eq!(first, Message::text("one"));
    let second = within(receiver.next())
        .await
        .expect("a message")
        .expect("no error");
    assert_eq!(second, Message::binary(vec![2u8]));
    assert_eq!(
        missed_pings.load(Ordering::Acquire),
        2,
        "text and bytes answer no ping"
    );

    let third = within(receiver.next())
        .await
        .expect("a message")
        .expect("no error");
    assert!(matches!(third, Message::Pong(_)), "got {third:?}");
    assert_eq!(
        missed_pings.load(Ordering::Acquire),
        0,
        "the stream counts the pong as the answer, as `recv` does"
    );

    client.close(None).await.expect("the client closes");
    let fourth = within(receiver.next())
        .await
        .expect("a message")
        .expect("no error");
    assert!(matches!(fourth, Message::Close(_)), "got {fourth:?}");
    let end = within(receiver.next()).await;
    assert!(
        end.is_none(),
        "the stream ends with the connection: {end:?}"
    );
}

#[tokio::test]
async fn a_close_ends_the_connection_for_every_sender() {
    let (socket, mut client, _) = pair().await;
    let first = socket.sender();
    let second = first.clone();
    assert!(!second.is_closed(), "the connection starts open");

    first
        .close(4000, "done")
        .await
        .expect("the close is accepted");

    match next_frame(&mut client).await {
        Message::Close(Some(close)) => {
            assert_eq!(u16::from(close.code), 4000);
            assert_eq!(close.reason.as_str(), "done");
        }
        other => panic!("expected the close frame, got {other:?}"),
    }

    // The socket itself is still alive here and holds a sender of its
    // own. The connection ended all the same.
    within(second.closed()).await;
    assert!(first.is_closed());
    assert!(second.is_closed());
    let refused = second
        .send_text("late")
        .await
        .expect_err("a send after the close fails");
    assert!(
        refused.to_string().contains("connection closed"),
        "got: {refused}"
    );
    drop(socket);
}

#[tokio::test]
async fn a_refused_close_sends_nothing_and_leaves_the_connection_open() {
    let (socket, mut client, _) = pair().await;
    let sender = socket.sender();

    let reserved = sender
        .close(1005, "")
        .await
        .expect_err("1005 never goes on the wire");
    assert!(
        reserved.to_string().contains("not allowed"),
        "got: {reserved}"
    );
    let long = sender
        .close(1000, "x".repeat(124))
        .await
        .expect_err("124 bytes is one more than the limit");
    assert!(
        long.to_string().contains("exceeds RFC 6455 limit"),
        "got: {long}"
    );
    assert!(!sender.is_closed(), "a refused close closes nothing");

    sender
        .send_text("still open")
        .await
        .expect("the queue accepts the frame");
    let frame = next_frame(&mut client).await;
    assert_eq!(
        frame
            .to_text()
            .expect("the first frame on the wire is text"),
        "still open"
    );
}

/// Pushes one frame from a task, gives its sender away, and returns at
/// the peer's first message.
struct GivesItsSenderAway {
    kept: Arc<Mutex<Option<WsSender>>>,
}

#[async_trait]
impl WebSocketHandler for GivesItsSenderAway {
    async fn handle(&self, mut socket: WsSocket, _req: Request) -> Result<(), FrameworkError> {
        let sender = socket.sender();
        let pushing = sender.clone();
        let push = tokio::spawn(async move { pushing.send_text("pushed").await });
        *self.kept.lock().expect("the slot is not poisoned") = Some(sender);

        socket.recv_text().await?;
        push.await
            .map_err(|e| FrameworkError::internal(format!("the push task failed: {e}")))?
    }
}

/// Serve one WebSocket route at `/ws` on a free port.
async fn serve(handler: impl WebSocketHandler) -> u16 {
    let config = WsConfig {
        // The test client sends no `Origin` header.
        origin_policy: OriginPolicy::AllowAny,
        ..Default::default()
    };
    let router = Arc::new(Router::new().ws_with_config("/ws", handler, config));
    let middleware = Arc::new(MiddlewareRegistry::new());

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let port = listener.local_addr().expect("the bound address").port();

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
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
                // Without `with_upgrades` the upgrade never resolves.
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .with_upgrades()
                    .await;
            });
        }
    });

    port
}

#[tokio::test]
async fn the_connection_ends_with_the_handler_and_not_with_its_senders() {
    let kept = Arc::new(Mutex::new(None));
    let port = serve(GivesItsSenderAway { kept: kept.clone() }).await;

    let (mut client, response) = within(tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{port}/ws"
    )))
    .await
    .expect("the upgrade succeeds");
    assert_eq!(response.status(), 101);

    // Sent by a task while the handler waits for the peer.
    let frame = next_frame(&mut client).await;
    assert_eq!(frame.to_text().expect("a text frame"), "pushed");

    // The handler returns at this message. Its sender stays alive in
    // `kept` for the rest of the test.
    client
        .send(Message::text("go"))
        .await
        .expect("the client sends");

    match next_frame(&mut client).await {
        Message::Close(Some(close)) => assert_eq!(u16::from(close.code), 1000),
        other => panic!("expected the close frame, got {other:?}"),
    }
    let end = within(client.next()).await;
    assert!(
        end.is_none(),
        "the server ends the connection while a sender is alive: {end:?}"
    );

    let sender = kept
        .lock()
        .expect("the slot is not poisoned")
        .take()
        .expect("the handler gave its sender away");
    within(sender.closed()).await;
    assert!(sender.is_closed());
    sender
        .send_text("late")
        .await
        .expect_err("a send after the handler returned fails");
}
