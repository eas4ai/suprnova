//! PAR-062 with PAR-012: with debug on, the response the error callback of
//! `Inertia::handle_exceptions_using` receives for a 5xx that carries an
//! error report, sent to a browser or an Inertia visit, is the development
//! error page. The default callback `InertiaConfig::error_page` installs
//! keeps it, and a page the application's callback renders stands.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use serial_test::serial;

use suprnova::testing::TestContainer;
use suprnova::{Inertia, InertiaConfig, Middleware, MiddlewareRegistry, Next, Request, Response};

use super::{
    BROWSER, DISK_ERROR, INERTIA_VISIT, INVOICE_ERROR, LEDGER_ERROR, PANIC_MESSAGE, Reply,
    SessionScope, assert_debug_page, assert_page_shows, chain, debug_mode, ledger_routes, request,
};

/// The Inertia stack `config` builds, inside a session, as global
/// middleware.
fn stack(config: &InertiaConfig) -> MiddlewareRegistry {
    MiddlewareRegistry::new()
        .append(SessionScope)
        .append(Inertia::middleware(config))
}

/// The configuration the tests start from. The visits send no
/// `X-Inertia-Version`, which reads as the empty version.
fn config() -> InertiaConfig {
    InertiaConfig::new().development(true).version("")
}

/// GET `path` from `ledger_routes()` behind the stack `config` builds.
async fn visit(config: &InertiaConfig, path: &str, headers: &[(&str, &str)]) -> Reply {
    super::exchange(
        ledger_routes(),
        stack(config),
        request("GET", path, headers, ""),
    )
    .await
}

#[tokio::test]
#[serial]
async fn inssr_with_debug_on_the_callback_receives_the_development_error_page() {
    let _debug = debug_mode(true, &[]).await;
    let _container = TestContainer::fake();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    Inertia::handle_exceptions_using(move |error| {
        let response = error.response();
        record.lock().unwrap().push((
            error.status(),
            response.header_value("Content-Type").map(str::to_string),
            String::from_utf8_lossy(response.body()).contains(INVOICE_ERROR),
        ));
        None
    });

    let reply = visit(&config(), "/invoice", BROWSER).await;

    assert_debug_page(&reply, 500);
    assert_page_shows(&reply, &[INVOICE_ERROR, LEDGER_ERROR, DISK_ERROR]);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(500, Some("text/html; charset=utf-8".to_string()), true)],
        "the callback receives the development error page as the response"
    );
}

#[tokio::test]
#[serial]
async fn inssr_with_debug_on_a_panic_reaches_the_callback_as_the_development_error_page() {
    let _debug = debug_mode(true, &[]).await;
    let _container = TestContainer::fake();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    Inertia::handle_exceptions_using(move |error| {
        record.lock().unwrap().push((
            error.error().is_panic(),
            error.error().chain().to_vec(),
            error
                .response()
                .header_value("Content-Type")
                .map(str::to_string),
        ));
        None
    });

    let reply = visit(&config(), "/ledger-index", INERTIA_VISIT).await;

    assert_debug_page(&reply, 500);
    assert_page_shows(&reply, &[PANIC_MESSAGE]);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(
            true,
            vec![PANIC_MESSAGE.to_string()],
            Some("text/html; charset=utf-8".to_string())
        )]
    );
}

#[tokio::test]
#[serial]
async fn inssr_with_debug_on_a_page_the_callback_renders_stands() {
    let _debug = debug_mode(true, &[]).await;
    let _container = TestContainer::fake();
    Inertia::handle_exceptions_using(|error| {
        let status = error.status();
        Some(error.render("Error", json!({ "status": status })))
    });

    let reply = visit(&config(), "/invoice", INERTIA_VISIT).await;

    assert_eq!(reply.status, 500, "body: {}", reply.body);
    assert_eq!(
        reply.header("x-inertia"),
        Some("true"),
        "the application's decision stands over the development error page; body: {}",
        reply.body
    );
    let page: Value = serde_json::from_str(&reply.body).expect("an Inertia page object");
    assert_eq!(page["component"], "Error");
    assert_eq!(page["props"]["status"], 500);
    assert!(
        !reply.body.contains(INVOICE_ERROR),
        "the page carries the props the callback gave it, not the error; body: {}",
        reply.body
    );
}

#[tokio::test]
#[serial]
async fn inssr_with_debug_on_error_page_keeps_the_development_error_page() {
    let _debug = debug_mode(true, &[]).await;
    let _container = TestContainer::fake();

    let reply = visit(&config().error_page("Error"), "/invoice", INERTIA_VISIT).await;

    assert_debug_page(&reply, 500);
    assert_page_shows(&reply, &[INVOICE_ERROR, LEDGER_ERROR, DISK_ERROR]);
}

/// What [`OuterUnavailable`]'s error chain says, outermost first.
const REPLICA_ERROR: &str = "reading the ledger replica failed";
const LAG_ERROR: &str = "replica ledger-2 is 40 minutes behind";
const LINK_ERROR: &str = "the replication link is down";

/// Answers every request itself, before the session and the Inertia stack
/// are reached: a `503` built from an error chain, the shape
/// `TimeoutMiddleware` answers a request it cancelled with.
struct OuterUnavailable;

#[async_trait::async_trait]
impl Middleware for OuterUnavailable {
    async fn handle(&self, _request: Request, _next: Next) -> Response {
        let error = chain(REPLICA_ERROR, LAG_ERROR, LINK_ERROR);
        Err(suprnova::HttpResponse::from(error).status(503))
    }
}

#[tokio::test]
#[serial]
async fn inssr_with_debug_on_the_callback_receives_the_development_page_for_an_outer_503() {
    let _debug = debug_mode(true, &[]).await;
    let _container = TestContainer::fake();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    Inertia::handle_exceptions_using(move |error| {
        let response = error.response();
        record.lock().unwrap().push((
            error.status(),
            response.header_value("Content-Type").map(str::to_string),
            String::from_utf8_lossy(response.body()).contains(REPLICA_ERROR),
        ));
        None
    });
    let registry = stack(&config()).prepend(OuterUnavailable);

    let reply = super::exchange(
        ledger_routes(),
        registry,
        request("GET", "/invoice", BROWSER, ""),
    )
    .await;

    assert_debug_page(&reply, 503);
    assert_page_shows(&reply, &[REPLICA_ERROR, LAG_ERROR, LINK_ERROR]);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(503, Some("text/html; charset=utf-8".to_string()), true)],
        "a middleware registered before the Inertia stack reaches the callback at the \
         server, which hands it the development error page"
    );
}
