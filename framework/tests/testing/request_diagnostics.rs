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
use suprnova::http::cookie::Cookie;
use suprnova::http::text;
use suprnova::middleware::into_boxed;
use suprnova::rate_limit::{BackendErrorPolicy, RateLimitMiddleware, SlidingWindowConfig};
use suprnova::session::{
    SessionBlock, SessionConfig, SessionData, SessionMiddleware, SessionStore,
};
use suprnova::testing::{AssertableInertia, TestContainer, TestResponse};
use suprnova::ws::{OriginPolicy, WebSocketHandler, WsConfig, WsSocket};
use suprnova::{
    BruteForce, CacheStore, Crypt, EncryptionKey, ErrorReport, FrameworkError, HttpResponse,
    InMemoryCache, InertiaErrorPageMiddleware, InertiaValidationRedirectMiddleware,
    LoginThrottleMiddleware, Middleware, MiddlewareRegistry, Next, RateLimiterDriver, Request,
    Response, Router, ThrottleRequestsMiddleware, TimeoutMiddleware, ValidationErrors,
    handle_request,
};

use crate::common::incoming_get_request;

/// The label of the report section a failing assertion appends.
const SECTION: &str = "error report";

/// The invoice handler's error chain, outermost first.
const INVOICE_ERROR: &str = "posting the invoice failed";
const LEDGER_ERROR: &str = "writing ledger entry 42 failed";
const DISK_ERROR: &str = "disk /var/ledger is full";

/// What the rates handler fails with. `from_external` copies it into the
/// wrapping error, so the chain repeats it.
const RATES_ERROR: &str = "the rates feed answered 503";

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

/// Fails with `from_external`, whose message is its source's message.
async fn fetch_rates(_req: Request) -> Response {
    Err(FrameworkError::from_external(RootCause(RATES_ERROR)).into())
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
        .get("/rates", fetch_rates)
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
    send(router, MiddlewareRegistry::new(), path, &[]).await
}

/// [`get`], with global middleware and request headers of the test's
/// choosing.
async fn send(
    router: &Arc<Router>,
    registry: MiddlewareRegistry,
    path: &str,
    headers: &[(&str, &str)],
) -> (TestResponse, String) {
    let request = incoming_get_request(path, headers).await;
    let response = handle_request(Arc::clone(router), Arc::new(registry), request).await;
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
async fn a_source_that_repeats_its_error_is_reported_once() {
    production_config();
    let (response, _) = get(&ledger_routes(), "/rates").await;

    assert_eq!(response.status(), 500);
    assert_eq!(report_of(&response).chain(), [RATES_ERROR]);
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

// ---------------------------------------------------------------------
// Framework middleware that answer a failure with a 5xx of their own
// ---------------------------------------------------------------------
//
// These never return a `FrameworkError` for `From` to convert: each logs
// the failure and builds its own response. The report has to be attached
// at each of those sites, so each gets a case here.

/// Fail if the report `response` carries does not name every one of
/// `texts`.
fn assert_report_names(response: &TestResponse, texts: &[&str]) {
    let report = report_of(response).to_string();
    for text in texts {
        assert!(
            report.contains(text),
            "the report must name {text:?}; report:\n{report}"
        );
    }
}

/// The session middleware refuses to run without an encryption key.
fn ensure_crypt() {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
}

fn session_config() -> SessionConfig {
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    config
}

/// What [`SessionStoreOutage`] fails with.
const SESSION_WRITE_ERROR: &str = "the sessions table is read-only";
const SESSION_DESTROY_ERROR: &str = "deleting session row 7 timed out";

/// A session store that reads every session it is asked for, signed in,
/// and can neither write nor delete one.
struct SessionStoreOutage;

#[async_trait]
impl SessionStore for SessionStoreOutage {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        let mut session = SessionData::new(id.to_string(), "d".repeat(40));
        session.user_id = Some("ledger-clerk".to_string());
        session.loaded_from_store = true;
        Ok(Some(session))
    }
    async fn write(&self, _session: &SessionData) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal(SESSION_WRITE_ERROR))
    }
    async fn destroy(&self, _id: &str) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal(SESSION_DESTROY_ERROR))
    }
    async fn destroy_for_user(&self, _user_id: &str) -> Result<u64, FrameworkError> {
        Ok(0)
    }
    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

