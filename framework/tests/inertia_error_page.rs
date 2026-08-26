//! Framework error responses render the app's Inertia error page.
//!
//! The Inertia client treats any response without `X-Inertia: true` as
//! non-Inertia (`inertia-3.6.1/packages/core/src/response.ts:68,173-175`)
//! and hands it to the full-screen error modal. So a `403` from a
//! permission middleware, a `404` for an unrouted path, or a `500`
//! reaches the user as "All Inertia requests must receive a valid Inertia
//! response, however a plain JSON response was received" - which is
//! exactly what a `member` clicking an admin link saw in production.
//!
//! These drive `handle_request` over a loopback socket with real Inertia
//! headers, because the behaviour lives in the interaction between the
//! middleware chain, the RBAC route middleware, and the page renderer -
//! none of which a unit test sees together.

use std::any::Any;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::json;
use serial_test::serial;

use suprnova::rbac::migrations::CreateRbacTables;
use suprnova::session::SessionData;
use suprnova::testing::TestDatabase;
use suprnova::{
    Auth, Authenticatable, FrameworkError, HasRoles, HttpResponse, Inertia, InertiaConfig,
    InertiaResponse, Middleware, MiddlewareRegistry, Next, PermissionMiddleware, Request, Response,
    Router, ValidationErrors, handle_request,
};

/// Asset version pinned into both the install and every Inertia visit
/// below. `InertiaVersionMiddleware` answers an Inertia GET whose
/// `X-Inertia-Version` does not match with a `409`, and an absent header
/// reads as the empty string - so without an explicit agreed version
/// every test here would exercise the version bounce instead of the
/// error page.
const ASSET_VERSION: &str = "test-version";

/// The page component the whole file is about.
const ERROR_PAGE: &str = "Error";

// ---------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------

#[derive(Clone)]
struct User {
    id: i64,
}

impl Authenticatable for User {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

impl HasRoles for User {}

struct TestMigrator;

impl sea_orm_migration::MigratorTrait for TestMigrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        vec![Box::new(CreateRbacTables)]
    }
}

/// A `member`: a real, authenticated user who simply does not hold
/// `articles.create`. This is the production shape of the bug.
async fn seed_member() -> TestDatabase {
    let db = TestDatabase::fresh::<TestMigrator>().await.unwrap();
    suprnova::rbac::create_role("member").await.unwrap();
    suprnova::rbac::create_permission("articles.create")
        .await
        .unwrap();
    suprnova::rbac::assign_role_to_model(&User { id: 7 }.rbac_model_type(), "7", "member")
        .await
        .unwrap();
    db
}

/// Route middleware that authenticates the request as user 7. Stands in
/// for the session guard so the RBAC middleware has a user to deny.
struct LoginAsMember;

#[async_trait::async_trait]
impl Middleware for LoginAsMember {
    async fn handle(&self, request: Request, next: Next) -> Response {
        Auth::set_user(Arc::new(User { id: 7 }));
        next(request).await
    }
}

/// Test-only stand-in for `SessionMiddleware`, so the validation-redirect
/// middleware has a flash bag to write into.
struct SeededSessionScope(Arc<Mutex<Option<SessionData>>>);

#[async_trait::async_trait]
impl Middleware for SeededSessionScope {
    async fn handle(&self, request: Request, next: Next) -> Response {
        suprnova::session::session_scope_for_test(self.0.clone(), next(request)).await
    }
}

static INSTALL: Once = Once::new();

/// Install the Inertia protocol layer once for the whole binary.
///
/// `Inertia::install` writes to the process-global middleware registry,
/// so it has to happen exactly once no matter how many tests run in
/// parallel. `development(true)` keeps the install off the Vite manifest
/// that a test checkout has never built.
fn install_inertia() {
    INSTALL.call_once(|| {
        Inertia::install(
            &InertiaConfig::new()
                .development(true)
                .version(ASSET_VERSION)
                .error_page(ERROR_PAGE),
        )
        .expect("dev-mode install needs no manifest");
    });
}

