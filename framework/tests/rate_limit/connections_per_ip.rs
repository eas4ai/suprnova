//! `RateLimitMiddleware::connections_per_ip`: each client address may hold
//! N connections open, and a connection is counted until it ends.
//!
//! Each test drives real requests through `handle_request_with_peer`, the
//! entry the server's accept loop uses, so `Request::ip` is the peer the
//! test names. The WebSocket tests use a real `tokio-tungstenite` client,
//! because the count has to be observed on the far side of the 101
//! handshake, after the middleware chain has returned.

use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tokio::sync::{Semaphore, mpsc};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use suprnova::http::{HttpResponse, Request, text};
use suprnova::middleware::into_boxed;
use suprnova::ws::{OriginPolicy, WebSocketHandler, WsConfig, WsSocket};
use suprnova::{
    ConnectionsPerIp, FrameworkError, Middleware, MiddlewareRegistry, Next, RateLimitMiddleware,
    Response, Router, handle_request_with_peer,
};

const PATIENCE: Duration = Duration::from_secs(5);
const FIRST: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1));
const SECOND: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 2));
const V6_HOME: IpAddr = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 1, 2, 0, 0, 0, 1));
const V6_SAME_HOME: IpAddr = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 1, 2, 0xaaaa, 0, 0, 7));
const V6_NEXT_HOME: IpAddr = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 1, 3, 0, 0, 0, 1));

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Answers every text message, and returns when the client is gone.
struct Echo;

#[async_trait]
impl WebSocketHandler for Echo {
    async fn handle(&self, mut socket: WsSocket, _req: Request) -> Result<(), FrameworkError> {
        while let Some(message) = socket.recv_text().await? {
            socket.send_text(format!("echo: {message}")).await?;
        }
        Ok(())
    }
}

/// Drops the request it is given at once and keeps the socket open. The
/// place has to outlive the request value, or this handler would give it
/// back while its socket is still open.
struct DropsTheRequest;

#[async_trait]
impl WebSocketHandler for DropsTheRequest {
    async fn handle(&self, mut socket: WsSocket, request: Request) -> Result<(), FrameworkError> {
        drop(request);
        while let Some(message) = socket.recv_text().await? {
            socket.send_text(format!("echo: {message}")).await?;
        }
        Ok(())
    }
}

/// Refuses every request: stands in for an auth gate that says no.
struct Refuses;

#[async_trait]
impl Middleware for Refuses {
    async fn handle(&self, _request: Request, _next: Next) -> Response {
        Err(HttpResponse::text("no").status(401))
    }
}

/// The test client sends no `Origin` header, so the route has to accept a
/// request without one. The origin policy is not what these tests are about.
fn any_origin() -> WsConfig {
    WsConfig {
        origin_policy: OriginPolicy::AllowAny,
        ..Default::default()
    }
}

fn echo_behind(cap: &ConnectionsPerIp) -> Arc<Router> {
    Arc::new(Router::new().ws_with_middleware_and_config(
        "/ws/echo",
        Echo,
        vec![into_boxed(cap.clone())],
        any_origin(),
    ))
}

/// A server for `router` whose every connection comes from `peer`.
async fn serve(router: &Arc<Router>, peer: Option<IpAddr>) -> SocketAddr {
    let router = router.clone();
    let registry = Arc::new(MiddlewareRegistry::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind an ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let router = router.clone();
            let registry = registry.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let registry = registry.clone();
                    async move {
                        Ok::<_, Infallible>(
                            handle_request_with_peer(router, registry, request, peer).await,
                        )
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .with_upgrades()
                    .await;
            });
        }
    });
    addr
}

async fn upgrade(addr: SocketAddr, path: &str) -> Result<Socket, WsError> {
    tokio::time::timeout(
        PATIENCE,
        tokio_tungstenite::connect_async(format!("ws://{addr}{path}")),
    )
    .await
    .expect("the upgrade timed out")
    .map(|(socket, _)| socket)
}

/// Open a socket that the server has to admit.
async fn open(addr: SocketAddr, path: &str) -> Socket {
    upgrade(addr, path)
        .await
        .expect("the server must admit this socket")
}

/// Ask for a socket that the server has to refuse, and return the status
/// it refused with.
async fn refused(addr: SocketAddr, path: &str) -> u16 {
    match upgrade(addr, path).await {
        Err(WsError::Http(response)) => response.status().as_u16(),
        Ok(_) => panic!("the server admitted a socket it had to refuse"),
        Err(other) => panic!("expected an HTTP refusal, got {other:?}"),
    }
}

async fn echoes(socket: &mut Socket) {
    socket.send(Message::text("hi")).await.expect("send");
    let reply = tokio::time::timeout(PATIENCE, socket.next())
        .await
        .expect("the reply timed out")
        .expect("the socket ended")
        .expect("the reply failed");
    assert_eq!(reply.to_text().expect("a text reply"), "echo: hi");
}

