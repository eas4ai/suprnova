//! Atomic installation of the framework-owned Live HTTP namespace.

use hyper::Method;

use crate::{FrameworkError, Router};

use super::attestation::LiveOperation;
use super::context::{LiveRouteMetadata, LiveRouteSecurityPolicy};

pub(crate) const LIVE_ROUTE_VERSION: u16 = 1;
pub(crate) const LIVE_UPDATE_PATH: &str = "/__live/v1/action";
pub(crate) const LIVE_UPLOAD_PATH: &str = "/__live/v1/upload";
const LIVE_HTTP_METHODS: [Method; 7] = [
    Method::GET,
    Method::POST,
    Method::PUT,
    Method::PATCH,
    Method::DELETE,
    Method::HEAD,
    Method::OPTIONS,
];

impl Router {
    /// Installs Suprnova Live's reserved HTTP namespace exactly once.
    ///
    /// Installation performs a complete collision preflight before adding
    /// the versioned update endpoint. Literal, parameterized, and catch-all
    /// application routes that can claim `/__live` cause startup to fail.
    pub fn try_live(self) -> Result<Self, FrameworkError> {
        install(self)
    }
}

fn install(mut router: Router) -> Result<Router, FrameworkError> {
    router.preflight_live_installation(LIVE_ROUTE_VERSION)?;

    router = router
        .try_methods(&LIVE_HTTP_METHODS, LIVE_UPDATE_PATH, super::action::handle)?
        .into();
    for method in LIVE_HTTP_METHODS {
        router.register_live_route_metadata(
            method,
            LIVE_UPDATE_PATH,
            LiveRouteMetadata::new(LiveOperation::Action, strict_action_policy()),
        )?;
    }
    router = router
        .try_methods(&LIVE_HTTP_METHODS, LIVE_UPLOAD_PATH, super::upload::handle)?
        .into();
    for method in LIVE_HTTP_METHODS {
        router.register_live_route_metadata(
            method,
            LIVE_UPLOAD_PATH,
            LiveRouteMetadata::new(LiveOperation::Upload, strict_action_policy()),
        )?;
    }
    router.mark_live_installed(LIVE_ROUTE_VERSION);
    Ok(router)
}

pub(crate) const fn strict_action_policy() -> LiveRouteSecurityPolicy {
    LiveRouteSecurityPolicy {
        trusted_internal_origin: false,
        stateless_csrf: false,
        stateless_session: false,
        anonymous_principal: false,
        tenantless: false,
        direct_peer: false,
        upstream_rate_limit: false,
        no_additional_middleware: false,
    }
}