/// A `Cookie` header naming a stored session, encrypted the way the
/// session middleware reads it.
fn session_cookie(config: &SessionConfig) -> String {
    let id = suprnova::session::generate_session_id();
    let cookie = Cookie::encrypted(&config.cookie_name, &id).expect("encrypt the session cookie");
    let mut value = String::new();
    for byte in cookie.value().bytes() {
        match byte {
            b'=' => value.push_str("%3D"),
            b'+' => value.push_str("%2B"),
            b'/' => value.push_str("%2F"),
            _ => value.push(byte as char),
        }
    }
    format!("{}={value}", config.cookie_name)
}

#[tokio::test]
async fn a_failed_session_write_carries_the_store_error() {
    production_config();
    ensure_crypt();
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/sign-in", |_req: Request| async {
                suprnova::session::set_auth_user("ledger-clerk");
                text("signed in")
            })
            .middleware(SessionMiddleware::with_store(
                session_config(),
                Arc::new(SessionStoreOutage),
            ))
            .into(),
    );

    let (response, _) = get(&router, "/sign-in").await;

    assert_eq!(response.status(), 500);
    assert_report_names(&response, &[SESSION_WRITE_ERROR]);
}

#[tokio::test]
async fn a_failed_session_rotation_carries_the_store_error() {
    production_config();
    ensure_crypt();
    let config = session_config();
    let cookie = session_cookie(&config);
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/elevate", |_req: Request| async {
                suprnova::session::regenerate_session_id();
                text("elevated")
            })
            .middleware(SessionMiddleware::with_store(
                config,
                Arc::new(SessionStoreOutage),
            ))
            .into(),
    );

    let (response, _) = send(
        &router,
        MiddlewareRegistry::new(),
        "/elevate",
        &[("Cookie", cookie.as_str())],
    )
    .await;

    assert_eq!(response.status(), 500);
    assert_report_names(&response, &[SESSION_DESTROY_ERROR]);
}

/// What [`CacheOutage`] fails with.
const CACHE_READ_ERROR: &str = "the cache replica at 10.0.0.7 refused the read";
const CACHE_LOCK_ERROR: &str = "the cache lock script was evicted";

/// A cache whose counters work and whose reads and locks fail: enough
/// for a throttle to count a hit and then fail to say when to retry.
struct CacheOutage(InMemoryCache);

#[async_trait]
impl CacheStore for CacheOutage {
    async fn get_raw(&self, _key: &str) -> Result<Option<String>, FrameworkError> {
        Err(FrameworkError::internal(CACHE_READ_ERROR))
    }
    async fn put_raw(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), FrameworkError> {
        self.0.put_raw(key, value, ttl).await
    }
    async fn add_raw(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<bool, FrameworkError> {
        self.0.add_raw(key, value, ttl).await
    }
    async fn has(&self, key: &str) -> Result<bool, FrameworkError> {
        self.0.has(key).await
    }
    async fn forget(&self, key: &str) -> Result<bool, FrameworkError> {
        self.0.forget(key).await
    }
    async fn flush(&self) -> Result<(), FrameworkError> {
        self.0.flush().await
    }
    async fn increment(&self, key: &str, amount: i64) -> Result<i64, FrameworkError> {
        self.0.increment(key, amount).await
    }
    async fn decrement(&self, key: &str, amount: i64) -> Result<i64, FrameworkError> {
        self.0.decrement(key, amount).await
    }
    async fn tagged_put_raw(
        &self,
        tags: &[&str],
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), FrameworkError> {
        self.0.tagged_put_raw(tags, key, value, ttl).await
    }
    async fn flush_tags(&self, tags: &[&str]) -> Result<(), FrameworkError> {
        self.0.flush_tags(tags).await
    }
    async fn acquire_lock(
        &self,
        _key: &str,
        _ttl: Duration,
    ) -> Result<Option<String>, FrameworkError> {
        Err(FrameworkError::internal(CACHE_LOCK_ERROR))
    }
    async fn release_lock(&self, key: &str, token: &str) -> Result<bool, FrameworkError> {
        self.0.release_lock(key, token).await
    }
    async fn refresh_lock(
        &self,
        key: &str,
        token: &str,
        ttl: Duration,
    ) -> Result<bool, FrameworkError> {
        self.0.refresh_lock(key, token, ttl).await
    }
    async fn touch(&self, key: &str, ttl: Duration) -> Result<bool, FrameworkError> {
        self.0.touch(key, ttl).await
    }
}

/// Run `request` with [`CacheOutage`] as the cache, for this test only.
async fn during_a_cache_outage<F: std::future::Future>(request: F) -> F::Output {
    TestContainer::scope(async {
        TestContainer::bind::<dyn CacheStore>(Arc::new(CacheOutage(InMemoryCache::new())));
        request.await
    })
    .await
}

#[tokio::test]
async fn a_session_lock_the_cache_cannot_take_carries_the_cache_error() {
    production_config();
    ensure_crypt();
    let config = session_config().block(SessionBlock::new(
        Duration::from_secs(5),
        Duration::from_millis(200),
    ));
    let cookie = session_cookie(&config);
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/notices", |_req: Request| async { text("notices") })
            .middleware(SessionMiddleware::with_store(
                config,
                Arc::new(SessionStoreOutage),
            ))
            .into(),
    );

