//! The application every agents test serves: its scopes, its tools and the
//! routes they run on.
//!
//! Every `#[tool]` of this binary is linked into every test, and a router
//! must route each of them (`Router::prepare_tools`), so all tools live here
//! and every test builds its router with [`router`] or [`router_with`].

use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use suprnova::agents::AgentsConfig;
use suprnova::auth::Authenticatable;
use suprnova::cache::{CacheStore, InMemoryCache};
use suprnova::container::testing::TestContainer;
use suprnova::rate_limit::ThrottleRequestsMiddleware;
use suprnova::{
    Gate, HttpResponse, JsonSchema, Middleware, MiddlewareRegistry, Next, Request, Response,
    Router, ToolCaller, handler, json_response, tool, tool_input,
};

suprnova::tool_scopes! {
    /// The fixture's scopes.
    pub mod scope {
        POSTS_READ = "posts:read" => "Read your posts";
        POSTS_WRITE = "posts:write" => "Create and edit your posts";
        POSTS_DELETE = "posts:delete" => "Delete your posts";
    }
}

/// A signed-in user of the fixture.
#[derive(Clone, Debug)]
pub struct TestUser {
    pub id: String,
}

impl TestUser {
    pub fn new(id: &str) -> Arc<dyn Authenticatable> {
        Arc::new(Self { id: id.to_string() })
    }
}

impl Authenticatable for TestUser {
    fn get_auth_identifier(&self) -> String {
        self.id.clone()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// The type the delete policy is keyed by.
#[derive(Default)]
pub struct Post;

/// The input of `posts.create`.
#[tool_input]
pub struct CreatePost {
    /// The post's title.
    #[validate(length(min = 1, max = 200))]
    pub title: String,
    /// The post's text.
    #[validate(length(max = 10000))]
    #[serde(rename = "content")]
    pub body: Option<String>,
    /// How many stars, 1 to 5.
    #[validate(range(min = 1, max = 5))]
    #[serde(default)]
    pub stars: Option<u8>,
}

/// What `posts.create` answers.
#[derive(Serialize, JsonSchema)]
#[schemars(crate = "suprnova::__schemars")]
pub struct PostView {
    pub id: u64,
    pub title: String,
}

/// The input of the tools on `/posts/{post}`.
#[tool_input]
pub struct PostRef {
    /// The post.
    pub post: String,
}

/// The input of `echo`.
#[tool_input]
pub struct Echo {
    /// What to echo.
    #[validate(length(min = 1))]
    pub text: String,
}

/// The input of `slow.wait`.
#[tool_input]
pub struct Wait {
    /// How long to wait, in milliseconds.
    pub millis: u64,
    /// A key the test reads the call's cancellation by.
    pub key: String,
}

/// Calls a counting middleware saw, by `User-Agent`, which a tool call
/// carries over from the MCP request.
pub fn seen(key: &str) -> usize {
    counts()
        .lock()
        .map(|counts| counts.get(key).copied().unwrap_or(0))
        .unwrap_or(0)
}

fn counts() -> &'static Mutex<HashMap<String, usize>> {
    static COUNTS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Counts every request it sees, by `User-Agent`.
pub struct Counting;

#[async_trait]
impl Middleware for Counting {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let key = request.header("User-Agent").unwrap_or("").to_string();
        if let Ok(mut counts) = counts().lock() {
            *counts.entry(key).or_default() += 1;
        }
        next(request).await
    }
}

/// The slow calls dropped before they finished, by key.
pub fn cancelled(key: &str) -> bool {
    cancellations()
        .lock()
        .map(|done| done.contains_key(key))
        .unwrap_or(false)
}

fn cancellations() -> &'static Mutex<HashMap<String, ()>> {
    static DONE: OnceLock<Mutex<HashMap<String, ()>>> = OnceLock::new();
    DONE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Records its key as cancelled when dropped before it is disarmed.
struct DropFlag {
    key: String,
    armed: bool,
}

impl Drop for DropFlag {
    fn drop(&mut self) {
        if self.armed
            && let Ok(mut done) = cancellations().lock()
        {
            done.insert(self.key.clone(), ());
        }
    }
}

/// How many times `slow.wait` began.
pub static SLOW_STARTED: AtomicUsize = AtomicUsize::new(0);

// ---------------------------------------------------------------- tools ----

#[tool(
    name = "echo",
    description = "Echo the text back.",
    callers = [public],
    input = Echo,
    annotations(read_only),
)]
#[handler]
pub async fn echo(caller: ToolCaller, form: Echo) -> Response {
    json_response!({ "text": form.text, "kind": caller.kind().as_str() })
}

#[tool(
    name = "posts.create",
    description = "Create a draft post for the signed-in user.",
    title = "Create a post",
    callers = [user, agent],
    scopes = [scope::POSTS_WRITE],
    input = CreatePost,
    output = PostView,
    annotations(read_only = false, destructive = false),
)]
#[handler]
pub async fn create_post(caller: ToolCaller, form: CreatePost) -> Response {
    json_response!({
        "id": 1,
        "title": form.title,
        "kind": caller.kind().as_str(),
        "browser": caller.via_browser(),
        "user": caller.user_id(),
        "client": caller.client_id(),
        "registration": caller.registration().map(|registration| registration.id().to_string()),
        "claimed": caller.registration().map(|registration| registration.is_claimed()),
        "auth_user": suprnova::Auth::id(),
    })
}

