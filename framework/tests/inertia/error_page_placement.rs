//! An app that places the Inertia error page middleware itself.
//!
//! `Inertia::install` puts `InertiaErrorPageMiddleware` innermost of the
//! Inertia layer, which covers the handler, the route middleware, and
//! everything registered *after* the install. It cannot cover anything
//! registered above it: a middleware that answers without calling `next`
//! hands its response to nothing registered inside it. An app whose
//! `CsrfMiddleware` sits above the install therefore answers a
//! lapsed-session form post with `419 {"message":"CSRF token mismatch."}`
//! that reaches the Inertia client as header-less JSON - the crash modal
//! the error page exists to remove. The fix is for the app to register the
//! middleware itself, further out, and for `install` to leave it alone.
//!
//! **Why this is its own test binary.** The global middleware registry is
//! process-global, and the shape under test needs it to start *empty*: the
//! app registers the error-page middleware, and `install` is then observed
//! skipping its own. `inertia_error_page.rs` installs the Inertia layer
//! before its first test runs and a dozen of its tests read the registry
//! without `#[serial]`, so clearing it there would race them. A separate
//! binary gets a registry of its own for free, the same way
//! `inertia_production_fail_closed.rs` does.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Once};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::json;

use suprnova::middleware::{global_middleware_count, register_global_middleware};
use suprnova::{
    HttpResponse, Inertia, InertiaConfig, InertiaErrorPageMiddleware, Middleware,
    MiddlewareRegistry, Next, Request, Response, Router, handle_request,
};

/// Asset version pinned into both the install and every Inertia visit, so
/// these tests exercise the error page rather than the version bounce.
const ASSET_VERSION: &str = "test-version";

/// The page component the whole file is about.
const ERROR_PAGE: &str = "Error";

// ---------------------------------------------------------------------
// The app's stack
// ---------------------------------------------------------------------

/// Stands in for `SessionMiddleware`. The error page resolves the app's
/// shared props on the way out, so it has to render inside a session
/// scope the way it does in a real app; a fresh slot per request keeps the
/// tests independent.
struct SessionScope;

#[async_trait::async_trait]
impl Middleware for SessionScope {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let slot = suprnova::session::new_session_slot_for_test();
        suprnova::session::session_scope_for_test(slot, next(request)).await
    }
}

/// Stands in for `CsrfMiddleware` registered above `Inertia::install` -
/// same status, same body (`framework/src/csrf/middleware.rs`
/// `reject_with_419`), and the same refusal to call `next`.
struct RejectsLikeCsrf;

#[async_trait::async_trait]
impl Middleware for RejectsLikeCsrf {
    async fn handle(&self, _request: Request, _next: Next) -> Response {
        Err(HttpResponse::json(json!({ "message": "CSRF token mismatch." })).status(419))
    }
}

static BOOT: Once = Once::new();

/// Boot the app's real stack once for the binary, in the order a
/// `bootstrap.rs` would write it: session, then the error page, then CSRF,
/// then `Inertia::install`.
///
/// `install` is told to use the same component and adds only its **four**
/// protocol middlewares, not a fifth, so exactly one error-page middleware
/// is in the chain. That it is the one the **app** placed - outside
/// `RejectsLikeCsrf` rather than innermost - is what every test below
/// proves: a `419` answered above the Inertia layer could not become a
/// page any other way.
///
/// Stated precisely, because the difference matters when reading a
/// failure: this delta shows the net contract (`install` adds no second),
/// not the mechanism. Registration is idempotent per type, so a duplicate
/// would be dropped by the registry even if `install` did not check first.
/// The explicit check is pinned by `error_page_action`'s unit test in
/// `framework/src/inertia/facade.rs`.
fn boot() -> MiddlewareRegistry {
    BOOT.call_once(|| {
        register_global_middleware(SessionScope);
        register_global_middleware(InertiaErrorPageMiddleware::new(ERROR_PAGE));
        register_global_middleware(RejectsLikeCsrf);

        let before = global_middleware_count();
        Inertia::install(
            &InertiaConfig::new()
                .development(true)
                .version(ASSET_VERSION)
                .error_page(ERROR_PAGE),
        )
        .expect("dev-mode install needs no manifest");
        assert_eq!(
            global_middleware_count() - before,
            4,
            "install must register its four protocol middlewares and skip the \
             error page the app already placed; a delta of 5 means a second \
             error-page middleware went in innermost"
        );
    });
    MiddlewareRegistry::from_global()
}

