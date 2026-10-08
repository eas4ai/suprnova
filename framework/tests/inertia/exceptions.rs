//! PAR-062: `Inertia::handle_exceptions_using` decides every error response
//! the framework renders, and `InertiaConfig::error_page` is the default
//! callback.
//!
//! Laravel's references are `ResponseFactory::handleExceptionsUsing` and
//! `ExceptionResponse` in inertia-laravel 3.5.1.
//!
//! Every test runs the real Inertia stack (`Inertia::middleware`) as global
//! middleware behind a loopback server, so the router's own `404` passes
//! through it too. The server is `protocol_harness::serve`, which answers
//! each connection with the in-process `handle_request` adapter, the
//! function `Server::run` calls, so the decision the server runs after the
//! whole stack runs here too. The callback lives on the test's own
//! container (`TestContainer::fake`); the server runs on the test's thread,
//! so the request sees it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use serial_test::serial;

use suprnova::indexmap::IndexMap;
use suprnova::testing::TestContainer;
use suprnova::{
    CsrfMiddleware, FrameworkError, HttpResponse, Inertia, InertiaConfig,
    InertiaErrorPageMiddleware, InertiaMiddlewareHooks, InertiaRequestExt, Middleware,
    MiddlewareRegistry, Next, Prop, Redirect, Request, Response, Router, ValidationErrors,
};

use crate::env_snapshot::{EnvSnapshot, set_env};
use crate::protocol_harness::{Client, Reply, SeededSessionScope, serve};

/// The asset version the stack and every Inertia visit agree on, so the
/// version check lets the visits through.
const VERSION: &str = "inssr-version";

/// What `/forbidden` is refused with.
const DENIAL: &str = "You may not close this ledger.";

/// What `/maintenance` fails with. A 5xx body never shows it.
const MAINTENANCE: &str = "the ledger is closed for the nightly reconciliation";

/// What `/panic` panics with.
const PANIC_MESSAGE: &str = "ledger index page 7 is unreadable";

fn routes() -> Router {
    Router::new()
        .get("/forbidden", |_req: Request| async {
            let response: Response = Err(FrameworkError::domain(DENIAL, 403).into());
            response
        })
        .get("/maintenance", |_req: Request| async {
            let response: Response = Err(FrameworkError::domain(MAINTENANCE, 503).into());
            response
        })
        .get("/panic", |_req: Request| async {
            panic!("{PANIC_MESSAGE}");
        })
        .get("/throttled", |_req: Request| async {
            let response: Response = Err(FrameworkError::rate_limited(
                Some(Duration::from_secs(30)),
                "too many requests",
            )
            .into());
            response
        })
        // A handler's own answer with an error status: no error became it.
        .get("/own-404", |_req: Request| async {
            let response: Response = Ok(HttpResponse::html("<h1>No such widget</h1>").status(404));
            response
        })
        // A redirect with a fragment: the headers middleware answers an
        // Inertia visit with `409` and `X-Inertia-Redirect`.
        .get("/fragment", |_req: Request| async {
            let response: Response = Redirect::to("/ledger#entry-7").into();
            response
        })
        // An external redirect: `409` and `X-Inertia-Location`.
        .get("/away", |_req: Request| async {
            let response: Response = Ok(Inertia::location("https://example.com/ledger"));
            response
        })
        // A validation failure, shaped exactly like a `FormRequest`
        // extraction failure.
        .post("/register", |_req: Request| async {
            let mut errors = ValidationErrors::new();
            errors.add("email", "The email field is required.");
            let response: Response = Err(FrameworkError::validation_errors(errors).into());
            response
        })
        .into()
}

/// The configuration every test starts from.
fn config() -> InertiaConfig {
    InertiaConfig::new().development(true).version(VERSION)
}

/// A client of `routes()` behind the Inertia stack `config` builds.
async fn client(config: &InertiaConfig) -> Client {
    let registry = MiddlewareRegistry::new().append(Inertia::middleware(config));
    Client::new(serve(routes(), registry).await)
}

/// An Inertia XHR visit.
const INERTIA_VISIT: &[(&str, &str)] = &[
    ("X-Inertia", "true"),
    ("X-Inertia-Version", VERSION),
    ("Accept", "text/html, application/xhtml+xml"),
];