/// Wait until `address` holds `expected` places.
///
/// The place is given back by the server's session task when it ends,
/// which is after the close handshake and is nothing the client can see.
/// So this lets the runtime run until the count is there, and fails when
/// it is not there in time.
async fn settles_at(cap: &ConnectionsPerIp, address: IpAddr, expected: usize) {
    let address = address.to_string();
    let settled = tokio::time::timeout(PATIENCE, async {
        while cap.open_for(&address) != expected {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(
        settled.is_ok(),
        "{address} holds {} places, expected {expected}",
        cap.open_for(&address)
    );
}

#[tokio::test]
async fn an_address_at_the_cap_is_refused_until_one_of_its_sockets_closes() {
    let cap = RateLimitMiddleware::connections_per_ip(2);
    let addr = serve(&echo_behind(&cap), Some(FIRST)).await;

    let mut first = open(addr, "/ws/echo").await;
    let mut second = open(addr, "/ws/echo").await;
    assert_eq!(
        cap.open_for("203.0.113.1"),
        2,
        "the places must still be held after the 101 handshake"
    );
    assert_eq!(refused(addr, "/ws/echo").await, 429);
    assert_eq!(
        cap.open_for("203.0.113.1"),
        2,
        "a refused upgrade takes no place"
    );
    // A refusal costs the sockets that are open nothing.
    echoes(&mut first).await;
    echoes(&mut second).await;

    first.close(None).await.expect("close the first socket");
    settles_at(&cap, FIRST, 1).await;

    let mut third = open(addr, "/ws/echo").await;
    echoes(&mut third).await;
    assert_eq!(refused(addr, "/ws/echo").await, 429);
}

#[tokio::test]
async fn each_address_has_places_of_its_own() {
    let cap = RateLimitMiddleware::connections_per_ip(1);
    let router = echo_behind(&cap);
    let first = serve(&router, Some(FIRST)).await;
    let second = serve(&router, Some(SECOND)).await;

    let _held = open(first, "/ws/echo").await;
    assert_eq!(refused(first, "/ws/echo").await, 429);

    let mut other = open(second, "/ws/echo").await;
    echoes(&mut other).await;
    assert_eq!(cap.open_for("203.0.113.1"), 1);
    assert_eq!(cap.open_for("203.0.113.2"), 1);
}

#[tokio::test]
async fn a_socket_that_goes_away_without_a_close_gives_its_place_back() {
    // A client that holds sockets open to use the cap up does not say
    // goodbye either. The place has to come back when the connection is
    // gone, not when a close frame arrives.
    let cap = RateLimitMiddleware::connections_per_ip(1);
    let addr = serve(&echo_behind(&cap), Some(FIRST)).await;

    let socket = open(addr, "/ws/echo").await;
    assert_eq!(refused(addr, "/ws/echo").await, 429);

    drop(socket);
    settles_at(&cap, FIRST, 0).await;
    let mut next = open(addr, "/ws/echo").await;
    echoes(&mut next).await;
}

#[tokio::test]
async fn the_place_is_held_for_the_socket_and_not_for_the_request_value() {
    let cap = RateLimitMiddleware::connections_per_ip(1);
    let router = Arc::new(Router::new().ws_with_middleware_and_config(
        "/ws/echo",
        DropsTheRequest,
        vec![into_boxed(cap.clone())],
        any_origin(),
    ));
    let addr = serve(&router, Some(FIRST)).await;

    let mut socket = open(addr, "/ws/echo").await;
    // The reply proves the handler ran, so it has dropped the request.
    echoes(&mut socket).await;

    assert_eq!(cap.open_for("203.0.113.1"), 1);
    assert_eq!(refused(addr, "/ws/echo").await, 429);
}

#[tokio::test]
async fn an_upgrade_a_later_middleware_refuses_gives_its_place_back() {
    let cap = RateLimitMiddleware::connections_per_ip(1);
    let router = Arc::new(
        Router::new()
            .ws_with_middleware_and_config(
                "/ws/members",
                Echo,
                vec![into_boxed(cap.clone()), into_boxed(Refuses)],
                any_origin(),
            )
            .ws_with_middleware_and_config(
                "/ws/echo",
                Echo,
                vec![into_boxed(cap.clone())],
                any_origin(),
            ),
    );
    let addr = serve(&router, Some(FIRST)).await;

    assert_eq!(refused(addr, "/ws/members").await, 401);
    assert_eq!(cap.open_for("203.0.113.1"), 0);
    // Had the first refusal kept its place, this would be a 429.
    assert_eq!(refused(addr, "/ws/members").await, 401);

    // The place the refusals gave back is there to take, and it is the
    // one place: the cap counts, it does not let everything pass.
    let _held = open(addr, "/ws/echo").await;
    assert_eq!(cap.open_for("203.0.113.1"), 1);
    assert_eq!(refused(addr, "/ws/echo").await, 429);
    assert_eq!(
        refused(addr, "/ws/members").await,
        429,
        "at the cap the address is refused by the cap, before the gate behind it"
    );
}

#[tokio::test]
async fn a_request_with_no_address_is_not_counted() {
    let cap = RateLimitMiddleware::connections_per_ip(1);
    let router = echo_behind(&cap);
    let nameless = serve(&router, None).await;
    let named = serve(&router, Some(FIRST)).await;

    let mut first = open(nameless, "/ws/echo").await;
    let mut second = open(nameless, "/ws/echo").await;
    echoes(&mut first).await;
    echoes(&mut second).await;

    // The same cap counts a peer that has an address, so the two sockets
    // above passed because they have none and not because nothing counts.
    let _held = open(named, "/ws/echo").await;
    assert_eq!(refused(named, "/ws/echo").await, 429);
}

#[tokio::test]
async fn the_addresses_of_one_ipv6_network_share_the_places() {
    let cap = RateLimitMiddleware::connections_per_ip(1);
    let router = echo_behind(&cap);
    let home = serve(&router, Some(V6_HOME)).await;
    let same_home = serve(&router, Some(V6_SAME_HOME)).await;
    let next_home = serve(&router, Some(V6_NEXT_HOME)).await;

    let _held = open(home, "/ws/echo").await;
    assert_eq!(
        refused(same_home, "/ws/echo").await,
        429,
        "another address of the same /64 is the same client"
    );

    let mut neighbour = open(next_home, "/ws/echo").await;
    echoes(&mut neighbour).await;
    assert_eq!(cap.open_for("2001:db8:1:2::5"), 1);
    assert_eq!(cap.open_for("2001:db8:1:3::5"), 1);
}

mod documented_form {
    use super::{Echo, any_origin};
    use suprnova::rate_limit::RateLimitMiddleware;
    use suprnova::{routes, ws};

    routes! {
        ws!("/ws/broadcast", Echo)
            .middleware(RateLimitMiddleware::connections_per_ip(1))
            .config(any_origin()),
    }
}

#[tokio::test]
async fn the_form_the_manual_shows_caps_a_ws_route() {
    let router = Arc::new(documented_form::register());
    let addr = serve(&router, Some(FIRST)).await;

    let mut held = open(addr, "/ws/broadcast").await;
    echoes(&mut held).await;
    assert_eq!(refused(addr, "/ws/broadcast").await, 429);
}

/// Send one GET and return its status.
async fn get(addr: SocketAddr, path: &'static str) -> u16 {
    let stream = TcpStream::connect(addr).await.expect("connect");
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .expect("handshake");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let request = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0")
        .body(Full::new(Bytes::new()))
        .expect("a request");
    let response = tokio::time::timeout(PATIENCE, sender.send_request(request))
        .await
        .expect("the request timed out")
        .expect("the request failed");
    let status = response.status().as_u16();
    let _ = response.into_body().collect().await.expect("a body");
    status
}

#[tokio::test]
async fn on_a_plain_route_the_place_is_held_while_the_request_is_handled() {
    let cap = RateLimitMiddleware::connections_per_ip(1);
    let (entered, mut has_entered) = mpsc::unbounded_channel::<()>();
    // The handler takes one permit to return. The test hands them out.
    let may_return = Arc::new(Semaphore::new(0));

    let router: Arc<Router> = Arc::new({
        let may_return = may_return.clone();
        Router::new()
            .get("/report", move |request: Request| {
                let entered = entered.clone();
                let may_return = may_return.clone();
                async move {
                    let _ = entered.send(());
                    if let Ok(permit) = may_return.acquire().await {
                        permit.forget();
                    }
                    drop(request);
                    text("ok")
                }
            })
            .middleware(cap.clone())
            .into()
    });
    let addr = serve(&router, Some(FIRST)).await;

    let slow = tokio::spawn(get(addr, "/report"));
    tokio::time::timeout(PATIENCE, has_entered.recv())
        .await
        .expect("the handler was not entered in time")
        .expect("the handler is gone");

    assert_eq!(cap.open_for("203.0.113.1"), 1);
    assert_eq!(
        get(addr, "/report").await,
        429,
        "the address is handling one request, and the cap is 1"
    );

    may_return.add_permits(1);
    assert_eq!(slow.await.expect("the first request"), 200);
    settles_at(&cap, FIRST, 0).await;

    may_return.add_permits(1);
    assert_eq!(get(addr, "/report").await, 200);
}
