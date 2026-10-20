//! The application every agents test serves: its scopes, its tools and the
//! routes they run on.
//!
//! Every `#[tool]` of this binary is linked into every test. A router that
//! leaves one unrouted serves without it and warns at boot, so the shared
//! router, [`router`] or [`router_with`], routes them all.

use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use suprnova::agents::AgentsConfig;
use suprnova::auth::Authenticatable;
use suprnova::cache::{CacheStore, InMemoryCache};
use suprnova::container::testing::TestContainer;
use suprnova::rate_limit::ThrottleRequestsMiddleware;
use suprnova::{
    Gate, HttpResponse, Middleware, MiddlewareRegistry, Next, Request, Response, Router,
    ToolCaller, Validate, handler, json_response, tool, tool_input, tool_output, tool_schema,
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
    /// The user `id`, as the guards take a user.
    pub fn arc(id: &str) -> Arc<dyn Authenticatable> {
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

/// The input of `create_post`.
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

/// What `create_post` answers.
#[tool_output]
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

/// The input of `wait_slowly`.
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

/// How many times `wait_slowly` began.
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
    name = "create_post",
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
    name = "show_post",
    description = "Show one post.",
    callers = [user, agent],
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
    name = "delete_post",
    description = "Delete one post.",
    callers = [user, agent],
    scopes = [scope::POSTS_READ, scope::POSTS_DELETE],
    input = PostRef,
    annotations(destructive),
)]
#[handler]
#[suprnova::authorize("delete_post", Post)]
pub async fn delete_post(post: String, form: PostRef) -> Response {
    json_response!({ "deleted": post, "same": post == form.post })
}

#[tool(
    name = "read_user_profile",
    description = "Read the signed-in user's profile.",
    callers = [user],
)]
#[handler]
pub async fn profile(caller: ToolCaller) -> Response {
    json_response!({ "user": caller.user_id(), "browser": caller.via_browser() })
}

#[tool(
    name = "wait_slowly",
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
    name = "peek_request",
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
    name = "ping_throttled",
    description = "Answer pong, at most twice a minute.",
    callers = [public],
)]
#[handler]
pub async fn throttled() -> Response {
    json_response!({ "pong": true })
}

#[tool(
    name = "read_html_page",
    description = "Answer with a page rather than data.",
    callers = [public],
)]
#[handler]
pub async fn page() -> Response {
    let render = PAGE_RENDERS.fetch_add(1, Ordering::SeqCst) + 1;
    Ok(HttpResponse::html(format!("<p>hello {render}</p>")))
}

/// How many times `read_html_page` rendered.
pub static PAGE_RENDERS: AtomicUsize = AtomicUsize::new(0);

#[tool(
    name = "read_slow_stream",
    description = "Answer with an event that arrives late.",
    callers = [public],
    input = Wait,
)]
#[handler]
pub async fn slow_stream(form: Wait) -> Response {
    let flag = DropFlag {
        key: form.key.clone(),
        armed: true,
    };
    let millis = form.millis;
    Ok(HttpResponse::sse(futures::stream::once(async move {
        let mut flag = flag;
        tokio::time::sleep(Duration::from_millis(millis)).await;
        flag.armed = false;
        suprnova::sse::SseEvent::data("late")
    })))
}

#[tool(
    name = "read_slow_bytes",
    description = "Answer with a body that arrives late.",
    callers = [public],
    input = Wait,
)]
#[handler]
pub async fn slow_bytes(form: Wait) -> Response {
    let flag = DropFlag {
        key: form.key.clone(),
        armed: true,
    };
    let millis = form.millis;
    Ok(
        HttpResponse::stream_bytes(futures::stream::once(async move {
            let mut flag = flag;
            tokio::time::sleep(Duration::from_millis(millis)).await;
            flag.armed = false;
            Ok::<_, std::convert::Infallible>(bytes::Bytes::from_static(b"{\"late\":true}"))
        }))
        .header("Content-Type", "application/json"),
    )
}

/// A postal address, nested in [`UpdateProfile`].
#[tool_schema]
#[derive(serde::Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct Address {
    /// The street.
    pub street: String,
    /// The city.
    pub city: String,
}

/// The input of `update_profile`: several required fields, one nested.
#[tool_input]
#[serde(deny_unknown_fields)]
pub struct UpdateProfile {
    /// The display name.
    pub name: String,
    /// The email address.
    pub email: String,
    /// The age in years.
    pub age: u32,
    /// Where the user lives.
    #[validate(nested)]
    pub address: Address,
}