/// A hard browser navigation.
const BROWSER: &[(&str, &str)] = &[(
    "Accept",
    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
)];

/// An API client that asks for JSON.
const JSON_CLIENT: &[(&str, &str)] = &[("Accept", "application/json")];

/// Debug mode off until dropped; the test must also be `#[serial]`.
///
/// With debug on, the default when `APP_ENV` is unset, a 5xx sent to an
/// Inertia visit or a browser is the development error page (PAR-012). The
/// tests that take this are about what production sends.
struct DebugOff {
    _env: EnvSnapshot,
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

async fn debug_off() -> DebugOff {
    let lock = crate::env_lock::lock_env_async().await;
    let snapshot = EnvSnapshot::capture(&["APP_DEBUG"]);
    set_env("APP_DEBUG", Some("false"));
    DebugOff {
        _env: snapshot,
        _lock: lock,
    }
}

/// Counts the callback's calls, so a test can tell "kept" from "never asked".
fn counter() -> (Arc<Mutex<u32>>, Arc<Mutex<u32>>) {
    let calls = Arc::new(Mutex::new(0));
    (Arc::clone(&calls), calls)
}

fn assert_json_error(reply: &Reply, status: u16, message: &str) {
    assert_eq!(reply.status, status, "{reply:?}");
    assert_eq!(reply.header("x-inertia"), None, "{reply:?}");
    let body: serde_json::Value = serde_json::from_str(&reply.body)
        .unwrap_or_else(|e| panic!("expected the JSON error body ({e}): {}", reply.body));
    assert_eq!(body["message"], message, "{}", reply.body);
}

/// The page object the HTML shell embeds.
fn embedded_page(html: &str) -> serde_json::Value {
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
        .unwrap_or_else(|e| panic!("the embedded page object did not parse ({e}):\n{html}"))
}

// ---------------------------------------------------------------------
// The callback decides
// ---------------------------------------------------------------------

#[tokio::test]
async fn inssr_a_callback_that_answers_a_404_with_a_418_replaces_it() {
    let _container = TestContainer::fake();
    Inertia::handle_exceptions_using(|error| {
        if error.status() == 404 {
            Some(error.respond_with(HttpResponse::text("I'm a teapot").status(418)))
        } else {
            None
        }
    });
    // `error_page` is set too: the callback wins.
    let mut client = client(&config().error_page("Error")).await;

    let reply = client.send("GET", "/no/such/page", INERTIA_VISIT).await;

    assert_eq!(reply.status, 418, "{reply:?}");
    assert_eq!(reply.body, "I'm a teapot");
    assert_eq!(reply.header("x-inertia"), None, "{reply:?}");
}

#[tokio::test]
async fn inssr_a_callback_returning_nothing_keeps_the_response() {
    let _container = TestContainer::fake();
    let (calls, seen) = counter();
    Inertia::handle_exceptions_using(move |_error| {
        *seen.lock().unwrap() += 1;
        None
    });
    // `error_page` is set too: the callback wins, and it kept the response.
    let mut client = client(&config().error_page("Error")).await;

    let reply = client.send("GET", "/forbidden", INERTIA_VISIT).await;

    assert_eq!(*calls.lock().unwrap(), 1, "the callback decides the 403");
    assert_json_error(&reply, 403, DENIAL);
}

#[tokio::test]
async fn inssr_a_page_the_callback_renders_keeps_the_original_status() {
    let _container = TestContainer::fake();
    Inertia::handle_exceptions_using(|error| {
        let status = error.status();
        Some(error.render(
            "Errors/Denied",
            json!({ "code": status, "reason": "closed" }),
        ))
    });
    let mut client = client(&config()).await;

    let reply = client.send("GET", "/forbidden", INERTIA_VISIT).await;
    assert_eq!(reply.status, 403, "{reply:?}");
    assert_eq!(reply.header("x-inertia"), Some("true"), "{reply:?}");
    assert_eq!(reply.header("cache-control"), Some("no-cache, private"));
    let page = reply.page();
    assert_eq!(page["component"], "Errors/Denied");
    assert_eq!(page["props"]["code"], 403);
    assert_eq!(page["props"]["reason"], "closed");

    // A browser navigation gets the same page in the HTML shell.
    let reply = client.send("GET", "/forbidden", BROWSER).await;
    assert_eq!(reply.status, 403, "{reply:?}");
    assert!(
        reply
            .header("content-type")
            .is_some_and(|c| c.starts_with("text/html")),
        "{reply:?}"
    );
    assert_eq!(embedded_page(&reply.body)["component"], "Errors/Denied");
}

/// Shares a prop through each middleware hook, `share` and `share_once`.
struct SharingHooks;

impl InertiaMiddlewareHooks for SharingHooks {
    fn version(&self, _request: &dyn InertiaRequestExt) -> Option<String> {
        Some(VERSION.to_string())
    }

