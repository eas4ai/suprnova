//! `TestClient` - the one test client every HTTP test reaches for, as
//! Laravel's `MakesHttpRequests` is.
//!
//! The framework's request path takes a `hyper::Request<Incoming>`, a body
//! type only hyper can build, so a request has to cross an HTTP/1.1
//! connection to reach it. The client opens that connection over an
//! in-memory pipe (`tokio::io::duplex`) instead of a port: the server half
//! hands each parsed request to [`crate::handle_request`], the client half
//! sends it. Both halves run inside the task that awaits
//! [`TestRequest::send`], so a [`crate::testing::TestContainer`] scope or
//! fake the test set up is the one the request sees.
//!
//! Between requests the client keeps every cookie a response sets and sends
//! them back, as a browser does, so a session started by one request is the
//! session of the next.

use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde::Serialize;

use super::response::TestResponse;
use crate::{ErrorReport, Method, MiddlewareRegistry, Router, SessionStore};

/// How long one request may take before [`TestRequest::send`] gives up on
/// it. A request that never answers is a hung test otherwise, with nothing
/// to say which request hung.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// The in-memory pipe's buffer. A larger body streams through it.
const PIPE_CAPACITY: usize = 64 * 1024;

/// The `Accept` header an Inertia visit sends, which
/// [`TestRequest::inertia`] copies.
const INERTIA_ACCEPT: &str = "text/html, application/xhtml+xml";

/// Drives requests through a router and middleware registry in-process and
/// returns each response as a [`TestResponse`].
///
/// ```rust,no_run
/// use suprnova::testing::TestClient;
/// use suprnova::{MiddlewareRegistry, Request, Router};
///
/// # async fn example() {
/// let router = Router::new().get("/", |_req: Request| async { suprnova::http::text("hi") });
/// let client = TestClient::new(router, MiddlewareRegistry::new());
///
/// client.get("/").send().await.assert_ok().assert_see("hi");
/// # }
/// ```
///
/// A clone shares the original's router, registry and cookies, so a
/// request sent through either continues the same session.
#[derive(Clone)]
pub struct TestClient {
    router: Arc<Router>,
    registry: Arc<MiddlewareRegistry>,
    session: Option<(Arc<dyn SessionStore>, String)>,
    cookies: Arc<Mutex<Vec<(String, String)>>>,
    timeout: Duration,
}

