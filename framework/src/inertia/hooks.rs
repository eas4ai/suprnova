//! The Inertia middleware's decisions, replaceable by the application -
//! Laravel's overridable `Inertia\Middleware` methods.
//!
//! A Laravel application subclasses `HandleInertiaRequests` and overrides
//! `version`, `share`, `shareOnce`, `rootView`, `urlResolver`,
//! `onEmptyResponse`, `onVersionChange` or `onRedirectWithFragment`. Here
//! the same decisions are the methods of [`InertiaMiddlewareHooks`], each
//! defaulting to what the framework does, installed with
//! [`InertiaConfig::hooks`] and run by the middleware stack
//! [`Inertia::install`](crate::Inertia::install) registers or
//! [`Inertia::middleware`](crate::Inertia::middleware) builds for a route
//! group.
//!
//! The stack as one middleware, [`InertiaMiddleware`], lives here too: it is
//! what lets an application put the Inertia layer on the route groups that
//! serve pages and keep it off an API group.

use std::sync::Arc;

use async_trait::async_trait;
use indexmap::IndexMap;

use super::config::InertiaConfig;
use super::prop::{InertiaRequestExt, Prop};
use super::visit::{self, Visit};
use crate::http::{HttpResponse, Request, Response};
use crate::middleware::{BoxedMiddleware, Middleware, MiddlewareFuture, Next, into_boxed};

/// A function deriving the page object's `url` from the request, as the
/// [`InertiaMiddlewareHooks::url_resolver`] hook returns it - the type of
/// [`InertiaConfig::url_resolver`]'s argument, boxed.
pub type PageUrlResolver = Arc<dyn Fn(&dyn InertiaRequestExt) -> String + Send + Sync>;

/// The decisions of the Inertia middleware an application can replace.
///
/// Every method has the framework's behaviour as its default, so an
/// implementation overrides only what it changes; [`DefaultInertiaHooks`]
/// implements none, and an override can call it to start from the
/// framework's answer. Install an implementation with
/// [`InertiaConfig::hooks`].
///
/// The `request` methods run before the handler, once per request through
/// the stack; the `on_*` methods run after it, on an Inertia visit, and
/// receive the response the framework would send.
///
/// ```rust,no_run
/// use suprnova::{HttpResponse, InertiaConfig, InertiaMiddlewareHooks, InertiaVisit};
///
/// struct Hooks;
///
/// impl InertiaMiddlewareHooks for Hooks {
///     // A handler that answers nothing means "done": answer 204, not a
///     // redirect back.
///     fn on_empty_response(&self, _visit: &InertiaVisit, _response: HttpResponse) -> HttpResponse {
///         HttpResponse::new().status(204)
///     }
/// }
///
/// let cfg = InertiaConfig::new().hooks(Hooks);
/// # let _ = cfg;
/// ```
pub trait InertiaMiddlewareHooks: Send + Sync + 'static {
    /// The asset version for this request, compared with the client's
    /// `X-Inertia-Version` and written into the page object. `None`, the
    /// default, keeps the configured version - Laravel's `version()`.
    fn version(&self, request: &dyn InertiaRequestExt) -> Option<String> {
        let _ = request;
        None
    }

    /// Props shared with the page this request renders, beside the
    /// application's shared data - Laravel's `share()`. Empty by default:
    /// the framework adds `errors` itself.
    fn share(&self, request: &dyn InertiaRequestExt) -> IndexMap<String, Prop> {
        let _ = request;
        IndexMap::new()
    }

    /// Props shared as once props: resolved once, then kept by the client
    /// across visits - Laravel's `shareOnce()`. A value that is not already
    /// a once prop is made one. Empty by default.
    fn share_once(&self, request: &dyn InertiaRequestExt) -> IndexMap<String, Prop> {
        let _ = request;
        IndexMap::new()
    }

    /// The configuration the first-visit document of this request is
    /// written with - Laravel's `rootView()`, which picks the root template.
    /// The default writes it with `config` as it is; return a changed
    /// configuration to change the document for this request.
    fn root_view(&self, request: &dyn InertiaRequestExt, config: InertiaConfig) -> InertiaConfig {
        let _ = request;
        config
    }

    /// How the page object's `url` is derived from the request - Laravel's
    /// `urlResolver()`. `None`, the default, keeps the configured
    /// [`InertiaConfig::url_resolver`].
    fn url_resolver(&self) -> Option<PageUrlResolver> {
        None
    }

    /// What answers an empty `200` on an Inertia visit - Laravel's
    /// `onEmptyResponse()`. The default redirects back: the same-origin
    /// `Referer`, else the previous URL, else `/`, with `302`; a `302` this
    /// returns becomes `303` for `PUT`, `PATCH` and `DELETE`.
    fn on_empty_response(&self, visit: &InertiaVisit, response: HttpResponse) -> HttpResponse {
        empty_response_default(visit, response)
    }

    /// What answers an Inertia `GET` whose asset version is stale -
    /// Laravel's `onVersionChange()`. `response` is the framework's `409`
    /// with `X-Inertia-Location`, which the default returns; the handler
    /// has not run.
    fn on_version_change(&self, visit: &InertiaVisit, response: HttpResponse) -> HttpResponse {
        let _ = visit;
        response
    }

    /// What answers a redirect with a `#fragment` on an Inertia visit that
    /// is not a prefetch - Laravel's `onRedirectWithFragment()`. The
    /// default answers `409` with `X-Inertia-Redirect`, which the client
    /// visits with the fragment intact.
    fn on_redirect_with_fragment(
        &self,
        visit: &InertiaVisit,
        response: HttpResponse,
    ) -> HttpResponse {
        let _ = visit;
        redirect_with_fragment_default(response)
    }
}

