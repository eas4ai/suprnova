use super::env_snapshot::{EnvSnapshot, set_env};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use suprnova::{DebugBar, HttpResponse, MiddlewareRegistry, Request, Response, Router};

/// Restore process settings after a test so one request's gate cannot affect another.
pub struct DebugMode {
    _env: EnvSnapshot,
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

/// Serialize environment changes while exercising the real debug gate.
pub async fn debug_mode(on: bool, extra: &[(&'static str, &str)]) -> DebugMode {
    let lock = super::env_lock::lock_env_async().await;
    let mut keys = vec![
        "APP_DEBUG",
        "DEBUGBAR",
        "DEBUGBAR_KEEP",
        "APP_ENV",
        "APP_URL",
    ];
    keys.extend(extra.iter().map(|(key, _)| *key));
    let snapshot = EnvSnapshot::capture(&keys);
    set_env("APP_DEBUG", Some(if on { "true" } else { "false" }));
    set_env("APP_ENV", Some("testing"));
    set_env("APP_URL", Some("http://localhost"));
    set_env("DEBUGBAR", None);
    set_env("DEBUGBAR_KEEP", None);
    for (key, value) in extra {
        set_env(key, Some(value));
    }
    assert_eq!(suprnova::Config::is_debug(), on);
    DebugMode {
        _env: snapshot,
        _lock: lock,
    }
}

#[derive(Debug)]
/// Keep the wire response available for independent body, header and record assertions.
pub struct Reply {
    /// Preserve the wire status to compare it with the recorded status.
    pub status: u16,
    /// Preserve duplicate headers, including the session cookies used by Live.
    pub headers: Vec<(String, String)>,
    /// Buffer fixture responses so assertions can inspect the final injected document.
    pub body: String,
}
impl Reply {
    /// Read a header without making test assertions depend on its wire casing.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
    /// Require the response id before following the public diagnostic route.
    pub fn id(&self) -> &str {
        self.header("x-debugbar-id").expect("record header")
    }
    /// Decode the public JSON response rather than inspecting collector internals.
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).expect("JSON reply")
    }
}

/// Drive an isolated registration through the same HTTP boundary as an application.
pub struct Harness {
    /// Retain the router so multiple requests share the same declared routes.
    pub router: Arc<Router>,
    /// Retain the middleware registration so later lookups reach its bounded store.
    pub registry: Arc<MiddlewareRegistry>,
}
impl Harness {
    /// Install diagnostics on a router without adding unrelated application middleware.
    pub fn new(router: Router) -> Self {
        Self::with_middleware(router, MiddlewareRegistry::new())
    }
    /// Exercise diagnostics alongside the middleware whose effects a test observes.
    pub fn with_middleware(router: Router, registry: MiddlewareRegistry) -> Self {
        Self {
            router: Arc::new(router),
            registry: Arc::new(
                registry.append(DebugBar::middleware().expect("debug bar configuration")),
            ),
        }
    }
    /// Send an ordinary HTTP request without bypassing routing or panic recovery.
    pub async fn get(&self, path: &str, headers: &[(&str, &str)]) -> Reply {
        exchange(
            Arc::clone(&self.router),
            Arc::clone(&self.registry),
            request("GET", path, headers),
        )
        .await
    }
    /// Follow the response header to prove the record is available to browser code.
    pub async fn record(&self, response: &Reply) -> serde_json::Value {
        let reply = self
            .get(&format!("/_debugbar/{}", response.id()), &[])
            .await;
        assert_eq!(reply.status, 200);
        reply.json()
    }
}

/// Build a wire request so duplicate headers and URI encoding reach the real adapter.
pub fn request(
    method: &str,
    target: &str,
    headers: &[(&str, &str)],
) -> hyper::Request<Full<Bytes>> {
    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(target)
        .header("Host", "localhost");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    builder
        .body(Full::new(Bytes::new()))
        .expect("valid wire request")
}

/// Supply a small document that makes the injection position easy to assert.
pub async fn page(_request: Request) -> Response {
    Ok(HttpResponse::html(
        "<!doctype html><html><body><h1>Debug bar fixture</h1></body></html>",
    ))
}

/// Serve HTTP/1 over in-memory IO to cover the production boundary without a listening socket.
pub async fn exchange(
    router: Arc<Router>,
    registry: Arc<MiddlewareRegistry>,
    request: hyper::Request<Full<Bytes>>,
) -> Reply {
    let (client_io, server_io) = tokio::io::duplex(1024 * 1024);

    let service = service_fn(move |incoming: hyper::Request<Incoming>| {
        let router = Arc::clone(&router);
        let registry = Arc::clone(&registry);
        async move { Ok::<_, Infallible>(suprnova::handle_request(router, registry, incoming).await) }
    });
    // Boxed: the request future is large, and held inline it can
    // overflow a test thread's stack.
    let server = Box::pin(
        hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(server_io), service),
    );

    let (mut sender, client) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(client_io))
            .await
            .expect("the in-memory HTTP/1 handshake needs no peer");

    let round_trip = Box::pin(async move {
        let response = sender
            .send_request(request)
            .await
            .expect("the server must answer the request");
        let (parts, body) = response.into_parts();
        let body = body
            .collect()
            .await
            .expect("the response body must arrive")
            .to_bytes();
        (parts, body)
    });
    let connections = async {
        let _ = tokio::join!(client, server);
    };

    let (parts, body) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::select! {
            done = round_trip => done,
            () = connections => panic!("the connection closed before the response arrived"),
        }
    })
    .await
    .expect("the request must finish within 30 seconds");

    Reply {
        status: parts.status.as_u16(),
        headers: parts
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_string(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect(),
        body: String::from_utf8_lossy(&body).into_owned(),
    }
}
