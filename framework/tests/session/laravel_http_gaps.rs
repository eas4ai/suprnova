//! The Laravel HTTP API gaps that live in the session and CSRF layer
//! (PAR-113): `CsrfMiddleware` checks every method but `GET`, `HEAD` and
//! `OPTIONS`, reads `_token` from a JSON body as from a form body, and
//! `regenerate_session_id()` issues a new CSRF token as Laravel's
//! `Session::regenerate` does. And PAR-111's clause for the session layer:
//! the `500` the session middleware answers a failed write with is
//! reported through `Exceptions`, once.
//!
//! The requests run through a `TestClient` whose registry holds a real
//! `SessionMiddleware` and `CsrfMiddleware`, the order an application
//! registers them in. A `GET` first gives the client its session and the
//! `XSRF-TOKEN` cookie, as a browser gets them.
//!
//! Gated on the `testing` feature, which `TestClient`, the session test
//! scopes and the Live test hooks need.
#![cfg(feature = "testing")]

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::json;
use serial_test::serial;
use suprnova::live::testing::{
    LiveSecurityCheck, LiveSecurityDisposition, LiveTestOperation, LiveTestRoutePolicy,
    inspect_request_attestation, register_live_route_for_test,
};
use suprnova::logging::current_request_id;
use suprnova::session::{
    SessionConfig, SessionData, SessionMiddleware, SessionStore, new_session_slot_for_test,
    regenerate_session_id, session, session_mut, session_scope_for_test,
};
use suprnova::testing::{TestClient, TestResponse};
use suprnova::{
    App, CsrfMiddleware, Exceptions, FrameworkError, HttpResponse, Method, MiddlewareRegistry,
    Request, Response, Router, query, routes,
};

/// A session store keyed by id that remembers which ids it destroyed, so
/// a test can see the old row of a regenerated session go.
#[derive(Default)]
struct SessionMap {
    sessions: Mutex<HashMap<String, SessionData>>,
    destroyed: Mutex<HashSet<String>>,
}

impl SessionMap {
    fn ids(&self) -> Vec<String> {
        self.sessions.lock().unwrap().keys().cloned().collect()
    }

    fn was_destroyed(&self, id: &str) -> bool {
        self.destroyed.lock().unwrap().contains(id)
    }
}

