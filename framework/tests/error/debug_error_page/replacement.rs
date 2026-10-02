//! PAR-012: with debug on, a 5xx carrying an error report, sent to an
//! Inertia visit or to a request whose `Accept` lists `text/html`, is
//! replaced by the development error page, which also takes the place of
//! the app's Inertia error page. Every other response, and every response
//! with debug off, stays as it is today.

use serde_json::Value;
use serial_test::serial;

use suprnova::{
    FrameworkError, HttpResponse, InertiaErrorPageMiddleware, MiddlewareRegistry, Request,
    Response, Router,
};

use super::{
    BROWSER, DISK_ERROR, INERTIA_VISIT, INVOICE_ERROR, JSON_CLIENT, LEDGER_ERROR, PANIC_FILE,
    PANIC_LINE, PANIC_MESSAGE, Reply, SessionScope, assert_debug_page, assert_page_shows,
    debug_mode, get, ledger_routes, names_location, post_invoice, request,
};

/// What `/maintenance` fails with.
const MAINTENANCE_ERROR: &str = "the ledger is closed for the nightly reconciliation";

/// What `/upstream` answers with, building the 504 itself.
const UPSTREAM_BODY: &str = "the upstream ledger timed out";

/// The page component the Inertia error page middleware renders.
const ERROR_PAGE: &str = "Error";

fn routes() -> Router {
    Router::new()
        .get("/invoice", post_invoice)
        .get("/ledger-index", super::read_ledger_index)
        .get("/maintenance", |_req: Request| async {
            let response: Response = Err(HttpResponse::from(FrameworkError::Domain {
                message: MAINTENANCE_ERROR.to_string(),
                status_code: 503,
            }));
            response
        })
        .get("/missing-param", |_req: Request| async {
            let response: Response = Err(HttpResponse::from(FrameworkError::param("account")));
            response
        })
        .get("/upstream", |_req: Request| async {
            // A 5xx the handler built itself: no error became it, so it
            // carries no error report.
            let response: Response = Err(HttpResponse::text(UPSTREAM_BODY).status(504));
            response
        })
        .into()
}

/// `/posts` fails the way `/invoice` does, behind the app's Inertia error
/// page.
fn inertia_routes() -> Router {
    Router::new()
        .get("/posts", post_invoice)
        .middleware(InertiaErrorPageMiddleware::new(ERROR_PAGE))
        .into()
}

async fn inertia_request(headers: &[(&str, &str)]) -> Reply {
    super::exchange(
        inertia_routes(),
        MiddlewareRegistry::new().append(SessionScope),
        request("GET", "/posts", headers, ""),
    )
    .await
}

/// Fail unless `reply` is today's JSON 5xx body: the generic message and
/// the request id, and nothing else.
fn assert_todays_json_error(reply: &Reply, status: u16) {
    assert_eq!(reply.status, status, "body: {}", reply.body);
    assert!(
        reply.content_type().starts_with("application/json"),
        "the response must stay today's JSON error; got content-type {:?} and body: {}",
        reply.content_type(),
        reply.body
    );
    let body: Value = serde_json::from_str(&reply.body)
        .unwrap_or_else(|e| panic!("today's error body is JSON: {e}; body: {}", reply.body));
    let mut keys: Vec<&str> = body
        .as_object()
        .map(|fields| fields.keys().map(String::as_str).collect())
        .unwrap_or_default();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["message", "request_id"],
        "with debug off, today's body holds only the message and the request id; body: {}",
        reply.body
    );
    assert_eq!(body["message"], "Internal Server Error");
    assert_eq!(
        body["request_id"].as_str(),
        reply.header("x-request-id"),
        "the body's request id is the one the response echoes; body: {}",
        reply.body
    );
}

#[tokio::test]
#[serial]
async fn with_debug_on_a_browser_request_to_a_failing_handler_gets_html_not_json() {
    let _debug = debug_mode(true, &[]).await;

    let reply = get(routes(), "/invoice", BROWSER).await;

    assert_debug_page(&reply, 500);
    assert!(
        serde_json::from_str::<Value>(&reply.body).is_err(),
        "a browser must not get JSON; body: {}",
        reply.body
    );
}

#[tokio::test]
#[serial]
async fn with_debug_on_the_page_shows_the_error_chain_and_keeps_the_500_status() {
    let _debug = debug_mode(true, &[]).await;

    let reply = get(routes(), "/invoice", BROWSER).await;

    assert_debug_page(&reply, 500);
    assert_page_shows(&reply, &[INVOICE_ERROR, LEDGER_ERROR, DISK_ERROR]);
}

#[tokio::test]
#[serial]
async fn with_debug_on_the_page_keeps_a_503_status_and_shows_its_message() {
    let _debug = debug_mode(true, &[]).await;

    let reply = get(routes(), "/maintenance", BROWSER).await;

    assert_debug_page(&reply, 503);
    assert_page_shows(&reply, &[MAINTENANCE_ERROR]);
}

