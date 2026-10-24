//! The Live components of the `{namespace}` library, and the routes that
//! preview them.
//!
//! `registry()` registers every Live component the library defines. Each is
//! compiled from its own file under `../components/` (see
//! `{namespace_module}/mod.rs`), so nothing is copied here.
//! `suprnova live:registry check` reads this builder to confirm that every
//! component's `register` entry is registered, under the same full path
//! `suprnova live:add` writes into an application. `routes()` installs the
//! reserved Live routes and the preview pages of `crate::preview`;
//! `routes_with_render_cache()` is what `cmd/main.rs` calls.

use std::sync::Arc;
use std::time::Duration;

use suprnova::live::{LiveRegistry, LiveTenantMiddleware, LiveTenantResolver, RegistryError};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::render_cache::{RenderCache, RenderCacheConfig};
use suprnova::{
    AuthMiddleware, FrameworkError, RateLimitMiddleware, Request, Router, SlidingWindowConfig,
    async_trait,
};

/// Builds the registry of every Live component the library defines.
pub fn registry() -> Result<LiveRegistry, RegistryError> {
    let registry = LiveRegistry::builder()
        .build();
    Ok(registry)
}

/// Installs the reserved Live routes and the preview pages.
///
/// A preview has nobody to sign in, so the guard records a principal when
/// there is one and lets an anonymous visitor continue: the public seeds the
/// preview pages mount then take actions from anyone. The tenant and
/// rate-limit decisions are the ones an application makes.
pub fn routes(router: Router) -> Result<Router, FrameworkError> {
    let limiter = Arc::new(InMemoryRateLimiter::new());
    let router = router.try_live_with(|guard| {
        guard
            .middleware(AuthMiddleware::optional())
            .middleware(LiveTenantMiddleware::new(Arc::new(SingleTenant)))
            .middleware(RateLimitMiddleware::new(
                limiter,
                SlidingWindowConfig {
                    max_requests: 600,
                    window: Duration::from_secs(60),
                },
                |request: &Request| {
                    format!("live:{}", request.ip().unwrap_or_else(|| "anon".into()))
                },
            ))
    })?;
    crate::preview::routes(router)
}

/// `routes()` followed by the RenderCache middleware, as an application
/// installs them. Set `RENDER_CACHE_ENABLED=false` to turn the cache off.
pub async fn routes_with_render_cache(router: Router) -> Result<Router, FrameworkError> {
    RenderCache::install(routes(router)?, RenderCacheConfig::from_env()?).await
}

/// The preview serves one tenant, so every Live request is tenantless.
struct SingleTenant;

#[async_trait]
impl LiveTenantResolver for SingleTenant {
    async fn resolve(&self, _request: &Request) -> Result<Option<String>, FrameworkError> {
        Ok(None)
    }
}
