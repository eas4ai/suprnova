//! Live dogfood surface: the component registry, the guarded reserved routes,
//! the application-owned upload reacquisition route, and two document routes.
//!
//! `bootstrap::register()` binds [`registry`] together with the upload host and
//! the gates in [`providers`]; `cmd/main.rs` installs
//! [`routes_with_render_cache`] through `Application::try_routes_async`, so the
//! server, the workers, and the `suprnova live:*` commands all see the same
//! components.

pub mod components;
pub mod pages;
pub mod providers;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use suprnova::live::{LiveMount, LiveRegistry, LiveTenantMiddleware, RegistryError};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::render_cache::{
    FreshnessPolicy, RenderCache, RenderCacheConfig, RenderCachePolicy, RepresentationClass,
    StorageLayers, VarianceDimension,
};
use suprnova::{
    AuthMiddleware, BackendErrorPolicy, FrameworkError, RateLimitMiddleware, RateLimiterDriver,
    Request, Router, SlidingWindowConfig, container::App,
};

use components::activity_feed::ActivityFeed;
use components::avatar_uploader::AvatarUploader;
use components::counter::Counter;
use providers::tenant::SingleTenant;

/// The authenticated dashboard document.
pub const DASHBOARD_PATH: &str = "/live";
/// The public document with one public seed.
pub const PUBLIC_PATH: &str = "/live/public";
/// The public todo listing, rendered from the ORM.
pub const TODOS_PATH: &str = "/live/todos";
/// The signed-in visitor's own account document.
pub const ME_PATH: &str = "/live/me";
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