/// The framework's decisions, with nothing replaced. An override can call
/// a method of this type to start from what the framework would answer.
pub struct DefaultInertiaHooks;

impl InertiaMiddlewareHooks for DefaultInertiaHooks {}

/// The Inertia request an `on_*` hook is answering, captured before the
/// handler consumed it: its method, path, URL and headers.
pub struct InertiaVisit {
    method: hyper::Method,
    path: String,
    path_and_query: String,
    full_url: String,
    headers: hyper::HeaderMap,
    visit: Arc<Visit>,
}

impl InertiaVisit {
    /// Capture `request`, with the visit facts the middleware scoped.
    pub(crate) fn capture(request: &Request, visit: Arc<Visit>) -> Self {
        Self {
            method: request.method().clone(),
            path: request.path().to_string(),
            path_and_query: InertiaRequestExt::path_and_query(request),
            full_url: request.full_url(),
            headers: request.headers().clone(),
            visit,
        }
    }

    /// The request method.
    pub fn method(&self) -> &hyper::Method {
        &self.method
    }

    /// Where a redirect back from this request goes: the same-origin
    /// `Referer`, else the session's previous URL, else `fallback`, else
    /// `/` - what the default [`InertiaMiddlewareHooks::on_empty_response`]
    /// redirects to.
    pub fn back_target(&self, fallback: Option<&str>) -> String {
        self.visit.back_target(fallback)
    }
}

impl InertiaRequestExt for InertiaVisit {
    fn path(&self) -> &str {
        &self.path
    }

    fn path_and_query(&self) -> String {
        self.path_and_query.clone()
    }

    fn full_url(&self) -> String {
        self.full_url.clone()
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }
}

/// The default answer to an empty `200`: a redirect back.
fn empty_response_default(visit: &InertiaVisit, response: HttpResponse) -> HttpResponse {
    super::headers_middleware::redirect_back(&visit.visit, response)
}

/// The default answer to a redirect with a fragment: `409` +
/// `X-Inertia-Redirect`. A response without `Location` is returned as it is.
fn redirect_with_fragment_default(response: HttpResponse) -> HttpResponse {
    match response.header_value("Location").map(str::to_string) {
        Some(location) => super::headers_middleware::redirect_with_fragment(location, response),
        None => response,
    }
}

/// Props from the `share` and `share_once` hooks for one request, the once
/// ones made once props.
pub(crate) fn hook_shares(
    hooks: &dyn InertiaMiddlewareHooks,
    request: &dyn InertiaRequestExt,
) -> IndexMap<String, Prop> {
    let mut shared = hooks.share(request);
    for (key, prop) in hooks.share_once(request) {
        let prop = if prop.is_once() { prop } else { prop.once() };
        shared.insert(key, prop);
    }
    shared
}

