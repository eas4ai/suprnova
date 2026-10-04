//! What the WebSocket upgrade response carries.
//!
//! Each test drives the real `handle_request` over a loopback socket with
//! `.with_upgrades()` and a real `tokio-tungstenite` client, because the
//! 101 handshake and the client's own validation of it cannot be observed
//! through a bare in-process call. Every test registers its own route path.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use suprnova::http::Request;
use suprnova::ws::{OriginPolicy, WebSocketHandler, WsConfig, WsSocket};
use suprnova::{FrameworkError, MiddlewareRegistry, Router};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// Echoes each text frame back, so a test can prove the socket works.
struct EchoHandler;

#[async_trait]
impl WebSocketHandler for EchoHandler {
    async fn handle(&self, mut socket: WsSocket, _req: Request) -> Result<(), FrameworkError> {
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("echo: {text}")).await?;
        }
        Ok(())
    }
}

/// The client in these tests sends no `Origin`, so every route opts out of
/// the default same-origin policy; origin checks are not under test here.
fn open_config() -> WsConfig {
    WsConfig {
        origin_policy: OriginPolicy::AllowAny,
        ..Default::default()
    }
}

/// Serve `router` with an empty global registry on a free port.
async fn spawn_server(router: Router) -> u16 {
    let router = Arc::new(router);
    let middleware = Arc::new(MiddlewareRegistry::new());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind free port");
    let port = listener.local_addr().expect("local_addr").port();

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

/// IDENTITY-033: the route accepts `chat`; the client offers only `CHAT`.
/// The match is case-insensitive, so the upgrade succeeds, and the 101
/// must name the token the client offered. A client that checks the
/// response against its own offer (tungstenite does, browsers do) fails
/// the handshake if the server answers with its own spelling.
#[tokio::test]
async fn subprotocol_echo_uses_the_client_spelling() {
    let port = spawn_server(Router::new().ws_with_config(
        "/ws/handshake/subprotocol",
        EchoHandler,
        WsConfig {
            accepted_protocols: vec!["chat".into()],
            ..open_config()
        },
    ))
    .await;

    let mut request = format!("ws://127.0.0.1:{port}/ws/handshake/subprotocol")
        .into_client_request()
        .expect("client request");
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        "CHAT".parse().expect("header value"),
    );

    let (mut ws, response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("the client must accept the 101: it names the token the client offered");
    assert_eq!(
        response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|v| v.to_str().ok()),
        Some("CHAT")
    );

    ws.send(Message::text("ping")).await.expect("send");
    let reply = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("reply in time")
        .expect("a frame")
        .expect("a valid frame");
    assert_eq!(reply, Message::text("echo: ping"));
}
