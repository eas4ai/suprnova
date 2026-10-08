//! `Inertia` static facade - Laravel-style entrypoint for the most
//! common Inertia helpers.

use crate::FrameworkError;
use crate::http::{Redirect, Request};
use crate::pagination::IntoInertiaScroll;

use super::config::{InertiaConfig, VersionResolver};
use super::flash::{self, FlashKey};
use super::hooks::{InertiaMiddleware, VersionChangeHook};
use super::providers::ProvidesInertiaProperties;
use super::response::PropEntry;
use super::response::{IntoInertiaData, reflash_session_values_after_eager_error};
use super::runtime::SsrCondition;
use super::ssr::SsrRequest;
use super::{
    DevToolsMiddleware, Inertia303Middleware, InertiaErrorPageMiddleware, InertiaHeadersMiddleware,
    InertiaResponse, InertiaValidationRedirectMiddleware, InertiaVersionMiddleware,
};
use serde_json::Value;
use std::sync::Arc;

/// Static facade. Today it exposes `Inertia::paginate`; future helpers
/// (render, location, etc.) will land here.
pub struct Inertia;

/// Log that the bound SSR gateway lacks the capability `call` needs, so a
/// setting that took no effect is visible rather than silently dropped.
fn warn_lacking(capability: &str, call: &str) {
    tracing::warn!(
        call,
        "the bound SSR gateway {capability}; Inertia::{call} has no effect"
    );
}

impl Inertia {
    /// Build an Inertia response with a single scroll-prop wired from
    /// a paginator.
    ///
    /// - `component` - the Inertia page component name (e.g. `"Users/Index"`).
    ///   This is what the frontend resolves to a real component.
    /// - `key` - the prop name under which the paginated rows land
    ///   (e.g. `"users"`). Scroll metadata is attached to the same key.
    ///
    /// The metadata page-name comes from the paginator itself:
    /// `"page"` for `LengthAwarePaginator`, `"cursor"` for
    /// `CursorPaginator`.
    #[track_caller]
    pub fn paginate<T>(
        component: &'static str,
        key: &'static str,
        paginator: impl IntoInertiaScroll<T>,
    ) -> InertiaResponse
    where
        T: serde::Serialize + 'static,
    {
        // `paginate` keeps the bare rows' merge at the prop's root.
        InertiaResponse::new(component).paginate(key, paginator)
    }

