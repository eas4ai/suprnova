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
