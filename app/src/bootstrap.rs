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
use std::time::Duration;

use suprnova::broadcasting::{BroadcastHub, ChannelRegistry, InMemoryBroadcastHub};
use suprnova::features::{FeatureMiddleware, bootstrap_database_cached};
use suprnova::queue::worker::register_job;
#[allow(unused_imports)]
use suprnova::{
    App, CsrfMiddleware, DB, EloquentUserProvider, EventFacade, FrameworkError, IncludeMiddleware,
    Inertia, InertiaConfig, InertiaRequestExt, InertiaSharedData, LocaleMiddleware, LocaleShare,
    Prop, S3Config, SessionConfig, SessionMiddleware, Storage, SupervisorRegistry, UserProvider,
    bind, global_middleware, singleton,
};

use crate::broadcasting::{ChatChannel, UserRegisteredChannel};
use crate::events::UserRegistered;
use crate::listeners::SendWelcomeEmailListener;
use crate::middleware;
use crate::models::users::User;

/// Register process-wide services.
///
/// Called from `cmd/main.rs` via `.bootstrap(bootstrap::register)`, before
/// `Server::from_config()`. This hook is process-wide: every subcommand runs
/// it, including the queue, schedule, and workflow workers and the console
/// binary, not only `serve`. Register database, container bindings, event
/// listeners, and job registration here. The HTTP stack (global middleware
/// and `Inertia::install`) is installed separately via
/// `.http_bootstrap(|| async { bootstrap::register_http_stack() })` in
/// `cmd/main.rs`, so it never runs on a process that ships no built frontend
/// assets.
pub async fn register() {
    // Initialize database connection
    DB::init().await.expect("Failed to connect to database");

    // Register the user provider for Auth::user() and the auth-flow facades.
    //
    // `EloquentUserProvider<User>` queries the typed `#[suprnova::model]` User
    // by primary key for session lookups (`Auth::user_as::<User>()`), exactly
    // like the previous `DatabaseUserProvider` did. Because `User` now impls
    // `MustVerifyEmail` + `CanResetPassword`, this provider ALSO implements the
    // auth-flow surface (`retrieve_by_email` / `mark_email_verified` /
    // `set_password` / `is_email_verified`), so `EmailVerification` and
    // `PasswordReset` — which resolve `active_user_provider()` → this binding —
    // are runtime-functional rather than erroring on the unsupported defaults.
    bind!(dyn UserProvider, EloquentUserProvider::<User>::new());

    // Inertia shared data — visible on every Inertia response.
    //
    // Static values: process-global, set once at boot.
    App::inertia_share("appName", "Suprnova");
    App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

    // Per-request shared data via the trait. The framework awaits
    // `share(&req)` on every Inertia response so this can read headers,
    // session, etc. — see `AppSharedData` below.
    App::register_inertia_shared(std::sync::Arc::new(AppSharedData));

    // The `lang` prop (active locale, fallback, and where to fetch its
    // Fluent catalog) on every Inertia response — mirrors the scaffold
    // template's `bootstrap.rs.tpl`, which registers this the same way
    // right after its own shared data. The frontend kits' `lib/lang`
    // wrapper reads this via `initLang(page)`.
    App::register_inertia_shared(std::sync::Arc::new(LocaleShare));

    // Broadcasting hub — in-process pub/sub. Registered in the container
    // as `dyn BroadcastHub` so SSE + WS handlers resolve it uniformly.
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

    // Channel registry — lists every named channel the WS handler accepts.
    // Registered as a concrete singleton so routes::register() can resolve
    // it when constructing BroadcastingWsHandler.
    let mut registry = ChannelRegistry::new();
    registry.register(UserRegisteredChannel);
    registry.register(ChatChannel);
    App::singleton(Arc::new(registry));

    // Event listeners. Registered once at boot; survive for the lifetime
    // of the process. The framework's dispatcher invokes them in
    // registration order.
    EventFacade::listen::<UserRegistered, _>(Arc::new(SendWelcomeEmailListener)).await;

    // Wire UserRegistered → BroadcastHub so every dispatch also publishes
    // the event's JSON payload on the "user_registered" channel. Both SSE
    // and WS subscribers receive it from the same hub.
    EventFacade::broadcast::<UserRegistered>(Arc::clone(&hub)).await;

    // Storage disks. Local `public` is always available; the S3 `uploads`
    // disk is env-gated so dev boots without AWS credentials.
    register_storage_disks();

    // Bootstrap rate-limit driver so App::resolve_make::<dyn RateLimiterDriver>()
    // succeeds when routes::register() runs immediately after bootstrap.
    // Server::run also calls bootstrap_from_env() later — that call is
    // idempotent (guarded by has_binding) so there is no double-init.
    suprnova::rate_limit::bootstrap_default().await;

    // Bind a queue driver from `QUEUE_DRIVER` (memory when unset).
    //
    // Registering job types is not the same as having somewhere to put
    // them: without this, anything that enqueues outside the `serve` path
    // — every console command, in particular — fails with "queue driver
    // not initialized". The serve and `queue:work` paths install their own
    // driver, which is why the gap stayed invisible.
    if let Err(e) = suprnova::queue::bootstrap_from_env().await {
        tracing::error!(error = %e, "queue driver bootstrap failed");
    }

    // Register queue jobs so the worker can dispatch them by name.
    register_job::<crate::jobs::welcome_log::WelcomeLog>();

    // Benchmark Phase 1 instruments. Registered unconditionally: they are
    // inert unless something enqueues them, and gating them behind a
    // feature would mean the benchmark measures a binary nobody ships.
    register_job::<crate::jobs::bench::BenchSleep>();
    register_job::<crate::jobs::bench::BenchAbort>();
    register_job::<crate::jobs::bench::BenchRecord>();

    // Phase 5B Task 20 — mail dogfood.
    //
    // Register the WelcomeEmail mailable factory so the worker can
    // re-hydrate it from a `SendMailJob` envelope, and register
    // `SendMailJob` itself as a worker-dispatchable job. Both calls are
    // idempotent (last-write-wins) per the framework's registry contract.
    suprnova::mail::register_mailable_factory::<crate::mail::welcome::WelcomeEmail>()
        .expect("register at boot");
    register_job::<suprnova::mail::send_job::SendMailJob>();

    // Phase 5B Task 20 — notifications dogfood.
    //
    // Register an OrderShipped factory so `SendNotificationJob` can rebuild
    // the notification from its JSON payload at dispatch time, and register
    // the notification job for worker dispatch. The factory is now
    // auto-derived from `N: Notification`'s `Deserialize` impl.
    suprnova::notifications::register_notification_factory::<
        crate::notifications::order_shipped::OrderShipped,
    >()
    .expect("register at boot");
    register_job::<suprnova::notifications::notify_job::SendNotificationJob>();

    // Phase 6A T7 — factory + seeder dogfood. Registers `BaseSeeder`
    // so a `suprnova db:seed` invocation (Phase 6B) will populate 50
    // users + 200 posts via the framework's Factory / Persistable
    // path. Tests reach the same path through `seed::run_all()`
    // directly without the CLI.
    suprnova::seed::register::<crate::seeders::BaseSeeder>();

    // Phase 7B T10 — supervised long-running tasks.
    //
    // Spawn every supervisor registered via `inventory::submit!` into its own
    // restart-loop task. The tasks are detached (v1 does not drain them on
    // shutdown); they run for the lifetime of the process.
    SupervisorRegistry::start_all().await;

    // Phase 10C T14 — install every `#[suprnova::observer(M)]` impl in
    // the binary. `bootstrap_observers()` drains the
    // `inventory::iter::<ObserverEntry>` set built up at compile time
    // and registers each observer's listener adapters with the
    // framework's `EventDispatcher`. Idempotent: each install closure
    // gates itself with an `AtomicBool` so re-running this call
    // (e.g. on a worker that re-bootstraps) does NOT double-register
    // the listeners.
    //
    // The app's `UserObserver` (in `crate::models::users`) lowercases
    // email on `creating` and traces id on `created`; both adapters
    // wire up here.
    suprnova::eloquent::observers::bootstrap_observers()
        .await
        .expect("observer install failed");

    // Phase 13 — feature flags.
    //
    // Wire the canonical Cached(Database) chain. After this call:
    //
    // * `is_enabled!("flag-name", default)` resolves through the
    //   per-process cache backed by the `features` table.
    // * `admin::upsert` / `admin::delete` propagate to the live
    //   evaluator before returning (sub-second kill-switch semantics).
    // * `FeatureMiddleware` below opens a per-request context with the
    //   authenticated user_id; no team extraction wired in this app
    //   since we don't yet have a multi-tenant story.
    //
    // 60-second TTL is a sensible default: long enough to amortize the
    // scope-resolution walk for hot paths, short enough that an
    // out-of-band SQL edit (e.g. ops console) reflects within the
    // minute. Operator-initiated changes via admin::* bypass the TTL
    // entirely via the FeatureSync fan-out.
    bootstrap_database_cached(Duration::from_secs(60))
        .await
        .expect("feature-flag chain wired");
}