    /// Build an Inertia response from a `#[derive(Data)]` DTO.
    ///
    /// Lazy fields registered via `#[data(lazy)]` / `#[data(auto_lazy)]`
    /// resolve against the request's `?include=` set; the per-DTO allowlist
    /// enforces default-deny - disallowed includes return 400.
    #[track_caller]
    pub fn data<T>(component: &'static str, dto: T) -> InertiaResponse
    where
        T: IntoInertiaData,
    {
        InertiaResponse::from_data_props(component, dto.__into_inertia_props())
    }

    /// Fallible sibling of [`data`](Self::data): returns
    /// `Err(FrameworkError)` (naming the offending field) if a DTO field's
    /// `Serialize` impl fails, instead of panicking.
    ///
    /// On the HTTP request path the panicking [`data`](Self::data) is fine -
    /// the panic-recovery middleware converts it to a 500. Prefer `try_data`
    /// when building an Inertia response off that path (queue workers,
    /// scheduled tasks, CLI) where no panic net applies, or whenever you
    /// want to handle the serialization failure explicitly.
    #[track_caller]
    pub fn try_data<T>(component: &'static str, dto: T) -> Result<InertiaResponse, FrameworkError>
    where
        T: IntoInertiaData,
    {
        let props = dto
            .__try_into_inertia_props()
            .map_err(reflash_session_values_after_eager_error)?;
        Ok(InertiaResponse::from_data_props(component, props))
    }

    /// Set the asset version at run time. Laravel's `Inertia::version`.
    ///
    /// Takes a string, a function that returns one (called on every read,
    /// as Laravel calls a version closure), a [`VersionResolver`], or
    /// `None::<String>` for the empty version (Laravel casts `null` to
    /// `""`). The version replaces the one on the config
    /// [`install`](Self::install) retained, or on the default config when
    /// nothing is installed, so [`get_version`](Self::get_version), every
    /// response built after the call, and the version middleware `install`
    /// registers all read it. A later `install` replaces it in turn, and a
    /// response given its own config with
    /// [`InertiaResponse::with_config`] keeps that config's version.
    ///
    /// The config lives on the active container's Inertia registry, so a
    /// version set under [`crate::testing::TestContainer::fake`] stays in
    /// that test.
    pub fn version(version: impl Into<VersionResolver>) {
        let registry = crate::App::inertia_registry();
        let mut config = registry.installed_config().unwrap_or_default();
        config.version = version.into();
        registry.set_installed_config(config);
    }

    /// The current asset version. Laravel's `Inertia::getVersion`.
    ///
    /// What [`version`](Self::version) set, else the installed config's
    /// version in Laravel's order: the asset URL's hash, the Vite
    /// manifest's hash, or the empty string. This is the value the version
    /// middleware [`install`](Self::install) registers compares a client's
    /// `X-Inertia-Version` against, and that the 409 sends back.
    pub fn get_version() -> String {
        crate::App::inertia_registry()
            .installed_config()
            .unwrap_or_default()
            .resolved_version()
    }

    /// A redirect to the previous location - Laravel's
    /// `Inertia::back($status, $headers, $fallback)`.
    ///
    /// The target is chosen in Laravel's order: the request's `Referer`
    /// when it passes the same-origin check the validation redirect applies
    /// (a path on this host, under the public root), else the session's
    /// previous URL, else `fallback`, else `/`. The `Referer` is client-set
    /// and lands in `Location`, which is why it is checked rather than
    /// followed.
    ///
    /// The `Referer` is read from the request the Inertia middleware is
    /// handling, so the first leg needs
    /// [`InertiaHeadersMiddleware`](crate::InertiaHeadersMiddleware) on the
    /// route; [`Inertia::install`] puts it on every route. Without it the
    /// target starts at the previous URL. Add headers or flash data to the
    /// returned [`Redirect`] as to any other.
    ///
    /// ```rust,no_run
    /// use suprnova::{Inertia, Response};
    ///
    /// async fn update() -> Response {
    ///     Inertia::back(302, Some("/settings")).with("status", "Saved").into()
    /// }
    /// ```
    pub fn back(status: u16, fallback: Option<&str>) -> Redirect {
        let target = match super::visit::current() {
            Some(visit) => visit.back_target(fallback),
            None => super::visit::back_target(None, None, "", fallback),
        };
        Redirect::to(target).status(status)
    }

    /// Share a value with every Inertia response - Laravel's
    /// `Inertia::share($key, $value)`.
    ///
    /// A dotted key nests when it is shared, as Laravel's `Arr::set` does:
    /// `share("user.name", "Todd")` sets `name` inside the shared `user`
    /// object, and a later `share("user", ...)` replaces that object whole.
    /// The value is serialized now; for one that has to be computed per
    /// response use [`App::inertia_share_lazy`](crate::App::inertia_share_lazy).
    /// Shares are process-wide (on the active container), not per request:
    /// call this at boot, or from a provider for per-request data.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] when `value`'s `Serialize` impl fails;
    /// nothing is shared then.
    #[track_caller]
    pub fn share<V: serde::Serialize>(
        key: impl Into<String>,
        value: V,
    ) -> Result<(), FrameworkError> {
        Self::share_many([(key, value)])
    }

    /// Share several values at once - Laravel's `Inertia::share([...])`.
    /// Each entry is shared as [`share`](Self::share) shares it.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] naming the key whose value fails to
    /// serialize; nothing is shared then.
    #[track_caller]
    pub fn share_many<I, K, V>(entries: I) -> Result<(), FrameworkError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: serde::Serialize,
    {
        let mut values = Vec::new();
        for (key, value) in entries {
            let key = key.into();
            let value = serde_json::to_value(&value).map_err(|e| {
                FrameworkError::internal(format!(
                    "Inertia shared value for '{key}' failed to serialize: {e}"
                ))
            })?;
            values.push((key, value));
        }
        let registry = crate::App::inertia_registry();
        for (key, value) in values {
            registry.share_nested(key, value);
        }
        Ok(())
    }

    /// Share the fields of a `#[derive(Data)]` object - Laravel's
    /// `Inertia::share($arrayable)`.
    ///
    /// Each eager field becomes a shared value under its name. A lazy field
    /// (`#[data(lazy)]` and its variants) is left out, as it is left out of
    /// a Data object's array form until a request includes it: a shared
    /// prop has no `?include=` gate, so sharing it would send it on every
    /// page.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] naming the field whose value fails to
    /// serialize; nothing is shared then.
    #[track_caller]
    pub fn share_data<T: IntoInertiaData>(data: T) -> Result<(), FrameworkError> {
        let entries = data.__try_into_inertia_props()?;
        let registry = crate::App::inertia_registry();
        for (key, entry) in entries {
            if let PropEntry::Eager(value) = entry {
                registry.share_nested(key, value);
            }
        }
        Ok(())
    }

    /// Share the props a provider produces for each response - Laravel's
    /// `Inertia::share($provider)`. The provider receives a
    /// [`RenderContext`](crate::RenderContext) of the request and the
    /// component being rendered, and its keys count as shared keys.
    ///
    /// The same registration as
    /// [`InertiaRegistry::share_provider`](crate::InertiaRegistry::share_provider):
    /// any number of providers, each expanded once per render, and
    /// [`App::flush_inertia_shared`](crate::App::flush_inertia_shared)
    /// clears them with the other shares.
    pub fn share_provider(provider: impl ProvidesInertiaProperties + 'static) {
        crate::App::inertia_registry().share_provider(provider);
    }

    /// Read a shared value back - Laravel's `Inertia::getShared($key,
    /// $default)`. A dotted key walks into nested values (a numeric segment
    /// indexes a list); `default` comes back when nothing is shared there.
    ///
    /// Reads what is registered, without resolving anything: a lazy share
    /// has no value yet and reads as `default`, as Laravel hands back the
    /// unresolved closure.
    pub fn get_shared(key: &str, default: impl Into<Value>) -> Value {
        crate::App::inertia_registry()
            .shared_value(key)
            .unwrap_or_else(|| default.into())
    }

    /// Every shared value, nested - Laravel's `Inertia::getShared()` with
    /// no key. Lazy shares are left out, as in [`get_shared`](Self::get_shared).
    pub fn get_shared_all() -> Value {
        Value::Object(crate::App::inertia_registry().shared_tree())
    }

    /// Rename components before they render - Laravel's
    /// `Inertia::transformComponentUsing($closure)`.
    ///
    /// `transformer` receives the name a response was built with and
    /// returns the name to render, or `None` to keep it. It runs for every
    /// response, whether the name came from `inertia_response!`,
    /// `InertiaResponse::new` or `Router::inertia`, before the
    /// [`ensure_pages_exist`](crate::InertiaConfig::ensure_pages_exist)
    /// check, which then checks the new name. A later call replaces it.
    ///
    /// ```rust,no_run
    /// use suprnova::Inertia;
    ///
    /// // Pages moved under `Legacy/` without touching every handler.
    /// Inertia::transform_component_using(|component| {
    ///     component.starts_with("Billing/").then(|| format!("Legacy/{component}"))
    /// });
    /// ```
    pub fn transform_component_using<F>(transformer: F)
    where
        F: Fn(&str) -> Option<String> + Send + Sync + 'static,
    {
        crate::App::inertia_registry()
            .runtime()
            .set_component_transformer(Arc::new(transformer));
    }

    /// An external redirect, answered as the request needs - Laravel's
    /// `Inertia::location($url)`: `409` + `X-Inertia-Location` to an
    /// Inertia visit, and to anything else a `302` to the URL or the
    /// [`Redirect`] given as it is. See [`InertiaResponse::location`].
    ///
    /// ```rust,no_run
    /// use suprnova::{Inertia, Response};
    ///
    /// async fn portal() -> Response {
    ///     Ok(Inertia::location("https://billing.example/portal"))
    /// }
    /// ```
    pub fn location(target: impl Into<super::InertiaLocation>) -> crate::HttpResponse {
        InertiaResponse::location(target)
    }

    /// Turn SSR off, or on, for every request - Laravel's
    /// `Inertia::disableSsr($bool)`.
    ///
    /// `disable_ssr(true)` keeps the worker out even where the
    /// configuration enables SSR; `disable_ssr(false)` sends every first
    /// visit to it even where the configuration has SSR off (the worker URL
    /// still comes from the configuration). The setting replaces the
    /// configuration's switch until it is set again. Excluded paths and
    /// [`App::disable_ssr_for_request`](crate::App::disable_ssr_for_request)
    /// still keep a request out.
    ///
    /// The setting goes to the bound [`SsrGateway`](crate::SsrGateway); a
    /// gateway without the capability logs a warning and ignores it.
    pub fn disable_ssr(disabled: bool) {
        if !super::ssr_gateway::gateway().disable(SsrCondition::Always(disabled)) {
            warn_lacking("cannot disable SSR", "disable_ssr");
        }
    }

    /// Decide per request whether SSR is off - Laravel's
    /// `Inertia::disableSsr($closure)`. The condition runs for every first
    /// visit and its answer replaces the configuration's switch, so it can
    /// turn SSR on as well as off. Like [`disable_ssr`](Self::disable_ssr)
    /// it sets the bound gateway's condition.
    ///
    /// ```rust,no_run
    /// use suprnova::{Inertia, InertiaRequestExt};
    ///
    /// // Render the admin area on the client only.
    /// Inertia::disable_ssr_if(|request: &dyn InertiaRequestExt| {
    ///     request.path().starts_with("/admin")
    /// });
    /// ```
    pub fn disable_ssr_if<F>(condition: F)
    where
        F: Fn(&dyn super::InertiaRequestExt) -> bool + Send + Sync + 'static,
    {
        if !super::ssr_gateway::gateway().disable(SsrCondition::When(Arc::new(condition))) {
            warn_lacking("cannot disable SSR", "disable_ssr_if");
        }
    }

    /// Exclude paths from SSR - Laravel's `Inertia::withoutSsr($paths)`.
    ///
    /// The patterns join [`InertiaConfig::ssr_exclude`]'s and follow
    /// Laravel's `ExcludesPaths` rules: slashes at either end are ignored,
    /// `*` matches any characters including `/`, and each pattern is tried
    /// against the path and the full URL. `admin/*` keeps `/admin/users`
    /// and `/admin/users/edit` out, not `/adminx`. The patterns go to the
    /// bound gateway; one without the capability logs a warning.
    pub fn without_ssr<I, S>(patterns: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let patterns = patterns.into_iter().map(Into::into).collect();
        if !super::ssr_gateway::gateway().except(patterns) {
            warn_lacking("cannot exclude paths from SSR", "without_ssr");
        }
    }

    /// Adjust the request sent to the SSR worker - Laravel's
    /// `Inertia::configureSsrRequestUsing($closure)`. A worker that needs a
    /// token, another header or a longer timeout is reached through it; a
    /// later call replaces it. It applies to the health check too, and goes
    /// to the bound gateway; one without the capability logs a warning.
    ///
    /// ```rust,no_run
    /// use suprnova::Inertia;
    ///
    /// Inertia::configure_ssr_request_using(|request| {
    ///     request.bearer_token(std::env::var("SSR_TOKEN").unwrap_or_default())
    /// });
    /// ```
    pub fn configure_ssr_request_using<F>(configure: F)
    where
        F: Fn(SsrRequest) -> SsrRequest + Send + Sync + 'static,
    {
        if !super::ssr_gateway::gateway().configure_request_using(Arc::new(configure)) {
            warn_lacking(
                "cannot configure the SSR request",
                "configure_ssr_request_using",
            );
        }
    }

    /// Whether the SSR worker is healthy, from the bound
    /// [`SsrGateway`](crate::SsrGateway)'s health check, Laravel's
    /// `HasHealthCheck::isHealthy` that `inertia:check-ssr` reads.
    ///
    /// The default gateway sends `GET {url}/health` with the installed
    /// configuration's worker URL and timeout, through the request
    /// configurator, and answers `Some(true)` for a 2xx and `Some(false)`
    /// for any other status or no answer. `None` means the bound gateway
    /// has no health check.
    ///
    /// ```rust,no_run
    /// use suprnova::Inertia;
    ///
    /// # async fn check() {
    /// match Inertia::ssr_is_healthy().await {
    ///     Some(true) => println!("the SSR worker is up"),
    ///     Some(false) => println!("the SSR worker is down"),
    ///     None => println!("the SSR gateway has no health check"),
    /// }
    /// # }
    /// ```
    pub async fn ssr_is_healthy() -> Option<bool> {
        let config = crate::App::inertia_registry()
            .installed_config()
            .unwrap_or_default();
        super::ssr_gateway::gateway().is_healthy(&config.ssr).await
    }

    /// Flash a value for the next page response - Laravel's
    /// `Inertia::flash($key, $value)`.
    ///
    /// The value lives in the session under `inertia.flash_data`, is
    /// emitted as `page.flash` by the next Inertia page response and
    /// removed by it. Unlike a prop it never enters the browser's history
    /// state, which is what one-shot toasts and highlights want. It does
    /// not depend on what this request answers, and the Inertia middleware
    /// keeps it across any number of redirects before a page shows it.
    ///
    /// `key` is a string or a type implementing [`FlashKey`], such as an
    /// application's enum of toast kinds. Without a session in scope the
    /// value rides on this request's own page response only.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] when `value`'s `Serialize` impl fails;
    /// nothing is flashed then.
    pub fn flash<K: FlashKey, V: serde::Serialize>(key: K, value: V) -> Result<(), FrameworkError> {
        Self::flash_many([(key, value)])
    }

    /// Flash several values at once - Laravel's `Inertia::flash([...])`.
    /// See [`flash`](Self::flash).
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] naming the key whose value fails to
    /// serialize; nothing is flashed then.
    pub fn flash_many<I, K, V>(entries: I) -> Result<(), FrameworkError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: FlashKey,
        V: serde::Serialize,
    {
        let mut map = serde_json::Map::new();
        for (key, value) in entries {
            let key = key.flash_key();
            let value = serde_json::to_value(&value).map_err(|e| {
                FrameworkError::internal(format!(
                    "Inertia flash value for '{key}' failed to serialize: {e}"
                ))
            })?;
            map.insert(key, value);
        }
        if !flash::put_in_session(map.clone()) {
            for (key, value) in map {
                flash::push(key, value);
            }
        }
        Ok(())
    }

    /// The Inertia flash data waiting for the next page response -
    /// Laravel's `Inertia::getFlashed($request)`.
    ///
    /// Reads the session of `request`, the one in scope while it is
    /// handled; empty without a session. What it returns is exactly what
    /// [`pull_flashed`](Self::pull_flashed) would remove.
    pub fn get_flashed(_request: &Request) -> serde_json::Map<String, serde_json::Value> {
        flash::get_from_session()
    }

    /// Remove and return the Inertia flash data - Laravel's
    /// `Inertia::pullFlashed($request)`. A page rendered afterwards shows
    /// none of it. Empty without a session.
    pub fn pull_flashed(_request: &Request) -> serde_json::Map<String, serde_json::Value> {
        flash::pull_from_session()
    }

    /// Clear the client's history state on the next page - Laravel's
    /// `Inertia::clearHistory()`. The same as
    /// [`App::clear_history`](crate::App::clear_history): the flag lives in
    /// the session until a page response emits it as `clearHistory: true`,
    /// however many redirects come first, which is what a logout that
    /// redirects needs.
    pub fn clear_history() {
        crate::App::clear_history();
    }

    /// Keep the URL fragment across the next redirect - Laravel's
    /// `Inertia::preserveFragment()`, the session form of
    /// [`Redirect::preserve_fragment`]. The flag lives in the session until
    /// a page response emits it as `preserveFragment: true`. A no-op
    /// without a session in scope.
    pub fn preserve_fragment() {
        flash::set_history_flag(flash::PRESERVE_FRAGMENT);
    }

    /// Decide every error response the framework renders - Laravel's
    /// `Inertia::handleExceptionsUsing($callback)`.
    ///
    /// The callback receives an [`InertiaErrorResponse`](crate::InertiaErrorResponse)
    /// for each error response the framework renders: a handler's or a
    /// middleware's `Err`, a panic in either, the router's `404`, a
    /// middleware's own `{"message": ...}` answer. Every request type is
    /// covered, an API client's included. It returns the value with a
    /// decision - [`render`](crate::InertiaErrorResponse::render) a page,
    /// [`respond_with`](crate::InertiaErrorResponse::respond_with) another
    /// response - or `None`, which keeps the response.
    ///
    /// The error-response middleware [`install`](Self::install) registers
    /// decides what passes through it, inside the request scopes the stack
    /// opens. The server decides the rest after the whole stack: the
    /// answer of a middleware registered before `install`, such as a
    /// `CsrfMiddleware`'s `419` or a `TimeoutMiddleware`'s `503`, and a
    /// panic the server's boundary caught outside the stack. Decided at the
    /// server, the callback sees the same request, error and response, and
    /// [`with_shared_data`](crate::InertiaErrorResponse::with_shared_data)
    /// reaches the shared registry and providers but no session data, no
    /// detected locale and none of the middleware hooks' shares, since
    /// every scope a middleware opened has closed. Each response is
    /// decided once.
    ///
    /// The callback replaces the rule
    /// [`InertiaConfig::error_page`](crate::InertiaConfig::error_page)
    /// installs: an app that sets both gets its callback. A later call
    /// replaces the callback. It lives on the active container's Inertia
    /// registry, so a callback installed under
    /// [`TestContainer::fake`](crate::testing::TestContainer::fake) stays in
    /// that test.
    ///
    /// A validation failure reaches the callback as the response the
    /// client would get. A JSON client's `422`, `{message, errors}`, and a
    /// Precognition dry run's are handed over like any other error. An
    /// Inertia visit's never arrives as a `422`: the error-response
    /// middleware sits outside [`InertiaValidationRedirectMiddleware`], so
    /// by the time the response reaches it, the redirect has made it the
    /// `303` back to the form with the errors flashed, which is not an
    /// error. A panic in the callback itself reaches the server's panic
    /// boundary, as a panic in any middleware does.
    ///
    /// ```rust,no_run
    /// use serde_json::json;
    /// use suprnova::Inertia;
    ///
    /// Inertia::handle_exceptions_using(|error| match error.status() {
    ///     403 | 404 | 500 | 503 => {
    ///         let status = error.status();
    ///         Some(error.render("Error", json!({ "status": status })).with_shared_data())
    ///     }
    ///     _ => None,
    /// });
    /// ```
    pub fn handle_exceptions_using<F>(callback: F)
    where
        F: Fn(super::InertiaErrorResponse<'_>) -> Option<super::InertiaErrorResponse<'_>>
            + Send
            + Sync
            + 'static,
    {
        crate::App::inertia_registry().set_exception_handler(Arc::new(callback));
    }

    /// The Inertia middleware stack as one middleware, for a route group -
    /// the way a Laravel app registers `HandleInertiaRequests` on its `web`
    /// group and keeps it off `api`.
    ///
    /// The stack is the one [`install`](Self::install) registers globally
    /// (headers and redirect rules, version check, `302 → 303`, validation
    /// redirect, and the error-response middleware), with
    /// `config`'s settings and [`hooks`](InertiaConfig::hooks). Install
    /// with [`InertiaConfig::register_globally`] off so the stack is not
    /// also on every route; `install` then registers it as the named
    /// middleware `inertia`, which a group can name instead of holding this
    /// value.
    ///
    /// ```rust,no_run
    /// use suprnova::{Inertia, InertiaConfig, Router};
    /// # use suprnova::{Request, Response, text};
    /// # async fn dashboard(_r: Request) -> Response { text("ok") }
    /// # async fn users(_r: Request) -> Response { text("ok") }
    ///
    /// # fn routes() -> Result<Router, suprnova::FrameworkError> {
    /// let cfg = InertiaConfig::new().register_globally(false);
    /// Inertia::install(&cfg)?;
    ///
    /// let router: Router = Router::new()
    ///     .group("/", |r| r.get("/dashboard", dashboard))
    ///     .middleware(Inertia::middleware(&cfg))
    ///     .into();
    /// // No Inertia stack on the API: no `Vary: X-Inertia`, no 303.
    /// let router: Router = router.group("/api", |r| r.get("/users", users)).into();
    /// # Ok(router) }
    /// ```
    pub fn middleware(config: &InertiaConfig) -> InertiaMiddleware {
        InertiaMiddleware::new(config)
    }

    /// Install the standard Inertia protocol middleware globally.
    ///
    /// When Inertia DevTools is enabled ([`InertiaConfig::devtools`], on
    /// by default in the `local` environment only), the
    /// [`DevToolsMiddleware`](crate::DevToolsMiddleware) that records each
    /// request for the browser extension and answers its entry endpoints
    /// is registered first, outermost of the Inertia layer. Then it
    /// registers five global middlewares in order:
    /// 1. [`InertiaHeadersMiddleware`] - sets `Vary: X-Inertia` on every
    ///    response; on an Inertia visit it turns an empty `200` into a
    ///    redirect back (`302`, `303` for `PUT`, `PATCH` and `DELETE`), a
    ///    redirect with a `#fragment` into `409` + `X-Inertia-Redirect`, and
    ///    records the visit as the session's previous URL
    ///    ([`InertiaConfig::store_previous_url`]). Registered first, so it
    ///    wraps everything, including the `409` the version middleware
    ///    returns below.
    /// 2. [`InertiaVersionMiddleware`] - emits `409 Conflict` +
    ///    `X-Inertia-Location` when the client's `X-Inertia-Version`
    ///    header doesn't match [`get_version`](Self::get_version), read
    ///    per request.
    ///    Without it, asset-version mismatches are silent and stale
    ///    clients keep hitting the new server with the old bundle.
    /// 3. [`Inertia303Middleware`] - converts `302` redirects on
    ///    non-GET Inertia visits to `303`, so the client's follow-up
    ///    request is explicitly a GET. Without it, browsers may
    ///    re-submit the original PUT/PATCH/DELETE to the redirect
    ///    target - silently breaking form-create-then-redirect flows.
    /// 4. [`InertiaErrorPageMiddleware`], the error-response middleware -
    ///    hands the framework's own error responses - a `403` denial, an
    ///    unrouted `404`, a `429`, a `500`, a handler's panic, a JSON
    ///    client's validation `422` - to the callback
    ///    [`handle_exceptions_using`](Self::handle_exceptions_using)
    ///    installed, or, without one, to the default callback that renders
    ///    the page [`InertiaConfig::error_page`] names, so they stop
    ///    reaching the client as the plain-JSON error modal. With neither,
    ///    it hands every request on and changes nothing. It is registered
    ///    whatever the config says because the callback may be installed
    ///    after this call. Outside the validation redirect, so an Inertia
    ///    visit's validation failure reaches it as the `303` back to the
    ///    form, not as a `422` a callback could replace.
    /// 5. [`InertiaValidationRedirectMiddleware`] - turns a validation
    ///    `422` on an Inertia visit into a `303` back with the errors
    ///    flashed. Innermost, so it sees the handler's raw `422`; the
    ///    `303` it emits passes untouched through the error-response
    ///    middleware and the `302 → 303` conversion above. Without it the
    ///    client sees a response with no `X-Inertia` header, treats it as
    ///    non-Inertia, and shows the error modal instead of populating
    ///    `form.errors`.
    ///
    /// A `CsrfMiddleware`, rate limiter, or auth guard registered above
    /// this call never hands its rejection to anything registered inside
    /// it. The server decides such a response after the whole stack, by
    /// the same rule, but every request scope a middleware opened has
    /// closed by then, so a page rendered there has no session data and
    /// the default locale. An app that wants those on the page registers
    /// [`InertiaErrorPageMiddleware`] itself, at the position it needs;
    /// `install` sees that registration, logs at `debug`, and skips its
    /// own, leaving both the app's placement and the component the app
    /// named intact. `error_page` on the config is then optional. See that
    /// type's documentation for where it may sit. Register it before this
    /// call: one registered after it sits inside the validation redirect,
    /// where an Inertia visit's validation failure reaches it as a `422`
    /// before the redirect back is built. Each error response is decided
    /// once, by the innermost instance.
    ///
    /// With [`InertiaConfig::register_globally`] off, none of them is
    /// registered globally: `install` registers the whole stack as the
    /// named middleware `inertia` for route groups instead (see
    /// [`middleware`](Self::middleware)), and retains the config as below.
    /// With [`InertiaConfig::hooks`] set, the headers middleware and the
    /// version check run them.
    ///
    /// One call wires all five, in their order - the headers middleware,
    /// the version check, the `302 → 303` conversion, the error-response
    /// middleware, then the validation redirect innermost - so an app
    /// cannot end up carrying some of them and silently missing the rest.
    /// Each closes a failure mode that surfaces only in production: cache
    /// poisoning across the two representations of a URL, a stale bundle
    /// after a deploy, a method-preserving redirect, an error response the
    /// client shows as its plain-JSON modal, and a form that reports its
    /// own validation errors as a crash.
    ///
    /// Call once at boot. The config is **cloned and retained** as the
    /// default that every [`InertiaResponse`] starts from, so a response
    /// the handler never handed a config to renders with the app's
    /// settings rather than [`InertiaConfig::default`]. That is what makes
    /// `.frontend(...)`, `.version(...)`, `.default_title(...)`,
    /// `.ssr(...)` and `.encrypt_history(...)` set here reach the page:
    /// the HTML shell loads the entry point of the frontend the project
    /// was built with, the page object's asset version comes from the same
    /// config as the version middleware's resolver, and SSR is switched on
    /// app-wide rather than per response.
    ///
    /// Per-response [`InertiaResponse::with_config`] still wins. The
    /// argument stays a `&` reference so callers keep ownership, and
    /// calling `install` again replaces the retained config - last write
    /// wins. The middleware registrations are appended, not replaced -
    /// call `install` exactly once per process.
    ///
    /// The config is retained on the *active* container's Inertia
    /// registry, so it follows the same task-local → thread-local →
    /// global lookup as Inertia shared data. A test that installs under
    /// [`crate::testing::TestContainer::fake`] cannot leak its config into
    /// tests running in parallel.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] when `config` is in production mode
    /// (`development == false` - the default whenever `APP_ENV=production`,
    /// see `InertiaConfig::default`) but no Vite manifest can be loaded
    /// from `config.manifest_path`. This is CFG-01's fail-closed guard:
    /// without it, a production boot with a missing/unbuilt frontend
    /// would silently fall back to a legacy hardcoded asset path rather
    /// than the operator learning about it at boot, the same way a
    /// missing `APP_KEY` fails closed in [`crate::Server::from_config`]
    /// rather than booting with a broken encryption key. Neither the
    /// middleware registration nor the config retention happens when this
    /// returns `Err` - nothing is half-installed, and responses keep
    /// rendering from `InertiaConfig::default()`.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use suprnova::{Inertia, InertiaConfig};
    ///
    /// pub fn register() -> Result<(), suprnova::FrameworkError> {
    ///     Inertia::install(
    ///         &InertiaConfig::new().version(env!("CARGO_PKG_VERSION")),
    ///     )
    /// }
    /// ```
    pub fn install(config: &InertiaConfig) -> Result<(), FrameworkError> {
        use crate::middleware::register_global_middleware;

        if !config.development && config.vite_manifest().is_none() {
            return Err(FrameworkError::internal(format!(
                "Inertia is configured for production (InertiaConfig::development = false, \
                 which is the default under APP_ENV=production) but no Vite manifest was found \
                 at '{}'. Build your frontend (e.g. `npm run build`) so the manifest exists \
                 before deploying, or point `.manifest_path(...)` at the right location. \
                 Suprnova refuses to boot in production without a manifest rather than silently \
                 falling back to a legacy hardcoded asset path.",
                config.manifest_path.display()
            )));
        }

        // Retain the config so `InertiaResponse::new` starts from it
        // instead of `InertiaConfig::default()`: the whole render path -
        // frontend and entry point, asset version, SSR, encrypt-history
        // default, dev-server URL - reads the app's settings rather than
        // a per-response default that never saw them.
        //
        // It goes through `App::inertia_registry()` rather than a
        // process-global for the same reason Inertia shared data does:
        // that resolver checks task-local, then thread-local, then
        // global, so an install performed under `TestContainer::fake()`
        // stays inside that test instead of changing what every other
        // test in the binary renders.
        crate::App::inertia_registry().set_installed_config(config.clone());

        // Registration order is execution order, and the first registered
        // is the outermost (`middleware/chain.rs:94`). The headers
        // middleware goes first so it wraps everything - including the
        // `409` the version middleware returns without ever calling the
        // handler, which is precisely a response a shared cache would
        // otherwise store with no `Vary`.
        let devtools = config.devtools_config();
        let devtools_enabled = devtools.is_enabled();
        if !config.register_globally {
            // The stack for route groups instead: named, so a group takes
            // it with `middleware_named("inertia")`, and a route outside
            // such a group gets nothing of Inertia's.
            let stack = InertiaMiddleware::new(config);
            crate::middleware::register_middleware_alias(MIDDLEWARE_NAME, move || stack.clone());
            // The group stack records the requests of its routes; the
            // extension's entry endpoints belong to no group, so they are
            // answered by a global DevTools middleware that records
            // nothing.
            if devtools_enabled {
                register_global_middleware(DevToolsMiddleware::endpoints_only(devtools));
            }
            return Ok(());
        }
        // Outermost of the Inertia layer, inside the session registered
        // before this call: an entry sees the response every Inertia
        // middleware below shaped.
        if devtools_enabled {
            register_global_middleware(DevToolsMiddleware::new(devtools));
        }
        register_global_middleware(InertiaHeadersMiddleware::from_config(config));
        // The middleware reads the version per request: the `version` hook's
        // answer when it gave one, else `Inertia::get_version`, so a later
        // `Inertia::version` call is what a stale client is compared
        // against, the same value the page object advertises.
        let version_check = InertiaVersionMiddleware::with_resolver(|| {
            super::visit::scoped_version().unwrap_or_else(Inertia::get_version)
        });
        match config.hooks.clone() {
            Some(hooks) => {
                register_global_middleware(VersionChangeHook::new(version_check, hooks));
            }
            None => register_global_middleware(version_check),
        }
        register_global_middleware(Inertia303Middleware::new());
        // Whatever the config says (PAR-062): the error callback may be
        // installed after this call, and the middleware reads it, and the
        // installed `error_page`, per request. Inside the rest of the layer,
        // so it sees the response the handler and the route middleware
        // actually produced - a `403` from `PermissionMiddleware` never
        // reaches the handler at all. Outside the validation redirect
        // registered next, so an Inertia visit's validation failure reaches
        // it as the redirect back, while a JSON client's `422` reaches the
        // callback.
        match error_page_action(crate::middleware::has_global_middleware::<
            InertiaErrorPageMiddleware,
        >()) {
            ErrorPageAction::Register => {
                register_global_middleware(InstalledErrorPage(
                    InertiaErrorPageMiddleware::with_component(None),
                ));
            }
            ErrorPageAction::KeepExisting => {
                tracing::debug!(
                    "an InertiaErrorPageMiddleware is already registered; keeping its position \
                     in the chain and the component it names, and skipping the one \
                     Inertia::install would add"
                );
            }
        }
        register_global_middleware(InertiaValidationRedirectMiddleware::new());
        Ok(())
    }
}

/// The error-response middleware [`Inertia::install`] registers.
///
/// Its own type, because global registration is idempotent per type: an
/// app that registers [`InertiaErrorPageMiddleware`] itself after `install`
/// keeps its registration rather than losing it as a duplicate of this one.
/// The inner of the two decides each error response, and the outer leaves
/// a decided response alone.
struct InstalledErrorPage(InertiaErrorPageMiddleware);

#[async_trait::async_trait]
impl crate::middleware::Middleware for InstalledErrorPage {
    async fn handle(
        &self,
        request: Request,
        next: crate::middleware::Next,
    ) -> crate::http::Response {
        self.0.handle(request, next).await
    }
}

/// The name [`Inertia::install`] registers the stack under when it is not
/// registered globally, for `GroupBuilder::middleware_named`.
const MIDDLEWARE_NAME: &str = "inertia";

/// What [`Inertia::install`] does about the error-response middleware.
#[derive(Debug, PartialEq, Eq)]
enum ErrorPageAction {
    /// Register one just outside the validation redirect, rendering the
    /// installed config's `error_page` when no callback is installed.
    Register,
    /// The app registered one before `install`. Leave it exactly where the
    /// app put it, rendering the component the app named.
    KeepExisting,
}

/// The whole decision, as a pure function of the fact it reads.
///
/// Split out from [`Inertia::install`] because that fact is the
/// process-global middleware registry, shared by every test in the binary,
/// so the rule itself would otherwise only be testable through whatever
/// registrations the rest of the suite happened to have made first.
///
/// An app that registered the middleware itself named its component
/// there, and that instance is the one in the chain - so `install` has
/// nothing left to decide beyond staying out of the way.
fn error_page_action(already_registered: bool) -> ErrorPageAction {
    if already_registered {
        ErrorPageAction::KeepExisting
    } else {
        ErrorPageAction::Register
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::get_global_middleware;

    /// The rule set on its own, away from the process-global registry the
    /// live call reads it from.
    #[test]
    fn error_page_registration_keeps_what_the_app_placed() {
        // The default: install places it, whatever the config says, since
        // the error callback may be installed after the call.
        assert_eq!(error_page_action(false), ErrorPageAction::Register);

        // The app placed it further out, ahead of a middleware that
        // answers before the Inertia layer is reached. Its position - and
        // the component it names - is what stands.
        assert_eq!(error_page_action(true), ErrorPageAction::KeepExisting);
    }

    #[test]
    fn install_registers_the_protocol_middlewares() {
        // `install` also retains the config on the active container's
        // Inertia registry. Without this guard that write lands on the
        // global registry, and `response.rs`'s
        // `build_page_object_eager_only` - same binary, running in
        // parallel - would see `version = "test-version"` where it
        // asserts an empty version. The guard gives this test its own registry,
        // cleared when it drops.
        let _guard = crate::testing::TestContainer::fake();
        let before = get_global_middleware().len();
        // Force dev mode rather than relying on the `APP_ENV`-derived
        // default: sibling unit tests in this binary set
        // `APP_ENV=production` under their own module locks, and a read
        // here can land inside that window. Dev mode never consults the
        // manifest, so install succeeds without a Vite build in the test
        // process's working directory.
        // DevTools off: it is on by default in the `local` environment, an
        // unset `APP_ENV` included, and adds a sixth middleware this test
        // is not about.
        Inertia::install(
            &InertiaConfig::new()
                .version("test-version")
                .development(true)
                .devtools(super::super::DevToolsConfig::new().enabled(false)),
        )
        .expect("dev-mode install must not require a manifest");
        let after = get_global_middleware().len();
        assert_eq!(
            after - before,
            5,
            "Inertia::install should register exactly five middlewares (headers + version + 303 \
             + error responses + validation redirect), got delta={}",
            after - before
        );
        // This asserts the count, not the registration ORDER (headers
        // outermost). `get_global_middleware()` returns
        // `Vec<Arc<dyn Fn(Request, Next) -> MiddlewareFuture + Send + Sync>>` -
        // fully type-erased, no `TypeId` or type name survives past
        // `into_boxed`, so there is nothing in that `Vec` to assert order
        // against without adding new runtime introspection. The order
        // guarantee is instead carried end-to-end by
        // `a_version_mismatch_409_still_carries_vary_x_inertia_when_headers_middleware_wraps_it`
        // in `framework/tests/inertia_middleware.rs`, which registers the
        // two middlewares in `Inertia::install`'s order and asserts the
        // observable consequence of that order: `Vary` on a `409` the
        // version middleware returns without calling the handler.

        // The error-response middleware is registered whatever the config
        // says, since the error callback may be installed after `install`;
        // naming an error page adds nothing more. Both installs live in
        // this one test rather than in a sibling because registration is
        // idempotent per type and process-global: two tests each measuring
        // their own delta would race over which of them registered the
        // five shared types.
        //
        // Known side effect: `TestContainer::fake` scopes
        // `set_installed_config`, but `register_global_middleware` really
        // is process-global, so from here on every test in this binary
        // that builds a chain from `get_global_middleware()` carries the
        // error-response middleware. It reads the callback and the
        // installed `error_page` from the active container per request,
        // so it changes nothing for a test that sets neither.
        Inertia::install(
            &InertiaConfig::new()
                .version("test-version")
                .development(true)
                .devtools(super::super::DevToolsConfig::new().enabled(false))
                .error_page("Error"),
        )
        .expect("dev-mode install must not require a manifest");
        assert_eq!(
            get_global_middleware().len(),
            after,
            "naming an error page adds no middleware: the fifth is already there"
        );
    }

    /// CFG-01 fail-closed guard. Deliberately uses `.production()` +
    /// `.manifest_path(...)` instead of mutating `APP_ENV` - this crate's
    /// unit tests all share one process/binary, so an env-var-free test
    /// avoids racing every other test that reads the environment
    /// concurrently. The `APP_ENV`-driven default itself (dev vs.
    /// production) is covered separately in its own isolated test
    /// binary - see `framework/tests/inertia_production_fail_closed.rs`.
    #[test]
    fn install_fails_closed_in_production_without_a_manifest() {
        // No before/after `get_global_middleware().len()` delta check
        // here (unlike `install_registers_three_middlewares`): that
        // registry is process-global and this crate's unit tests run
        // massively parallel in one binary, so unrelated tests
        // registering middleware concurrently would make an exact-count
        // assertion flaky. The validation check running before any
        // `register_global_middleware` call (visible directly in
        // `Inertia::install`'s source, right above) is what actually
        // guarantees the failed path registers nothing.
        let cfg = InertiaConfig::new()
            .production()
            .manifest_path("this/path/does/not/exist/manifest.json");

        let _guard = crate::testing::TestContainer::fake();
        let err = Inertia::install(&cfg)
            .expect_err("production install without a manifest must fail closed");
        assert!(
            crate::App::inertia_registry().installed_config().is_none(),
            "a failed install must retain no config either - a response \
             must not render from settings the operator was just told are \
             unusable"
        );
        let msg = format!("{err}");
        assert!(
            msg.contains("Vite manifest"),
            "error should name the missing manifest: {msg}"
        );
        assert!(
            msg.contains("production"),
            "error should explain this is a production-mode requirement: {msg}"
        );
    }
}