impl TestClient {
    /// A client for `router` behind `registry`, the same two values
    /// [`crate::handle_request`] takes. Build the registry the way the
    /// application does (`MiddlewareRegistry::from_global()` after the
    /// bootstrap registered its stack) to test the whole application, or
    /// by hand to test one middleware.
    pub fn new(router: impl Into<Router>, registry: MiddlewareRegistry) -> Self {
        Self {
            router: Arc::new(router.into()),
            registry: Arc::new(registry),
            session: None,
            cookies: Arc::new(Mutex::new(Vec::new())),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Attach the session store and cookie name the test's
    /// `SessionMiddleware` uses, so every response this client returns can
    /// read the session it names
    /// ([`TestResponse::assert_session_has`]). The store lives behind the
    /// server, not in the response, which is why the client has to be told
    /// where it is.
    pub fn with_session_store(
        mut self,
        store: Arc<dyn SessionStore>,
        cookie_name: impl Into<String>,
    ) -> Self {
        self.session = Some((store, cookie_name.into()));
        self
    }

    /// Change how long one request may take before [`TestRequest::send`]
    /// panics naming it. Ten seconds unless set; raise it for a handler
    /// that does slow work on purpose, such as hashing a password in a
    /// debug build.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Start a `GET` request to `path` (path and query, `/users?page=2`).
    pub fn get(&self, path: impl Into<String>) -> TestRequest {
        self.send(Method::GET, path)
    }

    /// Start a `POST` request to `path`.
    pub fn post(&self, path: impl Into<String>) -> TestRequest {
        self.send(Method::POST, path)
    }

    /// Start a `PUT` request to `path`.
    pub fn put(&self, path: impl Into<String>) -> TestRequest {
        self.send(Method::PUT, path)
    }

    /// Start a `PATCH` request to `path`.
    pub fn patch(&self, path: impl Into<String>) -> TestRequest {
        self.send(Method::PATCH, path)
    }

    /// Start a `DELETE` request to `path`.
    pub fn delete(&self, path: impl Into<String>) -> TestRequest {
        self.send(Method::DELETE, path)
    }

    /// Start a request with any `method`, for the ones without their own
    /// method here (`OPTIONS`, `HEAD`). Laravel's `call($method, $uri)`.
    pub fn send(&self, method: Method, path: impl Into<String>) -> TestRequest {
        TestRequest {
            client: self.clone(),
            method,
            path: path.into(),
            headers: Vec::new(),
            body: Bytes::new(),
        }
    }

    /// The cookies the client holds, as `(name, wire value)` pairs, in the
    /// order they were first set.
    fn cookie_pairs(&self) -> Vec<(String, String)> {
        self.cookies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Keep every cookie `headers` sets, and drop every cookie they expire.
    fn remember_cookies(&self, headers: &[(String, String)]) {
        let mut jar = self.cookies.lock().unwrap_or_else(PoisonError::into_inner);
        for (_, line) in headers.iter().filter(|(name, _)| name == "set-cookie") {
            let mut parts = line.split(';');
            let Some((name, value)) = parts.next().and_then(|pair| pair.split_once('=')) else {
                continue;
            };
            let (name, value) = (name.trim(), value.trim());
            if name.is_empty() {
                continue;
            }
            let expired = value.is_empty() || expires_now(parts);
            jar.retain(|(held, _)| held != name);
            if !expired {
                jar.push((name.to_string(), value.to_string()));
            }
        }
    }

    /// Send `request` over a fresh in-memory connection and read the whole
    /// response, with the error report the framework attached to it.
    async fn exchange(
        &self,
        request: hyper::Request<Full<Bytes>>,
    ) -> Result<Exchanged, hyper::Error> {
        let (client_io, server_io) = tokio::io::duplex(PIPE_CAPACITY);
        // The report lives in the response's extensions, which hyper drops
        // when it writes the response to the wire; the service takes it out
        // first.
        let report: Arc<Mutex<Option<ErrorReport>>> = Arc::default();
        let service = {
            let router = self.router.clone();
            let registry = self.registry.clone();
            let report = report.clone();
            service_fn(move |req: hyper::Request<Incoming>| {
                let router = router.clone();
                let registry = registry.clone();
                let report = report.clone();
                async move {
                    let response = crate::handle_request(router, registry, req).await;
                    let (mut parts, body) = response.into_parts();
                    if let Some(taken) = parts.extensions.remove::<ErrorReport>() {
                        *report.lock().unwrap_or_else(PoisonError::into_inner) = Some(taken);
                    }
                    Ok::<_, Infallible>(hyper::Response::from_parts(parts, body))
                }
            })
        };
        let server = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(server_io), service);
        let client = async move {
            let (mut sender, connection) =
                hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(client_io))
                    .await?;
            // The sender is dropped when this block ends, which lets the
            // connection close and the server half see the end of input.
            let exchange = async move {
                let response = sender.send_request(request).await?;
                let (parts, body) = response.into_parts();
                let body = body.collect().await?.to_bytes();
                Ok::<_, hyper::Error>((parts, body))
            };
            let (result, _closed) = tokio::join!(exchange, connection);
            result
        };
        let (result, _served) = tokio::join!(client, server);
        let (parts, body) = result?;
        let headers = parts
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_string(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        let report = report.lock().unwrap_or_else(PoisonError::into_inner).take();
        Ok(Exchanged {
            status: parts.status.as_u16(),
            headers,
            body,
            report,
        })
    }
}

/// One response, read off the in-memory connection.
struct Exchanged {
    status: u16,
    headers: Vec<(String, String)>,
    body: Bytes,
    report: Option<ErrorReport>,
}

/// Whether a `Set-Cookie` line's attributes expire the cookie now: a
/// `Max-Age` of zero or less, or, without a `Max-Age`, an `Expires` date
/// that has passed. RFC 6265 gives `Max-Age` precedence.
fn expires_now<'a>(attributes: impl Iterator<Item = &'a str>) -> bool {
    let mut max_age = None;
    let mut expires = None;
    for attribute in attributes {
        let Some((key, value)) = attribute.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if key.eq_ignore_ascii_case("max-age") {
            max_age = value.parse::<i64>().ok();
        } else if key.eq_ignore_ascii_case("expires") {
            expires = httpdate::parse_http_date(value).ok();
        }
    }
    match (max_age, expires) {
        (Some(seconds), _) => seconds <= 0,
        (None, Some(at)) => at <= SystemTime::now(),
        (None, None) => false,
    }
}

/// One request being built: headers and a body, then [`Self::send`].
#[must_use = "a TestRequest does nothing until `.send().await`"]
pub struct TestRequest {
    client: TestClient,
    method: Method,
    path: String,
    headers: Vec<(String, String)>,
    body: Bytes,
}

impl TestRequest {
    /// Set a request header, replacing one of the same name (compared
    /// without case) set before, so a later call overrides what
    /// [`Self::inertia`] or [`Self::json`] put there.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into();
        self.headers
            .retain(|(held, _)| !held.eq_ignore_ascii_case(&name));
        self.headers.push((name, value.into()));
        self
    }