/// One route, never reached: the `419` is answered above the router. It
/// exists so the request has somewhere to have been going.
fn router() -> Router {
    Router::new()
        .post("/register", |_req| async {
            let ok: Response = Ok(HttpResponse::text("registered"));
            ok
        })
        .into()
}

// ---------------------------------------------------------------------
// Loopback harness
// ---------------------------------------------------------------------

async fn spawn_server(
    router: impl Into<Router>,
    registry: MiddlewareRegistry,
    accepts: usize,
) -> SocketAddr {
    let router = Arc::new(router.into());
    let middleware = Arc::new(registry);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let io = TokioIo::new(stream);
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    addr
}

/// Send a request; return `(status, lowercased headers, body)`.
async fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> (u16, HashMap<String, String>, String) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
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
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let req = builder.body(Full::new(Bytes::new())).unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_request timeout")
        .expect("hyper send_request");

    let (parts, body) = resp.into_parts();
    let status = parts.status.as_u16();
    let header_map = parts
        .headers
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_lowercase(),
                v.to_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    let bytes = body.collect().await.unwrap().to_bytes();
    (
        status,
        header_map,
        String::from_utf8_lossy(&bytes).to_string(),
    )
}

fn page_object(body: &str) -> serde_json::Value {
    serde_json::from_str(body)
        .unwrap_or_else(|e| panic!("expected an Inertia page object: {e}\n{body}"))
}

/// Pull the page object out of the `<script type="application/json"
/// data-page="app">` element the HTML shell embeds. The renderer escapes
/// every `/` as `\/` so a literal `</script>` cannot terminate the tag.
fn embedded_page_object(html: &str) -> serde_json::Value {
    let open = "<script type=\"application/json\" data-page=\"app\">";
    let start = html
        .find(open)
        .unwrap_or_else(|| panic!("no embedded page object in:\n{html}"))
        + open.len();
    let end = start
        + html[start..]
            .find("</script>")
            .unwrap_or_else(|| panic!("unterminated page script in:\n{html}"));
    serde_json::from_str(&html[start..end].replace("\\/", "/"))
        .unwrap_or_else(|e| panic!("embedded page object did not parse: {e}"))
}

// ---------------------------------------------------------------------
// The 419 the default placement cannot reach
// ---------------------------------------------------------------------

#[tokio::test]
async fn an_app_placed_error_page_covers_a_419_answered_before_the_inertia_layer() {
    let addr = spawn_server(router(), boot(), 2).await;

    let (status, headers, body) = request(
        addr,
        "POST",
        "/register",
        &[
            ("X-Inertia", "true"),
            ("X-Inertia-Version", ASSET_VERSION),
            ("Accept", "text/html, application/xhtml+xml"),
        ],
    )
    .await;

    assert_eq!(status, 419, "the lapsed session keeps its status");
    assert_eq!(
        headers.get("x-inertia").map(String::as_str),
        Some("true"),
        "without this the client shows the crash modal this feature exists to \
         remove; got {headers:?}"
    );
    let page = page_object(&body);
    assert_eq!(page["component"], ERROR_PAGE);
    assert_eq!(page["props"]["status"], 419);
    assert_eq!(page["props"]["message"], "CSRF token mismatch.");
}

#[tokio::test]
async fn the_same_419_renders_the_html_shell_for_a_browser_navigation() {
    let addr = spawn_server(router(), boot(), 2).await;

    let (status, headers, body) = request(
        addr,
        "POST",
        "/register",
        &[(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )],
    )
    .await;

    assert_eq!(status, 419);
    assert!(
        headers
            .get("content-type")
            .is_some_and(|c| c.starts_with("text/html")),
        "a hard navigation gets the same shell a first load of any page gets; \
         got {headers:?}"
    );
    let page = embedded_page_object(&body);
    assert_eq!(page["component"], ERROR_PAGE);
    assert_eq!(page["props"]["status"], 419);
}

#[tokio::test]
async fn an_api_client_still_gets_the_419_json_untouched() {
    let addr = spawn_server(router(), boot(), 2).await;

    let (status, headers, body) =
        request(addr, "POST", "/register", &[("Accept", "application/json")]).await;

    assert_eq!(status, 419);
    assert!(
        !headers.contains_key("x-inertia"),
        "an API client never asked for a page; got {headers:?}"
    );
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("JSON error body");
    assert_eq!(parsed["message"], "CSRF token mismatch.");
    assert!(
        parsed.get("component").is_none(),
        "the body is the middleware's own, not a page object; got {body}"
    );
}
