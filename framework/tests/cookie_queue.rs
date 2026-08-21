//! Integration tests for `Cookie::queue`/`queued`/`unqueue`/`expire` —
//! Laravel's `Cookie::queue()` family. The jar is the existing
//! `PENDING_COOKIES` task-local in `session::middleware` (already used
//! by `Auth::login_remember`), drained onto the response by
//! `SessionMiddleware` right after the session cookie.
//!
//! Two harnesses, each earning its own keep:
//!
//! - `run` / a directly-driven `middleware.handle(...)` call: drives
//!   `SessionMiddleware::handle` against a fake `SessionStore` with a
//!   hand-rolled `Next` closure standing in for the rest of the chain —
//!   no router, no `MiddlewareRegistry`, no database. Mirrors
//!   `tests/session_lazy_persistence.rs`. Covers the majority of cases
//!   here, including the two internal fail-closed 500 paths
//!   (`ReadFailsStore`, `WriteFailsStore`) that need a store which
//!   actually fails a read or a write — `NullStore` never does.
//! - `incoming_get_request` + `handle_request`: builds a genuine
//!   `hyper::Request<Incoming>` over an in-memory `tokio::io::duplex`
//!   pipe and drives it through a real `Router` + `MiddlewareRegistry`
//!   chain, the same idiom `tests/streamed_responses.rs` uses. This is
//!   the coverage the first harness structurally cannot provide: a
//!   response-mutating middleware registered around `SessionMiddleware`
//!   (here, `CorsMiddleware`) could in principle strip or reorder the
//!   queued cookie's `Set-Cookie` header on its way out, and the
//!   hand-rolled-`Next` tests would stay green because they never run a
//!   real middleware chain at all.
//!
//! Every await on this file's own critical path - the ones between a
//! test's start and its completion - is bounded by
//! `tokio::time::timeout`: a regression that makes
//! `SessionMiddleware::handle` (or the request-building plumbing
//! around it) hang fails that one test instead of stalling the suite.
//! The one exception is the `serve_connection` await inside each
//! helper's `tokio::spawn`ed connection loop: it's a detached
//! background task the test itself never awaits, so a hang there
//! leaks a task rather than blocking test completion —
//! `incoming_get_request`'s handler is even built to run forever on
//! purpose (see its doc comment).

#![cfg(feature = "testing")]

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use suprnova::middleware::{Middleware, Next};
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::{
    Cookie, CorsConfig, CorsMiddleware, Crypt, EncryptionKey, FrameworkError, MiddlewareRegistry,
    Router, SameSite, handle_request,
};

/// How long any single bounded wait in this file is allowed to take.
const WAIT: Duration = Duration::from_secs(5);

fn ensure_crypt() {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
}

/// No session ever exists to find and no write ever fails — most tests
/// in this file drive a cookieless request with no session mutation,
/// so persistence itself is out of scope (`session_lazy_persistence.rs`
/// covers that in depth).
struct NullStore;

#[async_trait]
impl SessionStore for NullStore {
    async fn read(&self, _id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(None)
    }
    async fn write(&self, _session: &SessionData) -> Result<(), FrameworkError> {
        Ok(())
    }
    async fn destroy(&self, _id: &str) -> Result<(), FrameworkError> {
        Ok(())
    }
    async fn destroy_for_user(&self, _user_id: &str) -> Result<u64, FrameworkError> {
        Ok(0)
    }
    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

/// A `SessionStore` whose `read` always fails — drives the
/// `session_read_failed && session.is_dirty()` fail-closed path in
/// `SessionMiddleware::handle` (existing session couldn't be loaded,
/// but the handler mutated the fallback session anyway).
struct ReadFailsStore;

#[async_trait]
impl SessionStore for ReadFailsStore {
    async fn read(&self, _id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Err(FrameworkError::internal("simulated session read failure"))
    }
    async fn write(&self, _session: &SessionData) -> Result<(), FrameworkError> {
        Ok(())
    }
    async fn destroy(&self, _id: &str) -> Result<(), FrameworkError> {
        Ok(())
    }
    async fn destroy_for_user(&self, _user_id: &str) -> Result<u64, FrameworkError> {
        Ok(0)
    }
    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

/// A `SessionStore` whose `write` always fails — drives the "session
/// write failed for a mutated session" fail-closed path in
/// `SessionMiddleware::handle`.
struct WriteFailsStore;

#[async_trait]
impl SessionStore for WriteFailsStore {
    async fn read(&self, _id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(None)
    }
    async fn write(&self, _session: &SessionData) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("simulated session write failure"))
    }
    async fn destroy(&self, _id: &str) -> Result<(), FrameworkError> {
        Ok(())
    }
    async fn destroy_for_user(&self, _user_id: &str) -> Result<u64, FrameworkError> {
        Ok(0)
    }
    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

/// Percent-encode a cookie value for use in a hand-built `Cookie:`
/// request header. Mirrors
/// `tests/session_lazy_persistence.rs::percent_encode_cookie_value`.
fn percent_encode_cookie_value(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'=' => encoded.push_str("%3D"),
            b'+' => encoded.push_str("%2B"),
            b'/' => encoded.push_str("%2F"),
            b';' => encoded.push_str("%3B"),
            b' ' => encoded.push_str("%20"),
            b',' => encoded.push_str("%2C"),
            _ => encoded.push(byte as char),
        }
    }
    encoded
}

