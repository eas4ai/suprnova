//! Application-configured tenant resolution with framework-owned attestation.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;

use crate::middleware::{Middleware, Next};
use crate::{FrameworkError, Request, Response};

use super::attestation::SecurityCheck;

/// Resolves the current application tenant from already-normalized request data.
///
/// Implementations may consult route parameters, authenticated principal state,
/// or application services. A resolver must not treat an untrusted header as
/// authoritative without validating it against application policy.
///
/// `Ok(None)` is a positive statement that this request has no tenant, and
/// the middleware records the tenant check as not required: an identity-bound
/// island then binds session and principal alone. A resolver that cannot
/// determine the tenant must return `Err`, never `Ok(None)`, so the request
/// fails instead of mounting untenanted.
#[async_trait]
pub trait LiveTenantResolver: Send + Sync {
    /// Returns the current tenant identity, or `None` for a tenantless request.
    async fn resolve(&self, request: &Request) -> Result<Option<String>, FrameworkError>;
}

tokio::task_local! {
    /// The tenant [`LiveTenantMiddleware`] resolved for this request, or
    /// `None` for a request its resolver called tenantless.
    static CURRENT_TENANT: Option<String>;
}

/// The current request's tenant, as [`LiveTenantMiddleware`] resolved it.
///
/// Records a tenant observation into the active RenderCache collector on
/// every call, and the value when there is one, exactly as
/// `Request::live_tenant` does, so a global scope or a gate body that
/// filters by tenant is visible to RenderCache. A gate closure receives no
/// `Request`, so this is the only instrumented tenant accessor such a body
/// can reach.
///
/// Returns `None`, and records a tenant read with no value, when no
/// middleware installed a tenant for this task. The bare read is
/// deliberate: a render that asked for the tenant and found none still
/// depends on the answer being none, and a route declaring no `Tenant`
/// dimension must decline rather than publish that answer for everyone.
#[must_use]
pub fn current_tenant() -> Option<String> {
    crate::render_cache::collector::observe_tenant_read();
    let resolved = CURRENT_TENANT
        .try_with(|tenant| tenant.clone())
        .unwrap_or(None);
    if let Some(tenant) = resolved.as_deref() {
        crate::render_cache::collector::observe_tenant_value(tenant);
    }
    resolved
}

/// Route middleware that turns a configured resolver outcome into Live tenant evidence.
pub struct LiveTenantMiddleware {
    resolver: Arc<dyn LiveTenantResolver>,
}

impl LiveTenantMiddleware {
    /// Creates tenant middleware backed by the application's resolver.
    #[must_use]
    pub fn new(resolver: Arc<dyn LiveTenantResolver>) -> Self {
        Self { resolver }
    }
}

impl fmt::Debug for LiveTenantMiddleware {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<LiveTenantMiddleware:redacted>")
    }
}

#[async_trait]
impl Middleware for LiveTenantMiddleware {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        let resolved = if let Some(tenant) = self.resolver.resolve(&request).await? {
            let tenant = tenant.trim();
            if tenant.is_empty() || tenant.len() > 512 {
                return Err(crate::http::HttpResponse::text("Invalid tenant context").status(400));
            }
            let tenant = tenant.to_owned();
            request.set_live_tenant(tenant.clone());
            request.record_live_security_check(SecurityCheck::Tenant, Some(tenant.as_bytes()));
            Some(tenant)
        } else {
            request.record_live_security_not_required(
                SecurityCheck::Tenant,
                suprnova_live::host::PolicyReason::TenantlessRoute,
            );
            None
        };
        // Scoped around the rest of the chain rather than stored on the
        // request: a gate closure and a global scope's `apply` both run
        // without a `Request` in hand, and this is the seam that reaches
        // them. `Request::live_tenant` is unchanged and still the accessor
        // a handler with a request should use.
        CURRENT_TENANT.scope(resolved, next(request)).await
    }
}