    let (response, _) = during_a_cache_outage(send(
        &router,
        MiddlewareRegistry::new(),
        "/notices",
        &[("Cookie", cookie.as_str())],
    ))
    .await;

    assert_eq!(response.status(), 500);
    assert_report_names(&response, &[CACHE_LOCK_ERROR]);
}

#[tokio::test]
async fn a_throttle_whose_cache_fails_carries_the_cache_error() {
    production_config();
    // No attempts allowed, so the first request trips the limit and the
    // throttle reads the cache to say when to retry.
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/statement", |_req: Request| async { text("statement") })
            .middleware(ThrottleRequestsMiddleware::with(0, 1, "statement"))
            .into(),
    );

    let (response, _) = during_a_cache_outage(get(&router, "/statement")).await;

    assert_eq!(response.status(), 500);
    assert_report_names(&response, &[CACHE_READ_ERROR]);
}

#[tokio::test]
async fn a_throttle_naming_an_undefined_limiter_reports_the_name() {
    production_config();
    const LIMITER: &str = "ledger-exports-never-defined";
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/exports", |_req: Request| async { text("exports") })
            .middleware(ThrottleRequestsMiddleware::by_name(LIMITER))
            .into(),
    );

    let (response, _) = get(&router, "/exports").await;

    assert_eq!(response.status(), 503);
    assert_report_names(&response, &[LIMITER]);
}

/// What [`LimiterOutage`] fails with.
const LIMITER_ERROR: &str = "the limiter's Redis at 10.0.0.9 is unreachable";

/// A rate limiter backend that can never decide.
struct LimiterOutage;

#[async_trait]
impl RateLimiterDriver for LimiterOutage {
    async fn try_acquire(
        &self,
        _key: &str,
        _config: &SlidingWindowConfig,
    ) -> Result<bool, FrameworkError> {
        Err(FrameworkError::internal(LIMITER_ERROR))
    }
    async fn retry_after(
        &self,
        _key: &str,
        _config: &SlidingWindowConfig,
    ) -> Result<Option<Duration>, FrameworkError> {
        Err(FrameworkError::internal(LIMITER_ERROR))
    }
}

#[tokio::test]
async fn a_rate_limiter_failing_closed_carries_the_backend_error() {
    production_config();
    let limiter = RateLimitMiddleware::new(
        Arc::new(LimiterOutage),
        SlidingWindowConfig {
            max_requests: 5,
            window: Duration::from_secs(60),
        },
        |_req| "ledger".to_string(),
    )
    .on_backend_error(BackendErrorPolicy::FailClosed);
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/payouts", |_req: Request| async { text("payouts") })
            .middleware(limiter)
            .into(),
    );

    let (response, _) = get(&router, "/payouts").await;

    assert_eq!(response.status(), 503);
    assert_report_names(&response, &[LIMITER_ERROR]);
}

