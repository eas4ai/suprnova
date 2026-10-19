//! The Live component the Live tool tests act on, and the application
//! serving it with Live and the MCP endpoint (AGENT-012, AGENT-013).

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use suprnova::agents::AgentsConfig;
use suprnova::live::testing::prepare_live_router_for_test;
use suprnova::live::{
    CanonicalValue, LiveBootstrapOptions, LiveComponent, LiveDocument, LiveMount, LiveRegistry,
    LiveRouteGuard, LiveTenantMiddleware, LiveTenantResolver, MountFlags, live,
};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::testing::TestClient;
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{
    App, AuthMiddleware, Crypt, CsrfMiddleware, EncryptionKey, FrameworkError, Gate, HttpResponse,
    LiveMountRequest, Middleware, MiddlewareRegistry, Next, RateLimitMiddleware, Request, Response,
    Router, SessionConfig, SessionData, SessionMiddleware, SessionStore, SlidingWindowConfig,
    StatusCode, ToolCaller,
};

/// The filters the page template uses.
pub mod filters {
    pub use suprnova::view::filters::trusted_html;
}

/// A counter an agent can add to: its count and label are shown to the
/// agent, its owner never is. A count over 100 does not validate, nor does
/// an owner longer than 16 characters.
#[derive(LiveComponent, validator::Validate)]
#[live(name = "tests.agent-counter", view = "live/tests/agent-counter.html")]
pub struct AgentCounter {
    #[agent_visible]
    #[validate(range(max = 100))]
    count: u64,
    #[agent_visible]
    label: String,
    #[locked]
    #[validate(length(max = 16))]
    owner: String,
}

/// The title `counter.rename` validates.
#[derive(validator::Validate)]
struct Title {
    #[validate(length(max = 8))]
    title: String,
}

#[live]
impl AgentCounter {
    #[mount]
    pub fn mount(
        start: u64,
        label: String,
        #[resolve(crate::live_fixture::owner_of)] owner: String,
    ) -> Self {
        Self {
            count: start,
            label,
            owner,
        }
    }

    #[tool(
        name = "counter.add",
        description = "Add to the counter.",
        callers = [user, agent],
        headless,
    )]
    #[action(authorize = "current")]
    pub fn add(&mut self, amount: u64) {
        self.count += amount;
    }

    #[tool(
        name = "counter.peek",
        description = "Read the counter.",
        callers = [public],
        annotations(read_only),
    )]
    #[action]
    pub fn peek(&mut self) {}

    #[tool(
        name = "counter.check",
        description = "Check the counter is in range.",
        callers = [user],
        headless,
    )]
    #[action(validate = "whole")]
    pub fn check(&mut self) {}

    #[tool(
        name = "counter.rename",
        description = "Rename the counter.",
        callers = [user],
        headless,
    )]
    #[action(validate = "arguments")]
    pub fn rename(&mut self, title: String) {
        self.label = title;
    }

    /// The argument validation `rename` asks for.
    #[validate(action = "rename")]
    pub fn validate_rename(&mut self, title: String) -> Result<(), validator::ValidationErrors> {
        validator::Validate::validate(&Title { title })
    }

    /// The whole-component validation `check` asks for.
    #[validate]
    pub fn validate_component(&self) -> Result<(), validator::ValidationErrors> {
        validator::Validate::validate(self)
    }

    #[action]
    pub fn reset(&mut self) {
        self.count = 0;
        let _ = &self.owner;
    }
}

/// A quota anyone can read: its limit comes from the caller, and a guest
/// has none; the note is the agent's to leave out.
#[derive(LiveComponent)]
#[live(name = "tests.agent-quota", view = "live/tests/agent-quota.html")]
pub struct AgentQuota {
    #[agent_visible]
    limit: Option<u64>,
    #[agent_visible]
    note: Option<String>,
    #[agent_visible]
    reads: u64,
    /// A Json field holding a value shaped like a 64-bit integer tag: an
    /// agent reads it as it is.
    #[agent_visible]
    shape: serde_json::Value,
}

#[live]
impl AgentQuota {
    #[mount]
    pub fn mount(
        #[resolve(crate::live_fixture::limit_of)] limit: Option<u64>,
        note: Option<String>,
    ) -> Self {
        Self {
            limit,
            note,
            reads: 0,
            shape: serde_json::json!({ "$live": "u64", "value": "5" }),
        }
    }

