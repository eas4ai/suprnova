use async_trait::async_trait;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::{Crypt, EncryptionKey, FrameworkError};

fn ensure_crypt() {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
}
fn insecure_config() -> SessionConfig {
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    config
}

#[derive(Default)]
struct CountingStore {
    reads: AtomicUsize,
    writes: AtomicUsize,
    read_fails: AtomicBool,
    write_fails: AtomicBool,
    session: Mutex<Option<SessionData>>,
}

impl CountingStore {
    fn with_session(session: SessionData) -> Self {
        Self {
            session: Mutex::new(Some(session)),
            ..Self::default()
        }
    }

    fn with_failing_read() -> Self {
        Self {
            read_fails: AtomicBool::new(true),
            ..Self::default()
        }
    }

    /// A store that cannot be written, like the database driver in a
    /// process that has no database connection.
    fn with_failing_write() -> Self {
        Self {
            write_fails: AtomicBool::new(true),
            ..Self::default()
        }
    }
}

#[async_trait]
impl SessionStore for CountingStore {
    async fn read(&self, _id: &str) -> Result<Option<SessionData>, FrameworkError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.read_fails.load(Ordering::SeqCst) {
            return Err(FrameworkError::internal("simulated session read failure"));
        }
        Ok(self.session.lock().unwrap().clone())
    }

    async fn write(&self, _session: &SessionData) -> Result<(), FrameworkError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        if self.write_fails.load(Ordering::SeqCst) {
            return Err(FrameworkError::internal("simulated session write failure"));
        }
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
    client.write_all(&http_bytes).await.unwrap();
    drop(client);
    req_rx.await.expect("request captured")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cookieless_clean_request_does_not_touch_store_or_emit_cookie() {
    use suprnova::middleware::{Middleware, Next};

    ensure_crypt();
    let store = Arc::new(CountingStore::default());
    let next: Next = Arc::new(move |_req| {
        Box::pin(async move {
            Ok(suprnova::HttpResponse::json(
                serde_json::json!({"ok": true}),
            ))
        })
    });
    let config = insecure_config();
    let middleware = SessionMiddleware::with_store(config, store.clone());

    let response = match middleware.handle(post_request(None).await, next).await {
        Ok(response) => response.into_hyper(),
        Err(response) => panic!("unexpected response status {}", response.status_code()),
    };

    assert_eq!(store.reads.load(Ordering::SeqCst), 0);
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
    assert!(
        response
            .headers()
            .get_all("set-cookie")
            .iter()
            .next()
            .is_none(),
        "state-free requests must not acquire session cookies"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_session_cookie_is_touched_and_reissued_once() {
    use suprnova::http::cookie::Cookie;
    use suprnova::middleware::{Middleware, Next};

    ensure_crypt();
    let session_id = "a".repeat(40);
    let session = SessionData::new(session_id.clone(), "b".repeat(40));
    let store = Arc::new(CountingStore::with_session(session));
    let next: Next = Arc::new(move |_req| {
        Box::pin(async move {
            Ok(suprnova::HttpResponse::json(
                serde_json::json!({"ok": true}),
            ))
        })
    });
    let config = insecure_config();
    let cookie = Cookie::encrypted(&config.cookie_name, &session_id).unwrap();
    let middleware = SessionMiddleware::with_store(config.clone(), store.clone());

    let response = match middleware
        .handle(
            post_request(Some((&config.cookie_name, cookie.value()))).await,
            next,
        )
        .await
    {
        Ok(response) => response.into_hyper(),
        Err(response) => panic!("unexpected response status {}", response.status_code()),
    };

    assert_eq!(store.reads.load(Ordering::SeqCst), 1);
    assert_eq!(
        store.writes.load(Ordering::SeqCst),
        1,
        "legacy cookies without a touch timestamp must refresh sliding expiry"
    );
    assert!(
        response
            .headers()
            .get_all("set-cookie")
            .iter()
            .next()
            .is_some(),
        "the refreshed activity timestamp must be carried in a new cookie"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recently_touched_session_is_loaded_without_write_or_cookie_churn() {
    use std::time::{SystemTime, UNIX_EPOCH};
    use suprnova::http::cookie::Cookie;
    use suprnova::middleware::{Middleware, Next};

    ensure_crypt();
    let session_id = "c".repeat(40);
    let session = SessionData::new(session_id.clone(), "d".repeat(40));
    let store = Arc::new(CountingStore::with_session(session));
    let next: Next = Arc::new(move |_req| {
        Box::pin(async move {
            Ok(suprnova::HttpResponse::json(
                serde_json::json!({"ok": true}),
            ))
        })
    });
    let config = insecure_config();
    let touched_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let payload = format!("{session_id}.{touched_at}");
    let cookie = Cookie::encrypted(&config.cookie_name, &payload).unwrap();
    let middleware = SessionMiddleware::with_store(config.clone(), store.clone());

    let response = match middleware
        .handle(
            post_request(Some((&config.cookie_name, cookie.value()))).await,
            next,
        )
        .await
    {
        Ok(response) => response.into_hyper(),
        Err(response) => panic!("unexpected response status {}", response.status_code()),
    };

    assert_eq!(store.reads.load(Ordering::SeqCst), 1);
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
    assert!(
        response
            .headers()
            .get_all("set-cookie")
            .iter()
            .next()
            .is_none(),
        "a fresh activity timestamp must not churn the session cookie"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cookie_without_backing_row_is_cleared_without_recreating_session() {
    use std::time::{SystemTime, UNIX_EPOCH};
    use suprnova::http::cookie::Cookie;
    use suprnova::middleware::{Middleware, Next};

    ensure_crypt();
    let session_id = "e".repeat(40);
    let store = Arc::new(CountingStore::default());
    let next: Next = Arc::new(move |_req| {
        Box::pin(async move {
            Ok(suprnova::HttpResponse::json(
                serde_json::json!({"ok": true}),
            ))
        })
    });
    let config = insecure_config();
    let touched_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let payload = format!("{session_id}.{touched_at}");
    let cookie = Cookie::encrypted(&config.cookie_name, &payload).unwrap();
    let middleware = SessionMiddleware::with_store(config.clone(), store.clone());

    let response = match middleware
        .handle(
            post_request(Some((&config.cookie_name, cookie.value()))).await,
            next,
        )
        .await
    {
        Ok(response) => response.into_hyper(),
        Err(response) => panic!("unexpected response status {}", response.status_code()),
    };

    assert_eq!(store.reads.load(Ordering::SeqCst), 1);
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
    let set_cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .expect("stale session cookie must be cleared");
    assert!(
        set_cookie.to_ascii_lowercase().contains("max-age=0"),
        "expected an expiring Set-Cookie header, got {set_cookie}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configured_touch_interval_is_capped_below_session_expiry() {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use suprnova::http::cookie::Cookie;
    use suprnova::middleware::{Middleware, Next};

    ensure_crypt();
    let session_id = "f".repeat(40);
    let session = SessionData::new(session_id.clone(), "g".repeat(40));
    let store = Arc::new(CountingStore::with_session(session));
    let next: Next = Arc::new(move |_req| {
        Box::pin(async move {
            Ok(suprnova::HttpResponse::json(
                serde_json::json!({"ok": true}),
            ))
        })
    });
    let mut config = insecure_config();
    config.lifetime = Duration::from_secs(10);
    config.touch_interval = Duration::from_secs(60);
    let touched_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_sub(6);
    let payload = format!("{session_id}.{touched_at}");
    let cookie = Cookie::encrypted(&config.cookie_name, &payload).unwrap();
    let middleware = SessionMiddleware::with_store(config.clone(), store.clone());

    let _response = middleware
        .handle(
            post_request(Some((&config.cookie_name, cookie.value()))).await,
            next,
        )
        .await;

    assert_eq!(store.reads.load(Ordering::SeqCst), 1);
    assert_eq!(
        store.writes.load(Ordering::SeqCst),
        1,
        "touch cadence must be capped before the database session can expire"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutation_after_existing_session_read_failure_fails_closed_without_write() {
    use suprnova::http::cookie::Cookie;
    use suprnova::middleware::{Middleware, Next};

    ensure_crypt();
    let session_id = "h".repeat(40);
    let store = Arc::new(CountingStore::with_failing_read());
    let next: Next = Arc::new(move |_req| {
        Box::pin(async move {
            suprnova::session::set_auth_user("user-1");
            Ok(suprnova::HttpResponse::json(
                serde_json::json!({"ok": true}),
            ))
        })
    });
    let config = insecure_config();
    let cookie = Cookie::encrypted(&config.cookie_name, &session_id).unwrap();
    let middleware = SessionMiddleware::with_store(config.clone(), store.clone());

    let response = middleware
        .handle(
            post_request(Some((&config.cookie_name, cookie.value()))).await,
            next,
        )
        .await;

    let error = match response {
        Err(error) => error,
        Ok(_) => panic!("mutation after a failed existing-session read must fail closed"),
    };
    assert_eq!(error.status_code(), 500);
    assert_eq!(store.reads.load(Ordering::SeqCst), 1);
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clean_request_survives_existing_session_read_failure_without_write() {
    use suprnova::http::cookie::Cookie;
    use suprnova::middleware::{Middleware, Next};

    ensure_crypt();
    let session_id = "i".repeat(40);
    let store = Arc::new(CountingStore::with_failing_read());
    let next: Next = Arc::new(move |_req| {
        Box::pin(async move {
            Ok(suprnova::HttpResponse::json(
                serde_json::json!({"ok": true}),
            ))
        })
    });
    let config = insecure_config();
    let cookie = Cookie::encrypted(&config.cookie_name, &session_id).unwrap();
    let middleware = SessionMiddleware::with_store(config.clone(), store.clone());

    let response = middleware
        .handle(
            post_request(Some((&config.cookie_name, cookie.value()))).await,
            next,
        )
        .await;

    assert!(response.is_ok());
    assert_eq!(store.reads.load(Ordering::SeqCst), 1);
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
}

/// The value of the `name` cookie a response set, if any.
fn set_cookie_value<B>(response: &hyper::Response<B>, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&prefix))
        .and_then(|value| value.split(';').next())
        .map(|pair| pair[prefix.len()..].to_owned())
}

/// `SessionMiddleware` around `CsrfMiddleware` around a handler that
/// answers `ok`, the order an application registers them in.
async fn through_session_and_csrf(
    sessions: &SessionMiddleware,
    request: suprnova::Request,
) -> hyper::Response<http_body_util::combinators::BoxBody<bytes::Bytes, std::convert::Infallible>> {
    through_session_csrf_and(sessions, request, || Ok(suprnova::HttpResponse::text("ok"))).await
}

/// `SessionMiddleware` around `CsrfMiddleware` around a handler that
/// answers with `answer`.
async fn through_session_csrf_and(
    sessions: &SessionMiddleware,
    request: suprnova::Request,
    answer: fn() -> suprnova::Response,
) -> hyper::Response<http_body_util::combinators::BoxBody<bytes::Bytes, std::convert::Infallible>> {
    use suprnova::Middleware;

    let next: suprnova::middleware::Next = Arc::new(move |request| {
        Box::pin(async move {
            let handler: suprnova::middleware::Next =
                Arc::new(move |_request| Box::pin(async move { answer() }));
            suprnova::CsrfMiddleware::new()
                .handle(request, handler)
                .await
        })
    });
    match sessions.handle(request, next).await {
        Ok(response) | Err(response) => response.into_hyper(),
    }
}

/// IDENTITY-014: a cookieless SPA bootstrap that is not a tracked HTML GET
/// (a JSON GET, or a HEAD) receives an `XSRF-TOKEN` together with the session
/// that token belongs to, so its next unsafe request passes the check.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cookieless_csrf_bootstrap_persists_the_session_its_token_belongs_to() {
    ensure_crypt();
    let config = insecure_config();
    let session_cookie = config.cookie_name.clone();
    for (method, headers) in [
        ("GET", vec![("accept", "application/json")]),
        ("HEAD", Vec::new()),
    ] {
        let store = Arc::new(crate::cookie_prefix_roundtrip::MemoryStore::default());
        let sessions = SessionMiddleware::with_store(config.clone(), store);

        let bootstrap = through_session_and_csrf(
            &sessions,
            suprnova::Request::for_test_with_headers(method, "/csrf-cookie", headers),
        )
        .await;
        let token = set_cookie_value(&bootstrap, "XSRF-TOKEN")
            .unwrap_or_else(|| panic!("{method}: the bootstrap hands out an XSRF-TOKEN"));
        let session = set_cookie_value(&bootstrap, &session_cookie).unwrap_or_else(|| {
            panic!("{method}: the token is useless without its session, which must persist")
        });

        let cookie = format!("{session_cookie}={session}");
        let submit = through_session_and_csrf(
            &sessions,
            suprnova::Request::for_test_with_headers(
                "POST",
                "/submit",
                [
                    ("cookie", cookie.as_str()),
                    ("x-xsrf-token", token.as_str()),
                ],
            ),
        )
        .await;
        assert_eq!(
            submit.status().as_u16(),
            200,
            "{method}: the token echoed with its session must pass the CSRF check"
        );
    }
}

/// What an auth gate answers an anonymous caller.
fn unauthenticated() -> suprnova::Response {
    Err(
        suprnova::HttpResponse::json(serde_json::json!({"message": "Unauthenticated."}))
            .status(401),
    )
}

/// A cookieless request the application refuses, such as an auth gate's 401
/// on an API route, creates no durable session: it gets no `XSRF-TOKEN` and
/// no session cookie, the store is never written, and the 401 reaches the
/// client even when the store cannot be written at all. A token handed out
/// here would belong to a session nobody stores.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_cookieless_request_hands_out_no_token_and_stores_no_session() {
    ensure_crypt();
    let config = insecure_config();
    let session_cookie = config.cookie_name.clone();
    for (method, headers) in [
        ("GET", vec![("accept", "application/json")]),
        ("GET", Vec::new()),
        ("HEAD", Vec::new()),
    ] {
        let store = Arc::new(CountingStore::with_failing_write());
        let sessions = SessionMiddleware::with_store(config.clone(), store.clone());

        let refused = through_session_csrf_and(
            &sessions,
            suprnova::Request::for_test_with_headers(method, "/api/users/1", headers),
            unauthenticated,
        )
        .await;

        assert_eq!(
            refused.status().as_u16(),
            401,
            "{method}: the refusal reaches the client; a refused request needs no session write"
        );
        assert_eq!(
            store.writes.load(Ordering::SeqCst),
            0,
            "{method}: a refused cookieless request must not store a session"
        );
        assert_eq!(
            set_cookie_value(&refused, "XSRF-TOKEN"),
            None,
            "{method}: a token whose session is not stored is useless"
        );
        assert_eq!(
            set_cookie_value(&refused, &session_cookie),
            None,
            "{method}: no session was stored, so no session cookie"
        );
    }
}

/// A stored session keeps receiving its token on a refused response: the
/// token belongs to a session the store already holds, and handing it out
/// writes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_request_with_a_stored_session_still_gets_its_token() {
    use std::time::{SystemTime, UNIX_EPOCH};
    use suprnova::http::cookie::Cookie;

    ensure_crypt();
    let config = insecure_config();
    let session_id = "e".repeat(40);
    let token = "f".repeat(40);
    let mut stored = SessionData::new(session_id.clone(), token.clone());
    stored.loaded_from_store = true;
    let store = Arc::new(CountingStore::with_session(stored));
    let sessions = SessionMiddleware::with_store(config.clone(), store.clone());
    let touched_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let cookie =
        Cookie::encrypted(&config.cookie_name, format!("{session_id}.{touched_at}")).unwrap();
    let cookie_header = format!(
        "{}={}",
        config.cookie_name,
        percent_encode_cookie_value(cookie.value())
    );

    let refused = through_session_csrf_and(
        &sessions,
        suprnova::Request::for_test_with_headers(
            "GET",
            "/api/users/1",
            [("cookie", cookie_header.as_str())],
        ),
        unauthenticated,
    )
    .await;

    assert_eq!(refused.status().as_u16(), 401);
    assert_eq!(
        set_cookie_value(&refused, "XSRF-TOKEN").as_deref(),
        Some(token.as_str()),
        "the stored session's token is still handed out"
    );
    assert_eq!(
        store.writes.load(Ordering::SeqCst),
        0,
        "handing out a stored session's token writes nothing"
    );
}
