//! Real Suprnova server for the `agents_browser` suite (AGENT-012): Live
//! islands whose actions are WebMCP tools, served the way an application
//! serves them.
//!
//! `suprnova-cli/tests/agents_browser.rs` starts this binary and drives it
//! in Chromium with WebMCP enabled. It serves:
//!
//! - `/tools`, three counters keyed `main`, `left` and `right`, each
//!   offering `counter.add`, with a button that removes the first one and a
//!   link to `/other`, a page with no island;
//! - `/login/{user}`, which signs the browser in as that user, and
//!   `/logout`; the counter's `live:examples.agents-counter.add` policy
//!   lets only `alice` add;
//! - `/test/requests`, every Live action request since `/test/reset`: its
//!   tool-call header, the security checks its guard saw, the execution
//!   phases it ran and the Gate abilities it asked.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use suprnova::auth::Authenticatable;
use suprnova::live::testing::{LiveSecurityCheck, inspect_request_attestation};
use suprnova::live::{
    CanonicalValue, LiveBootstrapOptions, LiveComponent, LiveDocument, LiveMount, LiveRegistry,
    LiveRouteGuard, LiveTenantMiddleware, LiveTenantResolver, MountFlags, live,
};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::view::{
    AssetSet, DocumentResponseIntent, TrustedHtml, TrustedMarkupReason, ViewName,
};
use suprnova::{
    App, Auth, AuthMiddleware, CsrfMiddleware, FrameworkError, Gate, HttpResponse, Middleware,
    Next, RateLimitMiddleware, Request, Response, Router, Server, SessionConfig, SessionData,
    SessionMiddleware, SessionStore, SlidingWindowConfig, StatusCode,
};
use tracing_subscriber::layer::SubscriberExt as _;

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

/// A counter whose `add` is a WebMCP tool: its count is shown to the agent,
/// its owner never is.
#[derive(LiveComponent)]
#[live(
    name = "examples.agents-counter",
    view = "live/examples/agents-counter.html"
)]
pub struct AgentsCounter {
    #[agent_visible]
    count: u64,
    #[locked]
    owner: String,
}

#[live]
impl AgentsCounter {
    #[tool(
        name = "counter.add",
        description = "Add to the counter.",
        title = "Add",
        callers = [user],
    )]
    #[action(authorize = "current")]
    pub fn add(&mut self, amount: u64) {
        self.count += amount;
        if self.owner.is_empty() {
            self.owner = "owner-only-the-server-sees".to_string();
        }
    }

    /// Changes nothing anyone may run: what a viewer the `add` policy
    /// refuses clicks to make the island an instance of its own.
    #[action]
    pub fn touch(&mut self) {}
}

#[suprnova::view(path = "live/examples/agents-page.html")]
struct Page<'a> {
    bootstrap: &'a TrustedHtml,
    first: &'a TrustedHtml,
    second: &'a TrustedHtml,
    third: &'a TrustedHtml,
}

/// A user known by its name.
struct User(String);

impl Authenticatable for User {
    fn get_auth_identifier(&self) -> String {
        self.0.clone()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }
}

/// Signs the request in as the user its `agents_user` cookie names, as an
/// application's session guard would.
struct CookieUser;

#[async_trait]
impl Middleware for CookieUser {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let user = request
            .header("cookie")
            .and_then(|cookie| {
                cookie
                    .split(';')
                    .map(str::trim)
                    .find_map(|pair| pair.strip_prefix("agents_user="))
                    .map(str::to_string)
            })
            .filter(|user| !user.is_empty());
        if let Some(user) = user {
            Auth::set_user(Arc::new(User(user)));
        }
        next(request).await
    }
}

struct Tenantless;

#[async_trait]
impl LiveTenantResolver for Tenantless {
    async fn resolve(&self, _request: &Request) -> Result<Option<String>, FrameworkError> {
        Ok(None)
    }
}

#[derive(Default)]
struct MemorySessionStore(Mutex<HashMap<String, SessionData>>);

#[async_trait]
impl SessionStore for MemorySessionStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.0.lock().map_err(lock_error)?.get(id).cloned())
    }
    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.0
            .lock()
            .map_err(lock_error)?
            .insert(session.id.clone(), session.clone());
        Ok(())
    }
    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.0.lock().map_err(lock_error)?.remove(id);
        Ok(())
    }
    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let mut sessions = self.0.lock().map_err(lock_error)?;
        let before = sessions.len();
        sessions.retain(|_, session| session.user_id.as_deref() != Some(user_id));
        Ok((before - sessions.len()) as u64)
    }
    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

