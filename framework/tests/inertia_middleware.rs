//! Integration tests for the Inertia protocol middlewares.
//!
//! `hyper::body::Incoming` cannot be constructed outside hyper's
//! connection machinery, so these drive `handle_request` over a loopback
//! socket — the same harness `framework/tests/cors_middleware.rs` uses.
//! `framework/tests/inertia.rs` covers the response builder through an
//! `InertiaRequestExt` mock; this file covers everything that only exists
//! once a real middleware chain is in play.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::http::text;
use suprnova::session::SessionData;
use suprnova::{
    HttpResponse, InertiaHeadersMiddleware, InertiaVersionMiddleware, Middleware,
    MiddlewareRegistry, Next, Redirect, Request, Response, Router, handle_request,
};

/// Test-only stand-in for `SessionMiddleware`: scopes a caller-supplied
/// session slot into the task-local for the whole chain, so the test can
/// seed flash data before the request and inspect it afterwards.
struct SeededSessionScope(Arc<Mutex<Option<SessionData>>>);

#[async_trait::async_trait]
impl Middleware for SeededSessionScope {
    async fn handle(&self, request: Request, next: Next) -> Response {
        suprnova::session::session_scope_for_test(self.0.clone(), next(request)).await
    }
}

fn router() -> Router {
    Router::new()
        .get("/home", |_req| async { text("home") })
        // Body-less 200 — the shape Laravel's `onEmptyResponse` catches.
        // The local binding pins the error half of `Response`, which a
        // bare `Ok(...)` leaves for inference to guess.
        .get("/empty", |_req| async {
            let empty: Response = Ok(HttpResponse::new());
            empty
        })
        .put("/save", |_req| async {
            let resp: Response = Redirect::to("/home").into();
            resp
        })
        .into()
}

async fn spawn_server(
    router: impl Into<Router>,
    registry: MiddlewareRegistry,
    accepts: usize,
) -> SocketAddr {
    let router = Arc::new(router.into());
    let middleware = Arc::new(registry);

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

/// Send a request; return `(status, lowercased headers, body)`.
async fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> (u16, HashMap<String, String>, String) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let req = builder.body(Full::new(Bytes::new())).unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_request timeout")
        .expect("hyper send_request");

    let (parts, body) = resp.into_parts();
    let status = parts.status.as_u16();
    let header_map = parts
        .headers
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_lowercase(),
                v.to_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    let bytes = body.collect().await.unwrap().to_bytes();
    (
        status,
        header_map,
        String::from_utf8_lossy(&bytes).to_string(),
    )
}

// ---- Vary: X-Inertia on every response ----

#[tokio::test]
async fn a_redirect_from_an_inertia_visit_carries_vary_x_inertia() {
    // Without `Vary`, a shared cache keyed on the URL alone can hand a
    // cached Inertia JSON page object to a hard browser navigation (which
    // renders as raw JSON) or the HTML shell to an XHR (which the client
    // rejects as non-Inertia). Laravel sets it on every response.
    let registry = MiddlewareRegistry::new().append(InertiaHeadersMiddleware::new());
    let addr = spawn_server(router(), registry, 2).await;
    let (status, headers, _body) = request(addr, "PUT", "/save", &[("X-Inertia", "true")]).await;

    assert_eq!(status, 302, "the handler's redirect status is untouched");
    assert_eq!(
        headers.get("vary").map(String::as_str),
        Some("X-Inertia"),
        "a redirect must advertise that it varies on X-Inertia"
    );
}

#[tokio::test]
async fn a_404_carries_vary_x_inertia() {
    let registry = MiddlewareRegistry::new().append(InertiaHeadersMiddleware::new());
    let addr = spawn_server(router(), registry, 2).await;
    let (status, headers, _body) = request(addr, "GET", "/nope", &[("X-Inertia", "true")]).await;

    assert_eq!(status, 404);
    assert_eq!(headers.get("vary").map(String::as_str), Some("X-Inertia"));
}

#[tokio::test]
async fn a_non_inertia_response_also_carries_vary_x_inertia() {
    // The header describes the CACHE KEY, not the request. A plain
    // browser GET must carry it too, or the cache stores the HTML shell
    // under a key an XHR can hit.
    let registry = MiddlewareRegistry::new().append(InertiaHeadersMiddleware::new());
    let addr = spawn_server(router(), registry, 2).await;
    let (status, headers, body) = request(addr, "GET", "/home", &[]).await;

    assert_eq!(status, 200);
    assert_eq!(body, "home");
    assert_eq!(headers.get("vary").map(String::as_str), Some("X-Inertia"));
}