    /// Send `body` serialized as JSON, with `Content-Type:
    /// application/json`, the body a handler reads with `req.json()`.
    ///
    /// # Panics
    ///
    /// Panics when `body` fails to serialize: the test built a body that
    /// cannot be sent.
    pub fn json<T: Serialize + ?Sized>(mut self, body: &T) -> Self {
        let encoded = serde_json::to_vec(body).unwrap_or_else(|e| {
            panic!(
                "TestRequest::json(): the body for {} {} does not serialize as JSON: {e}",
                self.method, self.path
            )
        });
        self.body = Bytes::from(encoded);
        self.header("Content-Type", "application/json")
    }

    /// Send `body` URL-encoded, with `Content-Type:
    /// application/x-www-form-urlencoded`, the body an HTML form posts and
    /// a handler reads with `req.form()`. Takes pairs (`&[("name", "Ada")]`),
    /// a map or a struct.
    ///
    /// # Panics
    ///
    /// Panics when `body` fails to URL-encode, as a nested map does.
    pub fn form<T: Serialize + ?Sized>(mut self, body: &T) -> Self {
        let encoded = serde_urlencoded::to_string(body).unwrap_or_else(|e| {
            panic!(
                "TestRequest::form(): the body for {} {} does not URL-encode: {e}",
                self.method, self.path
            )
        });
        self.body = Bytes::from(encoded);
        self.header("Content-Type", "application/x-www-form-urlencoded")
    }

    /// Send the headers the Inertia client sends on a visit after the first:
    /// `X-Inertia: true`, its `Accept`, and `X-Inertia-Version` set to the
    /// installed configuration's asset version (the empty string when no
    /// configuration is installed), so the version check lets the visit
    /// through. Override the version with [`Self::inertia_version`].
    pub fn inertia(self) -> Self {
        let version = crate::App::inertia_registry()
            .installed_config()
            .map(|config| config.resolved_version())
            .unwrap_or_default();
        self.header("X-Inertia", "true")
            .header("Accept", INERTIA_ACCEPT)
            .header("X-Inertia-Version", version)
    }

