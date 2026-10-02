//! PAR-012 to PAR-015: with debug on, a 5xx that carries an error report
//! becomes the development error page for a browser or an Inertia visit.
//!
//! Each submodule holds one requirement. Every test is named after the
//! falsifier clause it observes.
//!
//! # How a request is driven
//!
//! [`exchange`] serves one request over an in-memory `tokio::io::duplex`
//! pipe: a hyper HTTP/1 client on one end, and on the other a hyper server
//! connection whose service is the real `handle_request`, the function
//! `Server::run` calls per connection. No port is bound. The client, the
//! server connection and the request all run in the test's own task, so a
//! `TestContainer::fake` or `TestContainer::scope` set up by the test is
//! the one the request sees. Running the request inside a hyper server
//! connection also puts hyper's frames on the stack, as in production,
//! which the PAR-013 tests need.
//!
//! # How debug mode is set
//!
//! "Debug on" is `suprnova::Config::is_debug()`. These tests set it
//! through `APP_DEBUG`, the env fallback that method reads when no
//! `AppConfig` is registered, under the binary's shared env lock and
//! `#[serial]`. They do not call `Config::register`: the config repository
//! has no way to unregister, and `responses.rs` in this binary drives debug
//! through `APP_DEBUG` with no registered `AppConfig`. Under plain
//! `cargo test`, where every test of the binary shares one process, a
//! registration here would outrank `APP_DEBUG` for every test that runs
//! after it. [`debug_mode`] checks that `Config::is_debug()` reads the
//! mode it set, so a registration added later fails loudly here.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::config::Config;
use suprnova::http::text;
use suprnova::{FrameworkError, Middleware, MiddlewareRegistry, Next, Request, Response, Router};

use crate::env_snapshot::{EnvSnapshot, set_env};

mod frames;
mod replacement;
mod request_context;
mod self_contained;

// ---------------------------------------------------------------------
// Debug mode
// ---------------------------------------------------------------------