// ---- empty 200 on an Inertia visit → 303 back ----

#[tokio::test]
async fn an_empty_200_on_an_inertia_visit_becomes_a_303_back() {
    // The Inertia client treats a response without `X-Inertia` as
    // non-Inertia and shows an error modal. A handler that falls through
    // to a body-less 200 would otherwise blow up the SPA. Laravel's
    // `onEmptyResponse` redirects back instead.
    let registry = MiddlewareRegistry::new().append(InertiaHeadersMiddleware::new());
    let addr = spawn_server(router(), registry, 2).await;
    let (status, headers, _body) = request(addr, "GET", "/empty", &[("X-Inertia", "true")]).await;

    assert_eq!(status, 303, "an empty 200 must become a redirect");
    assert!(
        headers.contains_key("location"),
        "the substituted redirect must carry a Location"
    );
    assert_eq!(headers.get("vary").map(String::as_str), Some("X-Inertia"));
}

#[tokio::test]
async fn an_empty_200_on_a_plain_browser_visit_is_left_alone() {
    // Only Inertia visits get the substitution — a REST endpoint that
    // legitimately returns a body-less 200 must keep doing so.
    let registry = MiddlewareRegistry::new().append(InertiaHeadersMiddleware::new());
    let addr = spawn_server(router(), registry, 2).await;
    let (status, _headers, body) = request(addr, "GET", "/empty", &[]).await;

    assert_eq!(status, 200);
    assert_eq!(body, "");
}

// ---- version-mismatch 409 reflashes the session ----

#[tokio::test]
async fn a_version_mismatch_reflashes_the_session_before_the_409() {
    // The client answers a 409 with a full-page GET. That GET is a NEW
    // request, so the session middleware ages `_flash.old.*` away before
    // the destination page can read it — a validation error flashed by
    // the previous request vanishes purely because the asset version
    // moved. Laravel reflashes first (Middleware.php:171-175).
    let slot = suprnova::session::new_session_slot_for_test();
    {
        let mut guard = slot.lock().unwrap();
        let session = guard.as_mut().unwrap();
        // As if the previous request flashed it and the session
        // middleware aged it into `_flash.old.*`.
        session.put("_flash.old.status", "Saved");
    }

    let registry = MiddlewareRegistry::new()
        .append(SeededSessionScope(slot.clone()))
        .append(InertiaVersionMiddleware::new("v2"));
    let addr = spawn_server(router(), registry, 2).await;

    let (status, headers, _body) = request(
        addr,
        "GET",
        "/home",
        &[("X-Inertia", "true"), ("X-Inertia-Version", "v1")],
    )
    .await;

    assert_eq!(status, 409, "a stale asset version bounces the client");
    assert!(headers.contains_key("x-inertia-location"));

    let guard = slot.lock().unwrap();
    let session = guard.as_ref().unwrap();
    assert!(
        session.has("_flash.new.status"),
        "the flash must be re-flashed for the follow-up GET"
    );
    assert!(
        !session.has("_flash.old.status"),
        "reflash moves the entry rather than copying it"
    );
}

#[tokio::test]
async fn a_matching_version_does_not_reflash() {
    // Reflash is not free: it extends every flashed value by one more
    // request. It must happen only on the bounce.
    let slot = suprnova::session::new_session_slot_for_test();
    {
        let mut guard = slot.lock().unwrap();
        guard.as_mut().unwrap().put("_flash.old.status", "Saved");
    }

    let registry = MiddlewareRegistry::new()
        .append(SeededSessionScope(slot.clone()))
        .append(InertiaVersionMiddleware::new("v2"));
    let addr = spawn_server(router(), registry, 2).await;

    let (status, _headers, _body) = request(
        addr,
        "GET",
        "/home",
        &[("X-Inertia", "true"), ("X-Inertia-Version", "v2")],
    )
    .await;

    assert_eq!(status, 200);
    let guard = slot.lock().unwrap();
    let session = guard.as_ref().unwrap();
    assert!(
        session.has("_flash.old.status"),
        "no bounce, no reflash — the entry stays where it was"
    );
    assert!(!session.has("_flash.new.status"));
}
