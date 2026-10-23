//! A CDN driver changes an origin prefix and purges both page representations.

use crate::FrameworkError;
use async_trait::async_trait;
use std::sync::Mutex;

#[cfg(feature = "cdn-cloudfront")]
mod cloudfront;
#[cfg(feature = "cdn-cloudfront")]
pub use cloudfront::{CloudFrontDriver, RESPONSE_HEADERS_FUNCTION};

#[cfg(feature = "cdn-cloudflare")]
mod cloudflare;
#[cfg(feature = "cdn-cloudflare")]
pub use cloudflare::CloudflareDriver;

/// Purge operations a driver implements without silently discarding a request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CdnCapabilities {
    /// The provider understands dependency tags.
    pub tags: bool,
    /// The provider invalidates exact public URLs.
    pub urls: bool,
    /// The provider invalidates an origin prefix.
    pub prefix: bool,
}

/// One invalidation request, selected according to provider capabilities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Purge {
    /// Invalidate every representation carrying one of these tags.
    Tags(Vec<String>),
    /// Invalidate both HTML and JSON URLs explicitly.
    Urls(Vec<String>),
    /// Invalidate everything below this prefix.
    Prefix(String),
}

/// The sole publish-side contract for CloudFront and application CDN drivers.
#[async_trait]
pub trait CdnDriver: Send + Sync + 'static {
    /// Advertises supported invalidation operations.
    fn capabilities(&self) -> CdnCapabilities;
    /// Whether the provider serves the live version through the bucket's stable
    /// mirror (`<path>/index.html`, `<path>/index.json`) instead of through a
    /// version switch of its own, as an R2 custom domain does. The publisher then
    /// writes the mirror after every manifest commit and before the purge, so no
    /// edge refetches an old copy and keeps it; a provider that switches an origin
    /// path, as CloudFront does, leaves this `false` and gets no mirror.
    fn serves_mirror(&self) -> bool {
        false
    }
    /// Rejects response headers the provider cannot preserve before any upload or switch.
    fn validate_headers(
        &self,
        _headers: &std::collections::BTreeMap<String, String>,
    ) -> Result<(), FrameworkError> {
        Ok(())
    }
    /// Reads provider selection so recovery does not trust another process's cached status.
    /// Unknown selection requires realignment with the durable manifest before rendering.
    async fn selected_version(&self) -> Result<Option<String>, FrameworkError> {
        Ok(None)
    }
    /// Points the page origin at `_pub/<version>/` after uploads finish.
    async fn switch_version(&self, version: &str) -> Result<(), FrameworkError>;
    /// Restores the previous live revision after a failed switch or manifest write.
    /// `None` means the first publication failed and the original provider configuration must return.
    async fn rollback_version(&self, previous: Option<&str>) -> Result<(), FrameworkError> {
        match previous {
            Some(version) => self.switch_version(version).await,
            None => Err(super::error(
                "CDN driver cannot restore its initial configuration",
            )),
        }
    }
    /// Invalidates a completed regeneration, never a transaction in progress.
    async fn purge(&self, request: Purge) -> Result<(), FrameworkError>;
}

/// A deterministic driver that records switches and purges without network I/O.
#[derive(Default)]
pub struct MemoryCdnDriver {
    events: Mutex<Vec<String>>,
    selected: Mutex<Option<String>>,
}

impl MemoryCdnDriver {
    /// Reads ordered operations so callers can assert switch and purge ordering.
    pub fn events(&self) -> Result<Vec<String>, FrameworkError> {
        self.events
            .lock()
            .map(|events| events.clone())
            .map_err(|_| super::error("CDN event lock poisoned"))
    }
}

#[async_trait]
impl CdnDriver for MemoryCdnDriver {
    fn capabilities(&self) -> CdnCapabilities {
        CdnCapabilities {
            tags: true,
            urls: true,
            prefix: true,
        }
    }
    async fn selected_version(&self) -> Result<Option<String>, FrameworkError> {
        self.selected
            .lock()
            .map(|selected| selected.clone())
            .map_err(|_| super::error("CDN selection lock poisoned"))
    }
    async fn switch_version(&self, version: &str) -> Result<(), FrameworkError> {
        *self
            .selected
            .lock()
            .map_err(|_| super::error("CDN selection lock poisoned"))? = Some(version.to_string());
        self.events
            .lock()
            .map_err(|_| super::error("CDN event lock poisoned"))?
            .push(format!("switch:{version}"));
        Ok(())
    }
    async fn rollback_version(&self, previous: Option<&str>) -> Result<(), FrameworkError> {
        *self
            .selected
            .lock()
            .map_err(|_| super::error("CDN selection lock poisoned"))? =
            previous.map(str::to_string);
        self.events
            .lock()
            .map_err(|_| super::error("CDN event lock poisoned"))?
            .push(format!("rollback:{}", previous.unwrap_or("initial")));
        Ok(())
    }
    async fn purge(&self, request: Purge) -> Result<(), FrameworkError> {
        self.events
            .lock()
            .map_err(|_| super::error("CDN event lock poisoned"))?
            .push(format!("purge:{request:?}"));
        Ok(())
    }
}
