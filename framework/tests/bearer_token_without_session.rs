//! Bearer-token auth must work with NO `SessionMiddleware` installed.
//!
//! This is the ordinary shape of a token-only API — it is exactly what
//! `suprnova new x --api` generates, and it never registers
//! `SessionMiddleware`. `BearerTokenMiddleware` used to publish the
//! authenticated id only through `set_auth_user`, which routes through
//! `session_mut` and is a silent no-op when `SESSION_CONTEXT` is not
//! scoped. The id was dropped, `Auth::check()` was always false, and
//! every token-guarded route returned 401 regardless of token validity.
//!
//! Do NOT add `session_scope_for_test` to this file. Installing a
//! session scope makes these tests pass for the wrong reason and
//! restores the blind spot they exist to close.
//!
//! # Harness
//!
//! Copied from `framework/tests/torii_integration.rs` (Torii boot against
//! an in-memory SQLite database, shared across tests via one tokio
//! `Runtime`) and `framework/tests/auth_http_middleware.rs` (`handle_request`
//! driven over a real loopback socket, since `handle_request` is what
//! installs the per-request `request_state` scope — the same scope a real
//! server installs, and the one this fix reads from). Deliberately does
//! NOT wrap anything in `session_scope_for_test`: the absence of a session
//! scope is the entire property under test.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use once_cell::sync::Lazy;
use tokio::runtime::Runtime;

use suprnova::http::text;
use suprnova::torii_integration::{ToriiConfig, init_torii, middleware::BearerTokenMiddleware};
use suprnova::{Auth, AuthMiddleware, MiddlewareRegistry, Router, handle_request};

/// One tokio runtime shared across every test in this file.
///
/// Mirrors `torii_integration.rs`: SQLx's in-memory SQLite pool is bound to
/// the `Runtime` it was created on, so every test must share one runtime or
/// later tests see "no such table" once an earlier test's runtime drops.
static RT: Lazy<Runtime> = Lazy::new(|| Runtime::new().expect("tokio runtime"));

/// One-time Torii initialisation shared across all tests in this file.
///
/// Only Torii's own database is needed here — unlike `torii_integration.rs`,
/// nothing in this file exercises passkey/OAuth ceremony tokens, so the
/// extra Suprnova `DbConnection` + migration that file sets up is not
/// required.
static SETUP: Lazy<()> = Lazy::new(|| {
    RT.block_on(async {
        let config = ToriiConfig::sqlite_in_memory()
            .await
            .expect("sqlite in-memory connection");
        init_torii(config).await.expect("init_torii");
    });
});

/// A minimal router with one guarded route. The handler echoes `Auth::id()`
/// as the response body so tests can assert on the resolved identity, not
/// just the status code.
fn router() -> Router {
    Router::new()
        .get("/protected", |_req| async {
            text(Auth::id().unwrap_or_default())
        })
        .into()
}

/// Spawn a test server with `registry` as the global middleware set,
/// accepting `accepts` connections. Copied from `auth_http_middleware.rs`.
async fn spawn_server(
    router: impl Into<Router>,
    registry: MiddlewareRegistry,
    accepts: usize,
) -> SocketAddr {
    let router = std::sync::Arc::new(router.into());
    let middleware = std::sync::Arc::new(registry);

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

/// Send a request and return `(status, lowercased response headers, body)`.
/// Copied from `auth_http_middleware.rs`.
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

/// The stack under test: `BearerTokenMiddleware` then the sync,
/// session-backed `AuthMiddleware::new()` gate — with no `SessionMiddleware`
/// anywhere in the chain. This is exactly the shape of the `--api` scaffold.
fn token_only_registry() -> MiddlewareRegistry {
    MiddlewareRegistry::new()
        .append(BearerTokenMiddleware)
        .append(AuthMiddleware::new())
}

/// Assertion 1: a request carrying a VALID bearer token, through
/// `BearerTokenMiddleware -> AuthMiddleware::new() -> handler`, with no
/// `SessionMiddleware` installed, must reach the handler — and `Auth::id()`
/// inside the handler must equal the token's user id.
///
/// This is the assertion that fails today: `BearerTokenMiddleware` only
/// published the id through `set_auth_user`, which is a silent no-op
/// without a `SessionMiddleware`-installed session scope, so
/// `AuthMiddleware::new()` always saw a guest and returned 401.
#[test]
fn valid_bearer_token_reaches_handler_without_session_middleware() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        Auth::password()
            .register("bearer-no-session@example.com", "Bearer1!")
            .await
            .unwrap();

        let (_user, torii_session) = Auth::password()
            .authenticate("bearer-no-session@example.com", "Bearer1!", None, None)
            .await
            .unwrap();

        // Freshly authenticated sessions always carry the plaintext token —
        // `None` is reserved for sessions loaded from storage (hash only).
        let token_str = torii_session
            .token
            .as_ref()
            .expect("freshly authenticated session must carry plaintext token")
            .expose_secret()
            .to_string();
        let expected_user_id = torii_session.user_id.as_str().to_string();

        let addr = spawn_server(router(), token_only_registry(), 1).await;

        let (status, _headers, body) = request(
            addr,
            "GET",
            "/protected",
            &[("Authorization", &format!("Bearer {token_str}"))],
        )
        .await;

        assert_ne!(
            status, 401,
            "a valid bearer token must not be rejected by AuthMiddleware::new() \
             even with no SessionMiddleware installed"
        );
        assert_eq!(
            body, expected_user_id,
            "Auth::id() inside the handler must equal the token's user id"
        );
    });
}

/// Assertion 2: the same stack with NO `Authorization` header returns 401.
/// Passes before and after the fix — proves the fix did not simply disable
/// the gate.
#[test]
fn missing_authorization_header_returns_401_without_session_middleware() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let addr = spawn_server(router(), token_only_registry(), 1).await;

        let (status, _headers, _body) = request(addr, "GET", "/protected", &[]).await;

        assert_eq!(status, 401);
    });
}

/// Assertion 3: the same stack with a syntactically valid but unknown
/// bearer token returns 401. Passes before and after the fix — proves the
/// fix did not simply disable the gate.
#[test]
fn unknown_bearer_token_returns_401_without_session_middleware() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let addr = spawn_server(router(), token_only_registry(), 1).await;

        let (status, _headers, _body) = request(
            addr,
            "GET",
            "/protected",
            &[(
                "Authorization",
                "Bearer syntactically_valid_but_unknown_xyz",
            )],
        )
        .await;

        assert_eq!(status, 401);
    });
}
