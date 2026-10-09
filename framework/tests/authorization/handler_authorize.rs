//! `#[authorize]` on a `#[handler]`: Laravel's `#[Authorize]` controller
//! attribute and the `can` middleware it applies.
//!
//! Every case drives a real request through `handle_request` over a loopback
//! socket, against a SQLite table the handlers bind their models from. The
//! cases cover each clause of the requirement: the check runs after route
//! model binding and before the handler body and the request body, it goes
//! through the async gate (so `#[policy]` methods, async gates and async
//! before-hooks all answer), and it answers 401 for a guest, 403 for a
//! denial, and 404 for a policy that denies as not found or a bound model
//! that does not exist. On a route behind `AuthMiddleware::for_guard`, the
//! check asks that guard for the user, never the default guard.

use std::any::Any;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Once};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serial_test::serial;

use suprnova::authorization::init_policies;
use suprnova::http::text;
use suprnova::testing::{TestContainer, TestDatabase};
use suprnova::{
    Auth, AuthConfig, AuthManager, AuthMiddleware, Authenticatable, FrameworkError, Gate,
    GateResponse, GuardConfig, Middleware, MiddlewareRegistry, Model, Next, Request, Response,
    RouteParam, Router, UserProvider, attrs, authorize, handle_request, handler, model, policy,
    request,
};

// ── Users ────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct HaUser {
    id: i64,
    can_create: bool,
}

impl Authenticatable for HaUser {
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

/// A second user type with no gates of its own: the check must dispatch on
/// the concrete type the guard resolved, and fail closed for this one.
#[derive(Clone)]
struct HaRobot;

impl Authenticatable for HaRobot {
    fn get_auth_identifier(&self) -> String {
        "robot".to_string()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// The author of every seeded post. May create posts.
const AUTHOR: &str = "author";
/// Authored nothing and may not create posts.
const STRANGER: &str = "stranger";
/// A user of a type no gate knows.
const ROBOT: &str = "robot";

/// The user one of the names above stands for.
fn user_named(name: &str) -> Option<Arc<dyn Authenticatable>> {
    match name {
        AUTHOR => Some(Arc::new(HaUser {
            id: 1,
            can_create: true,
        })),
        STRANGER => Some(Arc::new(HaUser {
            id: 2,
            can_create: false,
        })),
        ROBOT => Some(Arc::new(HaRobot)),
        _ => None,
    }
}

/// Logs the request in on the default guard as the user the `X-Test-User`
/// header names, or leaves it a guest.
struct LoginAs;

#[async_trait::async_trait]
impl Middleware for LoginAs {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if let Some(user) = request.header("X-Test-User").and_then(user_named) {
            Auth::set_user(user);
        }
        next(request).await
    }
}

/// A guard other than the default. Its resolver finds the user the
/// `X-Partner` header names.
const PARTNER: &str = "partner";

/// The provider the guards are declared with. The guards under test never
/// ask it for a user.
struct NoUsers;

#[async_trait::async_trait]
impl UserProvider for NoUsers {
    async fn retrieve_by_id(
        &self,
        _id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(None)
    }
}

/// Installs a container-scoped manager: the default `web` session guard,
/// which `LoginAs` signs in, and the `partner` request guard.
fn install_partner_guard() {
    let driver = AuthManager::via_request_driver(PARTNER);
    let config = AuthConfig::new("web").guard(PARTNER, GuardConfig::custom(driver, "users"));
    TestContainer::singleton(AuthManager::new(config));
    Auth::register_provider("users", Arc::new(NoUsers)).unwrap();
    Auth::via_request(PARTNER, |request| {
        let user = request.header("X-Partner").and_then(user_named);
        Box::pin(async move { Ok(user) })
    })
    .unwrap();
}

// ── Model, policy, gates ─────────────────────────────────────────────────────

#[model(table = "ha_posts", fillable = ["author_id", "title", "hidden"])]
pub struct HaPost {
    pub id: i64,
    pub author_id: i64,
    pub title: String,
    pub hidden: bool,
}

// The raw `ha_post::Model` binding resolves through `EntityExt`.
impl suprnova::database::EntityExt for ha_post::Entity {}

/// Registers `view-ha-post` (rich) and `create-ha-post` (bool).
pub struct HaPostPolicy;

#[policy(HaUser, HaPost)]
impl HaPostPolicy {
    fn view(user: &HaUser, post: &HaPost) -> GateResponse {
        if post.author_id == user.id || !post.hidden {
            GateResponse::allow()
        } else {
            GateResponse::deny_as_not_found()
        }
    }