/// Build a real `suprnova::Request` over an in-memory duplex connection
/// — `hyper::body::Incoming` cannot be constructed by hand. `cookie`
/// optionally carries an inbound session cookie (name, raw value),
/// needed to exercise the existing-session-read path. Mirrors
/// `tests/session_lazy_persistence.rs::post_request`.
async fn post_request(cookie: Option<(&str, &str)>) -> suprnova::Request {
    use bytes::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use std::convert::Infallible;
    use suprnova::Request;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::oneshot;

    let cookie_header = cookie
        .map(|(name, value)| format!("Cookie: {name}={}\r\n", percent_encode_cookie_value(value)))
        .unwrap_or_default();
    let http_bytes = format!(
        "POST /api/health/live HTTP/1.1\r\nHost: localhost\r\nAccept: application/json\r\n{cookie_header}Content-Length: 0\r\n\r\n"
    )
    .into_bytes();
    let (req_tx, req_rx) = oneshot::channel::<Request>();
    let req_tx = std::sync::Mutex::new(Some(req_tx));
    let (client_io, server_io) = tokio::io::duplex(http_bytes.len() + 64 * 1024);

    tokio::spawn(async move {
        let svc = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
            let wrapped = Request::new(req);
            if let Ok(mut guard) = req_tx.lock()
                && let Some(tx) = guard.take()
            {
                let _ = tx.send(wrapped);
            }
            async {
                Ok::<_, Infallible>(hyper::Response::new(
                    http_body_util::Full::new(Bytes::new()),
                ))
            }
        });
        let _ = http1::Builder::new()
            .serve_connection(TokioIo::new(server_io), svc)
            .await;
    });

    let mut client = client_io;
    tokio::time::timeout(WAIT, client.write_all(&http_bytes))
        .await
        .expect("timed out writing the request bytes")
        .unwrap();
    drop(client);
    tokio::time::timeout(WAIT, req_rx)
        .await
        .expect("timed out waiting for the request to be captured")
        .expect("request captured")
}

/// Build a genuine `hyper::Request<hyper::body::Incoming>` for a `GET`
/// on `path` carrying `headers`, by parsing real HTTP/1.1 bytes through
/// an in-memory `tokio::io::duplex` pipe rather than binding a TCP
/// port. Mirrors `tests/streamed_responses.rs::incoming_get_request`.
async fn incoming_get_request(
    path: &str,
    headers: &[(&str, &str)],
) -> hyper::Request<hyper::body::Incoming> {
    use bytes::Bytes;
    use http_body_util::Empty;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use std::convert::Infallible;
    use std::sync::Mutex;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::oneshot;

    let mut http_bytes = Vec::new();
    http_bytes.extend_from_slice(format!("GET {path} HTTP/1.1\r\n").as_bytes());
    http_bytes.extend_from_slice(b"Host: localhost\r\n");
    for (name, value) in headers {
        http_bytes.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    http_bytes.extend_from_slice(b"\r\n");

    let (req_tx, req_rx) = oneshot::channel::<hyper::Request<hyper::body::Incoming>>();
    let req_tx = Mutex::new(Some(req_tx));

    let (client_io, server_io) = tokio::io::duplex(http_bytes.len() + 64 * 1024);

    tokio::spawn(async move {
        let svc = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
            if let Ok(mut guard) = req_tx.lock()
                && let Some(tx) = guard.take()
            {
                let _ = tx.send(req);
            }
            async {
                std::future::pending::<()>().await;
                Ok::<_, Infallible>(hyper::Response::new(Empty::<Bytes>::new()))
            }
        });
        let _ = http1::Builder::new()
            .serve_connection(TokioIo::new(server_io), svc)
            .await;
    });

    let mut client = client_io;
    tokio::time::timeout(WAIT, client.write_all(&http_bytes))
        .await
        .expect("timed out writing the request bytes")
        .unwrap();

    tokio::time::timeout(WAIT, req_rx)
        .await
        .expect("timed out building the incoming request")
        .expect("server should have received the request")
}