    fn share(&self, request: &dyn InertiaRequestExt) -> IndexMap<String, Prop> {
        let mut props = IndexMap::new();
        props.insert("path".to_string(), Prop::eager(json!(request.path())));
        props
    }

    fn share_once(&self, _request: &dyn InertiaRequestExt) -> IndexMap<String, Prop> {
        let mut props = IndexMap::new();
        props.insert("plans".to_string(), Prop::eager(json!(["pro"])));
        props
    }
}

#[tokio::test]
async fn inssr_a_rendered_page_carries_shared_props_only_with_shared_data() {
    let _container = TestContainer::fake();
    Inertia::share("app_name", "Ledger").unwrap();
    Inertia::handle_exceptions_using(|error| {
        let status = error.status();
        let with_shared = error.request().header("X-With-Shared").is_some();
        let page = error.render("Error", json!({ "status": status }));
        if with_shared {
            Some(page.with_shared_data())
        } else {
            Some(page)
        }
    });
    let mut client = client(&config().hooks(SharingHooks)).await;

    let reply = client.send("GET", "/forbidden", INERTIA_VISIT).await;
    assert_eq!(reply.status, 403, "{reply:?}");
    let props = reply.page()["props"].clone();
    assert_eq!(props["status"], 403);
    for shared in ["app_name", "path", "plans"] {
        assert!(
            props.get(shared).is_none(),
            "{shared} is shared data, left out without with_shared_data(); got {props}"
        );
    }

    let mut headers = INERTIA_VISIT.to_vec();
    headers.push(("X-With-Shared", "1"));
    let reply = client.send("GET", "/forbidden", &headers).await;
    assert_eq!(reply.status, 403, "{reply:?}");
    let props = reply.page()["props"].clone();
    assert_eq!(
        props["app_name"], "Ledger",
        "the shared registry; got {props}"
    );
    assert_eq!(props["path"], "/forbidden", "the share hook; got {props}");
    assert_eq!(
        props["plans"],
        json!(["pro"]),
        "the share_once hook; got {props}"
    );
}

#[tokio::test]
async fn inssr_the_callback_sees_the_status_the_error_the_request_and_the_response() {
    let _container = TestContainer::fake();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    Inertia::handle_exceptions_using(move |error| {
        let body: serde_json::Value =
            serde_json::from_slice(error.response().body()).unwrap_or_default();
        record.lock().unwrap().push(json!({
            "status": error.status(),
            "error": error.error().chain(),
            "panic": error.error().is_panic(),
            "method": error.method(),
            "path": error.request().path(),
            "url": error.request().path_and_query(),
            "probe": error.request().header("X-Probe"),
            "response_status": error.response().status_code(),
            "response_message": body["message"],
        }));
        None
    });
    let mut client = client(&config()).await;

    let mut headers = JSON_CLIENT.to_vec();
    headers.push(("X-Probe", "ledger-7"));
    let reply = client.send("GET", "/maintenance?ledger=7", &headers).await;

    // Kept: a 5xx body never says what went wrong.
    assert_json_error(&reply, 503, "Internal Server Error");
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(
        seen[0],
        json!({
            "status": 503,
            "error": [MAINTENANCE],
            "panic": false,
            "method": "GET",
            "path": "/maintenance",
            "url": "/maintenance?ledger=7",
            "probe": "ledger-7",
            "response_status": 503,
            "response_message": "Internal Server Error",
        })
    );
}

#[tokio::test]
async fn inssr_a_later_callback_replaces_the_earlier_one() {
    let _container = TestContainer::fake();
    Inertia::handle_exceptions_using(|error| {
        Some(error.respond_with(HttpResponse::text("first").status(418)))
    });
    Inertia::handle_exceptions_using(|error| {
        Some(error.respond_with(HttpResponse::text("second").status(451)))
    });
    let mut client = client(&config()).await;

    let reply = client.send("GET", "/forbidden", INERTIA_VISIT).await;

    assert_eq!(reply.status, 451, "{reply:?}");
    assert_eq!(reply.body, "second");
}

#[tokio::test]
async fn inssr_a_rendered_page_keeps_the_headers_an_error_page_keeps() {
    let _container = TestContainer::fake();
    Inertia::handle_exceptions_using(|error| Some(error.render("Error", ())));
    let mut client = client(&config()).await;

    let reply = client.send("GET", "/throttled", INERTIA_VISIT).await;

    assert_eq!(reply.status, 429, "{reply:?}");
    assert_eq!(reply.page()["component"], "Error");
    assert_eq!(reply.header("retry-after"), Some("30"), "{reply:?}");
    assert_eq!(reply.header("cache-control"), Some("no-cache, private"));
    assert_eq!(
        reply.header("content-type"),
        Some("application/json"),
        "the page's own content type, not the error body's"
    );
    let length: usize = reply
        .header("content-length")
        .expect("a buffered page carries its length")
        .parse()
        .unwrap();
    assert_eq!(length, reply.body.len(), "the length describes the page");
}

#[tokio::test]
async fn inssr_props_that_are_not_an_object_keep_the_response() {
    let _container = TestContainer::fake();
    let (calls, seen) = counter();
    Inertia::handle_exceptions_using(move |error| {
        *seen.lock().unwrap() += 1;
        Some(error.render("Error", vec![1, 2]))
    });
    let mut client = client(&config()).await;

    let reply = client.send("GET", "/forbidden", INERTIA_VISIT).await;

    assert_eq!(*calls.lock().unwrap(), 1, "the callback decides the 403");
    assert_json_error(&reply, 403, DENIAL);
}

// ---------------------------------------------------------------------
// Every rendered error, every request type
// ---------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn inssr_the_callback_sees_a_handler_panic() {
    let _debug = debug_off().await;
    let _container = TestContainer::fake();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    Inertia::handle_exceptions_using(move |error| {
        record.lock().unwrap().push((
            error.status(),
            error.error().is_panic(),
            error.error().chain().to_vec(),
            error.error().panic_location().is_some(),
        ));
        let status = error.status();
        Some(error.render("Error", json!({ "status": status })))
    });
    let mut client = client(&config()).await;