    fn create(user: &HaUser, _post: &HaPost) -> bool {
        user.can_create
    }
}

fn register_gates() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        init_policies();
        // An async gate under the falsifier's own ability name.
        Gate::define_async_with::<HaUser, HaPost, _, _>("update", |user, post| {
            let owns = post.author_id == user.id;
            async move {
                if owns {
                    GateResponse::allow()
                } else {
                    GateResponse::deny()
                }
            }
        });
        // A gate on the raw SeaORM model the unscoped binding hands over.
        Gate::define::<HaUser, ha_post::Model>("delete", |user, post| user.id == post.author_id);
        // A synchronous gate that allows, which the async before-hook below
        // overrules. Only the async evaluation path sees that hook.
        Gate::define::<HaUser, HaPost>("archive", |_user, _post| true);
        // The shape of the RBAC gate bridge: an async before-hook answering
        // for an ability no gate defines. It also denies `archive`, so a
        // check that skipped async hooks would let `archive` through.
        Gate::before_async::<HaUser, _, _>(|_user, ability| {
            let ability = ability.to_owned();
            async move {
                match ability.as_str() {
                    "publish" => Some(true),
                    "archive" => Some(false),
                    _ => None,
                }
            }
        });
    });
}

// ── Handlers ─────────────────────────────────────────────────────────────────

#[request]
pub struct StoreHaPost {
    #[validate(length(min = 1))]
    pub title: String,
}

#[handler]
#[authorize("view-ha-post", post)]
pub async fn show(post: RouteParam<HaPost>) -> Response {
    text(format!("show {}", post.id))
}

#[handler]
#[authorize("update", post)]
pub async fn update(post: RouteParam<HaPost>) -> Response {
    post.into_inner()
        .update(attrs! { title: "changed" })
        .await?;
    text("updated")
}

#[handler]
#[authorize("create-ha-post", HaPost)]
pub async fn store(form: StoreHaPost) -> Response {
    text(format!("stored {}", form.title))
}

#[handler]
#[authorize("delete", post)]
pub async fn destroy(post: ha_post::Model) -> Response {
    text(format!("destroyed {}", post.id))
}

#[handler]
#[authorize("view-ha-post", post)]
#[authorize("publish", post)]
pub async fn publish(post: RouteParam<HaPost>) -> Response {
    text(format!("published {}", post.id))
}

// `#[authorize]` above `#[handler]` applies the same way as below it.
#[authorize("view-ha-post", post)]
#[authorize("archive", post)]
#[handler]
pub async fn archive(post: RouteParam<HaPost>) -> Response {
    text(format!("archived {}", post.id))
}

// The pattern form: `post` is the model inside the wrapper, and the route
// parameter is named after it.
#[handler]
#[authorize("view-ha-post", post)]
pub async fn show_destructured(RouteParam(post): RouteParam<HaPost>) -> Response {
    text(format!("destructured {}", post.id))
}

fn build_router() -> Router {
    Router::new()
        .get("/posts/{post}", show)
        .put("/posts/{post}", update)
        .post("/posts", store)
        .delete("/posts/{post}", destroy)
        .post("/posts/{post}/publish", publish)
        .post("/posts/{post}/archive", archive)
        .get("/destructured/{post}", show_destructured)
        .into()
}

// ── Harness ──────────────────────────────────────────────────────────────────

