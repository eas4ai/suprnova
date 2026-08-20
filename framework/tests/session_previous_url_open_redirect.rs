//! Regression tests: `SessionMiddleware` must never persist an unsafe
//! `_previous.url`.
//!
//! `_previous.url` backs `Redirect::back`, `Redirect::refresh`, and
//! `url::previous` — none of those readers sanitize what they read back,
//! they trust the session verbatim. `current_url` (the value considered
//! for the write) is built straight from `request.path()` + query, and
//! an origin-form HTTP request-target is syntactically free to start
//! with `//` (httparse's `URI_MAP` permits it — this isn't rejected at
//! the HTTP-parse layer). A `fallback!` route that answers any unmatched
//! path with `200` — the standard Inertia/SPA app-shell pattern — would,
//! without a write-time guard, let `GET //evil.test/anything` persist
//! `//evil.test/anything` as the previous URL, and every later
//! `Redirect::back()` would hand the browser that off-origin `Location`.
//!
//! `SessionMiddleware::handle` now guards the write with
//! `routing::url::root_relative_or_none` — the same sanitizer
//! `InertiaValidationRedirectMiddleware` uses for its own `Referer`
//! guard (see `framework/src/inertia/validation_redirect_middleware.rs`).
//! These tests drive `SessionMiddleware::handle` directly with a real
//! `Request` (parsed from raw HTTP bytes over an in-memory duplex pipe —
//! the same technique `session_persistence_fail_closed.rs` uses), since
//! that's the one place in the framework this can be enforced.

use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use suprnova::middleware::{Middleware, Next};
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::{Crypt, EncryptionKey, FrameworkError};

/// `Crypt` is a process-global; install a key exactly once so cookie
/// encryption/decryption doesn't bail.
fn ensure_crypt() {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| {
        Crypt::init(EncryptionKey::generate());
    });
}

/// A store that remembers whatever `SessionData` it was last asked to
/// persist, and hands that back on the next `read` — a minimal
/// in-memory session store, just enough to let two successive
/// `SessionMiddleware::handle` calls share state the way two requests
/// in the same browser session would.
struct CapturingStore {
    last_written: Arc<Mutex<Option<SessionData>>>,
}

#[async_trait]
impl SessionStore for CapturingStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        if let Some(existing) = self.last_written.lock().unwrap().clone() {
            return Ok(Some(existing));
        }
        Ok(Some(SessionData::new(id.to_string(), "b".repeat(40))))
    }
    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        *self.last_written.lock().unwrap() = Some(session.clone());
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

fn test_config() -> SessionConfig {
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    config
}

/// Percent-encode the handful of bytes that would otherwise break a raw
/// `Cookie:` request header. Copied from
/// `session_persistence_fail_closed.rs::post_request_with_cookie` — kept
/// duplicated rather than shared, since these are two independent test
/// binaries and helpers here are module-private.
fn encode_cookie_value(value: &str) -> String {
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

/// Build a real `Request` for `GET <raw_target>` by feeding raw HTTP
/// bytes through a hyper service over an in-memory duplex pipe.
/// `Request::new` only accepts a `hyper::Request<Incoming>`, and
/// `Incoming` bodies can't be synthesized directly, so we let hyper
/// parse a real request and hand it back over a oneshot — the request
/// line is built by hand so `raw_target` can carry a `//`-leading path
/// a higher-level request builder might normalize away before we ever
/// get to exercise the guard.
async fn get_request(raw_target: &str, cookie: Option<(&str, &str)>) -> suprnova::Request {
    use bytes::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use std::convert::Infallible;
    use suprnova::Request;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::oneshot;

    let cookie_header = match cookie {
        Some((name, value)) => format!("Cookie: {name}={}\r\n", encode_cookie_value(value)),
        None => String::new(),
    };
    let http_bytes = format!(
        "GET {raw_target} HTTP/1.1\r\nHost: app.test\r\n{cookie_header}Content-Length: 0\r\n\r\n"
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
                Ok::<_, Infallible>(hyper::Response::new(http_body_util::Full::new(
                    Bytes::from_static(b""),
                )))
            }
        });
        let _ = http1::Builder::new()
            .serve_connection(TokioIo::new(server_io), svc)
            .await;
    });

    let mut client = client_io;
    client.write_all(&http_bytes).await.unwrap();
    drop(client);
    req_rx.await.expect("server received request")
}