    let reply = client.send("GET", "/panic", INERTIA_VISIT).await;

    assert_eq!(reply.status, 500, "{reply:?}");
    assert_eq!(reply.header("x-inertia"), Some("true"), "{reply:?}");
    assert_eq!(reply.page()["component"], "Error");
    let seen = seen.lock().unwrap();
    assert_eq!(
        *seen,
        vec![(500, true, vec![PANIC_MESSAGE.to_string()], true)],
        "the callback sees the panic's message and where it was raised"
    );
}

#[tokio::test]
async fn inssr_a_json_clients_404_is_kept_when_the_callback_returns_nothing() {
    let _container = TestContainer::fake();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    Inertia::handle_exceptions_using(move |error| {
        record.lock().unwrap().push((
            error.status(),
            error.request().header("Accept").map(str::to_string),
            error.error().chain().to_vec(),
        ));
        None
    });
    let mut client = client(&config()).await;

    let reply = client.send("GET", "/no/such/page", JSON_CLIENT).await;

    assert_eq!(reply.status, 404, "{reply:?}");
    assert_eq!(reply.body, "404 Not Found");
    assert_eq!(reply.header("content-type"), Some("text/plain"));
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(
            404,
            Some("application/json".to_string()),
            vec!["Not Found".to_string()]
        )],
        "an API client's error reaches the callback too, and the router's 404 \
         reports its reason phrase"
    );
}