    /// Send `version` as `X-Inertia-Version`, as a client holding an older
    /// or newer build does, to test the version check.
    pub fn inertia_version(self, version: impl Into<String>) -> Self {
        self.header("X-Inertia-Version", version)
    }

    /// Send the request and read the whole response.
    ///
    /// The response keeps the error report the framework attached to it
    /// ([`TestResponse::error_report`]) and, when the client was given one,
    /// the session store. Every cookie the response sets is kept for the
    /// client's next request.
    ///
    /// # Panics
    ///
    /// Panics naming the method and path when the path is not a valid
    /// request target, when the connection fails, or when the response does
    /// not arrive within the client's [`TestClient::timeout`].
    pub async fn send(self) -> TestResponse {
        let TestRequest {
            client,
            method,
            path,
            headers,
            body,
        } = self;
        let request = build_request(&method, &path, headers, body, &client.cookie_pairs());
        let exchanged = match tokio::time::timeout(client.timeout, client.exchange(request)).await {
            Ok(Ok(exchanged)) => exchanged,
            Ok(Err(e)) => panic!("TestClient: {method} {path} failed: {e}"),
            Err(_) => panic!(
                "TestClient: {method} {path} did not answer within {:?}; a handler or \
                 middleware is waiting on something that never comes, or raise the limit with \
                 TestClient::timeout",
                client.timeout
            ),
        };
        client.remember_cookies(&exchanged.headers);
        let mut response = TestResponse::new(exchanged.status, exchanged.headers, exchanged.body)
            .with_error_report(exchanged.report)
            .with_client_cookies(client.cookie_pairs());
        if let Some((store, cookie_name)) = &client.session {
            response = response.with_session_store(store.clone(), cookie_name.clone());
        }
        response
    }
}

/// The hyper request for `method` and `path`: the caller's headers,
/// `Host: localhost` unless set, and the client's cookies after any
/// `Cookie` the caller set.
///
/// # Panics
///
/// Panics naming the method and path when hyper refuses the request.
fn build_request(
    method: &Method,
    path: &str,
    headers: Vec<(String, String)>,
    body: Bytes,
    cookies: &[(String, String)],
) -> hyper::Request<Full<Bytes>> {
    let mut builder = hyper::Request::builder().method(method.clone()).uri(path);
    let mut has_host = false;
    let mut cookie_header = None;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("host") {
            has_host = true;
        }
        if name.eq_ignore_ascii_case("cookie") {
            cookie_header = Some(value);
            continue;
        }
        builder = builder.header(name, value);
    }
    if !has_host {
        builder = builder.header("Host", "localhost");
    }
    let held = cookies
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ");
    let cookie_header = match (cookie_header, held.is_empty()) {
        (Some(set), true) => Some(set),
        (Some(set), false) => Some(format!("{set}; {held}")),
        (None, false) => Some(held),
        (None, true) => None,
    };
    if let Some(cookie_header) = cookie_header {
        builder = builder.header("Cookie", cookie_header);
    }
    builder
        .body(Full::new(body))
        .unwrap_or_else(|e| panic!("TestClient: {method} {path} is not a valid request: {e}"))
}

#[cfg(test)]
mod tests {
    use super::expires_now;

    #[test]
    fn intt_a_set_cookie_expires_by_max_age_before_expires() {
        let expired = |line: &str| expires_now(line.split(';').skip(1));
        assert!(expired("a=1; Max-Age=0; Path=/"));
        assert!(expired("a=1; max-age=-5"));
        assert!(!expired("a=1; Max-Age=60"));
        assert!(expired("a=1; Expires=Thu, 01 Jan 1970 00:00:00 GMT"));
        assert!(!expired("a=1; Expires=Fri, 01 Jan 2100 00:00:00 GMT"));
        // RFC 6265: Max-Age wins over Expires.
        assert!(!expired(
            "a=1; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Max-Age=60"
        ));
        assert!(!expired("a=1; Path=/; HttpOnly"));
    }
}