/// Seeds post 1 (visible) and post 2 (hidden), both authored by user 1, and
/// serves the router. The returned database must stay alive for the test.
async fn boot() -> (TestDatabase, SocketAddr) {
    register_gates();
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE ha_posts (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            author_id INTEGER NOT NULL, \
            title TEXT NOT NULL, \
            hidden BOOLEAN NOT NULL\
         )",
    )
    .await
    .unwrap();
    HaPost::create(attrs! { author_id: 1_i64, title: "open", hidden: false })
        .await
        .unwrap();
    HaPost::create(attrs! { author_id: 1_i64, title: "secret", hidden: true })
        .await
        .unwrap();

    let addr = serve(MiddlewareRegistry::new().append(LoginAs)).await;
    (db, addr)
}

/// Serves the router through `registry`. Inside a `TestContainer::scope`,
/// the server sees that scope's container.
async fn serve(registry: MiddlewareRegistry) -> SocketAddr {
    let router = Arc::new(build_router());
    let registry = Arc::new(registry);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");
    TestContainer::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let router = router.clone();
            let registry = registry.clone();
            TestContainer::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let registry = registry.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, registry, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), svc)
                    .await;
            });
        }
    });
    addr
}

/// Sends one request as `user` (`None` for a guest) with an optional JSON
/// body, and returns the status and body.
async fn send(
    addr: SocketAddr,
    method: &str,
    path: &str,
    user: Option<&str>,
    json: Option<&str>,
) -> (u16, String) {
    let headers: Vec<(&str, &str)> = user.map(|user| ("X-Test-User", user)).into_iter().collect();
    send_with(addr, method, path, &headers, json).await
}

/// Sends one request with `headers` and an optional JSON body, and returns
/// the status and body.
async fn send_with(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    json: Option<&str>,
) -> (u16, String) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let body = json.unwrap_or("").to_owned();
    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", body.len().to_string());
    if json.is_some() {
        builder = builder.header("Content-Type", "application/json");
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let req = builder.body(Full::new(Bytes::from(body))).unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_request timeout")
        .expect("hyper send_request");
    let status = resp.status().as_u16();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

async fn title_of(id: i64) -> String {
    HaPost::find(id).await.unwrap().unwrap().title
}

// ── Param form: the bound model ──────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn allowed_user_runs_the_handler_body() {
    let (_db, addr) = boot().await;

    let (status, body) = send(addr, "PUT", "/posts/1", Some(AUTHOR), None).await;

    assert_eq!((status, body.as_str()), (200, "updated"));
    assert_eq!(title_of(1).await, "changed");
}

#[tokio::test]
#[serial]
async fn denied_user_gets_403_and_the_body_never_runs() {
    let (_db, addr) = boot().await;

    let (status, body) = send(addr, "PUT", "/posts/1", Some(STRANGER), None).await;

    assert_eq!(status, 403, "body: {body}");
    assert!(
        body.contains("This action is unauthorized."),
        "body: {body}"
    );
    assert_eq!(title_of(1).await, "open", "the handler body must not run");
}

#[tokio::test]
#[serial]
async fn guest_gets_401_and_the_body_never_runs() {
    let (_db, addr) = boot().await;

    let (status, body) = send(addr, "PUT", "/posts/1", None, None).await;

    assert_eq!(status, 401, "body: {body}");
    assert!(body.contains("Unauthenticated."), "body: {body}");
    assert_eq!(title_of(1).await, "open", "the handler body must not run");
}

#[tokio::test]
#[serial]
async fn policy_deny_as_not_found_answers_404() {
    let (_db, addr) = boot().await;

    let (status, body) = send(addr, "GET", "/posts/2", Some(STRANGER), None).await;
    assert_eq!(status, 404, "body: {body}");

    // The same policy allows the author, and allows anyone a visible post.
    assert_eq!(
        send(addr, "GET", "/posts/2", Some(AUTHOR), None).await,
        (200, "show 2".to_string())
    );
    assert_eq!(
        send(addr, "GET", "/posts/1", Some(STRANGER), None).await,
        (200, "show 1".to_string())
    );
}