#[async_trait]
impl SessionStore for SessionMap {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.sessions.lock().unwrap().get(id).cloned())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.sessions
            .lock()
            .unwrap()
            .insert(session.id.clone(), session.clone());
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.sessions.lock().unwrap().remove(id);
        self.destroyed.lock().unwrap().insert(id.to_owned());
        Ok(())
    }

    async fn destroy_for_user(&self, _user_id: &str) -> Result<u64, FrameworkError> {
        Ok(0)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

/// Answers with the request's method and its whole body, so a test sees
/// both that the handler ran and that the CSRF check left the body whole.
async fn echo(request: Request) -> Response {
    let method = request.method().to_string();
    let (_, body) = request.body_bytes().await?;
    Ok(HttpResponse::text(format!(
        "{method}:{}",
        String::from_utf8_lossy(&body)
    )))
}

/// Rotates the session id, as a login or a privilege change does.
async fn regenerate(_request: Request) -> Response {
    regenerate_session_id();
    Ok(HttpResponse::text("regenerated"))
}

/// A client whose registry runs `SessionMiddleware` over `store`, then
/// `CsrfMiddleware` with its defaults, in front of `router`.
fn client(router: Router, store: Arc<SessionMap>) -> TestClient {
    suprnova::testing::install_test_encryption_key();
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    let registry = MiddlewareRegistry::new()
        .append(SessionMiddleware::with_store(config, store))
        .append(CsrfMiddleware::new());
    TestClient::new(router, registry)
}

/// `GET` a page, as a browser's first visit does, and return the session's
/// CSRF token from the `XSRF-TOKEN` cookie it hands out.
async fn bootstrap(client: &TestClient) -> String {
    let response = client.get("/gaps/page").send().await;
    response.assert_ok();
    response
        .cookie("XSRF-TOKEN")
        .expect("the first visit hands out the session's CSRF token")
}

fn routes_with_echo() -> Router {
    routes! {
        query!("/gaps/search", echo),
    }
    register()
        .get("/gaps/page", echo)
        .post("/gaps/submit", echo)
        .put("/gaps/submit", echo)
        .patch("/gaps/submit", echo)
        .delete("/gaps/submit", echo)
        .post("/gaps/regenerate", regenerate)
        .into()
}

fn method(name: &str) -> Method {
    Method::from_bytes(name.as_bytes()).expect("a valid method")
}

fn assert_refused(response: &TestResponse, what: &str) {
    assert_eq!(response.status(), 419, "{what}: {}", response.body_text());
    assert!(
        response.body_text().contains("CSRF token mismatch."),
        "{what}: {}",
        response.body_text()
    );
}

/// A `QUERY` request changes no state by its definition, but Laravel's
/// `isReading` names `GET`, `HEAD` and `OPTIONS` only, so `QUERY` is checked
/// like `POST`: without a token it never reaches its `query!` route, and
/// with the session's token it does.
#[tokio::test]
async fn a_query_request_without_a_token_is_refused() {
    let client = client(routes_with_echo(), Arc::default());
    let token = bootstrap(&client).await;

    let refused = client.send(method("QUERY"), "/gaps/search").send().await;
    assert_refused(&refused, "QUERY without a token");
    assert!(
        !refused.body_text().starts_with("QUERY:"),
        "the query! route ran"
    );

    let passed = client
        .send(method("QUERY"), "/gaps/search")
        .header("X-CSRF-TOKEN", token)
        .send()
        .await;
    passed.assert_ok();
    assert_eq!(passed.body_text(), "QUERY:");
}

/// An extension method such as WebDAV's `PROPFIND` is no reading method
/// either. No route serves it here, so with the token it reaches the
/// framework's `404`, and without one the CSRF check answers `419` first.
#[tokio::test]
async fn an_extension_method_without_a_token_is_refused() {
    let client = client(routes_with_echo(), Arc::default());
    let token = bootstrap(&client).await;

    let refused = client.send(method("PROPFIND"), "/gaps/dav").send().await;
    assert_refused(&refused, "PROPFIND without a token");

    let passed = client
        .send(method("PROPFIND"), "/gaps/dav")
        .header("X-CSRF-TOKEN", token)
        .send()
        .await;
    assert_eq!(
        passed.status(),
        404,
        "with the token the request passes the check and finds no route"
    );
}

/// `GET`, `HEAD` and `OPTIONS` stay unchecked, and every method that
/// changes state stays checked.
#[tokio::test]
async fn reading_methods_pass_and_writing_methods_stay_checked() {
    let client = client(routes_with_echo(), Arc::default());
    bootstrap(&client).await;

    for name in ["GET", "HEAD", "OPTIONS"] {
        let response = client.send(method(name), "/gaps/page").send().await;
        assert_ne!(response.status(), 419, "{name} is a reading method");
    }
    for name in ["POST", "PUT", "PATCH", "DELETE"] {
        let response = client.send(method(name), "/gaps/submit").send().await;
        assert_refused(&response, name);
    }
}

/// Laravel reads the token with `$request->input('_token')`, which reads a
/// JSON body: a JSON `POST` that carries the session's token in `_token`
/// and no header passes, and the handler still reads the whole body.
#[tokio::test]
async fn a_json_body_token_passes_without_a_header() {
    let client = client(routes_with_echo(), Arc::default());
    let token = bootstrap(&client).await;

    let body = json!({ "_token": token, "title": "Hello" });
    let passed = client.post("/gaps/submit").json(&body).send().await;
    passed.assert_ok();
    let echoed: serde_json::Value = serde_json::from_str(
        passed
            .body_text()
            .strip_prefix("POST:")
            .expect("the handler ran"),
    )
    .expect("the handler read the whole JSON body");
    assert_eq!(echoed, body);

    // Any media type that names JSON is read, as Laravel's `isJson` reads it.
    let passed = client
        .post("/gaps/submit")
        .json(&json!({ "_token": token }))
        .header("Content-Type", "application/vnd.api+json")
        .send()
        .await;
    passed.assert_ok();

    // A wrong body token is refused.
    let refused = client
        .post("/gaps/submit")
        .json(&json!({ "_token": "not-the-session-token" }))
        .send()
        .await;
    assert_refused(&refused, "a wrong JSON _token");
}

/// The body's `_token` comes before `X-CSRF-TOKEN` and `X-XSRF-TOKEN`, as
/// in a form, and `""` and `"0"` count as no value, so the headers decide.
#[tokio::test]
async fn a_json_body_token_decides_before_the_headers_unless_it_is_empty() {
    let client = client(routes_with_echo(), Arc::default());
    let token = bootstrap(&client).await;

    for empty in ["", "0"] {
        let passed = client
            .post("/gaps/submit")
            .json(&json!({ "_token": empty }))
            .header("X-CSRF-TOKEN", token.clone())
            .send()
            .await;
        assert_eq!(
            passed.status(),
            200,
            "an _token of {empty:?} is no value, so the valid header decides"
        );
        let passed = client
            .post("/gaps/submit")
            .json(&json!({ "_token": empty }))
            .header("X-XSRF-TOKEN", token.clone())
            .send()
            .await;
        assert_eq!(passed.status(), 200, "X-XSRF-TOKEN decides after {empty:?}");
    }

    let refused = client
        .post("/gaps/submit")
        .json(&json!({ "_token": "stale" }))
        .header("X-CSRF-TOKEN", token.clone())
        .send()
        .await;
    assert_refused(&refused, "a set JSON _token decides before a valid header");

    // A PHP-falsy `_token` such as `null` holds no token: the header decides.
    let passed = client
        .post("/gaps/submit")
        .json(&json!({ "_token": null, "count": 3 }))
        .header("X-CSRF-TOKEN", token)
        .send()
        .await;
    assert_eq!(passed.status(), 200);
}

/// PHP-falsy values other than `""` and `"0"` also fall through to the
/// headers, as Laravel's `input('_token') ?:` reads them.
#[tokio::test]
async fn a_falsy_json_body_token_falls_through_to_the_headers() {
    let client = client(routes_with_echo(), Arc::default());
    let token = bootstrap(&client).await;

    for falsy in [json!(0), json!(0.0), json!(false), json!([]), json!({})] {
        let passed = client
            .post("/gaps/submit")
            .json(&json!({ "_token": falsy }))
            .header("X-CSRF-TOKEN", token.clone())
            .send()
            .await;
        assert_eq!(
            passed.status(),
            200,
            "a falsy _token of {falsy} is no value, so the valid header decides"
        );
    }
}

/// A JSON `_token` that is present but not a string can never match the
/// session token, so the request is refused with 419 and the headers are
/// not consulted. Laravel keeps such a value rather than falling through.
#[tokio::test]
async fn a_non_string_json_body_token_refuses_before_the_headers() {
    let client = client(routes_with_echo(), Arc::default());
    let token = bootstrap(&client).await;

    for value in [
        json!(123),
        json!(-1),
        json!(0.5),
        json!(true),
        json!([1]),
        json!({ "a": 1 }),
    ] {
        let refused = client
            .post("/gaps/submit")
            .json(&json!({ "_token": value }))
            .header("X-CSRF-TOKEN", token.clone())
            .send()
            .await;
        assert_refused(
            &refused,
            &format!("a JSON _token of {value} is kept, so the valid header is not read"),
        );
    }
}

/// A body sent as JSON that does not parse holds no token, whatever text
/// it carries: a form-encoded `_token` under a JSON media type is not read,
/// and the headers decide.
#[tokio::test]
async fn a_body_that_is_not_json_holds_no_token() {
    let client = client(routes_with_echo(), Arc::default());
    let token = bootstrap(&client).await;

    let refused = client
        .post("/gaps/submit")
        .form(&[("_token", token.as_str())])
        .header("Content-Type", "application/json")
        .send()
        .await;
    assert_refused(&refused, "a form body under a JSON media type");

    let passed = client
        .post("/gaps/submit")
        .form(&[("_token", "stale")])
        .header("Content-Type", "application/json")
        .header("X-CSRF-TOKEN", token)
        .send()
        .await;
    assert_eq!(passed.status(), 200, "the header decides");
}

/// The same-origin pass stays off by default: a request the browser marks
/// `same-origin` still needs its token.
#[tokio::test]
async fn a_same_origin_request_without_a_token_is_refused_by_default() {
    let client = client(routes_with_echo(), Arc::default());
    bootstrap(&client).await;

    let refused = client
        .post("/gaps/submit")
        .header("Sec-Fetch-Site", "same-origin")
        .send()
        .await;
    assert_refused(&refused, "same-origin without a token");
}

/// `regenerate_session_id()` issues a new CSRF token beside the new id, as
/// Laravel's `Session::regenerate` calls `regenerateToken`.
#[tokio::test]
async fn regenerating_the_session_issues_a_new_csrf_token() {
    let slot = new_session_slot_for_test();
    let (before, after) = session_scope_for_test(slot, async {
        let before = session().expect("a session is in scope");
        regenerate_session_id();
        let after = session().expect("a session is in scope");
        (before, after)
    })
    .await;
    assert_ne!(after.id, before.id, "the session id rotates");
    assert_ne!(
        after.csrf_token, before.csrf_token,
        "the CSRF token rotates with it"
    );
}

/// Over a real session: the token read before the regeneration is refused
/// after it, the new token passes, and the old session row is destroyed.
#[tokio::test]
async fn a_token_read_before_regeneration_is_refused_after_it() {
    let store = Arc::new(SessionMap::default());
    let client = client(routes_with_echo(), store.clone());
    let old_token = bootstrap(&client).await;
    let old_ids = store.ids();
    assert_eq!(old_ids.len(), 1, "the first visit stored one session");

    let regenerated = client
        .post("/gaps/regenerate")
        .header("X-CSRF-TOKEN", old_token.clone())
        .send()
        .await;
    regenerated.assert_ok();
    let new_token = regenerated
        .cookie("XSRF-TOKEN")
        .expect("the regenerated session hands out its new token");
    assert_ne!(new_token, old_token);

    let refused = client
        .post("/gaps/submit")
        .header("X-CSRF-TOKEN", old_token)
        .send()
        .await;
    assert_refused(&refused, "the token from before the regeneration");

    let passed = client
        .post("/gaps/submit")
        .header("X-CSRF-TOKEN", new_token)
        .send()
        .await;
    passed.assert_ok();

    assert!(
        store.was_destroyed(&old_ids[0]),
        "the old session row is destroyed"
    );
    assert!(
        !store.ids().contains(&old_ids[0]),
        "the old session row is gone from the store"
    );
}

/// The Live read branch keeps its behaviour: a Live event stream is a `GET`
/// with the browser's origin proof and no session token, and it reaches its
/// handler with the CSRF check recorded as not required.
#[tokio::test]
async fn a_live_stream_still_passes_without_a_token() {
    suprnova::testing::install_test_encryption_key();
    App::init();
    let captured = Arc::new(Mutex::new(None));
    let captured_in_handler = Arc::clone(&captured);
    let mut router: Router = Router::new()
        .get("/__live/sse/events", move |request: Request| {
            let captured = Arc::clone(&captured_in_handler);
            async move {
                *captured.lock().unwrap() = Some(inspect_request_attestation(&request));
                Ok(HttpResponse::text("stream"))
            }
        })
        .middleware(CsrfMiddleware::new())
        .into();
    register_live_route_for_test(
        &mut router,
        Method::GET,
        "/__live/sse/events",
        LiveTestOperation::SseControl,
        LiveTestRoutePolicy {
            trusted_internal_origin: false,
            stateless_csrf: false,
            stateless_session: true,
            anonymous_principal: true,
            tenantless: true,
            direct_peer: false,
            upstream_rate_limit: true,
            no_additional_middleware: true,
        },
    )
    .expect("register the Live stream route");

    let client = TestClient::new(router, MiddlewareRegistry::new());
    let response = client
        .get("/__live/sse/events")
        .header("Sec-Fetch-Site", "same-origin")
        .send()
        .await;
    response.assert_ok();
    assert_eq!(response.body_text(), "stream");

    let report = captured
        .lock()
        .unwrap()
        .take()
        .expect("the Live stream handler ran");
    assert_eq!(
        report.disposition(LiveSecurityCheck::Origin),
        Some(LiveSecurityDisposition::Passed)
    );
    assert_eq!(
        report.disposition(LiveSecurityCheck::Csrf),
        Some(LiveSecurityDisposition::NotRequired)
    );
}

// ---- PAR-111: the session layer's 500s are reported ----------------------
//
// The `Exceptions` registry is process-wide. Each test below runs
// `#[serial]`, holds the binary's env lock, empties the registry at its
// start and end ([`Isolated`]), and counts only the errors reported while
// its own request, named by its `X-Request-Id`, is in flight.

/// Empties the `Exceptions` registry when made and when dropped, and holds
/// the env lock so no other test of this binary that registers callbacks
/// overlaps.
struct Isolated {
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

impl Isolated {
    async fn new() -> Self {
        let lock = crate::env_lock::lock_env_async().await;
        Exceptions::reset();
        Self { _lock: lock }
    }
}

impl Drop for Isolated {
    fn drop(&mut self) {
        Exceptions::reset();
    }
}

/// Record the text of each error reported while the request `request_id`
/// is in flight.
fn record_reports_for(request_id: &'static str) -> Arc<Mutex<Vec<String>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    Exceptions::reportable(move |error: &FrameworkError| {
        let current = current_request_id();
        if current.as_ref().map(|id| id.as_str()) == Some(request_id) {
            record.lock().unwrap().push(error.to_string());
        }
    });
    seen
}

