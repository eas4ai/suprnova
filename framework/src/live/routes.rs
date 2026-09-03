//! Atomic installation of the framework-owned Live HTTP namespace.

use std::fmt;

use crate::middleware::{BoxedMiddleware, Middleware, into_boxed};
use crate::routing::MultiMethodRouteBuilder;
use hyper::Method;

use crate::ws::{OriginPolicy, WsConfig};
use crate::{FrameworkError, Router};

use super::async_updates::{
    LIVE_ASYNC_EVENTS_PATH, LIVE_ASYNC_MEMBERSHIP_PATH, LIVE_ASYNC_SOCKET_PATH,
    LIVE_ASYNC_SUBSCRIPTION_PATH,
};
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

/// Application middleware attached to every reserved Live request route.
///
/// The action, upload, asynchronous control, and WebSocket handshake routes
/// carry the strict Live policy: session, origin, CSRF, principal, tenant, and
/// rate-limit facts must all be present. Framework middleware records the
/// session and the configured CSRF proof globally; the principal, tenant, and
/// rate-limit facts come from the application's own middleware, which this
/// guard attaches to exactly those routes in the given order. The immutable
/// asset routes never carry the guard.
#[derive(Default)]
pub struct LiveRouteGuard {
    middleware: Vec<BoxedMiddleware>,
}

impl LiveRouteGuard {
    /// Appends one middleware to every reserved Live request route.
    #[must_use]
    pub fn middleware<M: Middleware + 'static>(mut self, middleware: M) -> Self {
        self.middleware.push(into_boxed(middleware));
        self
    }

    fn apply(&self, builder: MultiMethodRouteBuilder) -> Router {
        self.middleware
            .iter()
            .cloned()
            .fold(builder, MultiMethodRouteBuilder::middleware_boxed)
            .into()
    }
}

impl fmt::Debug for LiveRouteGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveRouteGuard")
            .field("middleware", &self.middleware.len())
            .finish()
    }
}

impl Router {
    /// Installs Suprnova Live's reserved HTTP namespace exactly once.
    ///
    /// Installation performs a complete collision preflight before adding
    /// the versioned update endpoint. Literal, parameterized, and catch-all
    /// application routes that can claim `/__live` cause startup to fail.
    /// The reserved request routes carry no application middleware; use
    /// [`Router::try_live_with`] to attach the principal, tenant, and
    /// rate-limit middleware the strict Live policy requires.
    pub fn try_live(self) -> Result<Self, FrameworkError> {
        self.try_live_with(|guard| guard)
    }

    /// Installs the reserved namespace with application middleware on every
    /// Live request route.
    ///
    /// ```rust,no_run
    /// use suprnova::{AuthMiddleware, Router};
    ///
    /// # fn main() -> Result<(), suprnova::FrameworkError> {
    /// let router = Router::new().try_live_with(|guard| guard.middleware(AuthMiddleware::new()))?;
    /// # let _ = router;
    /// # Ok(())
    /// # }
    /// ```
    pub fn try_live_with<F>(self, configure: F) -> Result<Self, FrameworkError>
    where
        F: FnOnce(LiveRouteGuard) -> LiveRouteGuard,
    {
        install(self, &configure(LiveRouteGuard::default()))
    }
}

fn install(mut router: Router, guard: &LiveRouteGuard) -> Result<Router, FrameworkError> {
    router.preflight_live_installation(LIVE_ROUTE_VERSION)?;

    router = guard.apply(router.try_methods(
        &LIVE_HTTP_METHODS,
        LIVE_UPDATE_PATH,
        super::action::handle,
    )?);
    for method in LIVE_HTTP_METHODS {
        router.register_live_route_metadata(
            method,
            LIVE_UPDATE_PATH,
            LiveRouteMetadata::new(LiveOperation::Action, strict_action_policy()),
        )?;
    }
    router = guard.apply(router.try_methods(
        &LIVE_HTTP_METHODS,
        LIVE_UPLOAD_PATH,
        super::upload::handle,
    )?);
    for method in LIVE_HTTP_METHODS {
        router.register_live_route_metadata(
            method,
            LIVE_UPLOAD_PATH,
            LiveRouteMetadata::new(LiveOperation::Upload, strict_action_policy()),
        )?;
    }
    router = guard.apply(router.try_methods(
        &LIVE_HTTP_METHODS,
        LIVE_ASYNC_SUBSCRIPTION_PATH,
        super::async_transport::subscriptions,
    )?);
    router = guard.apply(router.try_methods(
        &LIVE_HTTP_METHODS,
        LIVE_ASYNC_MEMBERSHIP_PATH,
        super::async_transport::memberships,
    )?);
    router = guard.apply(router.try_methods(
        &LIVE_HTTP_METHODS,
        LIVE_ASYNC_EVENTS_PATH,
        super::async_transport::events,
    )?);
    for path in [
        LIVE_ASYNC_SUBSCRIPTION_PATH,
        LIVE_ASYNC_MEMBERSHIP_PATH,
        LIVE_ASYNC_EVENTS_PATH,
    ] {
        for method in LIVE_HTTP_METHODS {
            router.register_live_route_metadata(
                method,
                path,
                LiveRouteMetadata::new(LiveOperation::SseControl, strict_action_policy()),
            )?;
        }
    }
    router = router.try_ws_boxed_with_middleware_and_config(
        LIVE_ASYNC_SOCKET_PATH,
        std::sync::Arc::new(super::async_transport::AsyncSocketHandler),
        guard.middleware.clone(),
        Some(WsConfig {
            origin_policy: OriginPolicy::SameOrigin,
            ..WsConfig::default()
        }),
    )?;
    router.register_live_route_metadata(
        Method::GET,
        LIVE_ASYNC_SOCKET_PATH,
        LiveRouteMetadata::new(LiveOperation::WebSocketHandshake, strict_action_policy()),
    )?;
    router = router
        .try_methods(
            &LIVE_HTTP_METHODS,
            super::assets::LIVE_ASSET_ROUTE,
            super::assets::handle,
        )?
        .into();
    router = router
        .try_methods(
            &LIVE_HTTP_METHODS,
            super::assets::LIVE_ASSET_MISS_ROUTE,
            super::assets::handle_miss,
        )?
        .into();
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
