//! `Inertia` static facade — Laravel-style entrypoint for the most
//! common Inertia helpers.

use crate::FrameworkError;
use crate::pagination::IntoInertiaScroll;

use super::config::InertiaConfig;
use super::response::IntoInertiaData;
use super::{
    Inertia303Middleware, InertiaHeadersMiddleware, InertiaResponse, InertiaVersionMiddleware,
};

/// Static facade. Today it exposes `Inertia::paginate`; future helpers
/// (render, location, etc.) will land here.
pub struct Inertia;

impl Inertia {
    /// Build an Inertia response with a single scroll-prop wired from
    /// a paginator.
    ///
    /// - `component` — the Inertia page component name (e.g. `"Users/Index"`).
    ///   This is what the frontend resolves to a real component.
    /// - `key` — the prop name under which the paginated rows land
    ///   (e.g. `"users"`). Scroll metadata is attached to the same key.
    ///
    /// The metadata page-name comes from the paginator itself:
    /// `"page"` for `LengthAwarePaginator`, `"cursor"` for
    /// `CursorPaginator`.
    pub fn paginate<T>(
        component: &'static str,
        key: &'static str,
        paginator: impl IntoInertiaScroll<T>,
    ) -> InertiaResponse
    where
        T: serde::Serialize + 'static,
    {
        let (meta, data) = paginator.into_inertia_scroll();
        InertiaResponse::new(component).scroll(key, meta, data)
    }

    /// Build an Inertia response from a `#[derive(Data)]` DTO.
    ///
    /// Lazy fields registered via `#[data(lazy)]` / `#[data(auto_lazy)]`
    /// resolve against the request's `?include=` set; the per-DTO allowlist
    /// enforces default-deny — disallowed includes return 400.
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
    /// On the HTTP request path the panicking [`data`](Self::data) is fine —
    /// the panic-recovery middleware converts it to a 500. Prefer `try_data`
    /// when building an Inertia response off that path (queue workers,
    /// scheduled tasks, CLI) where no panic net applies, or whenever you
    /// want to handle the serialization failure explicitly.
    pub fn try_data<T>(component: &'static str, dto: T) -> Result<InertiaResponse, FrameworkError>
    where
        T: IntoInertiaData,
    {
        Ok(InertiaResponse::from_data_props(
            component,
            dto.__try_into_inertia_props()?,
        ))
    }

    /// Install the standard Inertia protocol middleware globally.
    ///
    /// Registers three global middlewares in order:
    /// 1. [`InertiaHeadersMiddleware`] — sets `Vary: X-Inertia` on every
    ///    response and turns an empty `200` on an Inertia visit into a
    ///    `303` back. Registered first, so it wraps everything, including
    ///    the `409` the version middleware returns below.
    /// 2. [`InertiaVersionMiddleware`] — emits `409 Conflict` +
    ///    `X-Inertia-Location` when the client's `X-Inertia-Version`
    ///    header doesn't match the server's configured version.
    ///    Without it, asset-version mismatches are silent and stale
    ///    clients keep hitting the new server with the old bundle.
    /// 3. [`Inertia303Middleware`] — converts `302` redirects on
    ///    non-GET Inertia visits to `303`, so the client's follow-up
    ///    request is explicitly a GET. Without it, browsers may
    ///    re-submit the original PUT/PATCH/DELETE to the redirect
    ///    target — silently breaking form-create-then-redirect flows.
    ///
    /// One call wires all three, so an app cannot end up carrying two of
    /// them and silently missing the third — each closes a failure mode
    /// that surfaces only in production: cache poisoning across the two
    /// representations of a URL, a stale bundle after a deploy, and a
    /// method-preserving redirect.
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
    /// calling `install` again replaces the retained config — last write
    /// wins. The middleware registrations are appended, not replaced —
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
    /// (`development == false` — the default whenever `APP_ENV=production`,
    /// see `InertiaConfig::default`) but no Vite manifest can be loaded
    /// from `config.manifest_path`. This is CFG-01's fail-closed guard:
    /// without it, a production boot with a missing/unbuilt frontend
    /// would silently fall back to a legacy hardcoded asset path rather
    /// than the operator learning about it at boot, the same way a
    /// missing `APP_KEY` fails closed in [`crate::Server::from_config`]
    /// rather than booting with a broken encryption key. Neither the
    /// middleware registration nor the config retention happens when this
    /// returns `Err` — nothing is half-installed, and responses keep
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
        // middleware goes first so it wraps everything — including the
        // `409` the version middleware returns without ever calling the
        // handler, which is precisely a response a shared cache would
        // otherwise store with no `Vary`.
        register_global_middleware(InertiaHeadersMiddleware::new());
        let version = config.version.clone();
        register_global_middleware(InertiaVersionMiddleware::with_resolver(move || {
            version.resolve()
        }));
        register_global_middleware(Inertia303Middleware::new());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::get_global_middleware;

    #[test]
    fn install_registers_three_middlewares() {
        // `install` also retains the config on the active container's
        // Inertia registry. Without this guard that write lands on the
        // global registry, and `response.rs`'s
        // `build_page_object_eager_only` — same binary, running in
        // parallel — would see `version = "test-version"` where it
        // asserts `"1.0"`. The guard gives this test its own registry,
        // cleared when it drops.
        let _guard = crate::testing::TestContainer::fake();
        let before = get_global_middleware().len();
        // Force dev mode rather than relying on the `APP_ENV`-derived
        // default: sibling unit tests in this binary set
        // `APP_ENV=production` under their own module locks, and a read
        // here can land inside that window. Dev mode never consults the
        // manifest, so install succeeds without a Vite build in the test
        // process's working directory.
        Inertia::install(
            &InertiaConfig::new()
                .version("test-version")
                .development(true),
        )
        .expect("dev-mode install must not require a manifest");
        let after = get_global_middleware().len();
        assert_eq!(
            after - before,
            3,
            "Inertia::install should register exactly three middlewares \
             (headers + version + 303), got delta={}",
            after - before
        );
        // This asserts the count, not the registration ORDER (headers
        // outermost). `get_global_middleware()` returns
        // `Vec<Arc<dyn Fn(Request, Next) -> MiddlewareFuture + Send + Sync>>`
        // — fully type-erased, no `TypeId` or type name survives past
        // `into_boxed`, so there is nothing in that `Vec` to assert order
        // against without adding new runtime introspection. The order
        // guarantee is instead carried end-to-end by
        // `a_version_mismatch_409_still_carries_vary_x_inertia_when_headers_middleware_wraps_it`
        // in `framework/tests/inertia_middleware.rs`, which registers the
        // two middlewares in `Inertia::install`'s order and asserts the
        // observable consequence of that order: `Vary` on a `409` the
        // version middleware returns without calling the handler.
    }

    /// CFG-01 fail-closed guard. Deliberately uses `.production()` +
    /// `.manifest_path(...)` instead of mutating `APP_ENV` — this crate's
    /// unit tests all share one process/binary, so an env-var-free test
    /// avoids racing every other test that reads the environment
    /// concurrently. The `APP_ENV`-driven default itself (dev vs.
    /// production) is covered separately in its own isolated test
    /// binary — see `framework/tests/inertia_production_fail_closed.rs`.
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
            "a failed install must retain no config either — a response \
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