fn reports(list: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    list.lock().unwrap().clone()
}

/// A session store whose every write fails, as an unreachable database
/// does. A cookieless request never reads.
struct WriteRefused;

#[async_trait]
impl SessionStore for WriteRefused {
    async fn read(&self, _id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(None)
    }

    async fn write(&self, _session: &SessionData) -> Result<(), FrameworkError> {
        Err(FrameworkError::internal(
            "laravel_http_gaps: the session store refused the write",
        ))
    }

    async fn destroy(&self, _id: &str) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn destroy_for_user(&self, _user_id: &str) -> Result<u64, FrameworkError> {
        Ok(0)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

/// Changes the session, so the middleware must store it.
async fn put_in_cart(_request: Request) -> Response {
    session_mut(|session| session.put("cart", 3));
    Ok(HttpResponse::text("added"))
}

/// Answers with a `500` of its own, as an application may.
async fn own_500(_request: Request) -> Response {
    Ok(HttpResponse::text("the kettle is down").status(500))
}

/// Fails with a `FrameworkError`, which the framework turns into a `500`.
async fn failing(_request: Request) -> Response {
    Err(HttpResponse::from(FrameworkError::internal(
        "laravel_http_gaps: the handler failed",
    )))
}

/// A client whose registry runs `SessionMiddleware` alone over `store`.
fn session_client(store: Arc<dyn SessionStore>) -> TestClient {
    suprnova::testing::install_test_encryption_key();
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    let router: Router = Router::new()
        .get("/gaps/cart", put_in_cart)
        .get("/gaps/own-500", own_500)
        .get("/gaps/failing", failing)
        .into();
    TestClient::new(
        router,
        MiddlewareRegistry::new().append(SessionMiddleware::with_store(config, store)),
    )
}

#[tokio::test]
#[serial]
async fn a_failed_session_write_is_reported_once() {
    let _isolated = Isolated::new().await;
    let seen = record_reports_for("gaps-session-write-refused");
    let client = session_client(Arc::new(WriteRefused));

    let response = client
        .get("/gaps/cart")
        .header("X-Request-Id", "gaps-session-write-refused")
        .send()
        .await;

    assert_eq!(response.status(), 500);
    assert!(
        response.body_text().contains("session persistence failed"),
        "the 500 is the failed write's: {}",
        response.body_text()
    );
    assert_eq!(
        reports(&seen),
        ["Internal server error: laravel_http_gaps: the session store refused the write"],
        "the write's error reaches the callbacks once"
    );
}

#[tokio::test]
#[serial]
async fn a_500_the_application_answers_with_is_not_reported() {
    let _isolated = Isolated::new().await;
    let seen = record_reports_for("gaps-session-own-500");
    let client = session_client(Arc::new(SessionMap::default()));

    let response = client
        .get("/gaps/own-500")
        .header("X-Request-Id", "gaps-session-own-500")
        .send()
        .await;

    assert_eq!(response.status(), 500);
    assert_eq!(response.body_text(), "the kettle is down");
    assert!(
        reports(&seen).is_empty(),
        "an application's own response is no framework error: {:?}",
        reports(&seen)
    );
}

#[tokio::test]
#[serial]
async fn a_framework_error_that_becomes_a_500_behind_the_session_is_reported_once() {
    let _isolated = Isolated::new().await;
    let seen = record_reports_for("gaps-session-handler-failed");
    let client = session_client(Arc::new(SessionMap::default()));

    let response = client
        .get("/gaps/failing")
        .header("X-Request-Id", "gaps-session-handler-failed")
        .send()
        .await;

    assert_eq!(response.status(), 500);
    assert_eq!(
        reports(&seen),
        ["Internal server error: laravel_http_gaps: the handler failed"],
        "the 5xx block reports the handler's error, and nothing reports it again"
    );
}
