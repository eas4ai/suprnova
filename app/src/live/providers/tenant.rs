//! This application serves one tenant, so every Live request is tenantless.

use suprnova::live::LiveTenantResolver;
use suprnova::{FrameworkError, Request, async_trait};

/// Resolves no tenant for any request.
pub struct SingleTenant;

#[async_trait]
impl LiveTenantResolver for SingleTenant {
    async fn resolve(&self, _request: &Request) -> Result<Option<String>, FrameworkError> {
        Ok(None)
    }
}