/// The app's real global stack: whatever `Inertia::install` registered.
fn stack() -> MiddlewareRegistry {
    install_inertia();
    MiddlewareRegistry::from_global()
}

fn router() -> Router {
    Router::new()
        // The production shape: a member visiting an admin page.
        .get("/admin/articles", |_req| async {
            let ok: Response = Ok(HttpResponse::text("admin"));
            ok
        })
        .middleware(LoginAsMember)
        .middleware(PermissionMiddleware::<User>::new("articles.create"))
        // A handler that fails with a 5xx carrying detail that must
        // never reach a browser.
        .get("/boom", |_req| async {
            let resp: Response = Err(HttpResponse::from(FrameworkError::Domain {
                message: "connection string postgres://admin:hunter2@db/app refused".to_string(),
                status_code: 500,
            }));
            resp
        })
        // A handler that panics. Recorded here because the panic net
        // sits ABOVE the middleware chain, so this response is one the
        // error page provably cannot reach.
        .get("/panic", |_req: Request| async {
            panic!("handler exploded");
        })
        // Middleware somewhere answers an unauthenticated Inertia visit
        // with 401 JSON rather than a redirect.
        .get("/needs-login", |_req| async {
            let resp: Response =
                Err(HttpResponse::json(json!({ "message": "Unauthenticated." })).status(401));
            resp
        })
        // A plain redirect. Nothing to render an error page for.
        .get("/moved", |_req| async {
            let resp: Response = Ok(HttpResponse::new().status(302).header("Location", "/login"));
            resp
        })
        // A handler that returns its own Inertia page WITH an error
        // status - already a valid Inertia response.
        .get("/gone", |req: Request| async move {
            InertiaResponse::new("Articles/Gone")
                .with("reason", "archived")
                .resolve(&req)
                .await
                .map(|r| r.status(410))
                .map_err(HttpResponse::from)
        })
        // A validating handler, shaped exactly like a `FormRequest`
        // extraction failure.
        .post("/register", |_req| async {
            let mut errs = ValidationErrors::new();
            errs.add("email", "The email field is required.");
            let resp: Response = Err(HttpResponse::from(FrameworkError::validation_errors(errs)));
            resp
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

/// Headers an Inertia XHR visit carries.
fn inertia_visit() -> Vec<(&'static str, &'static str)> {
    vec![
        ("X-Inertia", "true"),
        ("X-Inertia-Version", ASSET_VERSION),
        ("Accept", "text/html, application/xhtml+xml"),
    ]
}

/// Headers a browser hard navigation carries.
fn browser_navigation() -> Vec<(&'static str, &'static str)> {
    vec![(
        "Accept",
        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
    )]
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
    let escaped = &html[start..end];
    serde_json::from_str(&escaped.replace("\\/", "/"))
        .unwrap_or_else(|e| panic!("embedded page object did not parse: {e}\n{escaped}"))
}

// ---------------------------------------------------------------------
// The behaviour this release adds
// ---------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn an_inertia_visit_denied_by_a_permission_middleware_renders_the_error_page() {
    let _db = seed_member().await;
    let addr = spawn_server(router(), stack(), 2).await;

    let (status, headers, body) = request(addr, "GET", "/admin/articles", &inertia_visit()).await;

    assert_eq!(status, 403, "the denial keeps its status");
    assert_eq!(
        headers.get("x-inertia").map(String::as_str),
        Some("true"),
        "without this header the client shows its plain-JSON modal; got {headers:?}"
    );
    let page = page_object(&body);
    assert_eq!(page["component"], ERROR_PAGE);
    assert_eq!(page["props"]["status"], 403);
    assert_eq!(page["props"]["message"], "This action is unauthorized.");
    assert!(
        page["props"]["request_id"].is_string(),
        "the request id must survive so the page can show it; got {body}"
    );
}

#[tokio::test]
#[serial]
async fn a_browser_navigation_to_the_same_denial_renders_the_html_shell() {
    let _db = seed_member().await;
    let addr = spawn_server(router(), stack(), 2).await;

    let (status, headers, body) =
        request(addr, "GET", "/admin/articles", &browser_navigation()).await;

    assert_eq!(status, 403);
    assert!(
        headers
            .get("content-type")
            .is_some_and(|c| c.starts_with("text/html")),
        "a hard navigation gets the same shell a first load of any page gets; got {headers:?}"
    );
    let page = embedded_page_object(&body);
    assert_eq!(page["component"], ERROR_PAGE);
    assert_eq!(page["props"]["status"], 403);
}

#[tokio::test]
async fn an_inertia_visit_to_an_unknown_route_renders_the_error_page() {
    let addr = spawn_server(router(), stack(), 2).await;

    let (status, headers, body) = request(addr, "GET", "/no/such/page", &inertia_visit()).await;

    assert_eq!(status, 404);
    assert_eq!(headers.get("x-inertia").map(String::as_str), Some("true"));
    let page = page_object(&body);
    assert_eq!(page["component"], ERROR_PAGE);
    assert_eq!(page["props"]["status"], 404);
    assert_eq!(
        page["props"]["message"], "Not Found",
        "the router's own 404 carries no message, so the reason phrase stands in; got {body}"
    );
}

#[tokio::test]
async fn a_failing_handler_renders_the_error_page_with_the_sanitized_message() {
    let addr = spawn_server(router(), stack(), 2).await;

    let (status, _headers, body) = request(addr, "GET", "/boom", &inertia_visit()).await;

    assert_eq!(status, 500);
    let page = page_object(&body);
    assert_eq!(page["component"], ERROR_PAGE);
    assert_eq!(page["props"]["status"], 500);
    assert_eq!(
        page["props"]["message"], "Internal Server Error",
        "the error page reads the same sanitized body the JSON path emits; got {body}"
    );
    assert!(
        !body.contains("hunter2"),
        "the underlying error must never reach the page; got {body}"
    );
}

#[tokio::test]
async fn a_401_answered_in_json_gets_the_page_too() {
    let addr = spawn_server(router(), stack(), 2).await;

    let (status, headers, body) = request(addr, "GET", "/needs-login", &inertia_visit()).await;

    assert_eq!(status, 401, "the status is the app's answer, not ours");
    assert_eq!(headers.get("x-inertia").map(String::as_str), Some("true"));
    let page = page_object(&body);
    assert_eq!(page["component"], ERROR_PAGE);
    assert_eq!(page["props"]["status"], 401);
    assert_eq!(page["props"]["message"], "Unauthenticated.");
}

// ---------------------------------------------------------------------
// Everything that must stay exactly as it was
// ---------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn an_api_client_asking_for_json_keeps_the_json_body() {
    let _db = seed_member().await;
    let addr = spawn_server(router(), stack(), 2).await;

    let (status, headers, body) = request(
        addr,
        "GET",
        "/admin/articles",
        &[("Accept", "application/json")],
    )
    .await;

    assert_eq!(status, 403);
    assert!(
        headers.get("x-inertia").is_none(),
        "an API client never asked for a page; got {headers:?}"
    );
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("JSON error body");
    assert_eq!(parsed["message"], "This action is unauthorized.");
    assert!(parsed.get("request_id").is_some());
}

#[tokio::test]
#[serial]
async fn error_page_off_leaves_the_denial_byte_for_byte() {
    // No error-page middleware in the chain at all - which is exactly
    // what `Inertia::install` produces when `error_page` is unset. The
    // gate itself is pinned by `install_registers_the_error_page_middleware_only_when_configured`
    // in `framework/src/inertia/facade.rs`.
    let _db = seed_member().await;
    let addr = spawn_server(router(), MiddlewareRegistry::new(), 2).await;

    let (status, headers, body) = request(addr, "GET", "/admin/articles", &inertia_visit()).await;

    assert_eq!(status, 403);
    assert!(headers.get("x-inertia").is_none());
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("JSON error body");
    assert_eq!(parsed["message"], "This action is unauthorized.");
    assert!(
        parsed["request_id"].is_string(),
        "the pre-1.3.6 body shape is `message` + `request_id`; got {body}"
    );
}

#[tokio::test]
async fn a_validation_failure_still_redirects_back() {
    let slot = suprnova::session::new_session_slot_for_test();
    slot.lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .put("_previous.url", "/register");
    // The session has to wrap the whole Inertia layer: the
    // validation-redirect middleware writes the error bag into it.
    let registry = stack().prepend(SeededSessionScope(slot.clone()));
    let addr = spawn_server(router(), registry, 2).await;

    let (status, headers, body) = request(
        addr,
        "POST",
        "/register",
        &[
            ("X-Inertia", "true"),
            ("X-Inertia-Version", ASSET_VERSION),
            ("Referer", "http://localhost/register"),
        ],
    )
    .await;

    assert_eq!(
        status, 303,
        "the validation-redirect middleware owns 422, not the error page"
    );
    assert_eq!(
        headers.get("location").map(String::as_str),
        Some("/register")
    );
    assert!(body.is_empty(), "got {body}");
    let guard = slot.lock().unwrap();
    let bag: serde_json::Value = guard
        .as_ref()
        .unwrap()
        .get("_flash.new.errors.default")
        .expect("errors must still be flashed");
    assert_eq!(bag["email"][0], "The email field is required.");
}

#[tokio::test]
async fn a_version_mismatch_still_bounces_with_an_inertia_location() {
    let addr = spawn_server(router(), stack(), 2).await;

    let (status, headers, body) = request(
        addr,
        "GET",
        "/admin/articles",
        &[
            ("X-Inertia", "true"),
            ("X-Inertia-Version", "stale-bundle"),
            ("Accept", "text/html, application/xhtml+xml"),
        ],
    )
    .await;

    assert_eq!(status, 409);
    assert_eq!(
        headers.get("x-inertia-location").map(String::as_str),
        Some("/admin/articles"),
        "the client needs the location to do its full-page reload"
    );
    assert!(body.is_empty(), "got {body}");
}

#[tokio::test]
async fn a_redirect_is_left_alone() {
    let addr = spawn_server(router(), stack(), 2).await;

    let (status, headers, body) = request(addr, "GET", "/moved", &inertia_visit()).await;

    assert_eq!(status, 302);
    assert_eq!(headers.get("location").map(String::as_str), Some("/login"));
    assert!(body.is_empty(), "got {body}");
}

#[tokio::test]
async fn a_handlers_own_inertia_page_keeps_its_component_even_on_an_error_status() {
    let addr = spawn_server(router(), stack(), 2).await;

    let (status, headers, body) = request(addr, "GET", "/gone", &inertia_visit()).await;

    assert_eq!(status, 410);
    assert_eq!(headers.get("x-inertia").map(String::as_str), Some("true"));
    let page = page_object(&body);
    assert_eq!(
        page["component"], "Articles/Gone",
        "a response that is already an Inertia page is never rewritten; got {body}"
    );
    assert_eq!(page["props"]["reason"], "archived");
}

#[tokio::test]
async fn a_panicking_handler_is_out_of_reach_of_the_error_page() {
    // `execute_chain_safely` (framework/src/server.rs) wraps the WHOLE
    // middleware chain in `catch_unwind`, so a panic unwinds every
    // middleware frame - this one included - before the synthesized 500
    // exists. No middleware can rewrite it. Pinned here so the gap is a
    // recorded fact rather than a surprise in production.
    let addr = spawn_server(router(), stack(), 2).await;

    let (status, headers, body) = request(addr, "GET", "/panic", &inertia_visit()).await;

    assert_eq!(status, 500);
    assert!(headers.get("x-inertia").is_none());
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("JSON error body");
    assert_eq!(parsed["message"], "Internal Server Error");
}