/// A protocol-relative current URL must never reach `_previous.url`.
///
/// Fails against the pre-fix code: with no guard on the write, this
/// request's `current_url` (`"//evil.test/anything"`) is exactly what
/// the old `if is_get && ... { s.set_previous_url(&current_url) }`
/// condition would have stored, since nothing else in that condition
/// inspects the URL's shape.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protocol_relative_current_url_never_reaches_previous_url() {
    ensure_crypt();

    let last_written: Arc<Mutex<Option<SessionData>>> = Arc::new(Mutex::new(None));
    let store = Arc::new(CapturingStore {
        last_written: last_written.clone(),
    });
    let middleware = SessionMiddleware::with_store(test_config(), store);

    // The handler makes an unrelated mutation so the session is
    // guaranteed dirty (and therefore persisted) regardless of whether
    // the `_previous.url` write itself happens — isolating this test
    // from the separate "does an unmodified session get written at all"
    // question `session_persistence_fail_closed.rs` already covers.
    let next: Next = Arc::new(|_req| {
        Box::pin(async move {
            suprnova::session::session_mut(|s| s.put("marker", true));
            Ok(suprnova::HttpResponse::text("app shell"))
        })
    });

    let request = get_request("//evil.test/anything", None).await;
    let response = middleware.handle(request, next).await;
    assert!(
        response.is_ok(),
        "the guard must not turn a normal 200 into an error"
    );

    let session = last_written
        .lock()
        .unwrap()
        .take()
        .expect("the handler's own mutation must have dirtied and persisted the session");
    assert_ne!(
        session.previous_url().as_deref(),
        Some("//evil.test/anything"),
        "a protocol-relative current URL must never be persisted verbatim"
    );
    assert_eq!(
        session.previous_url(),
        None,
        "declining the write leaves no previous URL recorded, rather than \
         inventing one — see the write-site comment in session/middleware.rs \
         for why 'store nothing' was chosen over 'store /'"
    );
}

/// End-to-end: `Redirect::back()` must never emit an off-origin
/// `Location` even after an attacker's poisoning attempt.
///
/// Two requests share one session (same encrypted cookie, same
/// in-memory store): the first is the attacker's `GET
/// //evil.test/anything` poisoning attempt; the second is an unrelated
/// later handler that calls `Redirect::back()`. Fails against the
/// pre-fix code: request 1 would have stored the malicious URL, and
/// request 2's `Redirect::back("/safe-fallback")` would have read it
/// straight back out and answered with `Location: //evil.test/anything`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn redirect_back_never_emits_an_off_origin_location_from_a_poisoned_session() {
    ensure_crypt();

    let last_written: Arc<Mutex<Option<SessionData>>> = Arc::new(Mutex::new(None));
    let store = Arc::new(CapturingStore {
        last_written: last_written.clone(),
    });
    let config = test_config();
    let session_id = "c".repeat(40);
    let cookie_value = suprnova::http::cookie::Cookie::encrypted(&config.cookie_name, &session_id)
        .expect("encrypt session cookie")
        .value()
        .to_string();

    let middleware = SessionMiddleware::with_store(config.clone(), store);

    // Request 1: the poisoning attempt. Same unrelated-mutation trick as
    // the sibling test, so this request is guaranteed to attempt a
    // persist regardless of the `_previous.url` outcome.
    let poison_next: Next = Arc::new(|_req| {
        Box::pin(async move {
            suprnova::session::session_mut(|s| s.put("marker", true));
            Ok(suprnova::HttpResponse::text("app shell"))
        })
    });
    let poison_request = get_request(
        "//evil.test/anything",
        Some((&config.cookie_name, &cookie_value)),
    )
    .await;
    let poison_response = middleware.handle(poison_request, poison_next).await;
    assert!(
        poison_response.is_ok(),
        "the poisoning attempt itself must not fail the request"
    );

    // Request 2: an unrelated later handler bounces the user back.
    let back_next: Next = Arc::new(|_req| {
        Box::pin(async move {
            let redirect: suprnova::Response = suprnova::Redirect::back("/safe-fallback").into();
            redirect
        })
    });
    let back_request = get_request("/go-back", Some((&config.cookie_name, &cookie_value))).await;
    let back_response = middleware.handle(back_request, back_next).await;

    let http = match back_response {
        Ok(r) => r,
        Err(r) => panic!(
            "Redirect::back must succeed; got status {}",
            r.status_code()
        ),
    };
    let location = http
        .header_value("Location")
        .expect("a redirect must carry a Location header")
        .to_string();
    assert_ne!(
        location, "//evil.test/anything",
        "Redirect::back must never resolve to what the earlier poisoning attempt tried to store"
    );
    assert_eq!(
        location, "/safe-fallback",
        "with nothing safe ever recorded, Redirect::back must use the caller's \
         fallback, never an off-origin value"
    );
}

