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
#[async_trait]
pub trait LiveTenantResolver: Send + Sync {
    /// Returns the current tenant identity, or `None` for a tenantless request.
    async fn resolve(&self, request: &Request) -> Result<Option<String>, FrameworkError>;
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
        if let Some(tenant) = self.resolver.resolve(&request).await? {
            let tenant = tenant.trim();
            if tenant.is_empty() || tenant.len() > 512 {
                return Err(crate::http::HttpResponse::text("Invalid tenant context").status(400));
            }
            request.set_live_tenant(tenant.to_owned());
            request.record_live_security_check(SecurityCheck::Tenant, Some(tenant.as_bytes()));
        } else {
            request.record_live_security_not_required(
                SecurityCheck::Tenant,
                suprnova_live::host::PolicyReason::TenantlessRoute,
            );
        }
        next(request).await
    }
}
