//! `TestClient` (PAR-063): requests driven through the framework's request
//! path over an in-memory connection, with the session cookie carried from
//! one request to the next and the error report and session store kept on
//! every `TestResponse` it returns.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::testing::{TestClient, TestContainer};
use suprnova::{
    Cookie, FrameworkError, HttpResponse, Inertia, InertiaConfig, InertiaResponse, Middleware,
    MiddlewareRegistry, Next, Request, Response, Router,
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