#[tokio::test]
async fn a_login_throttle_failing_closed_carries_the_backend_error() {
    production_config();
    const EMAIL: &str = "clerk@ledger.test";
    // No Magnetar is bound in this binary, so the brute-force backend
    // cannot answer; ask it directly for the error the middleware sees.
    let backend_error = BruteForce::get_lockout_status(EMAIL)
        .await
        .expect_err("with no Magnetar bound the brute-force backend must fail");
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/login", |_req: Request| async { text("login") })
            .middleware(LoginThrottleMiddleware::new(|req: &Request| {
                req.header("X-Login-Email").map(str::to_string)
            }))
            .into(),
    );

    let (response, _) = send(
        &router,
        MiddlewareRegistry::new(),
        "/login",
        &[("X-Login-Email", EMAIL)],
    )
    .await;

    assert_eq!(response.status(), 503);
    assert_eq!(report_of(&response).chain()[0], backend_error.to_string());
}

#[tokio::test]
async fn a_request_past_its_timeout_reports_the_route_and_the_deadline() {
    production_config();
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/ledger-rebuild", |_req: Request| async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                text("rebuilt")
            })
            .middleware(TimeoutMiddleware::new(Duration::from_millis(50)))
            .into(),
    );

    let (response, _) = get(&router, "/ledger-rebuild").await;

    assert_eq!(response.status(), 503);
    assert_report_names(&response, &["/ledger-rebuild", "50 ms"]);
}

/// What the WebSocket route's middleware panics with.
const UPGRADE_PANIC: &str = "the ledger feed lost its cursor";

/// The line [`PanicsBeforeUpgrade`] panics on, recorded when it runs.
static UPGRADE_PANIC_LINE: AtomicU32 = AtomicU32::new(0);

/// A middleware on a WebSocket route that panics before the upgrade.
struct PanicsBeforeUpgrade;

#[async_trait]
impl Middleware for PanicsBeforeUpgrade {
    async fn handle(&self, _request: Request, _next: Next) -> Response {
        UPGRADE_PANIC_LINE.store(line!() + 1, Ordering::SeqCst);
        panic!("{UPGRADE_PANIC}");
    }
}

/// Never reached: the upgrade is aborted before it.
struct LedgerFeed;

#[async_trait]
impl WebSocketHandler for LedgerFeed {
    async fn handle(&self, _socket: WsSocket, _request: Request) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// The headers of a well-formed WebSocket upgrade (RFC 6455).
const UPGRADE: &[(&str, &str)] = &[
    ("Connection", "Upgrade"),
    ("Upgrade", "websocket"),
    ("Sec-WebSocket-Version", "13"),
    ("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ=="),
];

#[tokio::test]
async fn a_websocket_upgrade_middleware_panic_reports_its_message_and_location() {
    production_config();
    let router = Arc::new(Router::new().ws_with_middleware_and_config(
        "/ledger-feed",
        LedgerFeed,
        vec![into_boxed(PanicsBeforeUpgrade)],
        WsConfig {
            origin_policy: OriginPolicy::AllowAny,
            ..WsConfig::default()
        },
    ));

    let (response, _) = send(&router, MiddlewareRegistry::new(), "/ledger-feed", UPGRADE).await;

    assert_eq!(response.status(), 500);
    let report = report_of(&response);
    assert!(
        report.is_panic(),
        "the report must be the panic's: {report}"
    );
    let report = report.to_string();
    assert!(
        report.contains(UPGRADE_PANIC),
        "the report must carry the panic message; report:\n{report}"
    );
    let line = UPGRADE_PANIC_LINE.load(Ordering::SeqCst);
    assert!(
        names_location(&report, file!(), line),
        "the report must name where the middleware panicked, {}:{line}; report:\n{report}",
        file!()
    );
}

// ---------------------------------------------------------------------
// Inertia
// ---------------------------------------------------------------------

/// The page component the error page middleware renders.
const ERROR_PAGE: &str = "Error";

/// The headers of an Inertia visit.
const INERTIA_VISIT: &[(&str, &str)] = &[("X-Inertia", "true")];

/// Stands in for `SessionMiddleware`: the Inertia error page and the
/// validation redirect read and write the session, so they run inside a
/// session scope as they do in an app. A fresh slot per request keeps the
/// tests independent.
struct SessionScope;

#[async_trait]
impl Middleware for SessionScope {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let slot = suprnova::session::new_session_slot_for_test();
        suprnova::session::session_scope_for_test(slot, next(request)).await
    }
}

