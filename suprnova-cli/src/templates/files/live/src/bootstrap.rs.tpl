//! Application Bootstrap
//!
//! This is where you register global middleware and services that need runtime configuration.
//! Services that don't need runtime config can use `#[service(ConcreteType)]` instead.
//!
//! # Example
//!
//! ```rust,ignore
//! // For services with no runtime config, use the macro:
//! #[service(RedisCache)]
//! pub trait CacheStore { ... }
//!
//! // For services needing runtime config, register here:
//! pub async fn register() {
//!     // Initialize database
//!     DB::init().await.expect("Failed to connect to database");
//!
//!     // Global middleware
//!     global_middleware!(middleware::LoggingMiddleware);
//!
//!     // Services
//!     bind!(dyn Database, PostgresDB::new());
//! }
//! ```

use std::sync::Arc;

use suprnova::{
    App, Auth, AuthConfig, AuthManager, CsrfMiddleware, DB, EloquentUserProvider, LocaleMiddleware,
    SessionConfig, SessionMiddleware, global_middleware,
};

use crate::controllers::errors::ErrorPage;
use crate::middleware;
use crate::models::user::User;

/// Register process-wide services.
///
/// Called from `cmd/main.rs` via `.bootstrap(bootstrap::register)`, before
/// `Server::from_config()`. This hook is process-wide: every subcommand runs
/// it, including the queue, schedule, and workflow workers, `down`, `up`, and
/// the console binary, not only `serve`. The migration commands run before
/// it and without it, so a fresh database can be migrated. Register services
/// and bindings that need runtime configuration here - things every process
/// needs, like the database and the auth provider. The HTTP stack lives in
/// [`register_http_stack`], wired separately via `.http_bootstrap(...)` in
/// `cmd/main.rs`, so it never runs on a worker or console process.
pub async fn register() {
    // Initialize database connection
    DB::init().await.expect("Failed to connect to database");
    // Database migrations run before this hook; the engine generation does not.
    crate::models::note::Note::initialize_search()
        .await
        .expect("Failed to initialize notes search index");

    // Register reusable notes fixtures for `db:seed` and `--class=NotesSeeder`.
    suprnova::seed::register::<crate::seeders::notes::NotesSeeder>();

    // Authentication: register the AuthManager (the config/auth.php analogue)
    // and a user provider so `Auth::attempt` and `Auth::user_as::<User>()`
    // resolve users. `EloquentUserProvider<User>` queries the typed model; the
    // SessionMiddleware below persists the authenticated id across requests.
    App::singleton(AuthManager::new(AuthConfig::from_env()));
    Auth::register_provider("users", Arc::new(EloquentUserProvider::<User>::new()))
        .expect("register users provider");

    // Live components: one immutable registry shared by the server, the
    // workers, and the `suprnova live:*` commands. `suprnova live:make`
    // registers new components in `src/live/mod.rs`.
    App::singleton(crate::live::registry().expect("Live component registry"));

    // The abilities Live asks the gate for: who may subscribe to the
    // dashboard's notes stream.
    crate::live::authorize();
}

/// Register the global middleware chain.
///
/// Called on the server path, via
/// `.http_bootstrap(|| async { bootstrap::register_http_stack() })` in
/// `cmd/main.rs`, after [`register`]. Never on the queue, schedule, or
/// workflow workers, and never on the console binary.
///
/// This kit installs no Inertia layer: every page is a server-rendered Live
/// document, so there is no frontend build, no Vite manifest and no asset
/// version to check. Order matters: the session first, because
/// authentication, the CSRF check and the toast all read it; the locale
/// next, since its detection checks the session; the error page after
/// both, so an error page knows the visitor and their language; the
/// request timeout after the error page, so its `503` reaches the error
/// page too; and CSRF last, so its `419` reaches the error page on the way
/// out.
pub fn register_http_stack() {
    // Global middleware (runs on every request in registration order)
    global_middleware!(middleware::LoggingMiddleware);

    // Session middleware (required for authentication). The config is
    // cloned because the CSRF middleware below reads the same values, so
    // the session cookie and the XSRF cookie cannot drift apart.
    let session_config = SessionConfig::from_env();
    global_middleware!(SessionMiddleware::new(session_config.clone()));

    // Locale detection - after SessionMiddleware, since its detection
    // chain checks the session first (then cookie, then Accept-Language).
    // Reads APP_LOCALE / APP_FALLBACK_LOCALE from the environment and
    // scopes the detected locale for the rest of the request, so
    // `Lang::get` / the `__!` macro resolve against it.
    global_middleware!(
        LocaleMiddleware::from_env().expect("locale config (APP_LOCALE / APP_FALLBACK_LOCALE)")
    );

    // Every framework error response a browser asked for (a 404, a 403, a
    // 419, a 500 outside debug mode) renders as the error document,
    // `templates/live/pages/error.html`, in place of a JSON body. It sits
    // inside the session and locale scopes, and outside CSRF, so a `419`
    // reaches it too.
    global_middleware!(ErrorPage);

    // Request timeout - a 30-second ceiling on the time to a response, so a
    // slow handler or a hung query cannot hold a connection open
    // indefinitely; past it the request answers `503`. Here, inside the
    // error page, so the `503` renders as the error document; and ahead of
    // CSRF and the handlers, so it bounds their work, the CSRF check's read
    // of a form body included. Streaming responses and WebSocket upgrades
    // are exempt by design - see `suprnova::timeout`. A per-route
    // `.middleware(TimeoutMiddleware::seconds(n))` tightens one endpoint; a
    // route that takes long uploads needs a longer value here.
    global_middleware!(suprnova::TimeoutMiddleware::default());

    // CSRF protection (validates tokens on POST/PUT/PATCH/DELETE). Live
    // requests verify the browser's `Sec-Fetch-Site` proof on their own, so
    // ordinary routes, such as the sign-out form, keep token validation
    // under the default policy.
    //
    // `with_session_config` copies the session cookie's `Secure`,
    // `SameSite`, `Domain`, `Path` and lifetime onto the `XSRF-TOKEN`
    // cookie, so it survives `SESSION_SECURE=false` over local HTTP.
    global_middleware!(CsrfMiddleware::new().with_session_config(&session_config));
}
