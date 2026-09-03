//! Live dogfood surface: the component registry, the guarded reserved routes,
//! the application-owned upload reacquisition route, and two document routes.
//!
//! `bootstrap::register()` binds [`registry`] together with the upload host and
//! the gates in [`providers`]; `cmd/main.rs` installs [`routes`] through
//! `Application::try_routes`, so the server, the workers, and the
//! `suprnova live:*` commands all see the same components.

pub mod components;
pub mod pages;
pub mod providers;

use std::sync::Arc;
use std::time::Duration;

use suprnova::live::{LiveMount, LiveRegistry, LiveTenantMiddleware, RegistryError};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::{
    AuthMiddleware, FrameworkError, RateLimitMiddleware, RateLimiterDriver, Request, Router,
    SlidingWindowConfig, container::App,
};

use components::activity_feed::ActivityFeed;
use components::avatar_uploader::AvatarUploader;
use components::counter::Counter;
use providers::tenant::SingleTenant;

/// The authenticated dashboard document.
pub const DASHBOARD_PATH: &str = "/live";
/// The public document with one public seed.
pub const PUBLIC_PATH: &str = "/live/public";
/// The application-owned upload reacquisition route, outside `/__live/`.
pub const REACQUIRE_PATH: &str = "/account/uploads/{handle}/reacquire";

/// Builds the registry of every Live component in this application.
pub fn registry() -> Result<LiveRegistry, RegistryError> {
    let registry = LiveRegistry::builder()
        .register::<Counter>()?
        .register::<AvatarUploader>()?
        .register::<ActivityFeed>()?
        .build();
    Ok(registry)
}

/// The identity-bound islands the dashboard renders.
#[derive(Clone)]
pub struct DashboardMounts {
    /// The counter island.
    pub counter: LiveMount<Counter>,
    /// The avatar uploader island.
    pub uploader: LiveMount<AvatarUploader>,
    /// The activity feed island.
    pub feed: LiveMount<ActivityFeed>,
}

impl DashboardMounts {
    /// Declares the dashboard's islands once; the router and the handler share them.
    pub fn declare() -> Result<Self, FrameworkError> {
        Ok(Self {
            counter: LiveMount::<Counter>::identity_bound(
                DASHBOARD_PATH,
                "counter",
                "dashboard-counter",
            )?,
            uploader: LiveMount::<AvatarUploader>::identity_bound(
                DASHBOARD_PATH,
                "uploader",
                "dashboard-uploader",
            )?,
            feed: LiveMount::<ActivityFeed>::identity_bound(
                DASHBOARD_PATH,
                "feed",
                "dashboard-feed",
            )?,
        })
    }
}

/// The public page's single seed.
#[derive(Clone)]
pub struct PublicMounts {
    /// The public counter island.
    pub counter: LiveMount<Counter>,
}

impl PublicMounts {
    /// Declares the public page's island.
    pub fn declare() -> Result<Self, FrameworkError> {
        Ok(Self {
            counter: LiveMount::<Counter>::public_seed(PUBLIC_PATH, "counter", "public-counter")?,
        })
    }
}

fn limiter() -> Arc<dyn RateLimiterDriver> {
    App::resolve_make::<dyn RateLimiterDriver>()
        .unwrap_or_else(|_| Arc::new(InMemoryRateLimiter::new()))
}

fn live_rate_limit() -> RateLimitMiddleware<impl Fn(&Request) -> String + Send + Sync + 'static> {
    RateLimitMiddleware::new(
        limiter(),
        SlidingWindowConfig {
            max_requests: 600,
            window: Duration::from_secs(60),
        },
        |request: &Request| format!("live:ip:{}", request.ip().unwrap_or_else(|| "anon".into())),
    )
}

fn tenant() -> LiveTenantMiddleware {
    LiveTenantMiddleware::new(Arc::new(SingleTenant))
}

/// Installs the reserved Live routes behind the session, tenant, and
/// rate-limit middleware, the authenticated reacquisition route, and the two
/// document routes with their islands.
pub fn routes(router: Router) -> Result<Router, FrameworkError> {
    let dashboard = DashboardMounts::declare()?;
    let public = PublicMounts::declare()?;

    let router = router.try_live_with(|guard| {
        guard
            .middleware(AuthMiddleware::new())
            .middleware(tenant())
            .middleware(live_rate_limit())
    })?;

    // Reacquisition stays outside the reserved namespace and carries the same
    // strict policy: a signed-in principal, a tenant decision, and a rate fact.
    let router: Router = router
        .try_live_upload_reacquisition(REACQUIRE_PATH)?
        .middleware(AuthMiddleware::new())
        .middleware(tenant())
        .middleware(live_rate_limit())
        .into();

    let handler_mounts = dashboard.clone();
    let router: Router = router
        .get(DASHBOARD_PATH, move |request: Request| {
            let mounts = handler_mounts.clone();
            async move { pages::dashboard(request, &mounts).await }
        })
        .middleware(AuthMiddleware::redirect_to("/login"))
        .middleware(tenant())
        .into();
    let router = router
        .try_live_mount(&dashboard.counter)?
        .try_live_mount(&dashboard.uploader)?
        .try_live_mount(&dashboard.feed)?;

    let handler_mounts = public.clone();
    let router: Router = router
        .get(PUBLIC_PATH, move |request: Request| {
            let mounts = handler_mounts.clone();
            async move { pages::public(request, &mounts).await }
        })
        .into();
    router.try_live_mount(&public.counter)
}