    #[tool(
        name = "quota.read",
        description = "Read the quota.",
        callers = [public, user],
        headless,
    )]
    #[action]
    pub fn read(&mut self) {
        self.reads += 1;
    }
}

/// The quota's limit for the caller: ten for a user, none for a guest.
pub async fn limit_of(caller: &ToolCaller) -> Result<Option<u64>, FrameworkError> {
    Ok(caller.user_id().map(|_| 10))
}

/// How many times [`owner_of`] ran.
pub static RESOLVED: AtomicUsize = AtomicUsize::new(0);

/// Fills the counter's owner from the caller: the agent cannot set it.
pub async fn owner_of(caller: &ToolCaller) -> Result<String, FrameworkError> {
    RESOLVED.fetch_add(1, Ordering::SeqCst);
    Ok(caller.user_id().unwrap_or_else(|| "nobody".to_string()))
}

/// How many requests the Live guard's probe saw.
pub static GUARDED: AtomicUsize = AtomicUsize::new(0);

/// Counts the requests the Live route guard runs on.
pub struct GuardProbe;

#[async_trait]
impl Middleware for GuardProbe {
    async fn handle(&self, request: Request, next: Next) -> Response {
        GUARDED.fetch_add(1, Ordering::SeqCst);
        next(request).await
    }
}

/// The counter's component name.
pub const COUNTER: &str = "tests.agent-counter";

/// A tenant resolver that names no tenant.
pub struct Tenantless;

#[async_trait]
impl LiveTenantResolver for Tenantless {
    async fn resolve(&self, _request: &Request) -> Result<Option<String>, FrameworkError> {
        Ok(None)
    }
}

/// The guard every reserved Live route runs: optional authentication, no
/// tenant, and a generous throttle.
pub fn guard(guard: LiveRouteGuard) -> LiveRouteGuard {
    guard
        .middleware(GuardProbe)
        .middleware(AuthMiddleware::optional())
        .middleware(LiveTenantMiddleware::new(Arc::new(Tenantless)))
        .middleware(RateLimitMiddleware::new(
            Arc::new(InMemoryRateLimiter::new()),
            SlidingWindowConfig {
                max_requests: 1_000,
                window: Duration::from_secs(60),
            },
            |_request| "agents-live".to_owned(),
        ))
}

/// Lets everyone but mallory mount a counter starting below 1000, lets
/// only alice add to one, and lets anyone mount a quota.
pub fn define_live_policies() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        Gate::define::<ToolCaller, LiveMountRequest>(
            "live:tests.agent-counter.mount",
            |caller, mount| {
                let start = mount
                    .property("start")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(u64::MAX);
                caller.user_id().as_deref() != Some("mallory") && start < 1_000
            },
        );
        Gate::define::<String, String>("live:tests.agent-counter.add", |user, _action| {
            user == "alice"
        });
        Gate::define::<ToolCaller, LiveMountRequest>(
            "live:tests.agent-quota.mount",
            |_caller, _mount| true,
        );
    });
}

/// The process's Live key material and the counter's registry, in the
/// test's container.
pub fn register_components() {
    static CRYPT: OnceLock<()> = OnceLock::new();
    CRYPT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<AgentCounter>()
            .expect("register the agent counter")
            .register::<AgentQuota>()
            .expect("register the agent quota")
            .build(),
    );
}

/// The page mounting a counter as a public seed, keyed `main`, and a quota
/// beside it, keyed `side`.
pub const PAGE: &str = "/agents/page";

/// The page's document.
#[suprnova::view(path = "live/tests/agent-page.html")]
pub struct AgentPage<'a> {
    pub bootstrap: &'a TrustedHtml,
    pub island: &'a TrustedHtml,
    pub side: &'a TrustedHtml,
}

/// The fixture's routes, the counter's page, and Live installed behind
/// [`guard`], serving agents with `config`.
pub fn live_router_with(config: AgentsConfig) -> Router {
    live_router().agents(config)
}

/// [`live_router_with`] without the agents: no MCP endpoint, but its Live
/// tools are still checked and counted by the Live tool gate, with the
/// default configuration.
pub fn live_router() -> Router {
    crate::fixture::define_policies();
    let mount = LiveMount::<AgentCounter>::public_seed(PAGE, "counter", "main")
        .expect("declare the counter's mount");
    let side = LiveMount::<AgentQuota>::public_seed(PAGE, "quota", "side")
        .expect("declare the quota's mount");
    let handler_mounts = (mount.clone(), side.clone());
    let router: Router = crate::fixture::routes()
        .get(PAGE, move |request: Request| {
            let (mount, side) = handler_mounts.clone();
            async move { render_page(request, mount, side).await }
        })
        .into();
    router
        .try_live_with(guard)
        .expect("install Live")
        .try_live_mount(&mount)
        .expect("register the counter's mount")
        .try_live_mount(&side)
        .expect("register the quota's mount")
}

