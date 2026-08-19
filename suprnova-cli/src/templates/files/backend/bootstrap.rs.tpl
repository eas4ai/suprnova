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

#[allow(unused_imports)]
use suprnova::{
    bind, global_middleware, singleton, App, Auth, AuthConfig, AuthManager, CsrfMiddleware,
    EloquentUserProvider, Frontend, IncludeMiddleware, Inertia, InertiaConfig, LocaleMiddleware,
    LocaleShare, SessionConfig, SessionMiddleware, DB,
};

use crate::middleware;
use crate::models::user::User;

/// Register process-wide services.
///
/// Called from `cmd/main.rs` via `.bootstrap(bootstrap::register)`, before
/// `Server::from_config()`. This hook is process-wide: every subcommand runs
/// it, including the queue, schedule, and workflow workers and the console
/// binary, not only `serve`. Register services and bindings that need
/// runtime configuration here - things every process needs, like the
/// database and the auth provider. The HTTP stack (global middleware and
/// `Inertia::install`) lives in [`register_http_stack`], wired separately
/// via `.http_bootstrap(...)` in `cmd/main.rs`, so it never runs on a
/// worker or console process that ships no built frontend assets.
pub async fn register() {
    // Initialize database connection
    DB::init().await.expect("Failed to connect to database");

    // Authentication: register the AuthManager (the config/auth.php analogue)
    // and a user provider so `Auth::attempt` and `Auth::user_as::<User>()`
    // resolve users. `EloquentUserProvider<User>` queries the typed model; the
    // SessionMiddleware above persists the authenticated id across requests.
    App::singleton(AuthManager::new(AuthConfig::from_env()));
    Auth::register_provider("users", Arc::new(EloquentUserProvider::<User>::new()))
        .expect("register users provider");

    // Example: Register a trait binding with runtime config
    // bind!(dyn Database, PostgresDB::new());

    // Example: Register a concrete singleton
    // singleton!(CacheService::new());

    // Add your middleware and service registrations here
}

/// Register the global middleware chain and the Inertia layer.
///
/// Called only on the server path, via
/// `.http_bootstrap(|| async { bootstrap::register_http_stack() })` in
/// `cmd/main.rs` - after [`register`], never on the queue, schedule, or
/// workflow workers, and never on the console binary. That split matters
/// because `Inertia::install` below fails closed in production when the
/// built frontend manifest is missing, which is exactly the state of a
/// worker or console container image that ships no `public/assets`.
/// Keeping this hook separate lets those images boot.
///
/// Order matters and mirrors the comments below: session before Inertia
/// (the version middleware re-flashes the session before it bounces a
/// stale client), locale and CSRF after the session they both depend on.
pub fn register_http_stack() {
    // Global middleware (runs on every request in registration order)
    global_middleware!(middleware::LoggingMiddleware);

    // Session middleware (required for authentication)
    let session_config = SessionConfig::from_env();
    global_middleware!(SessionMiddleware::new(session_config));

    // Inertia protocol layer, four middlewares in one call: the headers
    // middleware (`Vary: X-Inertia` on every response, and an empty `200` on an
    // Inertia visit substituted with a `303` back), the version middleware
    // (409 + `X-Inertia-Location` when the client's asset version doesn't match
    // the current one), the 303 middleware (302 -> 303 on non-GET Inertia
    // redirects, so the client's follow-up request is explicitly a GET rather
    // than a replayed PUT/DELETE), and the validation-redirect middleware
    // (turns a `422` carrying an `errors` object into the redirect-back the
    // Inertia client expects, so a failed validation restores the form with
    // its messages instead of surfacing a raw 422).
    //
    // This sits here, after SessionMiddleware, rather than beside the container
    // wiring below, for two reasons. It registers middleware of its own, so
    // moving it would silently move the whole Inertia layer within the chain.
    // And the version middleware re-flashes the session before it bounces a
    // stale client: the client answers a 409 with a full-page GET, and without
    // the re-flash a validation error flashed by the previous request is aged
    // away before the destination page can read it. It can only re-flash inside
    // a session scope.
    //
    // The frontend is pinned here rather than left to SUPRNOVA_FRONTEND:
    // `InertiaConfig::default()` falls back to Svelte, so a React project
    // whose environment forgot that variable would render Svelte's
    // `src/main.ts` entry point with no refresh preamble - a blank page with
    // no error in it. `manifest_path` keeps its default of
    // `public/assets/.vite/manifest.json`, which is where this project's
    // `frontend/vite.config.ts` writes it; in production the install fails
    // closed when that manifest is missing, rather than serving asset URLs
    // that point at a Vite dev server nobody is running.
    //
    // No asset-version override here: `InertiaConfig::default()` already
    // hashes that same manifest for the asset version, so a frontend build
    // changes the version automatically and a stale client gets the 409
    // reload above for free. Reach for the `version` (or `version_with`)
    // builder method instead when you need a hand-managed value - a CDN
    // cache key, a value shared across more than one process, or anything
    // else the manifest hash can't stand in for.
    //
    // Everything set on this config reaches every page: `Inertia::install`
    // retains it as the default each `InertiaResponse` starts from.
    Inertia::install(&InertiaConfig::new().frontend(Frontend::{frontend_variant}))
        .expect("Inertia install failed (production needs a built frontend manifest)");

    // Locale detection — registered after SessionMiddleware, since its
    // detection chain checks the session first (then cookie, then
    // Accept-Language). Reads APP_LOCALE / APP_FALLBACK_LOCALE from the
    // environment and scopes the detected locale for the rest of the
    // request, so `Lang::get` / the `__!` macro resolve against it.
    global_middleware!(
        LocaleMiddleware::from_env().expect("locale config (APP_LOCALE / APP_FALLBACK_LOCALE)")
    );

    // CSRF protection (validates tokens on POST/PUT/PATCH/DELETE)
    global_middleware!(CsrfMiddleware::new());

    // Parse `?include=`/`?exclude=`/`?only=`/`?except=` and `?fields[...]=`
    // into the per-request task-local so `#[derive(Data)]` responses,
    // `Resource::single`, and lazy `Prop` resolution honour the client's
    // requested shape out of the box. Without this, Data DTOs silently
    // ignore include/fieldset query parameters.
    global_middleware!(IncludeMiddleware);

    // Inertia shared data: the `lang` prop (active locale, fallback, and
    // where to fetch its Fluent catalog) on every Inertia response. The
    // frontend kit's `lib/lang.ts` wrapper reads this via `initLang(page)`.
    App::register_inertia_shared(Arc::new(LocaleShare));
}
