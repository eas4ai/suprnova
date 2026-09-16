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
use suprnova::session::{
    SessionBlock, SessionConfig, SessionData, SessionMiddleware, SessionStore,
};
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
    Ok(suprnova::HttpResponse::json(
        serde_json::json!({"ok": true}),
    ))
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

fn ensure_cache() {
    use suprnova::cache::{CacheStore, InMemoryCache};
    suprnova::App::bind_if_absent::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
}

fn blocking_config(wait: Duration) -> SessionConfig {
    insecure_config().block(SessionBlock::new(Duration::from_secs(5), wait))
}

/// With blocking on, the second request cannot load until the first has
/// written, so it reads the flash the first one set and its own write
/// carries it forward.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_blocking_the_second_request_loads_after_the_first_wrote() {
    ensure_crypt();
    ensure_cache();
    let config = blocking_config(Duration::from_secs(5));
    let (_, session, cookie_value) = stored_session(&config);
    let store = Arc::new(RecordingStore::with_session(session));
    let middleware = Arc::new(SessionMiddleware::with_store(config.clone(), store.clone()));
    let second_saw_flash = Arc::new(std::sync::atomic::AtomicBool::new(false));

    // The first request flashes and then lingers, long enough for the
    // second request to load the session if nothing held it back.
    let first: suprnova::middleware::Next = Arc::new(move |_req| {
        Box::pin(async move {
            suprnova::session_mut(|s| s.flash("notice", "saved"));
            tokio::time::sleep(Duration::from_millis(300)).await;
            ok_json()
        })
    });
    let seen = second_saw_flash.clone();
    let second: suprnova::middleware::Next = Arc::new(move |_req| {
        let seen = seen.clone();
        Box::pin(async move {
            // The flash the first request set has been aged once by this
            // request's load, so it reads through the flash accessor.
            let visible = suprnova::session_mut(|s| s.get_flash::<String>("notice"))
                .flatten()
                .is_some();
            seen.store(visible, Ordering::SeqCst);
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
    assert_eq!(second_task.await.unwrap(), Ok(200));

    assert_eq!(
        store.events(),
        vec!["read", "write", "read", "write"],
        "the second load waited for the first write"
    );
    assert!(second_saw_flash.load(Ordering::SeqCst));
    assert!(store.stored().has("touched"));
}

/// A request that cannot take the lock inside the wait bound answers 503
/// without loading the session, instead of waiting indefinitely.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_wait_to_acquire_is_bounded_and_ends_in_a_503() {
    ensure_crypt();
    ensure_cache();
    let config = blocking_config(Duration::from_millis(200));
    let (session_id, session, cookie_value) = stored_session(&config);
    let store = Arc::new(RecordingStore::with_session(session));
    let middleware = Arc::new(SessionMiddleware::with_store(config.clone(), store.clone()));

    // Another holder of this session's lock: the request must wait behind it.
    let held = suprnova::Cache::lock(
        &format!("session:block:{session_id}"),
        Duration::from_secs(5),
    )
    .await
    .unwrap()
    .expect("the test holds the session lock first");

    let next: suprnova::middleware::Next = Arc::new(move |_req| Box::pin(async move { ok_json() }));
    let started = tokio::time::Instant::now();
    let request = post_request(Some((&config.cookie_name, &cookie_value))).await;
    let outcome = spawn_request(middleware, request, next).await.unwrap();
    let elapsed = started.elapsed();
    held.release().await.unwrap();

    assert_eq!(outcome, Err(503));
    assert!(
        elapsed >= Duration::from_millis(200) && elapsed < Duration::from_secs(2),
        "the wait bound is the whole story: waited {elapsed:?}"
    );
    assert_eq!(
        store.reads.load(Ordering::SeqCst),
        0,
        "a refused request never loads the session"
    );
}

/// A route opts in on its own through the builder, with the global option
/// off; a request matched elsewhere stays unserialized.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_route_level_block_applies_without_the_global_option() {
    use suprnova::Router;

    ensure_crypt();
    ensure_cache();
    let config = insecure_config();
    assert!(config.block.is_none());
    let (session_id, session, cookie_value) = stored_session(&config);
    let store = Arc::new(RecordingStore::with_session(session));
    let middleware = Arc::new(SessionMiddleware::with_store(config.clone(), store.clone()));

    // Registering the route records its block; the router itself is not
    // needed afterwards, the server stamps the pattern on the request.
    let _router: Router = Router::new()
        .post("/session-blocking/notice", |_req| async move { ok_json() })
        .block_session(SessionBlock::new(
            Duration::from_secs(5),
            Duration::from_millis(200),
        ))
        .into();

    let held = suprnova::Cache::lock(
        &format!("session:block:{session_id}"),
        Duration::from_secs(5),
    )
    .await
    .unwrap()
    .expect("the test holds the session lock first");
    let next: suprnova::middleware::Next = Arc::new(move |_req| Box::pin(async move { ok_json() }));

    let blocked = post_request(Some((&config.cookie_name, &cookie_value)))
        .await
        .with_route_pattern("/session-blocking/notice");
    let blocked = spawn_request(middleware.clone(), blocked, next.clone())
        .await
        .unwrap();
    assert_eq!(
        blocked,
        Err(503),
        "the route's block waited behind the held lock"
    );
    assert_eq!(store.reads.load(Ordering::SeqCst), 0);

    let elsewhere = post_request(Some((&config.cookie_name, &cookie_value)))
        .await
        .with_route_pattern("/session-blocking/other");
    let elsewhere = spawn_request(middleware, elsewhere, next).await.unwrap();
    held.release().await.unwrap();
    assert_eq!(elsewhere, Ok(200), "a route without a block never waits");
    assert_eq!(store.reads.load(Ordering::SeqCst), 1);
}

/// A request without a session cookie names no row two requests could
/// race over, so blocking leaves it alone: no lock, no store access.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cookieless_request_is_not_serialized() {
    ensure_crypt();
    ensure_cache();
    let config = blocking_config(Duration::from_millis(200));
    let store = Arc::new(RecordingStore::default());
    let middleware = Arc::new(SessionMiddleware::with_store(config, store.clone()));
    let next: suprnova::middleware::Next = Arc::new(move |_req| Box::pin(async move { ok_json() }));

    let outcome = spawn_request(middleware, post_request(None).await, next)
        .await
        .unwrap();

    assert_eq!(outcome, Ok(200));
    assert_eq!(store.reads.load(Ordering::SeqCst), 0);
    assert!(store.events().is_empty());
}
