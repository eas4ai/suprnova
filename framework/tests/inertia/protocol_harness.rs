//! Shared harness for the Inertia protocol tests (`inp_*`): a loopback
//! server, a client that carries the session cookie from one request to the
//! next, an in-memory session store, and a session scope seeded by the test.
//!
//! The redirect, flash and previous-URL rules only show across several
//! requests that share one session, which is what the cookie-carrying
//! client gives; `SeededSessionScope` covers the single-request cases where
//! the test inspects the session afterwards.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use suprnova::session::{SessionConfig, SessionData, SessionStore};
use suprnova::{
    FrameworkError, Middleware, MiddlewareRegistry, Next, Request, Response, Router, handle_request,
};

/// A session store that keeps every session in memory, so a test can read
/// what the session middleware stored.
#[derive(Default)]
pub struct MemoryStore {
    pub sessions: Mutex<HashMap<String, SessionData>>,
}

impl MemoryStore {
    /// The one stored session; the tests drive a single visitor.
    pub fn only_session(&self) -> SessionData {
        let sessions = self.sessions.lock().unwrap();
        assert_eq!(sessions.len(), 1, "expected exactly one stored session");
        sessions.values().next().unwrap().clone()
    }
}

#[async_trait]
impl SessionStore for MemoryStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.sessions.lock().unwrap().get(id).cloned())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.sessions
            .lock()
            .unwrap()
            .insert(session.id.clone(), session.clone());
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.sessions.lock().unwrap().remove(id);
        Ok(())
    }

    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let mut sessions = self.sessions.lock().unwrap();
        let before = sessions.len();
        sessions.retain(|_, session| session.user_id.as_deref() != Some(user_id));
        Ok((before - sessions.len()) as u64)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

/// The session cookie is encrypted, so the key has to exist first.
pub fn ensure_crypt() {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| {
        suprnova::Crypt::init(suprnova::EncryptionKey::generate());
    });
}

/// A session config whose cookie travels over the plain-HTTP test socket.
pub fn session_config() -> SessionConfig {
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    config
}

/// Scopes a caller-supplied session slot over the whole chain, so a test
/// can seed the session before the request and read it afterwards.
pub struct SeededSessionScope(pub Arc<Mutex<Option<SessionData>>>);

#[async_trait]
impl Middleware for SeededSessionScope {
    async fn handle(&self, request: Request, next: Next) -> Response {
        suprnova::session::session_scope_for_test(self.0.clone(), next(request)).await
    }
}

/// Serve `router` behind `registry` on a loopback port until the test ends.
pub async fn serve(router: impl Into<Router>, registry: MiddlewareRegistry) -> SocketAddr {
    let router = Arc::new(router.into());
    let middleware = Arc::new(registry);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), svc)
                    .await;
            });
        }
    });
    addr
}

/// What came back from one request.
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    /// Lowercased header names; a repeated header keeps every value.
    pub headers: HashMap<String, Vec<String>>,
    pub body: String,
}

impl Reply {
    /// The first value of `name`.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .and_then(|values| values.first())
            .map(String::as_str)
    }

    /// The body as the JSON page object of an Inertia visit.
    pub fn page(&self) -> serde_json::Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("not a JSON page ({e}): {}", self.body))
    }
}

/// A client that keeps the session cookie the server sets and sends it on
/// every later request, as a browser does.
pub struct Client {
    addr: SocketAddr,
    cookie: Option<String>,
}

impl Client {
    pub fn new(addr: SocketAddr) -> Self {
        Self { addr, cookie: None }
    }

    pub async fn send(&mut self, method: &str, path: &str, headers: &[(&str, &str)]) -> Reply {
        let stream = tokio::net::TcpStream::connect(self.addr).await.unwrap();
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
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
        if let Some(cookie) = &self.cookie {
            builder = builder.header(
                "Cookie",
                format!("suprnova_session={}", percent_encode_cookie_value(cookie)),
            );
        }
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            sender.send_request(builder.body(Full::new(Bytes::new())).unwrap()),
        )
        .await
        .expect("send_request timeout")
        .expect("hyper send_request");

        let (parts, body) = response.into_parts();
        let mut headers: HashMap<String, Vec<String>> = HashMap::new();
        for (name, value) in parts.headers.iter() {
            headers
                .entry(name.as_str().to_ascii_lowercase())
                .or_default()
                .push(value.to_str().unwrap_or("").to_string());
        }
        if let Some(cookie) = headers.get("set-cookie").and_then(|values| {
            values.iter().find_map(|header| {
                header
                    .strip_prefix("suprnova_session=")
                    .and_then(|rest| rest.split(';').next())
                    .map(ToOwned::to_owned)
            })
        }) {
            self.cookie = Some(cookie);
        }
        let bytes = body.collect().await.unwrap().to_bytes();
        Reply {
            status: parts.status.as_u16(),
            headers,
            body: String::from_utf8_lossy(&bytes).to_string(),
        }
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

/// Minimal `InertiaRequestExt` for driving `InertiaResponse::resolve`
/// without a server.
pub struct MockReq {
    pub path: String,
    pub headers: HashMap<String, String>,
}

impl MockReq {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            headers: HashMap::new(),
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers
            .insert(name.to_ascii_lowercase(), value.to_string());
        self
    }

    pub fn inertia(self) -> Self {
        self.header("X-Inertia", "true")
    }
}

impl suprnova::InertiaRequestExt for MockReq {
    fn path(&self) -> &str {
        &self.path
    }
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

/// The JSON page object an Inertia visit answered with.
pub fn page_of(response: suprnova::HttpResponse) -> serde_json::Value {
    let bytes = response.body().to_vec();
    serde_json::from_slice(&bytes).expect("a JSON page object")
}
