//! `Inertia` static facade - Laravel-style entrypoint for the most
//! common Inertia helpers.

use crate::FrameworkError;
use crate::pagination::IntoInertiaScroll;

use super::config::InertiaConfig;
use super::response::IntoInertiaData;
use super::{
    Inertia303Middleware, InertiaErrorPageMiddleware, InertiaHeadersMiddleware, InertiaResponse,
    InertiaValidationRedirectMiddleware, InertiaVersionMiddleware,
};

/// Static facade. Today it exposes `Inertia::paginate`; future helpers
/// (render, location, etc.) will land here.
pub struct Inertia;

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
    /// enforces default-deny - disallowed includes return 400.
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
    /// Registers four global middlewares in order:
    /// 1. [`InertiaHeadersMiddleware`] - sets `Vary: X-Inertia` on every
    ///    response and turns an empty `200` on an Inertia visit into a
    ///    `303` back. Registered first, so it wraps everything, including
    ///    the `409` the version middleware returns below.
    /// 2. [`InertiaVersionMiddleware`] - emits `409 Conflict` +
    ///    `X-Inertia-Location` when the client's `X-Inertia-Version`
    ///    header doesn't match the server's configured version.
    ///    Without it, asset-version mismatches are silent and stale
    ///    clients keep hitting the new server with the old bundle.
    /// 3. [`Inertia303Middleware`] - converts `302` redirects on
    ///    non-GET Inertia visits to `303`, so the client's follow-up
    ///    request is explicitly a GET. Without it, browsers may
    ///    re-submit the original PUT/PATCH/DELETE to the redirect
    ///    target - silently breaking form-create-then-redirect flows.
    /// 4. [`InertiaValidationRedirectMiddleware`] - turns a validation
    ///    `422` on an Inertia visit into a `303` back with the errors
    ///    flashed. Innermost, so it sees the handler's raw `422`; the
    ///    `303` it emits passes untouched through the `302 → 303`
    ///    conversion above. Without it the client sees a response with no
    ///    `X-Inertia` header, treats it as non-Inertia, and shows the
    ///    error modal instead of populating `form.errors`.
    ///
    /// A fifth, [`InertiaErrorPageMiddleware`], is registered innermost
    /// **only when** [`InertiaConfig::error_page`] names a component. It
    /// rewrites the framework's own error responses - a `403` denial, an
    /// unrouted `404`, a `429`, a `500` - into that page, so they stop
    /// reaching the client as the plain-JSON error modal. Without an
    /// `error_page` nothing is registered and error responses are
    /// untouched.
    ///
    /// Innermost is the wrong place for an app whose stack answers
    /// *before* the Inertia layer is reached - a `CsrfMiddleware`, rate
    /// limiter, or auth guard registered above this call never hands its
    /// rejection to anything registered inside it. Such an app registers
    /// [`InertiaErrorPageMiddleware`] itself, at the position it needs;
    /// `install` sees that registration, logs at `debug`, and skips its
    /// own, leaving both the app's placement and the component the app
    /// named intact. `error_page` on the config is then optional. See that
    /// type's documentation for where it may sit.
    ///
    /// One call wires all four, so an app cannot end up carrying two of
    /// them and silently missing the third - each closes a failure mode
    /// that surfaces only in production: cache poisoning across the two
    /// representations of a URL, a stale bundle after a deploy, a
    /// method-preserving redirect, and a form that reports its own
    /// validation errors as a crash.
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
        register_global_middleware(InertiaHeadersMiddleware::new());
        let version = config.version.clone();
        register_global_middleware(InertiaVersionMiddleware::with_resolver(move || {
            version.resolve()
        }));
        register_global_middleware(Inertia303Middleware::new());
        register_global_middleware(InertiaValidationRedirectMiddleware::new());
        // Innermost, and only when the app named a component. It has to
        // see the response the handler and the route middleware actually
        // produced - a `403` from `PermissionMiddleware` never reaches
        // the handler at all - and it deliberately declines the `422`
        // the validation middleware above it is about to bounce.
        match error_page_action(
            config.error_page.as_deref(),
            crate::middleware::has_global_middleware::<InertiaErrorPageMiddleware>(),
        ) {
            ErrorPageAction::Register(component) => {
                register_global_middleware(InertiaErrorPageMiddleware::new(component));
            }
            ErrorPageAction::KeepExisting => {
                tracing::debug!(
                    "an InertiaErrorPageMiddleware is already registered; keeping its position \
                     in the chain and the component it names, and skipping the one \
                     Inertia::install would add"
                );
            }
            ErrorPageAction::None => {}
        }
        Ok(())
    }
}