/// [`InertiaVersionMiddleware`](crate::InertiaVersionMiddleware) with the
/// `on_version_change` hook applied to the `409` it answers.
///
/// The version middleware answers before the handler, so the `409` is
/// recognised by the handler not having been reached.
pub(crate) struct VersionChangeHook {
    inner: super::InertiaVersionMiddleware,
    hooks: Arc<dyn InertiaMiddlewareHooks>,
}

impl VersionChangeHook {
    pub(crate) fn new(
        inner: super::InertiaVersionMiddleware,
        hooks: Arc<dyn InertiaMiddlewareHooks>,
    ) -> Self {
        Self { inner, hooks }
    }
}

#[async_trait]
impl Middleware for VersionChangeHook {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if !request.is_inertia() || *request.method() != hyper::Method::GET {
            return self.inner.handle(request, next).await;
        }
        let scoped = visit::current().unwrap_or_else(|| Arc::new(Visit::capture(&request)));
        let inertia_visit = InertiaVisit::capture(&request, scoped);
        let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mark = reached.clone();
        let marked: Next = Arc::new(move |req| {
            mark.store(true, std::sync::atomic::Ordering::Relaxed);
            next(req)
        });
        let response = self.inner.handle(request, marked).await;
        if reached.load(std::sync::atomic::Ordering::Relaxed) {
            return response;
        }
        match response {
            Ok(http) => Ok(self.hooks.on_version_change(&inertia_visit, http)),
            Err(http) => Err(self.hooks.on_version_change(&inertia_visit, http)),
        }
    }
}

/// The Inertia middleware stack as one middleware, for a route group -
/// what Laravel's `HandleInertiaRequests` is on a group.
///
/// Built by [`Inertia::middleware`](crate::Inertia::middleware) from an
/// [`InertiaConfig`]: the headers middleware (`Vary`, the redirect rules,
/// the previous URL and the hooks), the version check, the `302 → 303`
/// rule, the error-response middleware, and the validation redirect, in
/// the order [`Inertia::install`](crate::Inertia::install)
/// registers them globally. A route outside the groups that carry it gets
/// none of them: no `Vary: X-Inertia`, no conversion.
#[derive(Clone)]
pub struct InertiaMiddleware {
    stack: Arc<[BoxedMiddleware]>,
}

impl InertiaMiddleware {
    /// Build the stack for `config`.
    pub fn new(config: &InertiaConfig) -> Self {
        Self {
            stack: stack(config).into(),
        }
    }
}

/// The Inertia middlewares for `config`, outermost first.
pub(crate) fn stack(config: &InertiaConfig) -> Vec<BoxedMiddleware> {
    vec![
        into_boxed(super::InertiaHeadersMiddleware::from_config(config)),
        version_middleware(config),
        into_boxed(super::Inertia303Middleware::new()),
        // Always present: the error callback can be installed after the
        // stack is built, and without one or an `error_page` the middleware
        // hands the request on and changes nothing. Outside the validation
        // redirect, so an Inertia visit's validation failure reaches it as
        // the redirect back, not as a `422` (PAR-062).
        into_boxed(super::InertiaErrorPageMiddleware::with_component(
            config.error_page.clone(),
        )),
        into_boxed(super::InertiaValidationRedirectMiddleware::new()),
    ]
}

/// The version middleware for `config`, comparing against the `version`
/// hook's answer when it gave one, else the installed configuration's
/// current version (so a later `Inertia::version` call counts, as for the
/// global stack), else `config`'s own when nothing is installed; with the
/// `on_version_change` hook when hooks are installed.
pub(crate) fn version_middleware(config: &InertiaConfig) -> BoxedMiddleware {
    let version = config.version.clone();
    let inner = super::InertiaVersionMiddleware::with_resolver(move || {
        visit::scoped_version().unwrap_or_else(|| {
            crate::App::inertia_registry()
                .installed_config()
                .map_or_else(
                    || version.resolve(),
                    |installed| installed.resolved_version(),
                )
        })
    });
    match config.hooks.clone() {
        Some(hooks) => into_boxed(VersionChangeHook::new(inner, hooks)),
        None => into_boxed(inner),
    }
}

#[async_trait]
impl Middleware for InertiaMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let mut next = next;
        for middleware in self.stack.iter().rev() {
            let inner = next;
            let middleware = middleware.clone();
            next = Arc::new(move |req| {
                let inner = inner.clone();
                let middleware = middleware.clone();
                Box::pin(async move { middleware(req, inner).await }) as MiddlewareFuture
            });
        }
        next(request).await
    }
}