#[tokio::test]
#[serial]
async fn with_debug_on_a_panicking_handler_gets_a_page_with_the_panic_message_and_location() {
    let _debug = debug_mode(true, &[]).await;

    let reply = get(routes(), "/ledger-index", BROWSER).await;

    assert_debug_page(&reply, 500);
    assert_page_shows(&reply, &[PANIC_MESSAGE]);
    let line = PANIC_LINE.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        names_location(&reply.text(), PANIC_FILE, line),
        "the page must show where the panic happened, {PANIC_FILE}:{line}; page:\n{}",
        reply.body
    );
}

#[tokio::test]
#[serial]
async fn with_debug_on_an_inertia_visit_gets_the_page_not_the_apps_inertia_error_page() {
    let _debug = debug_mode(true, &[]).await;

    let reply = inertia_request(INERTIA_VISIT).await;

    assert_debug_page(&reply, 500);
    assert_page_shows(&reply, &[INVOICE_ERROR, LEDGER_ERROR, DISK_ERROR]);
}

#[tokio::test]
#[serial]
async fn with_debug_on_a_browser_navigation_gets_the_page_not_the_apps_inertia_error_page() {
    let _debug = debug_mode(true, &[]).await;

    let reply = inertia_request(BROWSER).await;

    assert_debug_page(&reply, 500);
    assert!(
        !reply.body.contains("data-page=\"app\""),
        "the app's Inertia error page shell must not stand in for the page; body: {}",
        reply.body
    );
    assert_page_shows(&reply, &[INVOICE_ERROR, LEDGER_ERROR, DISK_ERROR]);
}

#[tokio::test]
#[serial]
async fn with_debug_on_a_request_that_does_not_list_text_html_keeps_the_json_error() {
    let _debug = debug_mode(true, &[]).await;

    let cases: [&[(&str, &str)]; 3] = [JSON_CLIENT, &[("Accept", "*/*")], &[]];
    for headers in cases {
        let reply = get(routes(), "/invoice", headers).await;

        assert_eq!(
            reply.status, 500,
            "headers {headers:?}; body: {}",
            reply.body
        );
        assert!(
            reply.content_type().starts_with("application/json"),
            "a request with headers {headers:?} must keep the JSON error; \
             got content-type {:?} and body: {}",
            reply.content_type(),
            reply.body
        );
        let body: Value = serde_json::from_str(&reply.body)
            .unwrap_or_else(|e| panic!("the JSON error body must parse: {e}; {}", reply.body));
        assert_eq!(body["message"], "Internal Server Error");
    }
}

#[tokio::test]
#[serial]
async fn with_debug_on_a_browser_request_to_a_4xx_error_is_left_as_it_is() {
    let _debug = debug_mode(true, &[]).await;

    let reply = get(routes(), "/missing-param", BROWSER).await;

    assert_eq!(reply.status, 400, "body: {}", reply.body);
    assert!(
        reply.content_type().starts_with("application/json"),
        "a 4xx is not replaced; got content-type {:?} and body: {}",
        reply.content_type(),
        reply.body
    );
    let body: Value = serde_json::from_str(&reply.body).expect("the 400 body is JSON");
    assert_eq!(body["message"], "Missing required parameter: account");
}

#[tokio::test]
#[serial]
async fn with_debug_on_a_browser_request_to_a_5xx_without_an_error_report_is_left_as_it_is() {
    let _debug = debug_mode(true, &[]).await;

    let reply = get(routes(), "/upstream", BROWSER).await;

    assert_eq!(reply.status, 504);
    assert!(
        reply.content_type().starts_with("text/plain"),
        "a 5xx no error became is not replaced; got content-type {:?}",
        reply.content_type()
    );
    assert_eq!(reply.body, UPSTREAM_BODY);
}

#[tokio::test]
#[serial]
async fn with_debug_off_a_browser_request_to_a_failing_or_panicking_handler_gets_todays_json() {
    let _debug = debug_mode(false, &[]).await;

    for path in ["/invoice", "/ledger-index"] {
        let reply = get(ledger_routes(), path, BROWSER).await;

        assert_todays_json_error(&reply, 500);
    }
}

#[tokio::test]
#[serial]
async fn with_debug_off_inertia_requests_still_get_the_apps_inertia_error_page() {
    let _debug = debug_mode(false, &[]).await;

    let visit = inertia_request(INERTIA_VISIT).await;
    assert_eq!(visit.status, 500, "body: {}", visit.body);
    assert_eq!(
        visit.header("x-inertia"),
        Some("true"),
        "an Inertia visit must still get the app's error page; body: {}",
        visit.body
    );
    let page: Value = serde_json::from_str(&visit.body).expect("an Inertia page object");
    assert_eq!(page["component"], ERROR_PAGE);
    assert_eq!(page["props"]["status"], 500);
    assert_eq!(page["props"]["message"], "Internal Server Error");

    let navigation = inertia_request(BROWSER).await;
    assert_eq!(navigation.status, 500, "body: {}", navigation.body);
    assert!(
        navigation.content_type().starts_with("text/html"),
        "a browser navigation must still get the Inertia shell; body: {}",
        navigation.body
    );
    assert!(
        navigation.body.contains("data-page=\"app\"")
            && navigation.body.contains("\"component\":\"Error\""),
        "a browser navigation must still get the app's Inertia error page; body: {}",
        navigation.body
    );
}
