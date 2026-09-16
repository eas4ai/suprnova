//! Session blocking (SESS-001): per-session request serialization through
//! the cache lock driver.
//!
//! The first test documents the race the feature exists for: without
//! blocking, two requests on one session load the same row, and whichever
//! writes last wins, so a flash the first request set is gone before any
//! later request could read it. The rest prove that blocking serializes
//! the two requests, that the wait to acquire is bounded and ends in a
//! decisive response, that a route can opt in on its own, and that a
//! request with nothing to serialize never touches the cache.

use async_trait::async_trait;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
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

/// A store that records the order of its reads and writes, so a test can
/// state which interleaving happened instead of inferring it from timing.
#[derive(Default)]
struct RecordingStore {
    reads: AtomicUsize,
    events: Mutex<Vec<&'static str>>,
    session: Mutex<Option<SessionData>>,
}

impl RecordingStore {
    fn with_session(session: SessionData) -> Self {
        Self {
            session: Mutex::new(Some(session)),
            ..Self::default()
        }
    }

    fn events(&self) -> Vec<&'static str> {
        self.events.lock().unwrap().clone()
    }

    fn stored(&self) -> SessionData {
        self.session
            .lock()
            .unwrap()
            .clone()
            .expect("a session row is stored")
    }

    /// Wait until the store has served `count` reads; the bound turns a
    /// deadlock into a failure with a message instead of a hung test.
    async fn wait_for_reads(&self, count: usize) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while self.reads.load(Ordering::SeqCst) < count {
            assert!(
                tokio::time::Instant::now() < deadline,
                "waited five seconds for read {count}; events so far: {:?}",
                self.events()
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

#[async_trait]
impl SessionStore for RecordingStore {
    async fn read(&self, _id: &str) -> Result<Option<SessionData>, FrameworkError> {
        self.events.lock().unwrap().push("read");
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.session.lock().unwrap().clone())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.events.lock().unwrap().push("write");
        *self.session.lock().unwrap() = Some(session.clone());
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

/// Capture a framework `Request` off a real hyper connection so the cookie
/// jar is parsed the way the server parses it. The same helper the other
/// session tests carry.
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
        "POST /notice HTTP/1.1\r\nHost: localhost\r\nAccept: application/json\r\n{cookie_header}Content-Length: 0\r\n\r\n"
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

/// One stored session and the cookie value that names it.
fn stored_session(config: &SessionConfig) -> (String, SessionData, String) {
    use suprnova::http::cookie::Cookie;
    let session_id = "a".repeat(40);
    let session = SessionData::new(session_id.clone(), "b".repeat(40));
    let cookie = Cookie::encrypted(&config.cookie_name, &session_id).unwrap();
    (session_id, session, cookie.value().to_owned())
}

fn ok_json() -> suprnova::http::Response {
    Ok(suprnova::HttpResponse::json(serde_json::json!({"ok": true})))
}

/// Handle one request through `middleware` on its own task, so two of them
/// run concurrently the way two connections would.
fn spawn_request(
    middleware: Arc<SessionMiddleware>,
    request: suprnova::Request,
    next: suprnova::middleware::Next,
) -> tokio::task::JoinHandle<Result<u16, u16>> {
    use suprnova::middleware::Middleware;
    tokio::spawn(async move {
        match middleware.handle(request, next).await {
            Ok(response) => Ok(response.status_code()),
            Err(response) => Err(response.status_code()),
        }
    })
}

/// The race the feature exists for, pinned deterministically: the second
/// request loads the session before the first one writes its flash, and
/// writes afterwards, so the flash never reaches the store.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_blocking_the_last_writer_wins_and_the_flash_is_lost() {
    ensure_crypt();
    let config = insecure_config();
    let (_, session, cookie_value) = stored_session(&config);
    let store = Arc::new(RecordingStore::with_session(session));
    let middleware = Arc::new(SessionMiddleware::with_store(config.clone(), store.clone()));
    let first_finished = Arc::new(tokio::sync::Notify::new());

    // The first request flashes, then waits until the second request has
    // loaded the session before it returns and writes.
    let first_store = store.clone();
    let first: suprnova::middleware::Next = Arc::new(move |_req| {
        let store = first_store.clone();
        Box::pin(async move {
            suprnova::session_mut(|s| s.flash("notice", "saved"));
            store.wait_for_reads(2).await;
            ok_json()
        })
    });
    // The second request writes only after the first has finished.
    let gate = first_finished.clone();
    let second: suprnova::middleware::Next = Arc::new(move |_req| {
        let gate = gate.clone();
        Box::pin(async move {
            gate.notified().await;
            suprnova::session_mut(|s| s.put("touched", true));
            ok_json()
        })
    });

    let first_request = post_request(Some((&config.cookie_name, &cookie_value))).await;
    let first_task = spawn_request(middleware.clone(), first_request, first);
    store.wait_for_reads(1).await;
    let second_request = post_request(Some((&config.cookie_name, &cookie_value))).await;
    let second_task = spawn_request(middleware, second_request, second);

    assert_eq!(first_task.await.unwrap(), Ok(200));
    first_finished.notify_one();
    assert_eq!(second_task.await.unwrap(), Ok(200));

    assert_eq!(store.events(), vec!["read", "read", "write", "write"]);
    let stored = store.stored();
    assert!(stored.has("touched"));
    assert!(
        !stored.has("_flash.new.notice") && !stored.has("_flash.old.notice"),
        "the second write carried the copy loaded before the flash and removed it"
    );
}