#[tokio::test]
async fn inssr_the_callback_decides_a_json_clients_error_too() {
    let _container = TestContainer::fake();
    Inertia::handle_exceptions_using(|error| {
        let status = error.status();
        Some(error.respond_with(HttpResponse::json(json!({ "error": "not_found" })).status(status)))
    });
    let mut client = client(&config()).await;

    let reply = client.send("GET", "/no/such/page", JSON_CLIENT).await;

    assert_eq!(reply.status, 404, "{reply:?}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&reply.body).unwrap(),
        json!({ "error": "not_found" })
    );
}

#[tokio::test]
async fn inssr_a_handlers_own_error_response_is_not_handed_to_the_callback() {
    let _container = TestContainer::fake();
    let (calls, seen) = counter();
    Inertia::handle_exceptions_using(move |error| {
        *seen.lock().unwrap() += 1;
        Some(error.render("Error", ()))
    });
    let mut client = client(&config()).await;

    let reply = client.send("GET", "/own-404", INERTIA_VISIT).await;

    assert_eq!(reply.status, 404, "{reply:?}");
    assert_eq!(reply.body, "<h1>No such widget</h1>");
    assert_eq!(
        *calls.lock().unwrap(),
        0,
        "a response the handler built itself is its answer, not an error"
    );
}

#[tokio::test]
async fn inssr_inertia_protocol_responses_are_never_handed_to_the_callback() {
    let _container = TestContainer::fake();
    let (calls, seen) = counter();
    Inertia::handle_exceptions_using(move |error| {
        *seen.lock().unwrap() += 1;
        Some(error.render("Error", ()))
    });
    // Placed by the app outside the whole Inertia stack, so the headers
    // middleware's own `409` passes through it too.
    let registry = MiddlewareRegistry::new()
        .append(InertiaErrorPageMiddleware::new("Error"))
        .append(Inertia::middleware(&config()));
    let mut client = Client::new(serve(routes(), registry).await);

    let reply = client.send("GET", "/fragment", INERTIA_VISIT).await;
    assert_eq!(reply.status, 409, "{reply:?}");
    assert_eq!(reply.header("x-inertia-redirect"), Some("/ledger#entry-7"));
    assert_eq!(reply.body, "");

    let reply = client.send("GET", "/away", INERTIA_VISIT).await;
    assert_eq!(reply.status, 409, "{reply:?}");
    assert_eq!(
        reply.header("x-inertia-location"),
        Some("https://example.com/ledger")
    );

    assert_eq!(
        *calls.lock().unwrap(),
        0,
        "an Inertia protocol response is an instruction to the client, not an error"
    );
}

/// A client of `routes()` inside a session, behind the Inertia stack, with
/// a callback that answers every error response with a `418`, and the
/// count of the callback's calls.
async fn teapot_client() -> (
    Client,
    Arc<Mutex<Option<suprnova::session::SessionData>>>,
    Arc<Mutex<u32>>,
) {
    let (calls, seen) = counter();
    Inertia::handle_exceptions_using(move |error| {
        *seen.lock().unwrap() += 1;
        Some(error.respond_with(HttpResponse::text("I'm a teapot").status(418)))
    });
    // The validation redirect flashes the errors into the session.
    let slot = suprnova::session::new_session_slot_for_test();
    let registry = MiddlewareRegistry::new()
        .append(SeededSessionScope(slot.clone()))
        .append(Inertia::middleware(&config()));
    (Client::new(serve(routes(), registry).await), slot, calls)
}

#[tokio::test]
async fn inssr_an_inertia_posts_validation_failure_never_reaches_the_callback() {
    let _container = TestContainer::fake();
    let (mut client, slot, calls) = teapot_client().await;

    let reply = client
        .send(
            "POST",
            "/register",
            &[
                ("X-Inertia", "true"),
                ("X-Inertia-Version", VERSION),
                ("Referer", "http://localhost/register"),
            ],
        )
        .await;

    assert_eq!(
        reply.status, 303,
        "the validation redirect owns a validation failure; got {reply:?}"
    );
    assert_eq!(reply.header("location"), Some("/register"), "{reply:?}");
    let flashed: serde_json::Value = slot
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .get("_flash.new.errors.default")
        .expect("the errors must be flashed for the form");
    assert_eq!(flashed["email"][0], "The email field is required.");
    assert_eq!(
        *calls.lock().unwrap(),
        0,
        "a validation result is the validation redirect's, not an error for the callback"
    );
}