#[tokio::test]
#[serial]
async fn missing_bound_model_answers_404_before_the_check() {
    let (_db, addr) = boot().await;

    // A user the gate would deny, and a guest: binding answers first.
    let (status, body) = send(addr, "PUT", "/posts/999", Some(STRANGER), None).await;
    assert_eq!(status, 404, "denied user, missing model: {body}");
    let (status, body) = send(addr, "PUT", "/posts/999", None, None).await;
    assert_eq!(status, 404, "guest, missing model: {body}");
}

#[tokio::test]
#[serial]
async fn raw_model_binding_authorizes_the_bound_row() {
    let (_db, addr) = boot().await;

    assert_eq!(
        send(addr, "DELETE", "/posts/1", Some(AUTHOR), None).await,
        (200, "destroyed 1".to_string())
    );
    let (status, body) = send(addr, "DELETE", "/posts/1", Some(STRANGER), None).await;
    assert_eq!(status, 403, "body: {body}");
}

#[tokio::test]
#[serial]
async fn user_of_a_type_no_gate_knows_is_denied() {
    let (_db, addr) = boot().await;

    let (status, body) = send(addr, "GET", "/posts/1", Some(ROBOT), None).await;

    assert_eq!(status, 403, "body: {body}");
}

// ── Type form: the model type ────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn type_form_authorizes_against_the_model_type() {
    let (_db, addr) = boot().await;

    assert_eq!(
        send(
            addr,
            "POST",
            "/posts",
            Some(AUTHOR),
            Some(r#"{"title":"new"}"#)
        )
        .await,
        (200, "stored new".to_string())
    );
    let (status, body) = send(
        addr,
        "POST",
        "/posts",
        Some(STRANGER),
        Some(r#"{"title":"new"}"#),
    )
    .await;
    assert_eq!(status, 403, "body: {body}");
    let (status, body) = send(addr, "POST", "/posts", None, Some(r#"{"title":"new"}"#)).await;
    assert_eq!(status, 401, "body: {body}");
}

#[tokio::test]
#[serial]
async fn the_check_runs_before_the_request_body_is_validated() {
    let (_db, addr) = boot().await;

    // A denied user never learns what the form would reject.
    let (status, body) = send(addr, "POST", "/posts", Some(STRANGER), Some("{}")).await;
    assert_eq!(status, 403, "body: {body}");
    let (status, body) = send(addr, "POST", "/posts", None, Some("{}")).await;
    assert_eq!(status, 401, "body: {body}");

    // An allowed user reaches validation.
    let (status, body) = send(addr, "POST", "/posts", Some(AUTHOR), Some("{}")).await;
    assert_eq!(status, 422, "body: {body}");
}

// ── The async gate, and several attributes ───────────────────────────────────

#[tokio::test]
#[serial]
async fn async_before_hook_answers_like_the_rbac_bridge() {
    let (_db, addr) = boot().await;

    // No gate defines `publish`; the async before-hook allows it.
    assert_eq!(
        send(addr, "POST", "/posts/1/publish", Some(STRANGER), None).await,
        (200, "published 1".to_string())
    );
}

#[tokio::test]
#[serial]
async fn async_before_hook_denial_is_enforced() {
    let (_db, addr) = boot().await;

    // The sync gate allows `archive`; only the async hook denies it.
    let (status, body) = send(addr, "POST", "/posts/1/archive", Some(AUTHOR), None).await;

    assert_eq!(status, 403, "body: {body}");
}

#[tokio::test]
#[serial]
async fn every_attribute_applies_in_order() {
    let (_db, addr) = boot().await;

    // The first attribute (`view-ha-post`) hides post 2 from a stranger,
    // although the second (`publish`) would allow it.
    let (status, body) = send(addr, "POST", "/posts/2/publish", Some(STRANGER), None).await;
    assert_eq!(status, 404, "body: {body}");

    // The first allows the author; the second (`archive`) denies.
    let (status, body) = send(addr, "POST", "/posts/2/archive", Some(AUTHOR), None).await;
    assert_eq!(status, 403, "body: {body}");

    // And a guest stops at the first.
    let (status, body) = send(addr, "POST", "/posts/1/publish", None, None).await;
    assert_eq!(status, 401, "body: {body}");
}

#[tokio::test]
#[serial]
async fn a_destructured_route_param_is_checked_as_the_bound_model() {
    let (_db, addr) = boot().await;

    assert_eq!(
        send(addr, "GET", "/destructured/2", Some(AUTHOR), None).await,
        (200, "destructured 2".to_string())
    );
    let (status, body) = send(addr, "GET", "/destructured/2", Some(STRANGER), None).await;
    assert_eq!(status, 404, "deny as not found: {body}");
    let (status, body) = send(addr, "GET", "/destructured/1", None, None).await;
    assert_eq!(status, 401, "guest: {body}");
    let (status, body) = send(addr, "GET", "/destructured/999", Some(AUTHOR), None).await;
    assert_eq!(status, 404, "missing model: {body}");
}

// ── The guard of the route ───────────────────────────────────────────────────

/// Serves the router behind `LoginAs`, then `auth`, with the `partner` guard
/// installed. Call it inside a `TestContainer::scope`.
async fn boot_guarded(auth: AuthMiddleware) -> SocketAddr {
    register_gates();
    install_partner_guard();
    serve(MiddlewareRegistry::new().append(LoginAs).append(auth)).await
}

/// A `POST /posts` (the type form, `create-ha-post`) with the default-guard
/// user `web` and the partner-guard user `partner`, each `None` for nobody.
async fn store_as(addr: SocketAddr, web: Option<&str>, partner: Option<&str>) -> (u16, String) {
    let mut headers = Vec::new();
    if let Some(web) = web {
        headers.push(("X-Test-User", web));
    }
    if let Some(partner) = partner {
        headers.push(("X-Partner", partner));
    }
    send_with(addr, "POST", "/posts", &headers, Some(r#"{"title":"new"}"#)).await
}

#[tokio::test]
#[serial]
async fn the_policy_decides_for_the_user_of_the_route_guard() {
    TestContainer::scope(async {
        let addr = boot_guarded(AuthMiddleware::new().for_guard(PARTNER)).await;

        assert_eq!(
            store_as(addr, None, Some(AUTHOR)).await,
            (200, "stored new".to_string())
        );
        let (status, body) = store_as(addr, None, Some(STRANGER)).await;
        assert_eq!(status, 403, "body: {body}");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn the_route_guard_user_is_checked_not_the_default_guard_user() {
    TestContainer::scope(async {
        let addr = boot_guarded(AuthMiddleware::new().for_guard(PARTNER)).await;

        // The default guard's user may create posts; the route's may not.
        let (status, body) = store_as(addr, Some(AUTHOR), Some(STRANGER)).await;
        assert_eq!(status, 403, "body: {body}");
        // And the other way round.
        assert_eq!(
            store_as(addr, Some(STRANGER), Some(AUTHOR)).await,
            (200, "stored new".to_string())
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_user_only_on_the_default_guard_gets_401_on_a_route_guard() {
    TestContainer::scope(async {
        // The middleware answers first when it requires the guard.
        let addr = boot_guarded(AuthMiddleware::new().for_guard(PARTNER)).await;
        let (status, body) = store_as(addr, Some(AUTHOR), None).await;
        assert_eq!(status, 401, "required guard: {body}");

        // An optional check lets the request through, and the check finds no
        // user on the route's guard. The default guard's user never stands in.
        let addr = boot_guarded(AuthMiddleware::optional().for_guard(PARTNER)).await;
        let (status, body) = store_as(addr, Some(AUTHOR), None).await;
        assert_eq!(status, 401, "optional guard: {body}");
        assert!(body.contains("Unauthenticated."), "body: {body}");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn without_a_route_guard_the_default_guard_user_is_checked() {
    TestContainer::scope(async {
        let addr = boot_guarded(AuthMiddleware::new()).await;

        assert_eq!(
            store_as(addr, Some(AUTHOR), Some(STRANGER)).await,
            (200, "stored new".to_string())
        );
        let (status, body) = store_as(addr, Some(STRANGER), Some(AUTHOR)).await;
        assert_eq!(status, 403, "body: {body}");
    })
    .await;
}
