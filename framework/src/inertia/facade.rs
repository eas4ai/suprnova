//! `Inertia` static facade — Laravel-style entrypoint for the most
//! common Inertia helpers.

use crate::FrameworkError;
use crate::pagination::IntoInertiaScroll;

use super::config::InertiaConfig;
use super::response::IntoInertiaData;
use super::{Inertia303Middleware, InertiaResponse, InertiaVersionMiddleware};

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
    /// Registers two global middlewares in order:
    /// 1. [`InertiaVersionMiddleware`] — emits `409 Conflict` +
    ///    `X-Inertia-Location` when the client's `X-Inertia-Version`
    ///    header doesn't match the server's configured version.
    ///    Without it, asset-version mismatches are silent and stale
    ///    clients keep hitting the new server with the old bundle.
    /// 2. [`Inertia303Middleware`] — converts `302` redirects on
    ///    non-GET Inertia visits to `303`, so the client's follow-up
    ///    request is explicitly a GET. Without it, browsers may
    ///    re-submit the original PUT/PATCH/DELETE to the redirect
    ///    target — silently breaking form-create-then-redirect flows.
    ///
    /// Both middlewares were previously opt-in via the `global_middleware!`
    /// macro, which meant generated apps that forgot either one quietly
    /// got stale-asset behaviour or method-preserving redirects in
    /// production. Calling this helper at boot guarantees both are wired.
    ///
    /// Call once at boot. The `config.version` value is cloned out of
    /// the supplied `InertiaConfig` so callers can keep ownership of
    /// the config for `InertiaResponse::with_config(...)`.
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
    /// rather than booting with a broken encryption key. Middleware
    /// registration does not happen when this returns `Err` — nothing is
    /// half-installed.
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
    fn install_registers_two_middlewares() {
        let before = get_global_middleware().len();
        // Dev mode (the default outside APP_ENV=production) never
        // consults the manifest, so this succeeds even though no Vite
        // build exists in the test process's working directory.
        Inertia::install(&InertiaConfig::new().version("test-version"))
            .expect("dev-mode install must not require a manifest");
        let after = get_global_middleware().len();
        assert_eq!(
            after - before,
            2,
            "Inertia::install should register exactly two middlewares \
             (version + 303), got delta={}",
            after - before
        );
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
        // here (unlike `install_registers_two_middlewares`): that
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

        let err = Inertia::install(&cfg)
            .expect_err("production install without a manifest must fail closed");
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
