//! PAR-010 and PAR-011: a failed request carries its error report into
//! `TestResponse`.
//!
//! Laravel keeps the exceptions a test request threw in a process-wide
//! `LoggedExceptionCollection`, and `TestResponse` appends them to a
//! failing assertion. Here the report rides on the response itself, in
//! its in-process extensions, so two requests in flight in one test
//! process never mix their reports.
//!
//! Every request goes through the real `handle_request`, the function
//! `Server::run` calls per connection. A handler or middleware error
//! becomes a response through `From<FrameworkError> for HttpResponse`,
//! and a panic becomes one at the panic boundary, `execute_chain_safely`.
//! The app config is production-shaped (`debug: false`): the 5xx body
//! then carries only a generic message, so a report that reached the
//! headers or the body would show.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Barrier;

use suprnova::config::{AppConfig, Config, Environment};
use suprnova::http::text;
use suprnova::testing::TestResponse;
use suprnova::{
    ErrorReport, FrameworkError, Middleware, MiddlewareRegistry, Next, Request, Response, Router,
    handle_request,
};

use crate::common::incoming_get_request;

/// The label of the report section a failing assertion appends.
const SECTION: &str = "error report";

/// The invoice handler's error chain, outermost first.
const INVOICE_ERROR: &str = "posting the invoice failed";
const LEDGER_ERROR: &str = "writing ledger entry 42 failed";
const DISK_ERROR: &str = "disk /var/ledger is full";

/// What the ledger index handler panics with.
const PANIC_MESSAGE: &str = "ledger index page 7 is unreadable";

/// The ledger middleware's error and its source.
const LOCK_ERROR: &str = "acquiring the ledger lock failed";
const LOCK_CAUSE: &str = "the lock is held by worker 9";

/// The two concurrent requests' errors and their sources.
const REFUND_ERROR: &str = "issuing the refund failed";
const REFUND_CAUSE: &str = "card ending 4242 was declined";
const PAYOUT_ERROR: &str = "sending the payout failed";
const PAYOUT_CAUSE: &str = "the bank is offline until 06:00";

/// A leaf error, with no source of its own.
#[derive(Debug)]
struct RootCause(&'static str);

impl std::fmt::Display for RootCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for RootCause {}

/// An error whose `source()` is a [`RootCause`]. Wrapped once more by
/// `FrameworkError::from_external_with`, it makes a three-link chain.
#[derive(Debug)]
struct Failed {
    message: &'static str,
    cause: RootCause,
}

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}

impl std::error::Error for Failed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

fn write_ledger_entry() -> Result<(), Failed> {
    Err(Failed {
        message: LEDGER_ERROR,
        cause: RootCause(DISK_ERROR),
    })
}

/// Fails the way application code does: a foreign error, given context
/// and propagated with `?`.
async fn post_invoice(_req: Request) -> Response {
    write_ledger_entry().map_err(|e| FrameworkError::from_external_with(INVOICE_ERROR, e))?;
    text("posted")
}

/// The line `read_ledger_index` panics on. It is recorded when the
/// handler runs, so the location check does not depend on how this file
/// is formatted.
static PANIC_LINE: AtomicU32 = AtomicU32::new(0);

async fn read_ledger_index(_req: Request) -> Response {
    PANIC_LINE.store(line!() + 1, Ordering::SeqCst);
    panic!("{PANIC_MESSAGE}");
}

/// Refuses every request before the handler runs.
struct RequireLedgerLock;

#[async_trait]
impl Middleware for RequireLedgerLock {
    async fn handle(&self, _request: Request, _next: Next) -> Response {
        Err(FrameworkError::from_external_with(LOCK_ERROR, RootCause(LOCK_CAUSE)).into())
    }
}

/// Wait until the other request has reached its handler too, then fail.
/// The barrier holds both requests in flight at once, so a report kept
/// anywhere shared, and not on its own response, would mix.
async fn fail_once_both_are_in_flight(
    barrier: Arc<Barrier>,
    message: &'static str,
    cause: &'static str,
) -> Response {
    barrier.wait().await;
    Err(FrameworkError::from_external_with(message, RootCause(cause)).into())
}

fn ledger_routes() -> Arc<Router> {
    let router = Router::new()
        .get("/invoice", post_invoice)
        .get("/ledger-index", read_ledger_index)
        .get("/ledger", |_req: Request| async { text("ledger") })
        .middleware(RequireLedgerLock)
        .get("/health", |_req: Request| async { text("all good") });
    Arc::new(router.into())
}

/// Register a production-shaped app config. With `debug` off, a 5xx body
/// carries only the generic message, so error text found on the wire
/// came from the report and not from `debug_message`.
fn production_config() {
    Config::register(
        AppConfig::builder()
            .name("request-diagnostics-test")
            .environment(Environment::Production)
            .debug(false)
            .url("http://localhost:0")
            .build(),
    );
}