#[tool(
    name = "update_profile",
    description = "Update the signed-in user's profile.",
    callers = [user],
    input = UpdateProfile,
)]
#[handler]
pub async fn update_profile(form: UpdateProfile) -> Response {
    json_response!({
        "name": form.name,
        "email": form.email,
        "age": form.age,
        "street": form.address.street,
        "city": form.address.city,
    })
}

/// How many times the after-response command ran.
pub static LATER_RUNS: AtomicUsize = AtomicUsize::new(0);

/// The command `enqueue_message` queues for after its response.
#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct Later;

impl suprnova::bus::command::Command for Later {
    type Output = ();
    fn command_name() -> &'static str {
        "agents-fixture-later"
    }
}

/// Counts the runs of [`Later`].
pub struct LaterHandler;

#[async_trait]
impl suprnova::bus::command::Handler<Later> for LaterHandler {
    async fn handle(&self, _command: Later) -> Result<(), suprnova::FrameworkError> {
        LATER_RUNS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tool(
    name = "enqueue_message",
    description = "Queue work for after the response.",
    callers = [public],
)]
#[handler]
pub async fn later() -> Response {
    suprnova::Bus::dispatch_after_response(Later).map_err(HttpResponse::from)?;
    json_response!({ "queued": true })
}

/// A user an API guard's own provider loads.
#[derive(Clone, Debug)]
pub struct ApiUser {
    pub id: String,
    pub name: String,
}

impl Authenticatable for ApiUser {
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

#[tool(
    name = "read_api_user",
    description = "Read the user an API guard signs in.",
    callers = [user],
    guard = "api",
)]
#[handler]
pub async fn api_profile() -> Response {
    let user = suprnova::Auth::user_as_arc::<ApiUser>().await?;
    json_response!({
        "user": user.as_ref().map(|user| user.id.clone()),
        "name": user.as_ref().map(|user| user.name.clone()),
    })
}

/// A recursive, internally tagged expression: every variant holds the
/// same field, so a checker that tries every branch of each level does
/// work exponential in the depth.
#[tool_schema]
#[derive(serde::Deserialize)]
#[serde(tag = "op")]
pub enum Expr {
    /// One variant.
    Neg { arg: Box<Expr> },
    /// Another.
    Abs { arg: Box<Expr> },
    /// A third.
    Sqrt { arg: Box<Expr> },
    /// A fourth.
    Exp { arg: Box<Expr> },
}

/// The input of `evaluate_math`.
#[tool_input]
pub struct Eval {
    /// The expression.
    pub expr: Expr,
}

#[tool(
    name = "evaluate_math",
    description = "Evaluate an expression.",
    callers = [public],
    input = Eval,
)]
#[handler]
pub async fn eval(form: Eval) -> Response {
    json_response!({ "depth": form.expr.depth() })
}

impl Expr {
    /// How many levels the expression has.
    pub fn depth(&self) -> usize {
        let (Self::Neg { arg } | Self::Abs { arg } | Self::Sqrt { arg } | Self::Exp { arg }) = self;
        1 + arg.depth()
    }
}

/// The input of `group_tags`: a map whose keys the caller chooses.
#[tool_input]
pub struct TagGroups {
    /// Tag ids by group name.
    pub groups: std::collections::BTreeMap<String, Vec<u32>>,
}

#[tool(
    name = "group_tags",
    description = "Group tags.",
    callers = [public],
    input = TagGroups,
)]
#[handler]
pub async fn group_tags(form: TagGroups) -> Response {
    json_response!({ "groups": form.groups.len() })
}

#[tool(
    name = "read_named_user",
    description = "A tool that names the route it calls.",
    callers = [user],
    route = "read_named_user",
)]
#[handler]
pub async fn named_profile(caller: ToolCaller) -> Response {
    json_response!({ "user": caller.user_id() })
}

/// A routed handler that is not a tool.
#[handler]
pub async fn plain() -> Response {
    json_response!({ "plain": true })
}

/// Lets users delete posts and forbids agents to, reading the caller.
pub fn define_policies() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        Gate::define::<TestUser, Post>("delete_post", |_user, _post| {
            !ToolCaller::current().is_some_and(|caller| caller.is_agent())
        });
    });
}

/// Every tool's route, the counting middleware on `create_post`, a route
/// throttle on `ping_throttled`, and a literal `/posts/new` beside
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
        .post("/slow-stream", slow_stream)
        .post("/slow-bytes", slow_bytes)
        .post("/profile", update_profile)
        .post("/later", later)
        .post("/api/me", api_profile)
        .post("/math", eval)
        .post("/tags", group_tags)
        .post("/named", named_profile)
        .name("read_named_user")
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
