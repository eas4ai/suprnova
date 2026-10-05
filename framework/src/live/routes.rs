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
pub(crate) const LIVE_UPDATE_PATH: &str = "/__live/action";
pub(crate) const LIVE_UPLOAD_PATH: &str = "/__live/upload";
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
    /// Installation performs a collision preflight over the routes registered
    /// so far before adding the versioned update endpoint: literal,
    /// parameterized, and catch-all application routes that can claim
    /// `/__live` cause startup to fail. Register application routes before
    /// installing Live so the preflight sees them.
    /// The reserved request routes carry no application middleware; use
    /// [`Router::try_live_with`] to attach the principal, tenant, and
    /// rate-limit middleware the strict Live policy requires.
    pub fn try_live(self) -> Result<Self, FrameworkError> {
        self.try_live_with(|guard| guard)
    }

    /// Serves the stylesheet and script of every vendored library component
    /// from `templates/suprnova-ui/<component>/` at
    /// `/suprnova-ui/<component>/<file>` (UI-017). Only `.css` and `.js`
    /// files with a closed component name are reachable.
    ///
    /// The directory is resolved under the application base path
    /// (`APP_BASE_PATH`, or the working directory) and read on each request,
    /// so a deployment ships `templates/suprnova-ui/` beside the binary.
    /// Installation fails when the directory cannot be read, so an
    /// application started anywhere else refuses to start instead of
    /// answering 404 for every component asset (UI-021).
    ///
    /// This is the one call for the shipped library's namespace, `suprnova`;
    /// a third-party library's components are served by
    /// [`Router::try_live_ui_assets_for`].
    pub fn try_live_ui_assets(self) -> Result<Self, FrameworkError> {
        self.try_live_ui_assets_from(
            crate::app::paths::base_path("templates").join(super::ui_assets::LIVE_UI_TEMPLATE_ROOT),
        )
    }

    /// The same route over an explicit component directory, for hosts whose
    /// template root is not the process base path. Installation fails when
    /// the directory cannot be read.
    pub fn try_live_ui_assets_from(
        self,
        root: impl Into<std::path::PathBuf>,
    ) -> Result<Self, FrameworkError> {
        install_ui_assets(
            self,
            super::ui_assets::LIVE_UI_ASSET_ROUTE,
            super::ui_assets::LIVE_UI_TEMPLATE_ROOT,
            root.into(),
        )
    }

    /// Serves the stylesheet and script of every component a third-party
    /// library installed under `namespace`, from
    /// `templates/<namespace>-ui/<component>/` at
    /// `/<namespace>-ui/<component>/<file>` (REG-017), under the contract
    /// [`Router::try_live_ui_assets`] has: closed component and file names,
    /// `.css` and `.js` only, at most 1 MiB, with an ETag, and nothing
    /// reached through a symbolic link below the root.
    ///
    /// Each library needs its own call, and `live:add` names it when it
    /// installs the library's first component. The namespace must pass the
    /// rule `live:add` applies (1 to 32 bytes of lowercase letters, digits
    /// and hyphens, starting with a letter, whose module form is not a Rust
    /// keyword), and the reserved `suprnova`, `sn` and `live` are refused:
    /// the shipped library's one call is [`Router::try_live_ui_assets`].
    /// Installation fails when the directory under the application base
    /// path cannot be read, or when the namespace's route is already
    /// installed.
    pub fn try_live_ui_assets_for(self, namespace: &str) -> Result<Self, FrameworkError> {
        let namespace = super::ui_assets::LibraryNamespace::parse(namespace)?;
        let root = crate::app::paths::base_path("templates").join(namespace.template_root());
        install_ui_assets(self, namespace.route(), namespace.template_root(), root)
    }

    /// The namespace's route over an explicit component directory, for
    /// hosts whose template root is not the process base path, such as a
    /// library's preview application serving its own `components/`. The
    /// namespace is checked as [`Router::try_live_ui_assets_for`] checks it,
    /// and installation fails when the directory cannot be read.
    pub fn try_live_ui_assets_for_from(
        self,
        namespace: &str,
        root: impl Into<std::path::PathBuf>,
    ) -> Result<Self, FrameworkError> {
        let namespace = super::ui_assets::LibraryNamespace::parse(namespace)?;
        install_ui_assets(
            self,
            namespace.route(),
            namespace.template_root(),
            root.into(),
        )
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

/// Installs one library's asset route over `root`. A directory that cannot
/// be read is refused at startup, naming it, so an application started
/// away from its templates fails loudly instead of answering 404 for every
/// asset (UI-021).
fn install_ui_assets(
    router: Router,
    route: &str,
    template_root: &str,
    root: std::path::PathBuf,
) -> Result<Router, FrameworkError> {
    if let Err(error) = std::fs::read_dir(&root) {
        return Err(FrameworkError::internal(format!(
            "cannot read the vendored Live component directory {}: {error}; start the application from its project directory, set APP_BASE_PATH to that directory, or ship templates/{template_root} with the binary",
            root.display()
        )));
    }
    let assets = std::sync::Arc::new(super::ui_assets::LiveUiAssets::from_root(root));
    let router: Router = router
        .try_methods(&LIVE_HTTP_METHODS, route, move |request: crate::Request| {
            let assets = std::sync::Arc::clone(&assets);
            async move { assets.serve(request).await }
        })?
        .into();
    Ok(router)
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