/// The Inertia configuration this app installs.
///
/// One function rather than an inline literal at the install site so the
/// version the server advertises and the version a protocol test sends
/// can never drift apart — both read it from here.
pub fn inertia_config() -> InertiaConfig {
    InertiaConfig::new()
}

/// The Inertia asset version this app advertises.
///
/// A request carrying a different `X-Inertia-Version` gets a 409 telling
/// the client to reload. Public because tests speaking the Inertia
/// protocol have to send the value the server will accept. Resolved
/// rather than hardcoded so it tracks the built frontend: with a Vite
/// manifest present it is that manifest's hash, and without one it is
/// the framework's static fallback.
pub fn inertia_version() -> String {
    inertia_config().version.resolve()
}

/// Register the global middleware chain, in order.
///
/// This is now both the test-harness entry point and the
/// `.http_bootstrap` hook wired in `cmd/main.rs` — the worker subcommands
/// and the console binary never call it, since they only run
/// [`register`]. That is what lets a worker or console container image
/// boot without a built frontend manifest: `Inertia::install` below fails
/// closed in production when `public/assets/.vite/manifest.json` is
/// missing, which is exactly the state a worker image is in by design.
///
/// Split out of [`register`] for the same reason as
/// [`register_storage_disks`]: a test harness needs the real stack without
/// the rest of bootstrap — `DB::init`, event listeners, the feature-flag
/// chain. Every HTTP test in this crate used to build
/// `MiddlewareRegistry::new()` by hand, so no global middleware had ever
/// run in one: session, CSRF and the feature context were absent from all
/// of them, and a test believing it exercised an authenticated route was
/// exercising a bare handler. Tests now call this and
/// `MiddlewareRegistry::from_global()`.
///
/// `Inertia::install` belongs here rather than beside the other container
/// wiring because it registers four middlewares of its own — hoisting the
/// `global_middleware!` calls around it would silently move the Inertia
/// layer to the front of the chain. It also needs to sit *after*
/// `SessionMiddleware`: the version-mismatch bounce re-flashes the
/// session before its `409`, and the empty-response substitution reads
/// the previous URL off the session for its redirect-back target, so both
/// are no-ops unless a session scope is already open around them.
///
/// Ordering, and why each sits where it does:
///
/// 1. `LoggingMiddleware` — outermost, so everything below is logged.
/// 2. `TimeoutMiddleware` — a 30s ceiling on time-to-response so a slow
///    handler or hung query cannot hold a connection open indefinitely.
///    Right after logging, so timed-out 503s are still logged on the way
///    out, and ahead of the rest so it bounds session/feature/handler
///    work. Streaming responses (SSE) and WebSocket upgrades are exempt by
///    design — see `suprnova::timeout`. A per-route
///    `.middleware(TimeoutMiddleware::seconds(n))` tightens one endpoint.
/// 3. `IncludeMiddleware` — scopes `?include=` and `?fields[type]=` into
///    task-local state for JSON:API resource handlers.
/// 4. `SessionMiddleware` — global so every route shares one session
///    lifecycle. `AuthMiddleware` and `Auth::user_as` both read state it
///    sets up, which is what makes auth-aware controllers work. Ahead of
///    the Inertia protocol middleware for the reason above.
/// 5. Inertia protocol, four middlewares registered together by
///    `Inertia::install`: `Vary: X-Inertia` on every response (outermost
///    of the four, so it also covers the `409` below); an empty `200`
///    on an Inertia visit substituted with a `303` back; `409` +
///    `X-Inertia-Location` on an `X-Inertia-Version` mismatch, re-flashing
///    the session first so a flashed error survives the client's
///    follow-up full-page GET; `302` → `303` on non-GET Inertia
///    redirects; and, innermost, a `422` carrying an `errors` object
///    turned into the redirect-back the Inertia client expects, so a
///    failed validation restores the form with its messages rather than
///    surfacing a raw `422`. `inertia_config()` installs the default version
///    resolver — a hash of the Vite build manifest — so the version
///    string tracks the built frontend rather than a literal that
///    someone has to remember to bump.
/// 6. `LocaleMiddleware` — immediately after the session it depends on:
///    detection runs Session -> Cookie -> Header, and the session slot
///    it reads first only exists once `SessionMiddleware` has run.
///    Scopes the detected locale (via `scope_locale`) for the rest of
///    the request, so `Lang::get` / `__!` / translated validation
///    messages all resolve against it downstream. `LocaleShare` — the
///    Inertia `lang` shared prop — is not part of the middleware chain;
///    it is registered in [`register`] alongside `AppSharedData`, since
///    Inertia shared-data providers run inside the Inertia response path
///    rather than as global middleware.
/// 7. `CsrfMiddleware` — immediately after the session it depends on.
///    `/api/ping`, `/api/welcome` and `/lang-demo` are excepted as
///    stateless demo endpoints with nothing ambient for a cross-site
///    POST to abuse. Every cookie-authenticated state change stays
///    protected, including `POST /api/posts` and `DELETE
///    /api/posts/{id}`.
/// 8. `FeatureMiddleware` — opens a `featureflag::Context` per request so
///    user-scoped `is_enabled!` calls resolve against the right scope.
///    After the session, so `Auth::id()` returns the live user id.
pub fn register_http_stack() {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(suprnova::TimeoutMiddleware::default());
    global_middleware!(IncludeMiddleware);

    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));

    Inertia::install(&inertia_config())
        .expect("Inertia install failed (CFG-01: fails closed in production without a built frontend manifest)");

    global_middleware!(
        LocaleMiddleware::from_env().expect("locale config (APP_LOCALE / APP_FALLBACK_LOCALE)")
    );

    global_middleware!(CsrfMiddleware::new().except(vec![
        "/api/ping",
        "/api/welcome",
        "/lang-demo"
    ]));

    global_middleware!(FeatureMiddleware::new());
}

