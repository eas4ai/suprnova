//! `TestClient` (PAR-063): requests driven through the framework's request
//! path over an in-memory connection, with the session cookie carried from
//! one request to the next and the error report and session store kept on
//! every `TestResponse` it returns.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use suprnova::config::{AppConfig, Config, Environment};
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::testing::{AssertableInertia, TestClient, TestContainer};
use suprnova::{
    App, Cookie, FrameworkError, HttpResponse, Inertia, InertiaConfig, InertiaRequestExt,
    InertiaResponse, Middleware, MiddlewareRegistry, Next, Request, Response, Router, SsrConfig,
    SsrGateway, SsrResponse,
};

/// A session store that keeps every session in memory.
#[derive(Default)]
pub(crate) struct MemoryStore {
    sessions: Mutex<HashMap<String, SessionData>>,
}

#[async_trait]
impl SessionStore for MemoryStore {
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
        Ok(())
    }

    async fn destroy_for_user(&self, _user_id: &str) -> Result<u64, FrameworkError> {
        Ok(0)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

/// A client whose registry runs a real `SessionMiddleware` over a memory
/// store, with the store attached to every response.
pub(crate) fn session_client(router: impl Into<Router>) -> (TestClient, Arc<MemoryStore>) {
    suprnova::testing::install_test_encryption_key();
    let store = Arc::new(MemoryStore::default());
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    let registry =
        MiddlewareRegistry::new().append(SessionMiddleware::with_store(config, store.clone()));
    let client =
        TestClient::new(router, registry).with_session_store(store.clone(), "suprnova_session");
    (client, store)
}

fn session_routes() -> Router {
    Router::new()
        .get("/remember", |_req: Request| async {
            suprnova::session::session_mut(|session| session.put("color", "blue"));
            suprnova::http::text("stored")
        })
        .get("/recall", |_req: Request| async {
            let color = suprnova::session::session()
                .and_then(|session| session.get::<String>("color"))
                .unwrap_or_else(|| "none".to_string());
            suprnova::http::text(color)
        })
        .into()
}

#[tokio::test]
async fn intt_client_carries_the_session_from_one_request_to_the_next() {
    let (client, _store) = session_client(session_routes());

    client.get("/remember").send().await.assert_ok();
    let recalled = client.get("/recall").send().await;

    recalled.assert_ok();
    assert_eq!(
        recalled.body_text(),
        "blue",
        "the second request must see the session"
    );
}

#[tokio::test]
async fn intt_client_response_asserts_the_session_through_the_attached_store() {
    let (client, _store) = session_client(session_routes());

    client
        .get("/remember")
        .send()
        .await
        .assert_session_has("color", "blue")
        .await;
    // A request that only reads the session sets no cookie; the session is
    // the one the client carries.
    client
        .get("/recall")
        .send()
        .await
        .assert_session_has("color", "blue")
        .await;
}

/// Stamps every response, to prove the registry's middleware ran.
struct Stamp;

#[async_trait]
impl Middleware for Stamp {
    async fn handle(&self, request: Request, next: Next) -> Response {
        match next(request).await {
            Ok(response) => Ok(response.header("X-Stamped", "yes")),
            Err(response) => Err(response.header("X-Stamped", "yes")),
        }
    }
}

#[tokio::test]
async fn intt_client_runs_the_middleware_the_registry_registers() {
    let router = Router::new().get("/", |_req: Request| async { suprnova::http::text("hi") });
    let client = TestClient::new(router, MiddlewareRegistry::new().append(Stamp));

    client
        .get("/")
        .send()
        .await
        .assert_ok()
        .assert_header("x-stamped", "yes");
}

#[tokio::test]
async fn intt_client_keeps_the_error_report_of_a_failed_handler() {
    let router = Router::new().get("/ledger", |_req: Request| async {
        let response: Response = Err(FrameworkError::domain("ledger closed", 503).into());
        response
    });
    let client = TestClient::new(router, MiddlewareRegistry::new());

    let response = client.get("/ledger").send().await;

    response.assert_status(503);
    let report = response
        .error_report()
        .expect("the handler's error must reach the TestResponse");
    assert!(
        report
            .chain()
            .iter()
            .any(|link| link.contains("ledger closed")),
        "the report must carry the handler's message: {report}"
    );
}

#[tokio::test]
async fn intt_client_inertia_visit_asserts_the_component() {
    let router = Router::new().get("/dashboard", |req: Request| async move {
        InertiaResponse::new("Dashboard")
            .with("count", 3)
            .resolve(&req)
            .await
            .map_err(HttpResponse::from)
    });
    let client = TestClient::new(router, MiddlewareRegistry::new());

    client
        .get("/dashboard")
        .inertia()
        .send()
        .await
        .assert_header("x-inertia", "true")
        .assert_inertia()
        .component("Dashboard")
        .where_("count", 3);
}

#[tokio::test]
async fn intt_client_inertia_visit_sends_the_installed_version() {
    let _container = TestContainer::fake();
    Inertia::install(
        &InertiaConfig::new()
            .development(true)
            .register_globally(false)
            .version("build-7"),
    )
    .expect("a development install");
    let router = Router::new().get("/version", |req: Request| async move {
        let sent = req
            .header("X-Inertia-Version")
            .unwrap_or("<none>")
            .to_string();
        let accept = req.header("Accept").unwrap_or("<none>").to_string();
        Ok(HttpResponse::json(
            json!({"version": sent, "accept": accept}),
        ))
    });
    let client = TestClient::new(router, MiddlewareRegistry::new());

    client
        .get("/version")
        .inertia()
        .send()
        .await
        .assert_json(json!({"version": "build-7", "accept": "text/html, application/xhtml+xml"}));
    client
        .get("/version")
        .inertia()
        .inertia_version("pinned")
        .send()
        .await
        .assert_json(json!({"version": "pinned"}));
}

#[tokio::test]
async fn intt_client_json_body_reaches_the_handler() {
    let router = Router::new().post("/echo", |req: Request| async move {
        let body: Value = req.json().await.map_err(HttpResponse::from)?;
        Ok(HttpResponse::json(json!({"received": body})))
    });
    let client = TestClient::new(router, MiddlewareRegistry::new());

    client
        .post("/echo")
        .json(&json!({"name": "Ada", "tags": ["x"]}))
        .send()
        .await
        .assert_ok()
        .assert_json(json!({"received": {"name": "Ada", "tags": ["x"]}}));
}

#[tokio::test]
async fn intt_client_form_body_and_headers_reach_the_handler() {
    let router = Router::new().put("/profile", |req: Request| async move {
        let custom = req.header("X-Custom").unwrap_or("<none>").to_string();
        let form: HashMap<String, String> = req.form().await.map_err(HttpResponse::from)?;
        Ok(HttpResponse::json(json!({"form": form, "custom": custom})))
    });
    let client = TestClient::new(router, MiddlewareRegistry::new());

    client
        .put("/profile")
        .header("X-Custom", "on")
        .form(&[("name", "Ada Lovelace"), ("city", "London")])
        .send()
        .await
        .assert_ok()
        .assert_json(json!({
            "form": {"name": "Ada Lovelace", "city": "London"},
            "custom": "on",
        }));
}

#[tokio::test]
async fn intt_client_sends_each_method_it_names() {
    let echo = |req: Request| async move { suprnova::http::text(req.method().to_string()) };
    let router = Router::new()
        .get("/m", echo)
        .post("/m", echo)
        .put("/m", echo)
        .patch("/m", echo)
        .delete("/m", echo);
    let client = TestClient::new(router, MiddlewareRegistry::new());

    assert_eq!(client.get("/m").send().await.body_text(), "GET");
    assert_eq!(client.post("/m").send().await.body_text(), "POST");
    assert_eq!(client.put("/m").send().await.body_text(), "PUT");
    assert_eq!(client.patch("/m").send().await.body_text(), "PATCH");
    assert_eq!(client.delete("/m").send().await.body_text(), "DELETE");
    assert_eq!(
        client
            .send(suprnova::Method::PATCH, "/m")
            .send()
            .await
            .body_text(),
        "PATCH"
    );
}

#[tokio::test]
async fn intt_client_drops_a_cookie_the_response_expires() {
    let router = Router::new()
        .get("/set", |_req: Request| async {
            Ok(HttpResponse::text("set").cookie(Cookie::new("flavor", "mint")))
        })
        .get("/forget", |_req: Request| async {
            Ok(HttpResponse::text("forgot").cookie(Cookie::forget("flavor")))
        })
        .get("/read", |req: Request| async move {
            suprnova::http::text(req.cookie("flavor").unwrap_or_else(|| "none".to_string()))
        });
    let client = TestClient::new(router, MiddlewareRegistry::new());

    client.get("/set").send().await.assert_ok();
    assert_eq!(client.get("/read").send().await.body_text(), "mint");
    client.get("/forget").send().await.assert_ok();
    assert_eq!(client.get("/read").send().await.body_text(), "none");
}

// ── PAR-064: `assert_inertia` on any page response ──────────────────

fn dashboard_routes() -> Router {
    Router::new()
        .get("/dashboard", |req: Request| async move {
            InertiaResponse::new("Dashboard")
                .with("count", 3)
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .into()
}

#[tokio::test]
async fn intt_assert_inertia_reads_the_first_visit_html_document() {
    let client = TestClient::new(dashboard_routes(), MiddlewareRegistry::new());

    let response = client.get("/dashboard").send().await;

    assert!(
        response.header("x-inertia").is_none(),
        "a first visit is the HTML document, not the JSON page"
    );
    response
        .assert_inertia()
        .component("Dashboard")
        .url("/dashboard")
        .where_("count", 3);
}

/// A gateway that renders every first visit and writes the page the way
/// Inertia 3.8's `buildSSRBody` does (`packages/core/src/ssrUtils.ts`):
/// `data-page` before `type`, every `/` as `\/` and every `<` as
/// `\u003c`, then the server-rendered mount element. The framework injects
/// that body into the document unchanged.
struct BuildSsrBody;

#[suprnova::async_trait]
impl SsrGateway for BuildSsrBody {
    async fn dispatch(
        &self,
        _config: &SsrConfig,
        _request: &dyn InertiaRequestExt,
        page: &Value,
    ) -> Result<Option<SsrResponse>, FrameworkError> {
        let json = page.to_string().replace('/', "\\/").replace('<', "\\u003c");
        Ok(Some(SsrResponse {
            head: Vec::new(),
            body: format!(
                "<script data-page=\"app\" type=\"application/json\">{json}</script>\
                 <div data-server-rendered=\"true\" id=\"app\"><p>rendered</p></div>"
            ),
        }))
    }
}

/// A first visit to `/dashboard` rendered through [`BuildSsrBody`], with
/// a check that the document is the server-rendered one.
async fn ssr_first_visit() -> suprnova::testing::TestResponse {
    App::bind::<dyn SsrGateway>(Arc::new(BuildSsrBody));
    let client = TestClient::new(dashboard_routes(), MiddlewareRegistry::new());
    let response = client.get("/dashboard").send().await;
    let document = response.body_text();
    assert!(
        document.contains("<script data-page=\"app\" type=\"application/json\">")
            && document.contains("data-server-rendered=\"true\""),
        "the first visit must be the SSR document: {document}"
    );
    response
}

#[tokio::test]
async fn intt_assert_inertia_reads_an_ssr_first_visit() {
    let _container = TestContainer::fake();
    let response = ssr_first_visit().await;

    response
        .assert_inertia()
        .component("Dashboard")
        .url("/dashboard")
        .where_("count", 3);
}

#[tokio::test]
async fn intt_from_response_reads_an_ssr_first_visit() {
    let _container = TestContainer::fake();
    let response = ssr_first_visit().await;

    AssertableInertia::from_response(&HttpResponse::html(response.body_text()))
        .component("Dashboard")
        .url("/dashboard")
        .where_("count", 3);
}

#[tokio::test]
async fn intt_assert_inertia_with_runs_the_callback_and_returns_the_response() {
    let client = TestClient::new(dashboard_routes(), MiddlewareRegistry::new());
    let mut seen = None;

    client
        .get("/dashboard")
        .inertia()
        .send()
        .await
        .assert_inertia_with(|page| {
            page.component("Dashboard");
            seen = Some(page.prop("count"));
        })
        .assert_ok()
        .assert_header("x-inertia", "true");

    assert_eq!(seen, Some(json!(3)), "the callback must have run");
}

// ── PAR-066: reloads through the client ─────────────────────────────

/// The partial-reload headers of one request: `X-Inertia-Partial-Component`
/// and `X-Inertia-Partial-Data`, each `None` when not sent.
type PartialHeaders = (Option<String>, Option<String>);

/// What the server saw on each request to `/users`.
#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<PartialHeaders>>>);

impl Seen {
    fn last(&self) -> PartialHeaders {
        self.0
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a request reached /users")
    }
}

fn users_routes(seen: Seen) -> Router {
    Router::new()
        .get("/users", move |req: Request| {
            let seen = seen.clone();
            async move {
                seen.0.lock().unwrap().push((
                    req.header("X-Inertia-Partial-Component")
                        .map(str::to_string),
                    req.header("X-Inertia-Partial-Data").map(str::to_string),
                ));
                InertiaResponse::new("Users/Index")
                    .with("users", json!([{"id": 1, "name": "Ada"}]))
                    .with("filters", json!({"q": ""}))
                    .defer_with(
                        "stats",
                        suprnova::DeferOptions::default().group("stats"),
                        || async { Ok::<_, FrameworkError>(json!({"total": 1})) },
                    )
                    .defer("permissions", || async {
                        Ok::<_, FrameworkError>(json!(["read"]))
                    })
                    .resolve(&req)
                    .await
                    .map_err(HttpResponse::from)
            }
        })
        .into()
}

async fn users_page(seen: &Seen) -> suprnova::testing::AssertableInertia {
    let client = TestClient::new(users_routes(seen.clone()), MiddlewareRegistry::new());
    client.get("/users").inertia().send().await.assert_inertia()
}

#[tokio::test]
async fn intt_a_client_response_reloads_with_nothing_attached() {
    let seen = Seen::default();
    let page = users_page(&seen).await;

    let reloaded = page.reload_only(["users"]).await;

    reloaded.has("users").missing("filters");
    assert_eq!(
        seen.last(),
        (Some("Users/Index".to_string()), Some("users".to_string()))
    );
    // The reloaded page carries the client forward.
    reloaded.reload_except(["filters"]).await.has("users");
}

#[tokio::test]
async fn intt_load_deferred_props_of_requests_only_the_named_groups() {
    let seen = Seen::default();
    let page = users_page(&seen).await;
    page.missing("stats").missing("permissions");

    let reloaded = page.load_deferred_props_of(["stats"]).await;

    assert_eq!(seen.last().1.as_deref(), Some("stats"));
    reloaded.where_("stats.total", 1).missing("permissions");
}

#[tokio::test]
async fn intt_load_deferred_props_of_no_group_requests_every_group() {
    let seen = Seen::default();
    let page = users_page(&seen).await;

    let reloaded = page.load_deferred_props_of(Vec::<String>::new()).await;

    let mut requested: Vec<String> = seen
        .last()
        .1
        .expect("a partial reload")
        .split(',')
        .map(str::to_string)
        .collect();
    requested.sort();
    assert_eq!(requested, ["permissions", "stats"]);
    reloaded.has("stats").has("permissions");
}

#[tokio::test]
async fn intt_load_deferred_props_of_an_unknown_group_fails_naming_it() {
    let seen = Seen::default();
    let page = users_page(&seen).await;

    let failure = tokio::spawn(async move {
        page.load_deferred_props_of(["nope"]).await;
    })
    .await
    .expect_err("an unknown group must fail");
    let message = failure.into_panic();
    let message = message
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert!(message.contains("\"nope\""), "{message}");
}

#[tokio::test]
async fn intt_load_deferred_props_with_runs_its_callback() {
    let seen = Seen::default();
    let page = users_page(&seen).await;
    let mut ran = false;

    page.load_deferred_props_with(["default"], |reloaded| {
        reloaded.where_("permissions", json!(["read"]));
        ran = true;
    })
    .await;

    assert!(ran);
    assert_eq!(seen.last().1.as_deref(), Some("permissions"));
}

#[tokio::test]
async fn intt_a_full_reload_sends_no_partial_header() {
    let seen = Seen::default();
    let page = users_page(&seen).await;

    let reloaded = page.reload().await;

    assert_eq!(
        seen.last(),
        (None, None),
        "a full reload is not a partial one"
    );
    reloaded
        .component("Users/Index")
        .has("users")
        .has("filters");
}

#[tokio::test]
async fn intt_reload_only_with_and_the_other_callbacks_run() {
    let seen = Seen::default();
    let page = users_page(&seen).await;
    let mut ran = Vec::new();

    page.reload_only_with(["users"], |reloaded| {
        reloaded.has("users");
        ran.push("only");
    })
    .await;
    page.reload_except_with(["users"], |reloaded| {
        reloaded.missing("users").has("filters");
        ran.push("except");
    })
    .await;
    page.reload_with(|reloaded| {
        reloaded.has("filters");
        ran.push("full");
    })
    .await;

    assert_eq!(ran, ["only", "except", "full"]);
}

// ── PAR-063, PAR-066: reloads under a public path prefix ────────────

/// One request to `/users`: its target (path and query) and its
/// `X-Forwarded-Prefix` header.
type Target = (String, Option<String>);

/// What `/users` saw on each request.
#[derive(Clone, Default)]
struct Targets(Arc<Mutex<Vec<Target>>>);

impl Targets {
    fn all(&self) -> Vec<Target> {
        self.0.lock().unwrap().clone()
    }
}

/// `/users`, a page with a deferred group, recording every request.
fn recording_users_routes(targets: Targets) -> Router {
    Router::new()
        .get("/users", move |req: Request| {
            let targets = targets.clone();
            async move {
                targets.0.lock().unwrap().push((
                    req.uri().to_string(),
                    req.header("X-Forwarded-Prefix").map(str::to_string),
                ));
                InertiaResponse::new("Users/Index")
                    .with("users", json!([{"id": 1, "name": "Ada"}]))
                    .with("filters", json!({"q": ""}))
                    .defer_with(
                        "stats",
                        suprnova::DeferOptions::default().group("stats"),
                        || async { Ok::<_, FrameworkError>(json!({"total": 1})) },
                    )
                    .resolve(&req)
                    .await
                    .map_err(HttpResponse::from)
            }
        })
        .into()
}

#[tokio::test]
async fn intt_a_page_under_an_app_url_path_reloads_through_the_internal_path() {
    // The `AppConfig` is process-wide: the test runs alone in a child.
    if crate::own_process_async::delegate(
        module_path!(),
        "intt_a_page_under_an_app_url_path_reloads_through_the_internal_path",
    )
    .await
    {
        return;
    }
    Config::register(
        AppConfig::builder()
            .environment(Environment::Testing)
            .debug(false)
            .url("https://example.test/billing")
            .build(),
    );
    let targets = Targets::default();
    let client = TestClient::new(
        recording_users_routes(targets.clone()),
        MiddlewareRegistry::new(),
    );

    let page = client
        .get("/users?page=2")
        .inertia()
        .send()
        .await
        .assert_inertia();
    // The page's url is the public one; the router matched `/users`.
    page.component("Users/Index").url("/billing/users?page=2");

    page.reload().await.has("users").has("filters");
    page.reload_only(["users"]).await.missing("filters");
    page.load_deferred_props()
        .await
        .where_("stats.total", 1)
        .missing("users");
    // A reloaded page reloads again the same way.
    page.reload().await.reload_except(["filters"]).await;

    let sent: Vec<String> = targets
        .all()
        .into_iter()
        .map(|(target, _)| target)
        .collect();
    assert_eq!(
        sent,
        vec!["/users?page=2"; 6],
        "every reload must reach the path the first visit did, query kept"
    );
}

#[tokio::test]
async fn intt_a_reload_sends_the_forwarded_prefix_the_page_request_sent() {
    let targets = Targets::default();
    let client = TestClient::new(
        recording_users_routes(targets.clone()),
        MiddlewareRegistry::new(),
    );

    let page = client
        .get("/users")
        .header("X-Forwarded-Prefix", "/billing")
        .inertia()
        .send()
        .await
        .assert_inertia();
    page.reload_only(["users"]).await;
    let plain = client.get("/users").inertia().send().await.assert_inertia();
    plain.reload().await;

    assert_eq!(
        targets.all(),
        vec![
            ("/users".to_string(), Some("/billing".to_string())),
            ("/users".to_string(), Some("/billing".to_string())),
            ("/users".to_string(), None),
            ("/users".to_string(), None),
        ],
        "a reload sends the X-Forwarded-Prefix its page's request sent, and only that"
    );
}

// ── PAR-067: page readers and the Inertia flash in the session ──────

#[tokio::test]
async fn intt_inertia_page_and_inertia_props_read_the_page() {
    let client = TestClient::new(dashboard_routes(), MiddlewareRegistry::new());
    let response = client.get("/dashboard").inertia().send().await;

    let page = response.inertia_page();
    assert_eq!(page["component"], "Dashboard");
    assert_eq!(page["url"], "/dashboard");
    assert_eq!(response.inertia_props(None), page["props"]);
    assert!(response.inertia_props(None).is_object());
    assert_eq!(response.inertia_props(Some("count")), json!(3));
    assert_eq!(response.inertia_props(Some("absent")), Value::Null);
}

#[tokio::test]
async fn intt_inertia_props_reads_a_nested_path() {
    let router = Router::new().get("/user", |req: Request| async move {
        InertiaResponse::new("User")
            .with("user", json!({"name": "Ada"}))
            .resolve(&req)
            .await
            .map_err(HttpResponse::from)
    });
    let client = TestClient::new(router, MiddlewareRegistry::new());

    let response = client.get("/user").send().await;

    assert_eq!(response.inertia_props(Some("user.name")), json!("Ada"));
}

fn flash_routes() -> Router {
    Router::new()
        .post("/save", |_req: Request| async {
            Inertia::flash("toast", "Saved").map_err(HttpResponse::from)?;
            let response: Response = suprnova::Redirect::to("/page").into();
            response
        })
        .get("/page", |req: Request| async move {
            InertiaResponse::new("Page")
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .into()
}

/// The panic message of `future`, or a failure of the test when it passes.
async fn async_failure_of(future: impl std::future::Future<Output = ()>) -> String {
    use futures::FutureExt;
    let payload = std::panic::AssertUnwindSafe(future)
        .catch_unwind()
        .await
        .expect_err("the assertion was expected to fail");
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}

#[tokio::test]
async fn intt_assert_inertia_flash_reads_the_flash_a_redirect_left() {
    let (client, _store) = session_client(flash_routes());

    let saved = client.post("/save").send().await;

    saved.assert_redirect(Some("/page"));
    saved
        .assert_inertia_flash("toast", Some("Saved"))
        .await
        .assert_inertia_flash("toast", None::<Value>)
        .await
        .assert_inertia_flash_missing("other")
        .await;

    let wrong = async_failure_of(async {
        saved.assert_inertia_flash("toast", Some("Deleted")).await;
    })
    .await;
    assert!(wrong.contains("toast"), "{wrong}");
    let present = async_failure_of(async {
        saved.assert_inertia_flash_missing("toast").await;
    })
    .await;
    assert!(present.contains("toast"), "{present}");
    let absent = async_failure_of(async {
        saved.assert_inertia_flash("other", None::<Value>).await;
    })
    .await;
    assert!(absent.contains("other"), "{absent}");
}

#[tokio::test]
async fn intt_the_page_after_the_redirect_shows_and_pulls_the_flash() {
    let (client, _store) = session_client(flash_routes());
    client
        .post("/save")
        .send()
        .await
        .assert_redirect(Some("/page"));

    let page = client.get("/page").inertia().send().await;

    page.assert_inertia().has_flash("toast", Some("Saved"));
    page.assert_inertia_flash_missing("toast").await;
}

#[tokio::test]
async fn intt_assert_inertia_flash_needs_a_session_store() {
    suprnova::testing::install_test_encryption_key();
    let client = TestClient::new(flash_routes(), MiddlewareRegistry::new());
    let response = client.get("/page").send().await;

    let failure = async_failure_of(async {
        response.assert_inertia_flash("toast", None::<Value>).await;
    })
    .await;
    assert!(failure.contains("with_session_store"), "{failure}");
}

#[tokio::test]
async fn intt_preserved_big_integers_reach_the_assertions_as_integers() {
    let router = Router::new().get("/big", |req: Request| async move {
        InertiaResponse::new("Big")
            .with("id", 9007199254740993_i64)
            .preserve_big_integers(true)
            .resolve(&req)
            .await
            .map_err(HttpResponse::from)
    });
    let client = TestClient::new(router, MiddlewareRegistry::new());

    let response = client.get("/big").inertia().send().await;

    assert!(
        response.body_text().contains("$bigint"),
        "the wire carries the marker: {}",
        response.body_text()
    );
    response.assert_inertia().where_("id", 9007199254740993_i64);
    assert_eq!(
        response.inertia_props(Some("id")),
        json!(9007199254740993_i64)
    );
}