#[tool(
    name = "posts.show",
    description = "Show one post.",
    callers = [public, user, agent],
    scopes = [scope::POSTS_READ],
    input = PostRef,
    annotations(read_only),
)]
#[handler]
pub async fn show_post(post: String, form: PostRef) -> Response {
    // The path value and the argument it was filled from agree.
    json_response!({ "post": post, "same": post == form.post })
}

#[tool(
    name = "posts.delete",
    description = "Delete one post.",
    callers = [user, agent],
    scopes = [scope::POSTS_READ, scope::POSTS_DELETE],
    input = PostRef,
    annotations(destructive),
)]
#[handler]
#[suprnova::authorize("posts.delete", Post)]
pub async fn delete_post(post: String) -> Response {
    json_response!({ "deleted": post })
}

#[tool(
    name = "users.profile",
    description = "Read the signed-in user's profile.",
    callers = [user],
)]
#[handler]
pub async fn profile(caller: ToolCaller) -> Response {
    json_response!({ "user": caller.user_id() })
}

#[tool(
    name = "slow.wait",
    description = "Wait, then answer.",
    callers = [public],
    input = Wait,
)]
#[handler]
pub async fn slow(form: Wait) -> Response {
    SLOW_STARTED.fetch_add(1, Ordering::SeqCst);
    let mut flag = DropFlag {
        key: form.key.clone(),
        armed: true,
    };
    tokio::time::sleep(Duration::from_millis(form.millis)).await;
    flag.armed = false;
    json_response!({ "waited": form.millis })
}

#[tool(
    name = "request.peek",
    description = "Report what the tool's request carried.",
    callers = [public, user],
)]
#[handler]
pub async fn peek(request: Request) -> Response {
    json_response!({
        "authorization": request.header("Authorization").is_some(),
        "cookie": request.header("Cookie").is_some(),
        "ip": request.ip(),
        "user_agent": request.header("User-Agent"),
    })
}

#[tool(
    name = "throttled.ping",
    description = "Answer pong, at most twice a minute.",
    callers = [public],
)]
#[handler]
pub async fn throttled() -> Response {
    json_response!({ "pong": true })
}

#[tool(
    name = "page.html",
    description = "Answer with a page rather than data.",
    callers = [public],
)]
#[handler]
pub async fn page() -> Response {
    let render = PAGE_RENDERS.fetch_add(1, Ordering::SeqCst) + 1;
    Ok(HttpResponse::html(format!("<p>hello {render}</p>")))
}

/// How many times `page.html` rendered.
pub static PAGE_RENDERS: AtomicUsize = AtomicUsize::new(0);

/// A routed handler that is not a tool.
#[handler]
pub async fn plain() -> Response {
    json_response!({ "plain": true })
}

/// Lets users delete posts and forbids agents to, reading the caller.
pub fn define_policies() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        Gate::define::<TestUser, Post>("posts.delete", |_user, _post| {
            !ToolCaller::current().is_some_and(|caller| caller.is_agent())
        });
    });
}

/// Every tool's route, the counting middleware on `posts.create`, a route
/// throttle on `throttled.ping`, and a literal `/posts/new` beside
/// `/posts/{post}`.
pub fn routes() -> Router {
    Router::new()
        .post("/echo", echo)
        .post("/posts", create_post)
        .middleware(Counting)
        .get("/posts/new", |_request: Request| async {
            json_response!({ "new": true })
        })
        .get("/posts/{post}", show_post)
        .delete("/posts/{post}", delete_post)
        .post("/me", profile)
        .post("/slow", slow)
        .post("/peek", peek)
        .post("/throttled", throttled)
        .middleware(ThrottleRequestsMiddleware::with(2, 1, "agents-fixture"))
        .get("/page", page)
        .post("/plain", plain)
        .into()
}

/// [`routes`] serving agents with `config`.
pub fn router_with(config: AgentsConfig) -> Router {
    define_policies();
    routes().agents(config)
}

/// [`routes`] serving agents with the default configuration.
pub fn router() -> Router {
    router_with(AgentsConfig::new())
}

/// A test client of [`router_with`] behind `registry`.
pub fn client_with(
    config: AgentsConfig,
    registry: MiddlewareRegistry,
) -> suprnova::testing::TestClient {
    suprnova::testing::TestClient::new(router_with(config), registry)
}

/// A test client of [`router`] with no global middleware.
pub fn client() -> suprnova::testing::TestClient {
    client_with(AgentsConfig::new(), MiddlewareRegistry::new())
}

/// The per-test container with an in-memory cache, which the call limits
/// and the route throttle count in. Keep the guard for the whole test.
pub fn setup() -> impl Drop {
    let guard = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
    guard
}