#[tokio::test]
async fn inssr_a_json_clients_validation_failure_reaches_the_callback() {
    let _container = TestContainer::fake();
    let (mut client, _slot, calls) = teapot_client().await;

    let reply = client.send("POST", "/register", JSON_CLIENT).await;

    assert_eq!(
        reply.status, 418,
        "a JSON client's 422 is an error response the callback decides, as \
         Laravel's respondUsing hands it every rendered exception; got {reply:?}"
    );
    assert_eq!(reply.body, "I'm a teapot");
    assert_eq!(*calls.lock().unwrap(), 1);

    // A callback returning `None` keeps the 422 with its errors.
    let (kept, seen) = counter();
    Inertia::handle_exceptions_using(move |_error| {
        *seen.lock().unwrap() += 1;
        None
    });

    let reply = client.send("POST", "/register", JSON_CLIENT).await;

    assert_eq!(reply.status, 422, "{reply:?}");
    let body: serde_json::Value = serde_json::from_str(&reply.body)
        .unwrap_or_else(|e| panic!("expected the validation body ({e}): {}", reply.body));
    assert_eq!(body["errors"]["email"][0], "The email field is required.");
    assert_eq!(*kept.lock().unwrap(), 1, "the callback decided the 422");
}

#[tokio::test]
async fn inssr_the_default_callback_keeps_a_json_clients_validation_failure() {
    let _container = TestContainer::fake();
    let slot = suprnova::session::new_session_slot_for_test();
    let registry = MiddlewareRegistry::new()
        .append(SeededSessionScope(slot))
        .append(Inertia::middleware(&config().error_page("Error")));
    let mut client = Client::new(serve(routes(), registry).await);

    let reply = client.send("POST", "/register", JSON_CLIENT).await;

    assert_eq!(
        reply.status, 422,
        "the default callback renders only for an Inertia visit or a request that \
         wants HTML; got {reply:?}"
    );
    assert_eq!(reply.header("x-inertia"), None, "{reply:?}");
    let body: serde_json::Value = serde_json::from_str(&reply.body)
        .unwrap_or_else(|e| panic!("expected the validation body ({e}): {}", reply.body));
    assert_eq!(body["errors"]["email"][0], "The email field is required.");
}

// ---------------------------------------------------------------------
// Responses from outside the Inertia stack reach the callback at the server
// ---------------------------------------------------------------------

/// What [`OuterUnavailable`] fails with. A 5xx body never shows it.
const OUTER_FAILURE: &str = "the ledger replica is 40 minutes behind";

/// What [`OuterPanic`] panics with.
const OUTER_PANIC: &str = "the ledger lock table is poisoned";

/// Answers every request itself, before the Inertia stack is reached: a
/// `503` built from an error, the shape `TimeoutMiddleware` answers a
/// request it cancelled with.
struct OuterUnavailable;

#[async_trait::async_trait]
impl Middleware for OuterUnavailable {
    async fn handle(&self, _request: Request, _next: Next) -> Response {
        Err(FrameworkError::domain(OUTER_FAILURE, 503).into())
    }
}

/// Panics before the Inertia stack is reached, so only the server's panic
/// boundary catches it.
struct OuterPanic;

#[async_trait::async_trait]
impl Middleware for OuterPanic {
    async fn handle(&self, _request: Request, _next: Next) -> Response {
        panic!("{OUTER_PANIC}");
    }
}

/// A client of `routes()` with `outer` registered before the Inertia stack
/// `config` builds, the order an app gets by registering `outer` before
/// `Inertia::install`.
async fn client_behind(outer: impl Middleware + 'static, config: &InertiaConfig) -> Client {
    let registry = MiddlewareRegistry::new()
        .append(outer)
        .append(Inertia::middleware(config));
    Client::new(serve(routes(), registry).await)
}