/// One limiter shared by every Live route, resolved once: separate
/// instances would give each route its own quota.
fn limiter() -> Arc<dyn RateLimiterDriver> {
    static LIMITER: OnceLock<Arc<dyn RateLimiterDriver>> = OnceLock::new();
    Arc::clone(LIMITER.get_or_init(|| {
        App::resolve_make::<dyn RateLimiterDriver>()
            .unwrap_or_else(|_| Arc::new(InMemoryRateLimiter::new()))
    }))
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
    .on_backend_error(BackendErrorPolicy::FailClosed)
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

    // Optional authentication: a signed-in principal is recorded, an
    // anonymous visitor continues, and the mount kind decides. Public seeds
    // accept the anonymous action; identity-bound islands refuse it.
    let router = router.try_live_with(|guard| {
        guard
            .middleware(AuthMiddleware::optional())
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
    let router = router.try_live_mount(&public.counter)?;

    // The todo listing and the account page mount no island; they are
    // ordinary document routes of this Live surface, registered here so
    // every route the RenderCache policies below name is declared in one
    // place. `/live/todos` is open to anyone; `/live/me` carries the same
    // login redirect the dashboard does, so an anonymous visitor is sent to
    // the login page rather than shown somebody's account.
    let router: Router = router.get(TODOS_PATH, pages::todos).into();
    let router: Router = router
        .get(ME_PATH, pages::me)
        .middleware(AuthMiddleware::redirect_to("/login"))
        .middleware(tenant())
        .into();

    // The public document is the one route here whose representation is the
    // same for every visitor: its template reads no translation, no feature
    // flag, and no session, and its single island is a public seed rather
    // than an identity-bound instance. So it declares `PublicShared` and no
    // variance beyond the default. Freshness is five minutes with a minute
    // of stale service and five minutes of stale-on-error; the seed's own
    // promotion deadline bounds the served `max-age` underneath that. A
    // signed-in visitor is served the same stored representation, which is
    // correct for this route and is exactly what `PublicShared` means; a
    // render that ever did observe an identity would be declined from
    // storage rather than published, so the declaration cannot become a
    // leak by drift.
    let router = router.try_render_cache(
        PUBLIC_PATH,
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
            .build()?,
    )?;

    // The todo listing is the same declaration as the public document and
    // for the same reason - one representation for every visitor, no
    // session, no translation, no feature flag - with one difference that
    // is the point of having it: its content comes from the `todos` table,
    // so the entry stops being current the moment a model write to that
    // table advances the table's generation, not when its five minutes run
    // out. Five minutes fresh, a minute of stale service (during which the
    // stored copy is served immediately and refreshed behind the request),
    // and five minutes of stale-on-error.
    //
    // `l0_and_l1` rather than the builder's in-process default: this is
    // the one document here that every node of a deployment can share
    // byte for byte, so a shared L1 tier is worth populating. Under the
    // embedded profile, where L1 is disabled, declaring it changes
    // nothing.
    let router = router.try_render_cache(
        TODOS_PATH,
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
            .layers(StorageLayers::l0_and_l1())
            .build()?,
    )?;

    // The account page is the one route here whose stored bytes belong to
    // one person. `PrivateCached` says so, and `vary(Principal)` is what
    // makes it safe rather than merely declared: the lookup key carries
    // opaque per-principal material, so two signed-in visitors never share
    // an entry and an anonymous visitor - who is redirected before the
    // handler runs - never derives one at all. The class is also refused at
    // build time without `Principal` or `Tenant` variance, so this pairing
    // is not a convention that could drift. Freshness is a minute with no
    // stale service: a private entry has no stale band in any case (its
    // dead edge is its fresh edge), and declaring one would only read as a
    // promise the cache does not keep.
    let router = router.try_render_cache(
        ME_PATH,
        RenderCachePolicy::builder(RepresentationClass::PrivateCached)
            .freshness(FreshnessPolicy::new(60_000, 0, 0)?)
            .vary(VarianceDimension::Principal)
            .build()?,
    )?;

    // The dashboard is the stitched route. Its shell - everything outside
    // the three islands - reads nothing private: `live/dashboard.html` is a
    // static template with no translation, no feature flag, and no session
    // read, and the handler hands it only the markup the mounts produced.
    // The islands themselves are identity-bound, so none of their bytes may
    // be shared, and none of them are: `PublicShellStitched` stores the
    // shell alone and re-mounts every island on every hit, under authority
    // derived for that request. The login redirect in front of the route is
    // a gate read, not a content read, and it runs again on every hit
    // because a stitched hit is served only after the route's own chain has
    // had its say - an anonymous visitor is redirected exactly as on a
    // miss. Freshness matches the public document (five minutes fresh, a
    // minute of stale service, five minutes of stale-on-error), and the
    // class refuses `s-maxage` outright, so no shared proxy is ever told to
    // keep bytes that were assembled for one principal.
    router.try_render_cache(
        DASHBOARD_PATH,
        RenderCachePolicy::builder(RepresentationClass::PublicShellStitched)
            .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
            .build()?,
    )
}

/// [`routes`] followed by the RenderCache middleware. This is the entry
/// point every server in this application uses.
///
/// Separate from [`routes`] rather than the last line of it, because
/// `RenderCache::install` is `async` (it probes for the generation ledger's
/// tables before assembling a runtime) while [`routes`] itself is not.
/// `cmd/main.rs` reaches it through `Application::try_routes_async`, the
/// asynchronous route hook that exists for exactly this shape, so the
/// `suprnova serve` binary installs the middleware; so do the browser
/// scenario's server in `examples/live_dogfood_host.rs` and the Live test
/// harness, which both already had a runtime to await on. [`routes`] stays
/// public and synchronous as the inner half: it registers the reserved Live
/// routes, the document route, and the cache policy, and installs no
/// middleware.
///
/// Ordering matters and is the caller's responsibility:
/// `register_global_middleware` appends, so this must run *after*
/// `bootstrap::register_http_stack`, whose session, locale, and feature
/// middleware establish the request-scoped state the cache middleware reads
/// while building a lookup key.
pub async fn routes_with_render_cache(router: Router) -> Result<Router, FrameworkError> {
    routes_with_render_cache_with_config(router, RenderCacheConfig::from_env()?).await
}

/// [`routes_with_render_cache`] over a caller-supplied configuration.
///
/// `#[doc(hidden)]`: a test seam, not part of this application's surface.
/// Every server here reaches the cache through [`routes_with_render_cache`],
/// which reads the environment; a test that needs to drive a freshness band
/// has no other way in, because the clock the runtime reads is settable only
/// on a `RenderCacheConfig` and `from_env` never sets it. Splitting the
/// configuration out rather than duplicating the install keeps the two on
/// one code path: a test installs the same routes, the same policies, and
/// the same middleware ordering the server does, and differs from it in
/// exactly the configuration it passed.
#[doc(hidden)]
pub async fn routes_with_render_cache_with_config(
    router: Router,
    config: RenderCacheConfig,
) -> Result<Router, FrameworkError> {
    RenderCache::install(routes(router)?, config).await
}