/// What [`Inertia::install`] does about the error-page middleware.
#[derive(Debug, PartialEq, Eq)]
enum ErrorPageAction {
    /// No `error_page` on the config: register nothing, as before.
    None,
    /// Register one innermost of the Inertia layer, for this component.
    Register(String),
    /// One is already in the chain. Leave it exactly where the app put
    /// it, rendering the component the app named.
    KeepExisting,
}

/// The whole decision, as a pure function of the two facts it reads.
///
/// Split out from [`Inertia::install`] because one of those facts is the
/// process-global middleware registry, shared by every test in the binary,
/// so the rule set itself would otherwise only be testable through
/// whatever registrations the rest of the suite happened to have made
/// first.
///
/// An app that registered the middleware itself named its component
/// there, and that instance is the one in the chain - so `install` has
/// nothing left to decide beyond staying out of the way. Registration is
/// idempotent per middleware type, so this only makes explicit what the
/// registry would have done anyway.
fn error_page_action(configured: Option<&str>, already_registered: bool) -> ErrorPageAction {
    match configured {
        None => ErrorPageAction::None,
        Some(_) if already_registered => ErrorPageAction::KeepExisting,
        Some(component) => ErrorPageAction::Register(component.to_string()),
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
        // Nothing configured: unchanged behaviour for an app that never
        // opted in.
        assert_eq!(
            error_page_action(None, false),
            ErrorPageAction::None,
            "no error_page means no middleware, whatever else is registered"
        );
        assert_eq!(error_page_action(None, true), ErrorPageAction::None);

        // The default: install places it.
        assert_eq!(
            error_page_action(Some("Error"), false),
            ErrorPageAction::Register("Error".to_string())
        );

        // The app placed it further out, ahead of a middleware that
        // answers before the Inertia layer is reached. Its position - and
        // the component it names - is what stands.
        assert_eq!(
            error_page_action(Some("Error"), true),
            ErrorPageAction::KeepExisting
        );
    }

    #[test]
    fn install_registers_the_protocol_middlewares() {
        // `install` also retains the config on the active container's
        // Inertia registry. Without this guard that write lands on the
        // global registry, and `response.rs`'s
        // `build_page_object_eager_only` - same binary, running in
        // parallel - would see `version = "test-version"` where it
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
            4,
            "Inertia::install should register exactly four middlewares (headers + version + 303 \
             + validation redirect), got delta={}",
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

        // The error-page middleware is the opt-in fifth. Both installs
        // live in this one test rather than in a sibling because
        // registration is idempotent per type and process-global: two
        // tests each measuring their own delta would race over which of
        // them registered the four shared types.
        //
        // Known side effect: `TestContainer::fake` scopes
        // `set_installed_config`, but `register_global_middleware` really
        // is process-global, so from here on every test in this binary
        // that builds a chain from `get_global_middleware()` carries an
        // error-page rewrite. Nothing depends on its absence today; if
        // something ever does, the fix is a registry the container owns,
        // not moving this assertion somewhere it would race.
        Inertia::install(
            &InertiaConfig::new()
                .version("test-version")
                .development(true)
                .error_page("Error"),
        )
        .expect("dev-mode install must not require a manifest");
        let with_error_page = get_global_middleware().len();
        assert_eq!(
            with_error_page - after,
            1,
            "naming an error page adds exactly one middleware on top of the four"
        );

        // An error page already in the chain keeps its position - which is
        // the whole point of letting an app register it further out, ahead
        // of a CSRF middleware or a rate limiter that answers before the
        // Inertia layer is reached. `install` must not append a second.
        Inertia::install(
            &InertiaConfig::new()
                .version("test-version")
                .development(true)
                .error_page("Error"),
        )
        .expect("dev-mode install must not require a manifest");
        assert_eq!(
            get_global_middleware().len(),
            with_error_page,
            "an error page already registered must not be joined by a second"
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