/// Drive one GET through `handle_request` in process and build the
/// `TestResponse` from what it returns. Also returns every header line
/// as it goes on the wire (`name: value`, one per line), so a test can
/// check all of them and not only the first of each name.
async fn get(router: &Arc<Router>, path: &str) -> (TestResponse, String) {
    let request = incoming_get_request(path, &[]).await;
    let response = handle_request(
        Arc::clone(router),
        Arc::new(MiddlewareRegistry::new()),
        request,
    )
    .await;
    let wire_headers = response
        .headers()
        .iter()
        .map(|(name, value)| format!("{name}: {}\n", String::from_utf8_lossy(value.as_bytes())))
        .collect();
    (TestResponse::from_response(response).await, wire_headers)
}

/// The message an assertion failed with.
fn failure_of(assertion: impl FnOnce()) -> String {
    let payload = std::panic::catch_unwind(AssertUnwindSafe(assertion))
        .expect_err("the assertion should have failed");
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else {
        panic!("the assertion failed with a payload that is not a string")
    }
}

/// The report a response carries. Fails when it carries none.
fn report_of(response: &TestResponse) -> &ErrorReport {
    response.error_report().unwrap_or_else(|| {
        panic!(
            "a {} response built from an error must carry its error report; body: {}",
            response.status(),
            response.body_text()
        )
    })
}

/// Whether `text` names `file:line`, with or without a column after it.
fn names_location(text: &str, file: &str, line: u32) -> bool {
    let site = format!("{file}:{line}");
    text.match_indices(&site)
        .any(|(at, _)| !text[at + site.len()..].starts_with(|c: char| c.is_ascii_digit()))
}

/// Fail if any of `texts` reached the response's headers or body.
fn assert_off_the_wire(response: &TestResponse, wire_headers: &str, texts: &[&str]) {
    let body = response.body_text();
    for text in texts {
        assert!(
            !wire_headers.contains(text),
            "{text:?} reached the response headers:\n{wire_headers}"
        );
        assert!(
            !body.contains(text),
            "{text:?} reached the response body: {body}"
        );
    }
}

#[tokio::test]
async fn a_handler_error_reports_its_source_chain() {
    production_config();
    let (response, _) = get(&ledger_routes(), "/invoice").await;

    assert_eq!(response.status(), 500);
    let report = report_of(&response).to_string();
    let positions: Vec<usize> = [INVOICE_ERROR, LEDGER_ERROR, DISK_ERROR]
        .into_iter()
        .map(|link| {
            report
                .find(link)
                .unwrap_or_else(|| panic!("the report must name {link:?}; report:\n{report}"))
        })
        .collect();
    assert!(
        positions.is_sorted(),
        "the report must read the chain outermost first; report:\n{report}"
    );
}

#[tokio::test]
async fn assert_ok_on_a_handler_error_shows_the_error_and_its_sources() {
    production_config();
    let (response, _) = get(&ledger_routes(), "/invoice").await;

    let failure = failure_of(|| {
        response.assert_ok();
    });
    for text in [SECTION, INVOICE_ERROR, LEDGER_ERROR, DISK_ERROR] {
        assert!(
            failure.contains(text),
            "assert_ok() on a failed request must show {text:?}; failure:\n{failure}"
        );
    }
}

#[tokio::test]
async fn assert_status_on_a_handler_error_shows_the_error_and_its_sources() {
    production_config();
    let (response, _) = get(&ledger_routes(), "/invoice").await;

    let failure = failure_of(|| {
        response.assert_status(200);
    });
    for text in [SECTION, INVOICE_ERROR, LEDGER_ERROR, DISK_ERROR] {
        assert!(
            failure.contains(text),
            "assert_status(200) on a failed request must show {text:?}; failure:\n{failure}"
        );
    }
}

#[tokio::test]
async fn a_handler_error_stays_out_of_the_response_headers_and_body() {
    production_config();
    let (response, wire_headers) = get(&ledger_routes(), "/invoice").await;

    assert_eq!(response.status(), 500);
    report_of(&response);
    assert_off_the_wire(
        &response,
        &wire_headers,
        &[INVOICE_ERROR, LEDGER_ERROR, DISK_ERROR],
    );
}

#[tokio::test]
async fn a_handler_panic_reports_its_message_and_location() {
    production_config();
    let (response, _) = get(&ledger_routes(), "/ledger-index").await;

    assert_eq!(response.status(), 500);
    let report = report_of(&response).to_string();
    assert!(
        report.contains(PANIC_MESSAGE),
        "the report must carry the panic message; report:\n{report}"
    );
    let line = PANIC_LINE.load(Ordering::SeqCst);
    assert!(
        names_location(&report, file!(), line),
        "the report must name where the handler panicked, {}:{line}; report:\n{report}",
        file!()
    );
}