/// `/posts` fails the way `/invoice` does, behind the Inertia error page.
fn inertia_routes() -> Arc<Router> {
    Arc::new(
        Router::new()
            .get("/posts", post_invoice)
            .middleware(InertiaErrorPageMiddleware::new(ERROR_PAGE))
            .into(),
    )
}

/// An Inertia visit to `/posts`, which comes back as the error page.
async fn failed_inertia_visit() -> TestResponse {
    let (response, _) = send(
        &inertia_routes(),
        MiddlewareRegistry::new().append(SessionScope),
        "/posts",
        INERTIA_VISIT,
    )
    .await;
    assert_eq!(
        response.header("x-inertia"),
        Some("true"),
        "the 500 must have become the Inertia error page; body: {}",
        response.body_text()
    );
    response
}

#[tokio::test]
async fn assert_inertia_on_an_error_page_shows_the_error_report() {
    production_config();
    let response = failed_inertia_visit().await;

    let failure = failure_of(|| {
        response.assert_inertia().component("Posts/Index");
    });
    for text in [
        "AssertableInertia::component",
        SECTION,
        INVOICE_ERROR,
        LEDGER_ERROR,
        DISK_ERROR,
    ] {
        assert!(
            failure.contains(text),
            "component() on an error page must show {text:?}; failure:\n{failure}"
        );
    }
}

#[tokio::test]
async fn assertable_inertia_from_an_error_page_response_shows_the_error_report() {
    production_config();
    let request = Request::new(incoming_get_request("/posts", INERTIA_VISIT).await);
    let next: Next = Arc::new(|request| Box::pin(post_invoice(request)));
    let rendered = suprnova::session::session_scope_for_test(
        suprnova::session::new_session_slot_for_test(),
        InertiaErrorPageMiddleware::new(ERROR_PAGE).handle(request, next),
    )
    .await;
    let Err(page) = rendered else {
        panic!("the error page must keep the failed request's Err side");
    };
    assert_eq!(
        page.header_value("X-Inertia"),
        Some("true"),
        "the 500 must have become the Inertia error page"
    );

    let failure = failure_of(|| {
        AssertableInertia::from_response(&page).has("posts");
    });
    for text in [
        "AssertableInertia::has",
        SECTION,
        INVOICE_ERROR,
        LEDGER_ERROR,
        DISK_ERROR,
    ] {
        assert!(
            failure.contains(text),
            "has() on an error page must show {text:?}; failure:\n{failure}"
        );
    }
}

#[tokio::test]
async fn the_inertia_error_page_keeps_the_report_of_the_error_it_replaces() {
    production_config();
    let response = failed_inertia_visit().await;

    assert_eq!(response.status(), 500);
    assert_report_names(&response, &[INVOICE_ERROR, LEDGER_ERROR, DISK_ERROR]);
}

#[tokio::test]
async fn the_inertia_validation_redirect_keeps_the_report_of_the_422_it_replaces() {
    production_config();
    let router: Arc<Router> = Arc::new(
        Router::new()
            .get("/invoices/new", |_req: Request| async {
                let mut errors = ValidationErrors::new();
                errors.add("amount", "The amount must be positive.");
                let response: Response = Err(HttpResponse::from(
                    FrameworkError::validation_errors(errors),
                ));
                response
            })
            .into(),
    );
    let registry = MiddlewareRegistry::new()
        .append(SessionScope)
        .append(InertiaValidationRedirectMiddleware::new());

    let (response, _) = send(
        &router,
        registry,
        "/invoices/new",
        &[
            ("X-Inertia", "true"),
            ("Referer", "http://localhost/invoices/new"),
        ],
    )
    .await;

    assert_eq!(
        response.status(),
        303,
        "the 422 must have become the redirect back; body: {}",
        response.body_text()
    );
    assert_eq!(report_of(&response).chain(), ["Validation failed"]);
}