/// Register the application's storage disks.
///
/// `public` is a local-filesystem disk rooted at `./storage/public`, suitable
/// for development. `uploads` is an S3-backed disk that is only registered
/// when `S3_BUCKET` is set in the environment — production deployments wire
/// it via env vars, while local dev and tests skip it.
///
/// Split out of `register()` so test harnesses can re-target the `public`
/// disk to a tempdir without re-running the rest of bootstrap.
pub fn register_storage_disks() {
    Storage::register_fs("public", "./storage/public").expect("register public disk");

    // Pre-create the `avatars/` subdirectory so the first upload to a
    // freshly-cloned checkout doesn't 500 on a missing parent. opendal's
    // fs service creates files but not intermediate dirs.
    std::fs::create_dir_all("./storage/public/avatars").ok();

    if let Ok(bucket) = std::env::var("S3_BUCKET") {
        Storage::register_s3(
            "uploads",
            S3Config {
                bucket,
                region: std::env::var("AWS_REGION").ok(),
                endpoint: std::env::var("S3_ENDPOINT").ok(),
                access_key_id: std::env::var("AWS_ACCESS_KEY_ID").ok(),
                secret_access_key: std::env::var("AWS_SECRET_ACCESS_KEY").ok(),
                root: std::env::var("S3_ROOT").ok(),
            },
        )
        .expect("register S3 uploads disk");
    }
}

/// Per-request Inertia shared data. Demonstrates the request-aware
/// pattern for things like the authenticated user, locale, or flash.
///
/// In a real app this would call `Auth::user()` to surface the current
/// user; for the dogfood test bed we synthesize a placeholder so the
/// frontend can render the layout without depending on the auth flow.
struct AppSharedData;

#[suprnova::async_trait]
impl InertiaSharedData for AppSharedData {
    async fn share(
        &self,
        _req: &dyn InertiaRequestExt,
        _component: &str,
    ) -> Result<suprnova::indexmap::IndexMap<String, Prop>, FrameworkError> {
        let mut shared = suprnova::indexmap::IndexMap::new();
        shared.insert(
            "auth".to_string(),
            Prop::eager(suprnova::serde_json::json!({
                "user": {
                    "name": "Demo User",
                    "email": "demo@suprnova.app",
                }
            })),
        );
        Ok(shared)
    }
}