/// Installs a callback that answers every error response with a `418` and
/// records the status it was handed.
fn install_teapot() -> Arc<Mutex<Vec<u16>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    Inertia::handle_exceptions_using(move |error| {
        record.lock().unwrap().push(error.status());
        Some(error.respond_with(HttpResponse::text("I'm a teapot").status(418)))
    });
    seen
}

#[tokio::test]
async fn inssr_a_503_from_a_middleware_registered_before_the_stack_reaches_the_callback() {
    let _container = TestContainer::fake();
    let seen = install_teapot();
    let mut client = client_behind(OuterUnavailable, &config()).await;

    for (audience, headers) in [
        ("an Inertia visit", INERTIA_VISIT),
        ("a browser navigation", BROWSER),
        ("a JSON client", JSON_CLIENT),
    ] {
        let reply = client.send("GET", "/forbidden", headers).await;
        assert_eq!(reply.status, 418, "{audience}: {reply:?}");
        assert_eq!(reply.body, "I'm a teapot", "{audience}");
        assert!(
            reply.header("x-request-id").is_some(),
            "{audience}: a response decided at the server still carries the request id; \
             got {reply:?}"
        );
    }
    assert_eq!(
        *seen.lock().unwrap(),
        vec![503, 503, 503],
        "the callback decides the outer 503 once per request"
    );
}

#[tokio::test]
async fn inssr_a_csrf_rejection_registered_before_the_stack_gets_the_callbacks_answer() {
    let _container = TestContainer::fake();
    let seen = install_teapot();
    // No session, so no token: `CsrfMiddleware` answers 419 without
    // calling `next`.
    let mut client = client_behind(CsrfMiddleware::new(), &config()).await;

    let reply = client
        .send(
            "POST",
            "/register",
            &[
                ("X-Inertia", "true"),
                ("X-Inertia-Version", VERSION),
                ("Accept", "text/html, application/xhtml+xml"),
            ],
        )
        .await;

    assert_eq!(reply.status, 418, "{reply:?}");
    assert_eq!(reply.body, "I'm a teapot");
    assert_eq!(*seen.lock().unwrap(), vec![419], "the callback saw the 419");
}

#[tokio::test]
#[serial]
async fn inssr_a_panic_in_a_middleware_registered_before_the_stack_reaches_the_callback() {
    let _debug = debug_off().await;
    let _container = TestContainer::fake();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    Inertia::handle_exceptions_using(move |error| {
        record.lock().unwrap().push((
            error.status(),
            error.error().is_panic(),
            error.error().chain().to_vec(),
            error.error().panic_location().is_some(),
        ));
        let status = error.status();
        Some(error.render("Error", json!({ "status": status })))
    });
    let mut client = client_behind(OuterPanic, &config()).await;

    let reply = client.send("GET", "/forbidden", INERTIA_VISIT).await;

    assert_eq!(reply.status, 500, "{reply:?}");
    assert_eq!(reply.header("x-inertia"), Some("true"), "{reply:?}");
    assert_eq!(reply.page()["component"], "Error");
    assert_eq!(reply.page()["props"]["status"], 500);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(500, true, vec![OUTER_PANIC.to_string()], true)],
        "the callback sees the panic the server's boundary caught, with its report"
    );
}

#[tokio::test]
async fn inssr_a_callback_returning_nothing_keeps_an_outer_503_for_a_json_client() {
    let _container = TestContainer::fake();
    let (calls, seen) = counter();
    Inertia::handle_exceptions_using(move |_error| {
        *seen.lock().unwrap() += 1;
        None
    });
    let mut client = client_behind(OuterUnavailable, &config()).await;

    let reply = client.send("GET", "/forbidden", JSON_CLIENT).await;

    assert_eq!(*calls.lock().unwrap(), 1, "the callback decides the 503");
    assert_json_error(&reply, 503, "Internal Server Error");
}