fn lock_error<T>(_: T) -> FrameworkError {
    FrameworkError::internal("the session store lock was poisoned")
}

/// What the suite reads back about the Live requests it caused.
#[derive(Default)]
struct Observations {
    requests: Vec<Value>,
    phases: Vec<String>,
    gate: Vec<String>,
}

fn observations() -> &'static Mutex<Observations> {
    static OBSERVATIONS: OnceLock<Mutex<Observations>> = OnceLock::new();
    OBSERVATIONS.get_or_init(Mutex::default)
}

fn observe<T>(update: impl FnOnce(&mut Observations) -> T) -> T {
    let mut guard = observations()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    update(&mut guard)
}

/// Records every Live execution phase the framework traces, in order.
struct PhaseLayer;

struct PhaseField(Option<String>);

impl tracing::field::Visit for PhaseField {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "phase" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for PhaseLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _context: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if !event
            .metadata()
            .target()
            .ends_with("live::ports::telemetry")
        {
            return;
        }
        let mut phase = PhaseField(None);
        event.record(&mut phase);
        if let Some(phase) = phase.0 {
            observe(|observations| observations.phases.push(phase));
        }
    }
}

static SEQUENCE: AtomicUsize = AtomicUsize::new(0);

/// The last middleware of the Live guard: records the request, the checks
/// its guard saw, and the phases and Gate asks it caused.
struct Recorder;

#[async_trait]
impl Middleware for Recorder {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let report = inspect_request_attestation(&request);
        let checks: serde_json::Map<String, Value> = [
            ("origin", LiveSecurityCheck::Origin),
            ("csrf", LiveSecurityCheck::Csrf),
            ("session", LiveSecurityCheck::Session),
            ("principal", LiveSecurityCheck::Principal),
            ("tenant", LiveSecurityCheck::Tenant),
            ("proxy", LiveSecurityCheck::Proxy),
            ("rateLimit", LiveSecurityCheck::RateLimit),
        ]
        .into_iter()
        .map(|(name, check)| {
            (
                name.to_string(),
                json!(
                    report
                        .disposition(check)
                        .map(|disposition| format!("{disposition:?}"))
                ),
            )
        })
        .collect();
        let tool = request.header("suprnova-tool-call").map(str::to_string);
        let path = request.path().to_string();
        let (phases_before, gate_before) =
            observe(|observations| (observations.phases.len(), observations.gate.len()));
        let response = next(request).await;
        let status = match &response {
            Ok(response) | Err(response) => response.status_code(),
        };
        observe(|observations| {
            let phases = observations.phases[phases_before..].to_vec();
            let gate = observations.gate[gate_before..].to_vec();
            observations.requests.push(json!({
                "sequence": SEQUENCE.fetch_add(1, Ordering::SeqCst),
                "path": path,
                "tool": tool,
                "status": status,
                "checks": checks,
                "phases": phases,
                "gate": gate,
            }));
        });
        response
    }
}

fn guard(guard: LiveRouteGuard) -> LiveRouteGuard {
    guard
        .middleware(AuthMiddleware::optional())
        .middleware(LiveTenantMiddleware::new(Arc::new(Tenantless)))
        .middleware(RateLimitMiddleware::new(
            Arc::new(InMemoryRateLimiter::new()),
            SlidingWindowConfig {
                max_requests: 10_000,
                window: Duration::from_secs(60),
            },
            |_request| "agents-live-host".to_owned(),
        ))
        .middleware(Recorder)
}

fn empty() -> TrustedHtml {
    TrustedHtml::framework_static("", TrustedMarkupReason::new("empty slot").expect("reason"))
        .expect("empty markup")
}

#[derive(Clone)]
struct Mounts {
    main: LiveMount<AgentsCounter>,
    left: LiveMount<AgentsCounter>,
    right: LiveMount<AgentsCounter>,
}

