//! `Router::inertia` — Laravel's `Route::inertia($uri, $component, $props)`.
//!
//! Drives real requests through `handle_request` over an ephemeral hyper
//! connection, because `hyper::body::Incoming` cannot be constructed
//! outside hyper and an Inertia visit is defined by its request header.
//! The harness mirrors `routing_verbs.rs`.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::json;
use std::sync::Arc;
use suprnova::routing::route_name_for_pattern;
use suprnova::{MiddlewareRegistry, Router, handle_request};

/// Spawn an ephemeral hyper server that serves `accepts` connections
/// through `handle_request` against the supplied router.
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

/// Send an HTTP/1.1 request with extra headers; capture status + body.
async fn send(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> (hyper::http::StatusCode, Bytes) {
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

    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0");
    for (k, v) in headers {
        builder = builder.header(*k, *v);
    }
    let req = builder
        .body(Full::new(Bytes::new()))
        .expect("build request");

    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_request timeout")
        .expect("hyper send_request");
    let (parts, body) = resp.into_parts();
    let collected = body.collect().await.expect("collect body").to_bytes();
    (parts.status, collected)
}

#[tokio::test]
async fn inertia_route_renders_the_component_with_its_static_props() {
    let router = Router::new().inertia("/about", "About", json!({ "team_size": 4 }));
    let addr = spawn_server(router, 1).await;

    let (status, body) = send(addr, "GET", "/about", &[("X-Inertia", "true")]).await;

    assert_eq!(status.as_u16(), 200);
    let page: serde_json::Value =
        serde_json::from_slice(&body).expect("Inertia visit returns a JSON page object");
    assert_eq!(page["component"], "About");
    assert_eq!(page["url"], "/about");
    assert_eq!(page["props"]["team_size"], 4);
}

#[tokio::test]
async fn inertia_route_serves_the_html_shell_to_a_hard_navigation() {
    let router = Router::new().inertia("/terms", "Terms", json!({ "version": "2026-01" }));
    let addr = spawn_server(router, 1).await;

    let (status, body) = send(addr, "GET", "/terms", &[]).await;

    assert_eq!(status.as_u16(), 200);
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("<!DOCTYPE html>"), "got: {html}");
    assert!(html.contains(r#""component":"Terms""#), "got: {html}");
}

#[tokio::test]
async fn inertia_route_answers_head_through_the_get_fallback() {
    let router = Router::new().inertia("/privacy", "Privacy", json!({}));
    let addr = spawn_server(router, 1).await;

    let (status, body) = send(addr, "HEAD", "/privacy", &[("X-Inertia", "true")]).await;

    assert_eq!(status.as_u16(), 200, "HEAD inherits the GET registration");
    assert!(body.is_empty(), "HEAD body must be stripped; got {body:?}");
}

#[tokio::test]
async fn inertia_route_returns_a_route_builder_so_it_can_be_named() {
    // This is what `Router::view` could not do: it returned `Router`,
    // so a static page could never be named or middleware'd.
    let _router: Router = Router::new()
        .inertia("/contact", "Contact", json!({}))
        .name("routing_inertia.contact");

    assert_eq!(
        route_name_for_pattern("/contact").as_deref(),
        Some("routing_inertia.contact")
    );
}

#[tokio::test]
async fn inertia_route_accepts_null_props_as_no_props() {
    let router = Router::new().inertia("/empty", "Empty", serde_json::Value::Null);
    let addr = spawn_server(router, 1).await;

    let (status, body) = send(addr, "GET", "/empty", &[("X-Inertia", "true")]).await;

    assert_eq!(status.as_u16(), 200);
    let page: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(page["component"], "Empty");
}

#[tokio::test]
async fn try_inertia_rejects_props_that_are_not_an_object() {
    // Silently dropping them would register a route that renders no
    // props and reports nothing — a bug you find in the browser.
    let err = Router::new()
        .try_inertia("/bad", "Bad", json!(["not", "an", "object"]))
        .err()
        .expect("array props must be rejected");

    let msg = format!("{err}");
    assert!(msg.contains("/bad"), "the error names the route: {msg}");
    assert!(msg.contains("object"), "the error names the problem: {msg}");
}

#[tokio::test]
async fn try_inertia_reports_a_duplicate_registration() {
    let router: Router = Router::new().inertia("/dupe", "Dupe", json!({})).into();
    let err = router
        .try_inertia("/dupe", "Dupe", json!({}))
        .err()
        .expect("a duplicate GET registration must be an error");
    assert!(format!("{err}").contains("/dupe"));
}