/// A store that always hands back a session whose `_previous.url`
/// already holds a poisoned value - standing in for a database row a
/// pre-fix server wrote before this guard existed. `SessionData::put`
/// (not `set_previous_url`) writes the key directly, exactly how a
/// pre-fix server would have: `set_previous_url` was itself just
/// `self.put("_previous.url", url.into())` with no check, so the raw
/// key is genuinely what an old row contains, not an artificial shortcut.
struct PrePoisonedStore;

#[async_trait]
impl SessionStore for PrePoisonedStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        let mut session = SessionData::new(id.to_string(), "b".repeat(40));
        session.put("_previous.url", "//evil.test/x");
        session.mark_clean();
        Ok(Some(session))
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

/// End-to-end: `Redirect::back()` on a session that was ALREADY
/// poisoned before this guard existed - simulating a cookie surviving
/// an upgrade from a release predating the fix - must emit a
/// same-origin `Location`, never the poisoned one. The write-time guard
/// (round 2) does nothing for this case: the value was never written by
/// this process, it was already sitting in the store on the very first
/// `read`. Fails without the read-side guard: `previous_url()` would
/// return `//evil.test/x` verbatim and `Redirect::back` would resolve
/// straight to it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn redirect_back_self_heals_a_session_poisoned_before_the_write_guard_existed() {
    ensure_crypt();

    let config = test_config();
    let session_id = "d".repeat(40);
    let cookie_value = suprnova::http::cookie::Cookie::encrypted(&config.cookie_name, &session_id)
        .expect("encrypt session cookie")
        .value()
        .to_string();

    let middleware = SessionMiddleware::with_store(config.clone(), Arc::new(PrePoisonedStore));

    let back_next: Next = Arc::new(|_req| {
        Box::pin(async move {
            let redirect: suprnova::Response = suprnova::Redirect::back("/safe-fallback").into();
            redirect
        })
    });
    let request = get_request("/go-back", Some((&config.cookie_name, &cookie_value))).await;
    let response = middleware.handle(request, back_next).await;

    let http = match response {
        Ok(r) => r,
        Err(r) => panic!(
            "Redirect::back must succeed; got status {}",
            r.status_code()
        ),
    };
    let location = http
        .header_value("Location")
        .expect("a redirect must carry a Location header")
        .to_string();
    assert_ne!(
        location, "//evil.test/x",
        "a session poisoned before this guard existed must not steer Redirect::back off-origin"
    );
    assert_eq!(
        location, "/safe-fallback",
        "the poisoned value must read back as absent, so Redirect::back falls back \
         to its caller-supplied default"
    );
}