fn config() -> SessionConfig {
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    config
}

/// Serializes this file's tests that touch the process-wide `Crypt`
/// test hooks — `Cookie::encrypted` directly, or
/// `crypto::_test_force_next_encrypt_failure` indirectly — against
/// each other. `#[tokio::test]` functions in one binary run
/// concurrently by default, so without this, one test's genuine
/// `Crypt::encrypt_string` call could spuriously consume another
/// test's forced-failure flag (or vice versa: an unrelated test could
/// "steal" the forced failure meant for a specific test). `tokio::
/// sync::Mutex`, not `std::sync::Mutex`: the guard is held across
/// `.await` points, which a non-async mutex guard can't do in a
/// `flavor = "multi_thread"` test.
fn crypt_hook_guard() -> &'static tokio::sync::Mutex<()> {
    static GUARD: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    GUARD.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Drive one independent request (own `SessionMiddleware`, own
/// `PENDING_COOKIES` scope) against `NullStore` and return the raw
/// hyper response.
async fn run(
    next: Next,
) -> hyper::Response<http_body_util::combinators::BoxBody<bytes::Bytes, std::convert::Infallible>> {
    let middleware = SessionMiddleware::with_store(config(), Arc::new(NullStore));
    let request = post_request(None).await;
    let result = tokio::time::timeout(WAIT, middleware.handle(request, next))
        .await
        .expect("SessionMiddleware::handle timed out");
    match result {
        Ok(response) => response.into_hyper(),
        Err(response) => panic!("unexpected error response: {}", response.status_code()),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_cookie_is_attached_to_the_response_as_set_cookie() {
    ensure_crypt();
    let next: Next = Arc::new(|_req| {
        Box::pin(async {
            Cookie::queue(Cookie::new("promo", "10OFF"));
            Ok(suprnova::HttpResponse::json(
                serde_json::json!({"ok": true}),
            ))
        })
    });

    let response = run(next).await;
    let set_cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("promo="))
        .expect("queued cookie must appear as a Set-Cookie header");
    assert!(set_cookie.contains("promo=10OFF"), "got: {set_cookie}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unqueue_removes_a_previously_queued_cookie() {
    ensure_crypt();
    let next: Next = Arc::new(|_req| {
        Box::pin(async {
            Cookie::queue(Cookie::new("promo", "10OFF"));
            assert!(
                Cookie::queued("promo").is_some(),
                "must be visible once queued"
            );
            Cookie::unqueue("promo");
            assert!(
                Cookie::queued("promo").is_none(),
                "must be gone once unqueued"
            );
            Ok(suprnova::HttpResponse::json(
                serde_json::json!({"ok": true}),
            ))
        })
    });

    let response = run(next).await;
    assert!(
        response
            .headers()
            .get_all("set-cookie")
            .iter()
            .next()
            .is_none(),
        "an unqueued cookie, with no session mutation, must leave zero Set-Cookie headers"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expire_queues_a_deletion_cookie_scoped_by_path_and_domain() {
    ensure_crypt();
    let next: Next = Arc::new(|_req| {
        Box::pin(async {
            Cookie::expire("promo", Some("/checkout"), Some("example.com"));
            Ok(suprnova::HttpResponse::json(
                serde_json::json!({"ok": true}),
            ))
        })
    });

    let response = run(next).await;
    let set_cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("promo="))
        .expect("expire must queue a deletion cookie");
    assert!(set_cookie.contains("Max-Age=0"), "got: {set_cookie}");
    assert!(set_cookie.contains("Path=/checkout"), "got: {set_cookie}");
    assert!(
        set_cookie.contains("Domain=example.com"),
        "got: {set_cookie}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_cookie_attaches_even_to_a_redirect_response() {
    ensure_crypt();
    let next: Next = Arc::new(|_req| {
        Box::pin(async {
            Cookie::queue(Cookie::new("promo", "10OFF"));
            let response: suprnova::Response = suprnova::Redirect::to("/thanks").into();
            response
        })
    });

    let response = run(next).await;
    assert_eq!(response.status().as_u16(), 302);
    let set_cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("promo="))
        .expect("queued cookie must ride the redirect response");
    assert!(set_cookie.contains("promo=10OFF"), "got: {set_cookie}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_cookie_does_not_leak_into_the_next_request() {
    ensure_crypt();
    let next_a: Next = Arc::new(|_req| {
        Box::pin(async {
            Cookie::queue(Cookie::new("promo", "10OFF"));
            Ok(suprnova::HttpResponse::json(
                serde_json::json!({"ok": true}),
            ))
        })
    });
    let response_a = run(next_a).await;
    assert!(
        response_a
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .any(|v| v.starts_with("promo=")),
        "sanity check: the first request must have queued the cookie"
    );

    let next_b: Next = Arc::new(|_req| {
        Box::pin(async {
            assert!(
                Cookie::queued("promo").is_none(),
                "a cookie queued on a prior request must not leak into this one"
            );
            Ok(suprnova::HttpResponse::json(
                serde_json::json!({"ok": true}),
            ))
        })
    });
    let response_b = run(next_b).await;
    assert!(
        response_b
            .headers()
            .get_all("set-cookie")
            .iter()
            .next()
            .is_none(),
        "the second request must carry no Set-Cookie at all"
    );
}

#[test]
fn queue_outside_a_request_scope_is_a_silent_no_op() {
    // No SessionMiddleware, no task-local scope of any kind — this is
    // what code outside `handle_request` looks like. None of the four
    // calls may panic, and `queued` must report nothing queued.
    Cookie::queue(Cookie::new("promo", "10OFF"));
    assert!(Cookie::queued("promo").is_none());
    Cookie::unqueue("promo");
    Cookie::expire("promo", None, None);
    assert!(Cookie::queued("promo").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queuing_the_same_name_twice_replaces_rather_than_duplicates() {
    ensure_crypt();
    // Brief design note 3: `queue_cookie` retains-then-pushes, so a
    // second `Cookie::queue` under a name already queued this request
    // replaces the first rather than adding a second `Set-Cookie` line
    // for it. A regression back to always-append would slip past every
    // other test in this file, which only ever queue one name once.
    let next: Next = Arc::new(|_req| {
        Box::pin(async {
            Cookie::queue(Cookie::new("promo", "5OFF"));
            Cookie::queue(Cookie::new("promo", "10OFF"));
            Ok(suprnova::HttpResponse::json(
                serde_json::json!({"ok": true}),
            ))
        })
    });

    let response = run(next).await;
    let promo_cookies: Vec<&str> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter(|v| v.starts_with("promo="))
        .collect();
    assert_eq!(
        promo_cookies.len(),
        1,
        "a second Cookie::queue under the same name must replace, not duplicate: {promo_cookies:?}"
    );
    assert!(
        promo_cookies[0].contains("promo=10OFF"),
        "the surviving Set-Cookie must carry the second call's value: got {promo_cookies:?}"
    );
    assert!(
        !promo_cookies[0].contains("5OFF"),
        "the first call's value must not leak into the surviving header: got {promo_cookies:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_cookie_keeps_its_httponly_secure_and_samesite_attributes() {
    ensure_crypt();
    // `expire`'s test already pins Max-Age/Path/Domain surviving the
    // queue-then-drain round trip. A queued cookie losing HttpOnly or
    // Secure is a vulnerability rather than a formatting bug, so those
    // (plus SameSite) get their own assertion here rather than relying
    // on "`queue_cookie` never touches `.options`" as an argument.
    let next: Next = Arc::new(|_req| {
        Box::pin(async {
            let cookie = Cookie::new("session_pref", "dark")
                .http_only(true)
                .secure(true)
                .same_site(SameSite::Strict);
            Cookie::queue(cookie);
            Ok(suprnova::HttpResponse::json(
                serde_json::json!({"ok": true}),
            ))
        })
    });

    let response = run(next).await;
    let set_cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("session_pref="))
        .expect("queued cookie must appear as a Set-Cookie header");
    assert!(
        set_cookie.contains("HttpOnly"),
        "a queued cookie must not lose HttpOnly: got {set_cookie}"
    );
    assert!(
        set_cookie.contains("Secure"),
        "a queued cookie must not lose Secure: got {set_cookie}"
    );
    assert!(
        set_cookie.contains("SameSite=Strict"),
        "a queued cookie must not lose its SameSite attribute: got {set_cookie}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_cookie_survives_a_session_read_failure_500() {
    ensure_crypt();
    // This test calls `Cookie::encrypted` directly below, and
    // `queued_cookie_survives_a_session_cookie_encryption_failure_500`
    // (further down this file) arms a process-wide, self-clearing
    // "make the next Crypt::encrypt_string call fail" flag. Hold the
    // shared guard so the two can't interleave and steal each other's
    // encrypt call.
    let _guard = crypt_hook_guard().lock().await;
    // Fix round 1, IMPORTANT 1: `session_read_failed` only turns on
    // when an *existing* session cookie's read fails, so this test (and
    // only this one) needs a real inbound session cookie naming a row
    // `ReadFailsStore` will fail to read.
    let cfg = config();
    let session_id = "r".repeat(40);
    let cookie = Cookie::encrypted(&cfg.cookie_name, &session_id).unwrap();
    let next: Next = Arc::new(|_req| {
        Box::pin(async {
            Cookie::queue(Cookie::new("promo", "10OFF"));
            suprnova::session::set_auth_user("user-1");
            Ok(suprnova::HttpResponse::json(
                serde_json::json!({"ok": true}),
            ))
        })
    });

    let middleware = SessionMiddleware::with_store(cfg.clone(), Arc::new(ReadFailsStore));
    let request = post_request(Some((&cfg.cookie_name, cookie.value()))).await;
    let response = tokio::time::timeout(WAIT, middleware.handle(request, next))
        .await
        .expect("SessionMiddleware::handle timed out");

    let error = match response {
        Err(error) => error,
        Ok(_) => panic!("mutation after a failed existing-session read must fail closed"),
    };
    assert_eq!(error.status_code(), 500);
    let hyper_response = error.into_hyper();
    let set_cookie = hyper_response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("promo="))
        .expect("a cookie queued before the session read failure must still reach the 500");
    assert!(set_cookie.contains("promo=10OFF"), "got: {set_cookie}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_cookie_survives_a_session_write_failure_500() {
    ensure_crypt();
    // Fix round 1, IMPORTANT 1: `WriteFailsStore` fails every write, so
    // a handler that dirties the session (here, `set_auth_user`) drives
    // `SessionMiddleware::handle`'s fail-closed 500 for an unpersisted
    // mutation — the second of the two paths that used to drop
    // `pending_cookies` before the drain loop ever ran.
    let next: Next = Arc::new(|_req| {
        Box::pin(async {
            Cookie::queue(Cookie::new("promo", "10OFF"));
            suprnova::session::set_auth_user("user-1");
            Ok(suprnova::HttpResponse::json(
                serde_json::json!({"ok": true}),
            ))
        })
    });

    let middleware = SessionMiddleware::with_store(config(), Arc::new(WriteFailsStore));
    let request = post_request(None).await;
    let response = tokio::time::timeout(WAIT, middleware.handle(request, next))
        .await
        .expect("SessionMiddleware::handle timed out");

    let error = match response {
        Err(error) => error,
        Ok(_) => panic!("a dirtied session with a failing store write must fail closed"),
    };
    assert_eq!(error.status_code(), 500);
    let hyper_response = error.into_hyper();
    let set_cookie = hyper_response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("promo="))
        .expect("a cookie queued before the session write failure must still reach the 500");
    assert!(set_cookie.contains("promo=10OFF"), "got: {set_cookie}");
}

/// Fix round 1, IMPORTANT 2: drives `Cookie::queue` through the real
/// `Router` + `MiddlewareRegistry` chain via `handle_request`, not the
/// hand-rolled `Next` closure the rest of this file uses. Registering
/// `CorsMiddleware` alongside `SessionMiddleware` proves two things at
/// once — the request actually reached the router (only a matched
/// route produces this body), and a second, response-mutating
/// middleware in the real chain does not strip or reorder the queued
/// cookie's `Set-Cookie` header.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_cookie_survives_the_router_and_middleware_chain() {
    ensure_crypt();
    let _guard = crypt_hook_guard().lock().await;
    let router: Router = Router::new()
        .get("/promo", |_req| async {
            Cookie::queue(Cookie::new("promo", "10OFF"));
            Ok(suprnova::HttpResponse::json(
                serde_json::json!({"ok": true}),
            ))
        })
        .into();
    let registry = MiddlewareRegistry::new()
        .append(SessionMiddleware::with_store(config(), Arc::new(NullStore)))
        .append(CorsMiddleware::new(
            CorsConfig::allow_origins(["https://app.example"]).allow_credentials(true),
        ));

    let req = incoming_get_request("/promo", &[("Origin", "https://app.example")]).await;
    let resp = tokio::time::timeout(
        WAIT,
        handle_request(Arc::new(router), Arc::new(registry), req),
    )
    .await
    .expect("handle_request timed out");

    assert_eq!(resp.status(), 200);
    let set_cookie = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("promo="))
        .expect("queued cookie must survive the real Router + MiddlewareRegistry pipeline");
    assert!(set_cookie.contains("promo=10OFF"), "got: {set_cookie}");
    assert_eq!(
        resp.headers()
            .get("access-control-allow-origin")
            .expect("global CORS middleware must have actually run on this route"),
        "https://app.example"
    );
}

/// Fix round 2, IMPORTANT 2: the third fail-closed path
/// (`create_session_cookie`'s own `Err` branch, `middleware.rs`) has no
/// seam through `SessionStore`/`SessionConfig` — the shared cookie
/// encryption path always succeeds given a real installed key, the
/// resolved AAD label, and caller-controlled plaintext, so nothing
/// reachable from a `SessionStore` fake can make it fail. `crypto::
/// _test_force_next_encrypt_failure` (`framework/src/crypto/mod.rs`) is a
/// self-clearing, `testing`-feature-gated hook built for exactly
/// this case; see its doc comment for why it clears itself after one
/// use rather than staying on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_cookie_survives_a_session_cookie_encryption_failure_500() {
    ensure_crypt();
    let _guard = crypt_hook_guard().lock().await;

    let next: Next = Arc::new(|_req| {
        Box::pin(async {
            Cookie::queue(Cookie::new("promo", "10OFF"));
            suprnova::session::set_auth_user("user-1");
            Ok(suprnova::HttpResponse::json(
                serde_json::json!({"ok": true}),
            ))
        })
    });

    // `NullStore` reads `None` and writes `Ok(())`, so the dirtied
    // session reaches `create_session_cookie` — the request has no
    // inbound cookie and hydrates no remember-me token, so
    // `create_session_cookie`'s `Cookie::encrypted` call (which uses
    // `Crypt::encrypt_string_for`) is the only cookie encryption this
    // request makes, and the armed flag lands on it exactly.
    let middleware = SessionMiddleware::with_store(config(), Arc::new(NullStore));
    let request = post_request(None).await;
    suprnova::crypto::_test_force_next_encrypt_failure();
    let response = tokio::time::timeout(WAIT, middleware.handle(request, next))
        .await
        .expect("SessionMiddleware::handle timed out");

    let error = match response {
        Err(error) => error,
        Ok(_) => panic!("a forced session-cookie encryption failure must fail closed"),
    };
    assert_eq!(error.status_code(), 500);
    let hyper_response = error.into_hyper();
    let set_cookie = hyper_response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("promo="))
        .expect(
            "a cookie queued before the session-cookie encryption failure must still reach the 500",
        );
    assert!(set_cookie.contains("promo=10OFF"), "got: {set_cookie}");
}