#[tokio::test]
async fn inssr_a_page_decided_at_the_server_carries_the_installed_shared_data() {
    let _container = TestContainer::fake();
    Inertia::share("app_name", "Ledger").unwrap();
    Inertia::handle_exceptions_using(|error| {
        let status = error.status();
        Some(
            error
                .render("Error", json!({ "status": status }))
                .with_shared_data(),
        )
    });
    let mut client = client_behind(OuterUnavailable, &config().hooks(SharingHooks)).await;

    let reply = client.send("GET", "/forbidden", INERTIA_VISIT).await;

    assert_eq!(reply.status, 503, "{reply:?}");
    assert_eq!(reply.header("x-inertia"), Some("true"), "{reply:?}");
    let props = reply.page()["props"].clone();
    assert_eq!(props["status"], 503, "{props}");
    assert_eq!(
        props["app_name"], "Ledger",
        "the shared registry reaches a page decided at the server; got {props}"
    );
    for hook in ["path", "plans"] {
        assert!(
            props.get(hook).is_none(),
            "the middleware hooks run inside the stack the outer 503 never reached; got {props}"
        );
    }
}

// ---------------------------------------------------------------------
// `error_page` is the default callback
// ---------------------------------------------------------------------

#[tokio::test]
async fn inssr_an_error_page_placed_outside_the_stack_leaves_a_fragment_redirect_alone() {
    let _container = TestContainer::fake();
    let registry = MiddlewareRegistry::new()
        .append(InertiaErrorPageMiddleware::new("Error"))
        .append(Inertia::middleware(&config()));
    let mut client = Client::new(serve(routes(), registry).await);

    let reply = client.send("GET", "/fragment", INERTIA_VISIT).await;

    assert_eq!(reply.status, 409, "{reply:?}");
    assert_eq!(
        reply.header("x-inertia-redirect"),
        Some("/ledger#entry-7"),
        "the client follows the fragment redirect by this header; got {reply:?}"
    );
    assert_eq!(reply.header("x-inertia"), None, "{reply:?}");
}

#[tokio::test]
async fn inssr_error_page_alone_renders_the_page_for_an_inertia_403() {
    let _container = TestContainer::fake();
    let mut client = client(&config().error_page("Error")).await;

    let reply = client.send("GET", "/forbidden", INERTIA_VISIT).await;

    assert_eq!(reply.status, 403, "{reply:?}");
    assert_eq!(reply.header("x-inertia"), Some("true"), "{reply:?}");
    assert_eq!(reply.header("cache-control"), Some("no-cache, private"));
    let page = reply.page();
    assert_eq!(page["component"], "Error");
    assert_eq!(page["props"]["status"], 403);
    assert_eq!(page["props"]["message"], DENIAL);
    assert!(page["props"]["request_id"].is_string(), "{page}");

    // An API client keeps its JSON.
    let reply = client.send("GET", "/forbidden", JSON_CLIENT).await;
    assert_json_error(&reply, 403, DENIAL);
}

#[tokio::test]
async fn inssr_error_page_renders_with_the_shared_props() {
    let _container = TestContainer::fake();
    Inertia::share("app_name", "Ledger").unwrap();
    let mut client = client(&config().hooks(SharingHooks).error_page("Error")).await;

    let reply = client.send("GET", "/forbidden", INERTIA_VISIT).await;

    assert_eq!(reply.status, 403, "{reply:?}");
    let props = reply.page()["props"].clone();
    assert_eq!(props["app_name"], "Ledger", "{props}");
    assert_eq!(props["path"], "/forbidden", "{props}");
    assert_eq!(props["plans"], json!(["pro"]), "{props}");
}

#[tokio::test]
#[serial]
async fn inssr_error_page_renders_the_page_for_a_handler_panic() {
    let _debug = debug_off().await;
    let _container = TestContainer::fake();
    let mut client = client(&config().error_page("Error")).await;

    let reply = client.send("GET", "/panic", INERTIA_VISIT).await;

    assert_eq!(reply.status, 500, "{reply:?}");
    assert_eq!(reply.header("x-inertia"), Some("true"), "{reply:?}");
    let page = reply.page();
    assert_eq!(page["component"], "Error");
    assert_eq!(page["props"]["status"], 500);
    assert_eq!(
        page["props"]["message"], "Internal Server Error",
        "the panic's message never reaches the page; got {page}"
    );

    // An API client keeps the JSON 500.
    let reply = client.send("GET", "/panic", JSON_CLIENT).await;
    assert_json_error(&reply, 500, "Internal Server Error");
}
