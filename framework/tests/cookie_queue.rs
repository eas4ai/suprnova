//! Integration tests for `Cookie::queue`/`queued`/`unqueue`/`expire` —
//! Laravel's `Cookie::queue()` family. The jar is the existing
//! `PENDING_COOKIES` task-local in `session::middleware` (already used
//! by `Auth::login_remember`), drained onto the response by
//! `SessionMiddleware` right after the session cookie. Drives
//! `SessionMiddleware::handle` directly against a fake `SessionStore`,
//! mirroring `tests/session_lazy_persistence.rs` — no router, no hyper
//! server, no database.

use async_trait::async_trait;
use std::sync::Arc;
use suprnova::middleware::{Middleware, Next};
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::{Cookie, Crypt, EncryptionKey, FrameworkError};

fn ensure_crypt() {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
}

/// No session ever exists to find and no write ever fails — every test
/// drives a cookieless request with no session mutation, so
/// persistence itself is out of scope (`session_lazy_persistence.rs`
/// covers that).
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

/// Build a real `suprnova::Request` over an in-memory duplex connection
/// — `hyper::body::Incoming` cannot be constructed by hand. Mirrors
/// `tests/session_lazy_persistence.rs::post_request`.
async fn post_request() -> suprnova::Request {
    use bytes::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use std::convert::Infallible;
    use suprnova::Request;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::oneshot;

    let http_bytes = b"POST /api/health/live HTTP/1.1\r\nHost: localhost\r\n\
                        Accept: application/json\r\nContent-Length: 0\r\n\r\n"
        .to_vec();
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
    client.write_all(&http_bytes).await.unwrap();
    drop(client);
    req_rx.await.expect("request captured")
}

fn config() -> SessionConfig {
    SessionConfig {
        cookie_secure: false,
        ..SessionConfig::default()
    }
}

/// Drive one independent request (own `SessionMiddleware`, own
/// `PENDING_COOKIES` scope) and return the raw hyper response.
async fn run(
    next: Next,
) -> hyper::Response<http_body_util::combinators::BoxBody<bytes::Bytes, std::convert::Infallible>> {
    let middleware = SessionMiddleware::with_store(config(), Arc::new(NullStore));
    match middleware.handle(post_request().await, next).await {
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
