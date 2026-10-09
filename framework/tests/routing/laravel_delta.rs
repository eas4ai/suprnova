//! PAR-095's violating example: a QUERY request to an `any!` route must reach it.
#![allow(dead_code)]

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use suprnova::http::text;
use suprnova::{MiddlewareRegistry, Request, Response, Router, handle_request};
use serial_test::serial;
use std::sync::OnceLock;
use suprnova::http::text;
use suprnova::{any, routes};

async fn anything(_req: Request) -> Response {
    text("reached")
}

routes! {
    any!("/delta-anything", anything),
}

#[tokio::test]
async fn query_reaches_an_any_route() {
    let addr = spawn_server(register(), 1).await;
    let (status, _headers, body) = send_request(addr, "QUERY", "/delta-anything").await;
    assert_eq!(status, hyper::http::StatusCode::OK, "body: {}", String::from_utf8_lossy(&body));
}

/// bound socket address.
async fn spawn_server(router: impl Into<Router>, accepts: usize) -> SocketAddr {
    let router = Arc::new(router.into());
    let middleware = Arc::new(MiddlewareRegistry::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let io = TokioIo::new(stream);
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    addr
}

/// Send an HTTP/1.1 request and capture status + headers + body.
async fn send_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
) -> (hyper::http::StatusCode, hyper::HeaderMap, Bytes) {
    let stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to test server");
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
        .await
        .expect("client handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let req = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0")
        .body(Full::new(Bytes::new()))
        .expect("build request");

    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_request timeout")
        .expect("hyper send_request");
    let (parts, body) = resp.into_parts();
    let collected = body.collect().await.expect("collect body bytes").to_bytes();
    (parts.status, parts.headers, collected)
}

/// HEAD against a GET-only route succeeds (RFC 9110 §9.3.2 fallback)
/// and returns the GET status with the body stripped to zero bytes.
///
/// Also pins the wire-level Content-Length behavior. Per RFC 9110
/// §9.3.2 a HEAD response SHOULD carry the Content-Length the GET
/// would have set; our `strip_body_for_head` helper preserves
/// `parts.headers` but swaps the body for `Full::new(Bytes::new())`
/// whose `size_hint().exact()` is `Some(0)`. Whether hyper writes
/// the preserved header or recomputes from the empty body's
/// size_hint is its wire-encoding decision. This assertion pins
/// whichever value hyper actually emits so a future hyper bump
/// that changes the behavior surfaces here.