async fn render_page(
    request: Request,
    mount: LiveMount<AgentCounter>,
    side: LiveMount<AgentQuota>,
) -> Result<HttpResponse, HttpResponse> {
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)
            .map_err(|error| FrameworkError::internal(format!("from_request {error}")))?;
        let island = document
            .mount(
                &mount,
                CanonicalValue::Object(BTreeMap::from([
                    (
                        "start".to_owned(),
                        // A 64-bit integer parameter carries the snapshot's
                        // lossless tag.
                        CanonicalValue::Object(BTreeMap::from([
                            ("$live".to_owned(), CanonicalValue::String("u64".to_owned())),
                            ("value".to_owned(), CanonicalValue::String("1".to_owned())),
                        ])),
                    ),
                    (
                        "label".to_owned(),
                        CanonicalValue::String("Page".to_owned()),
                    ),
                    (
                        "owner".to_owned(),
                        CanonicalValue::String("page".to_owned()),
                    ),
                ])),
                MountFlags::empty(),
            )
            .await
            .map_err(|error| FrameworkError::internal(format!("mount {error}")))?;
        // The quota's optional parameters are left out: it mounts with
        // none of them.
        let side = document
            .mount(
                &side,
                CanonicalValue::Object(BTreeMap::new()),
                MountFlags::empty(),
            )
            .await
            .map_err(|error| FrameworkError::internal(format!("mount side {error}")))?;
        let bootstrap = document
            .bootstrap(LiveBootstrapOptions::esm())
            .map_err(|error| FrameworkError::internal(format!("bootstrap {error}")))?;
        document
            .render(
                ViewName::parse("live/tests/agent-page.html")
                    .map_err(|_| FrameworkError::internal("view identity"))?,
                &AgentPage {
                    bootstrap: bootstrap.html(),
                    island: island.html(),
                    side: side.html(),
                },
                DocumentResponseIntent::html(StatusCode::OK)
                    .map_err(|_| FrameworkError::internal("response intent"))?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(|error| HttpResponse::text(format!("page failed: {error}")).status(500))
}

/// A session store in memory, for the page's browser calls.
#[derive(Default)]
pub struct MemorySessionStore(Mutex<HashMap<String, SessionData>>);

#[async_trait]
impl SessionStore for MemorySessionStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.0.lock().expect("session store lock").get(id).cloned())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.0
            .lock()
            .expect("session store lock")
            .insert(session.id.clone(), session.clone());
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.0.lock().expect("session store lock").remove(id);
        Ok(())
    }

    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let mut sessions = self.0.lock().expect("session store lock");
        let before = sessions.len();
        sessions.retain(|_, session| session.user_id.as_deref() != Some(user_id));
        Ok((before - sessions.len()) as u64)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

/// The browser's global stack: a session, then the CSRF check.
pub fn browser_middleware() -> MiddlewareRegistry {
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    MiddlewareRegistry::new()
        .append(SessionMiddleware::with_store(
            config,
            Arc::new(MemorySessionStore::default()),
        ))
        .append(CsrfMiddleware::new())
}

/// A test client of [`live_router_with`] with its Live runtime prepared,
/// the counter's policies defined when `policies` holds.
pub fn live_client_with(config: AgentsConfig, policies: bool) -> TestClient {
    register_components();
    if policies {
        define_live_policies();
    }
    let router = live_router_with(config);
    prepare_live_router_for_test(&router).expect("prepare the Live runtime");
    TestClient::new(router, browser_middleware())
}

/// A test client of [`live_router`], which serves no MCP endpoint, with
/// its Live runtime prepared and the counter's policies defined.
pub fn live_client_without_agents() -> TestClient {
    register_components();
    define_live_policies();
    let router = live_router();
    prepare_live_router_for_test(&router).expect("prepare the Live runtime");
    TestClient::new(router, browser_middleware())
}

/// [`live_client_with`] with the default configuration and the policies.
pub fn live_client() -> TestClient {
    live_client_with(AgentsConfig::new(), true)
}