#[tokio::test]
async fn assert_ok_on_a_handler_panic_shows_the_panic_message_and_location() {
    production_config();
    let (response, _) = get(&ledger_routes(), "/ledger-index").await;

    let failure = failure_of(|| {
        response.assert_ok();
    });
    for text in [SECTION, PANIC_MESSAGE] {
        assert!(
            failure.contains(text),
            "assert_ok() on a panic boundary 500 must show {text:?}; failure:\n{failure}"
        );
    }
    let line = PANIC_LINE.load(Ordering::SeqCst);
    assert!(
        names_location(&failure, file!(), line),
        "assert_ok() must show where the handler panicked, {}:{line}; failure:\n{failure}",
        file!()
    );
}

#[tokio::test]
async fn a_handler_panic_stays_out_of_the_response_headers_and_body() {
    production_config();
    let (response, wire_headers) = get(&ledger_routes(), "/ledger-index").await;

    assert_eq!(response.status(), 500);
    report_of(&response);
    assert_off_the_wire(&response, &wire_headers, &[PANIC_MESSAGE, file!()]);
}

#[tokio::test]
async fn a_middleware_error_carries_its_report() {
    production_config();
    let (response, wire_headers) = get(&ledger_routes(), "/ledger").await;

    assert_eq!(response.status(), 500);
    let report = report_of(&response).to_string();
    for text in [LOCK_ERROR, LOCK_CAUSE] {
        assert!(
            report.contains(text),
            "the report must name {text:?}; report:\n{report}"
        );
    }
    let failure = failure_of(|| {
        response.assert_status(200);
    });
    for text in [SECTION, LOCK_ERROR, LOCK_CAUSE] {
        assert!(
            failure.contains(text),
            "assert_status(200) on a middleware error must show {text:?}; failure:\n{failure}"
        );
    }
    assert_off_the_wire(&response, &wire_headers, &[LOCK_ERROR, LOCK_CAUSE]);
}

#[tokio::test]
async fn concurrent_requests_each_carry_only_their_own_error() {
    production_config();
    let barrier = Arc::new(Barrier::new(2));
    let (refund_barrier, payout_barrier) = (Arc::clone(&barrier), barrier);
    let router = Router::new()
        .get("/refund", move |_req: Request| {
            fail_once_both_are_in_flight(Arc::clone(&refund_barrier), REFUND_ERROR, REFUND_CAUSE)
        })
        .get("/payout", move |_req: Request| {
            fail_once_both_are_in_flight(Arc::clone(&payout_barrier), PAYOUT_ERROR, PAYOUT_CAUSE)
        });
    let router: Arc<Router> = Arc::new(router.into());

    let ((refund, _), (payout, _)) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(get(&router, "/refund"), get(&router, "/payout"))
    })
    .await
    .expect("both requests must finish; a handler that never met the other one hangs here");

    let cases = [
        (
            &refund,
            [REFUND_ERROR, REFUND_CAUSE],
            [PAYOUT_ERROR, PAYOUT_CAUSE],
        ),
        (
            &payout,
            [PAYOUT_ERROR, PAYOUT_CAUSE],
            [REFUND_ERROR, REFUND_CAUSE],
        ),
    ];
    for (response, own, other) in cases {
        let report = report_of(response).to_string();
        let failure = failure_of(|| {
            response.assert_ok();
        });
        for text in own {
            assert!(
                report.contains(text),
                "the report must name its own request's {text:?}; report:\n{report}"
            );
            assert!(
                failure.contains(text),
                "the failure must name its own request's {text:?}; failure:\n{failure}"
            );
        }
        for text in other {
            assert!(
                !report.contains(text),
                "the report names the other request's {text:?}; report:\n{report}"
            );
            assert!(
                !failure.contains(text),
                "the failure names the other request's {text:?}; failure:\n{failure}"
            );
        }
    }
}

#[tokio::test]
async fn a_successful_response_carries_no_report() {
    production_config();
    let (response, _) = get(&ledger_routes(), "/health").await;

    response.assert_ok();
    assert!(
        response.error_report().is_none(),
        "a 200 built without an error must carry no error report"
    );
    let failure = failure_of(|| {
        response.assert_status(404);
    });
    assert!(
        failure.contains("assert_status(404)"),
        "the assertion must still fail on its own terms; failure:\n{failure}"
    );
    assert!(
        !failure.contains(SECTION),
        "a response with no report must print no report section; failure:\n{failure}"
    );
}