/// Holds the env lock and restores every variable [`debug_mode`] set when
/// dropped. The snapshot is declared first so it is restored before the
/// lock is released.
pub(super) struct DebugMode {
    _env: EnvSnapshot,
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

/// Turn debug on or off for one test, and set `extra_env` beside it.
///
/// Hold the returned guard for the whole test.
pub(super) async fn debug_mode(on: bool, extra_env: &[(&'static str, &str)]) -> DebugMode {
    let lock = crate::env_lock::lock_env_async().await;
    let mut keys = vec!["APP_DEBUG"];
    keys.extend(extra_env.iter().map(|(key, _)| *key));
    let snapshot = EnvSnapshot::capture(&keys);
    set_env("APP_DEBUG", Some(if on { "true" } else { "false" }));
    for (key, value) in extra_env {
        set_env(key, Some(value));
    }
    assert_eq!(
        Config::is_debug(),
        on,
        "Config::is_debug() must read the APP_DEBUG this test set; \
         an AppConfig registered in this binary would outrank it"
    );
    DebugMode {
        _env: snapshot,
        _lock: lock,
    }
}

// ---------------------------------------------------------------------
// Request headers
// ---------------------------------------------------------------------

/// The `Accept` a browser sends on a hard navigation.
pub(super) const BROWSER_ACCEPT: &str =
    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";

/// A hard browser navigation.
pub(super) const BROWSER: &[(&str, &str)] = &[("Accept", BROWSER_ACCEPT)];

/// An Inertia XHR visit. It sends no `Accept`, so only `X-Inertia` can
/// make it a page's audience.
pub(super) const INERTIA_VISIT: &[(&str, &str)] = &[
    ("X-Inertia", "true"),
    ("X-Requested-With", "XMLHttpRequest"),
];

/// An API client that asks for JSON.
pub(super) const JSON_CLIENT: &[(&str, &str)] = &[("Accept", "application/json")];

// ---------------------------------------------------------------------
// Failing handlers
// ---------------------------------------------------------------------

/// The invoice handler's error chain, outermost first.
pub(super) const INVOICE_ERROR: &str = "posting the invoice failed";
pub(super) const LEDGER_ERROR: &str = "writing ledger entry 42 failed";
pub(super) const DISK_ERROR: &str = "disk ledger-volume-3 is full";

/// What [`read_ledger_index_page`] panics with.
pub(super) const PANIC_MESSAGE: &str = "ledger index page 7 is unreadable";

/// A leaf error, with no source of its own.
#[derive(Debug)]
pub(super) struct RootCause(pub(super) String);

impl std::fmt::Display for RootCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RootCause {}

/// An error whose `source()` is a [`RootCause`]. Wrapped once more by
/// `FrameworkError::from_external_with`, it makes a three-link chain.
#[derive(Debug)]
pub(super) struct Failed {
    pub(super) message: String,
    pub(super) cause: RootCause,
}

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Failed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

/// The error `outer`, caused by `middle`, caused by `root`.
pub(super) fn chain(outer: &str, middle: &str, root: &str) -> FrameworkError {
    FrameworkError::from_external_with(
        outer.to_string(),
        Failed {
            message: middle.to_string(),
            cause: RootCause(root.to_string()),
        },
    )
}

fn write_invoice() -> Result<(), FrameworkError> {
    Err(chain(INVOICE_ERROR, LEDGER_ERROR, DISK_ERROR))
}

/// Fails the way application code does: a foreign error, given context
/// and propagated with `?`.
pub(super) async fn post_invoice(_req: Request) -> Response {
    write_invoice()?;
    text("posted")
}

/// The line [`read_ledger_index_page`] panics on. It is recorded when the
/// function runs, so the location check does not depend on how this file
/// is formatted.
pub(super) static PANIC_LINE: AtomicU32 = AtomicU32::new(0);

/// The file [`read_ledger_index_page`] panics in.
pub(super) const PANIC_FILE: &str = file!();

/// A handler that calls a function that panics.
pub(super) async fn read_ledger_index(_req: Request) -> Response {
    let page = read_ledger_index_page();
    text(page)
}

/// Panics. PAR-013 expects its name among the page's shown frames.
pub(super) fn read_ledger_index_page() -> String {
    PANIC_LINE.store(line!() + 1, Ordering::SeqCst);
    panic!("{PANIC_MESSAGE}");
}

/// Whether `text` names `file:line`, with or without a column after it.
pub(super) fn names_location(text: &str, file: &str, line: u32) -> bool {
    let site = format!("{file}:{line}");
    text.match_indices(&site)
        .any(|(at, _)| !text[at + site.len()..].starts_with(|c: char| c.is_ascii_digit()))
}

/// Stands in for `SessionMiddleware`: the Inertia error page reads and
/// writes the session, so it runs inside a session scope as it does in an
/// app. A fresh slot per request keeps the tests independent.
pub(super) struct SessionScope;

#[async_trait]
impl Middleware for SessionScope {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let slot = suprnova::session::new_session_slot_for_test();
        suprnova::session::session_scope_for_test(slot, next(request)).await
    }
}

// ---------------------------------------------------------------------
// Driving a request
// ---------------------------------------------------------------------

/// What came back over the connection.
#[derive(Debug)]
pub(super) struct Reply {
    pub(super) status: u16,
    /// Every header line, in wire order, names lower-cased.
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: String,
}

impl Reply {
    /// The first value of header `name`.
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Every value of header `name`.
    pub(super) fn header_values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> {
        self.headers
            .iter()
            .filter(move |(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Every header line as it goes on the wire, one per line.
    pub(super) fn wire_headers(&self) -> String {
        self.headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}\n"))
            .collect()
    }

    pub(super) fn content_type(&self) -> &str {
        self.header("content-type").unwrap_or("")
    }

    /// The body with HTML character references decoded: what a reader of
    /// the page sees. Presence checks use this, so they do not depend on
    /// which characters the page chose to escape.
    pub(super) fn text(&self) -> String {
        decode_character_references(&self.body)
    }
}

/// Build a request for [`exchange`].
pub(super) fn request(
    method: &str,
    target: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> hyper::Request<Full<Bytes>> {
    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(target)
        .header("Host", "localhost");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    builder
        .header("Content-Length", body.len())
        .body(Full::new(Bytes::from(body.to_string())))
        .expect("the test builds a well-formed request")
}

/// Serve `request` through `handle_request` over an in-memory HTTP/1
/// connection and return what the client received.
pub(super) async fn exchange(
    router: Router,
    registry: MiddlewareRegistry,
    request: hyper::Request<Full<Bytes>>,
) -> Reply {
    let router = Arc::new(router);
    let registry = Arc::new(registry);
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

/// GET `target` with `headers`, with no global middleware.
pub(super) async fn get(router: Router, target: &str, headers: &[(&str, &str)]) -> Reply {
    exchange(
        router,
        MiddlewareRegistry::new(),
        request("GET", target, headers, ""),
    )
    .await
}

// ---------------------------------------------------------------------
// Reading the page
// ---------------------------------------------------------------------

/// Fail unless `reply` is the development error page with `status`.
///
/// The page is one HTML document: `text/html`, opening with a doctype and
/// holding a single `<html>` element. It is not an Inertia response, and
/// not the JSON error body.
pub(super) fn assert_debug_page(reply: &Reply, status: u16) {
    assert_eq!(
        reply.status, status,
        "the development error page keeps the error's status; body: {}",
        reply.body
    );
    assert!(
        reply.content_type().starts_with("text/html"),
        "with debug on, the error must be the development error page, an HTML document; \
         got content-type {:?} and body: {}",
        reply.content_type(),
        reply.body
    );
    assert!(
        reply.header("x-inertia").is_none() && !reply.body.contains("data-page="),
        "the development error page is not an Inertia response or the Inertia HTML shell; \
         headers:\n{}body: {}",
        reply.wire_headers(),
        reply.body
    );
    let lower = reply.body.to_ascii_lowercase();
    assert!(
        lower.trim_start().starts_with("<!doctype html"),
        "the page must open with an HTML doctype; body: {}",
        reply.body
    );
    assert_eq!(
        lower.matches("<html").count(),
        1,
        "the page must be one HTML document; body: {}",
        reply.body
    );
    assert!(
        lower.trim_end().ends_with("</html>"),
        "the page must close its document; body: {}",
        reply.body
    );
}

/// Fail unless the page shows every one of `texts`.
pub(super) fn assert_page_shows(reply: &Reply, texts: &[&str]) {
    let shown = reply.text();
    for text in texts {
        assert!(
            shown.contains(text),
            "the page must show {text:?}; page:\n{}",
            reply.body
        );
    }
}

/// Decode HTML character references: the named ones an escaper uses and
/// every numeric one. Anything else is left as it is.
pub(super) fn decode_character_references(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let decoded = rest.find(';').and_then(|end| {
            let name = &rest[1..end];
            let ch = match name {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                    .and_then(char::from_u32),
            };
            ch.map(|ch| (ch, end + 1))
        });
        match decoded {
            Some((ch, consumed)) => {
                out.push(ch);
                rest = &rest[consumed..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------
// Routes most tests share
// ---------------------------------------------------------------------

/// `/invoice` fails with a three-link chain; `/ledger-index` panics.
pub(super) fn ledger_routes() -> Router {
    Router::new()
        .get("/invoice", post_invoice)
        .get("/ledger-index", read_ledger_index)
        .into()
}
