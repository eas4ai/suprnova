//! Live components and routes.
//!
//! `registry()` builds the immutable registry of every Live component;
//! `bootstrap::register()` binds it so the server, the workers, and the
//! `suprnova live:*` commands all see the same components. `routes()` installs
//! the reserved Live routes behind this application's session authentication,
//! tenant, and rate-limit middleware; register document routes and mounts
//! there as well.
//!
//! `suprnova live:make <name>` adds a component module and registers it here.

use std::sync::Arc;
use std::time::Duration;

use suprnova::live::{LiveRegistry, LiveTenantMiddleware, LiveTenantResolver, RegistryError};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::{
    AuthMiddleware, FrameworkError, RateLimitMiddleware, Request, Router, SlidingWindowConfig,
    async_trait,
};

/// Builds the registry of every Live component in this application.
pub fn registry() -> Result<LiveRegistry, RegistryError> {
    let registry = LiveRegistry::builder()
        .build();
    Ok(registry)
}

/// Installs the reserved Live routes and every Live document mount.
///
/// The reserved action, upload, and asynchronous routes require a signed-in
/// principal, a tenant decision, and a rate-limit decision; the guard attaches
/// exactly those middleware to them. Register a document route with
/// `router.get(...)` and its islands with `try_live_mount(&mount)` after the
/// install.
pub fn routes(router: Router) -> Result<Router, FrameworkError> {
    let limiter = Arc::new(InMemoryRateLimiter::new());
    router.try_live_with(|guard| {
        guard
            .middleware(AuthMiddleware::new())
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
    })
}

/// This application serves one tenant, so every Live request is tenantless.
struct SingleTenant;

#[async_trait]
impl LiveTenantResolver for SingleTenant {
    async fn resolve(&self, _request: &Request) -> Result<Option<String>, FrameworkError> {
        Ok(None)
    }
}