async fn tools_page(request: Request, mounts: Mounts) -> Result<HttpResponse, String> {
    let mut document = LiveDocument::from_request(&request).map_err(|error| error.to_string())?;
    let mut islands = Vec::new();
    for mount in [&mounts.main, &mounts.left, &mounts.right] {
        islands.push(
            document
                .mount(
                    mount,
                    CanonicalValue::Object(BTreeMap::new()),
                    MountFlags::empty(),
                )
                .await
                .map_err(|error| error.to_string())?,
        );
    }
    let bootstrap = document
        .bootstrap(LiveBootstrapOptions::esm())
        .map_err(|error| error.to_string())?;
    document
        .render(
            ViewName::parse("live/examples/agents-page.html").map_err(|error| error.to_string())?,
            &Page {
                bootstrap: bootstrap.html(),
                first: islands[0].html(),
                second: islands[1].html(),
                third: islands[2].html(),
            },
            DocumentResponseIntent::html(StatusCode::OK).map_err(|error| error.to_string())?,
            AssetSet::empty(),
        )
        .map_err(|error| error.to_string())
}

async fn other_page(request: Request) -> Result<HttpResponse, String> {
    let mut document = LiveDocument::from_request(&request).map_err(|error| error.to_string())?;
    let bootstrap = document
        .bootstrap(LiveBootstrapOptions::esm())
        .map_err(|error| error.to_string())?;
    let empty = empty();
    document
        .render(
            ViewName::parse("live/examples/agents-page.html").map_err(|error| error.to_string())?,
            &Page {
                bootstrap: bootstrap.html(),
                first: &empty,
                second: &empty,
                third: &empty,
            },
            DocumentResponseIntent::html(StatusCode::OK).map_err(|error| error.to_string())?,
            AssetSet::empty(),
        )
        .map_err(|error| error.to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::filter::LevelFilter::TRACE)
        .with(PhaseLayer);
    tracing::subscriber::set_global_default(subscriber)?;
    let port = std::env::var("SUPRNOVA_AGENTS_HOST_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(4178);
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<AgentsCounter>()
            .expect("register the counter")
            .build(),
    );
    Gate::define::<String, String>("live:examples.agents-counter.add", |user, _action| {
        observe(|observations| {
            observations
                .gate
                .push(format!("live:examples.agents-counter.add as {user}"));
        });
        user == "alice"
    });
    let mounts = Mounts {
        main: LiveMount::public_seed("/tools", "counter-main", "main").expect("main mount"),
        left: LiveMount::public_seed("/tools", "counter-left", "left").expect("left mount"),
        right: LiveMount::public_seed("/tools", "counter-right", "right").expect("right mount"),
    };
    let page_mounts = mounts.clone();
    let mut router: Router = Router::new()
        .get("/health", |_request: Request| async {
            Ok(HttpResponse::text("ok"))
        })
        .get("/tools", move |request: Request| {
            let mounts = page_mounts.clone();
            async move {
                tools_page(request, mounts).await.map_err(|error| {
                    HttpResponse::text(format!("page failed: {error}")).status(500)
                })
            }
        })
        .get("/other", |request: Request| async move {
            other_page(request)
                .await
                .map_err(|error| HttpResponse::text(format!("page failed: {error}")).status(500))
        })
        .get("/login/{user}", |request: Request| async move {
            let user = request.param("user").unwrap_or("").to_string();
            Ok(HttpResponse::text("signed in").header(
                "Set-Cookie",
                format!("agents_user={user}; Path=/; SameSite=Lax"),
            ))
        })
        .get("/logout", |_request: Request| async {
            Ok(HttpResponse::text("signed out").header(
                "Set-Cookie",
                "agents_user=; Path=/; Max-Age=0; SameSite=Lax",
            ))
        })
        .get("/test/requests", |_request: Request| async {
            let requests = observe(|observations| observations.requests.clone());
            Ok(HttpResponse::json(json!({ "requests": requests })))
        })
        .get("/test/reset", |_request: Request| async {
            observe(|observations| {
                observations.requests.clear();
                observations.phases.clear();
                observations.gate.clear();
            });
            Ok(HttpResponse::text("reset"))
        })
        .into();
    router = router.try_live_with(guard)?;
    for mount in [&mounts.main, &mounts.left, &mounts.right] {
        router = router.try_live_mount(mount)?;
    }
    let mut session = SessionConfig::default();
    session.cookie_secure = false;
    Server::new(router)
        .middleware(SessionMiddleware::with_store(
            session,
            Arc::new(MemorySessionStore::default()),
        ))
        .middleware(CsrfMiddleware::new())
        .middleware(CookieUser)
        .host("127.0.0.1")
        .port(port)
        .run()
        .await
}
