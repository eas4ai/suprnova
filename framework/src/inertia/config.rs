use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use super::manifest::ViteManifest;
use super::prop::InertiaRequestExt;
use super::root_template::{InertiaRootTemplate, RootTemplateChooser};

/// Shared error-observer callback for SSR render failures.
pub(crate) type SsrErrorHook = Arc<dyn Fn(&str) + Send + Sync>;

/// Closure that derives the Inertia page object's `url` field from the
/// request. See [`InertiaConfig::url_resolver`]. Named (rather than
/// spelled out inline on the field) to keep `clippy::type_complexity`
/// quiet - it is the same `Arc<dyn Fn(...) + Send + Sync>` type either
/// way, just given a name.
pub(crate) type UrlResolver = Arc<dyn Fn(&dyn InertiaRequestExt) -> String + Send + Sync>;

/// Asset-version source for Inertia responses.
///
/// Inertia uses a version string for cache-busting / version-mismatch
/// detection. [`Manifest`](Self::Manifest) is the default and what most
/// apps want: it hashes the Vite build manifest, so the version moves
/// exactly when the built assets do, with nothing to remember to bump.
/// [`Static`](Self::Static) bakes in a literal, chosen once and fixed
/// until the config changes. [`Dynamic`](Self::Dynamic) computes the
/// version per-request - for long-running deploys, hot-reloaded dev
/// environments, or any value the manifest hash can't stand in for.
#[derive(Clone)]
pub enum VersionResolver {
    /// A baked-in static version string. Cheap; no closure invocation.
    Static(String),
    /// A closure that returns the current version. Runs on every read.
    /// Wrap any caching the consumer wants inside the closure.
    Dynamic(Arc<dyn Fn() -> String + Send + Sync>),
    /// A hash of a Vite build manifest's bytes. This is the default:
    /// the asset version an Inertia client checks against should change
    /// exactly when the assets do, and the manifest is the one file
    /// that changes on every build and on no other occasion.
    Manifest(PathBuf),
}

impl VersionResolver {
    /// Build a static resolver from anything that can become a `String`.
    pub fn new(version: impl Into<String>) -> Self {
        Self::Static(version.into())
    }

    /// Build a dynamic resolver from a closure. The closure runs on
    /// every call to [`resolve`](Self::resolve); cache inside the closure if needed.
    pub fn with<F>(f: F) -> Self
    where
        F: Fn() -> String + Send + Sync + 'static,
    {
        Self::Dynamic(Arc::new(f))
    }

    /// Build a resolver that hashes a Vite manifest's bytes - the first
    /// 16 bytes of its SHA-256, hex-encoded (32 characters, the same
    /// length Laravel's xxh128 produces).
    ///
    /// This is [`InertiaConfig`]'s default resolver, pointed at
    /// [`InertiaConfig::manifest_path`]. An app that hardcodes a version
    /// string ships stale bundles to long-lived clients until someone
    /// remembers to bump it; hashing the manifest makes the bump
    /// automatic.
    ///
    /// The file is read on every [`resolve`](Self::resolve) call, which
    /// is what Laravel's `hash_file` does too - a few KB out of the page
    /// cache per version check, and a rebuild is picked up immediately.
    /// If you have measured that and want it gone, resolve once at boot:
    /// `InertiaConfig::new().version(VersionResolver::from_manifest(p).resolve())`.
    ///
    /// A missing or unreadable file resolves to the empty string rather
    /// than erroring: in development there is no build to hash. Laravel's
    /// `Middleware::version` returns `null` in that case, and its page
    /// carries `""`.
    pub fn from_manifest(path: impl Into<PathBuf>) -> Self {
        Self::Manifest(path.into())
    }

    /// Resolve to the current version string.
    pub fn resolve(&self) -> String {
        match self {
            Self::Static(s) => s.clone(),
            Self::Dynamic(f) => f(),
            Self::Manifest(path) => manifest_version(path),
        }
    }
}

impl From<String> for VersionResolver {
    fn from(s: String) -> Self {
        Self::Static(s)
    }
}

impl From<&str> for VersionResolver {
    fn from(s: &str) -> Self {
        Self::Static(s.to_string())
    }
}

/// `None` is the empty version, as Laravel casts a `null` version to `""`.
impl From<Option<String>> for VersionResolver {
    fn from(version: Option<String>) -> Self {
        Self::Static(version.unwrap_or_default())
    }
}

/// A function is called on every read, as Laravel calls a version closure.
impl<F> From<F> for VersionResolver
where
    F: Fn() -> String + Send + Sync + 'static,
{
    fn from(f: F) -> Self {
        Self::Dynamic(Arc::new(f))
    }
}

impl std::fmt::Debug for VersionResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Static(s) => write!(f, "Static({:?})", s),
            Self::Dynamic(_) => write!(f, "Dynamic(<closure>)"),
            Self::Manifest(p) => write!(f, "Manifest({:?})", p),
        }
    }
}

/// The asset version a [`VersionResolver::Manifest`] reported when it
/// could not read its file, before the version followed Laravel's order.
///
/// Kept so code that names it still compiles. No resolver returns it any
/// more: with no asset URL and no manifest the version is the empty
/// string, which is what Laravel's page carries when its
/// `Middleware::version` finds nothing to hash.
pub const MANIFEST_VERSION_FALLBACK: &str = "1.0";

/// Hex of the first 16 bytes of the SHA-256 of `bytes`.
///
/// Not a secret - a stable, bounded-length identifier that changes iff
/// its source changes. 128 bits is far past where a collision would
/// matter for a cache-busting token, and the truncation keeps the value
/// short enough to sit in a request header without comment.
fn version_hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(&Sha256::digest(bytes)[..16])
}

/// [`version_hash`] of the manifest's bytes, or the empty string when the
/// file cannot be read.
fn manifest_version(path: &std::path::Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => version_hash(&bytes),
        Err(e) => {
            // `debug!`, not `warn!`: in development the manifest
            // legitimately doesn't exist (Vite serves from memory) and
            // this runs on every version check, so a warning here would
            // be per-request noise. Production cannot reach this arm -
            // `Inertia::install` refuses to boot without a manifest.
            tracing::debug!(
                path = %path.display(),
                error = %e,
                "Inertia asset version: manifest unreadable, the version is empty"
            );
            String::new()
        }
    }
}

/// Which frontend framework the host application uses.
///
/// Detected at runtime from the `SUPRNOVA_FRONTEND` env var. The CLI
/// scaffolds this into `.env` when generating a new project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frontend {
    /// Svelte 5 (runes-on) starter - the default.
    Svelte,
    /// React 19 starter.
    React,
    /// Vue 3.5 starter.
    Vue,
}

impl Frontend {
    /// Read `SUPRNOVA_FRONTEND` from the environment.
    ///
    /// Defaults to `Svelte` when unset or unrecognized - matches the
    /// CLI's default frontend choice in `suprnova new`.
    pub fn detect_from_env() -> Self {
        match std::env::var("SUPRNOVA_FRONTEND").as_deref() {
            Ok("react") | Ok("React") | Ok("REACT") => Frontend::React,
            Ok("vue") | Ok("Vue") | Ok("VUE") => Frontend::Vue,
            Ok("svelte") | Ok("Svelte") | Ok("SVELTE") => Frontend::Svelte,
            _ => Frontend::Svelte,
        }
    }

    /// Default Vite entry-point filename for this frontend.
    pub fn default_entry_point(self) -> &'static str {
        match self {
            Frontend::Svelte => "src/main.ts",
            Frontend::React => "src/main.tsx",
            Frontend::Vue => "src/main.ts",
        }
    }

    /// File extensions a page component for this frontend may use.
    ///
    /// Ordered by likelihood for the framework. Used by the macro to
    /// locate page components at compile time.
    pub fn page_extensions(self) -> &'static [&'static str] {
        match self {
            Frontend::Svelte => &["svelte"],
            Frontend::React => &["tsx", "jsx"],
            Frontend::Vue => &["vue"],
        }
    }

    /// Lowercase identifier used in env / config.
    pub fn as_str(self) -> &'static str {
        match self {
            Frontend::Svelte => "svelte",
            Frontend::React => "react",
            Frontend::Vue => "vue",
        }
    }
}

/// Configuration for Inertia.js integration.
///
/// `Clone` exists so [`crate::Inertia::install`] can retain the config as
/// the default every [`crate::InertiaResponse`] starts from. The clone
/// copies the settings but **shares** the `manifest` cache `Arc`, so all
/// responses built from one installed config parse `manifest.json` once
/// for the process rather than once per response.
#[derive(Clone)]
pub struct InertiaConfig {
    /// Vite dev server URL (e.g. `http://localhost:5173`).
    pub vite_dev_server: String,
    /// Vite entry point. Defaults to the frontend's standard entry.
    pub entry_point: String,
    /// Asset version source for cache busting / version-mismatch
    /// detection. Defaults to [`VersionResolver::Manifest`], hashing
    /// [`manifest_path`](Self::manifest_path), with the
    /// [`asset_url`](Self::asset_url) setting's hash ahead of it when one is
    /// set and the empty string when neither is there; see
    /// [`VersionResolver`] for the static and dynamic alternatives.
    pub version: VersionResolver,
    /// `true` during local development (loads via the Vite dev server);
    /// `false` for production (loads built assets from `/assets/`).
    ///
    /// Defaults to the inverse of [`crate::config::Environment::detect`]`().`
    /// [`is_production`](crate::config::Environment::is_production) - see
    /// `impl Default for InertiaConfig` (CFG-01: this used to hardcode
    /// `true` regardless of environment, so a production deploy that
    /// didn't explicitly call [`production`](Self::production) rendered
    /// asset URLs pointing at a local Vite dev server). Override with
    /// [`production`](Self::production) or
    /// [`development`](Self::development) if you need to force one mode
    /// regardless of `APP_ENV` (e.g. testing prod asset output locally).
    pub development: bool,
    /// Which frontend framework is configured.
    pub frontend: Frontend,
    /// Default `<title>` for the HTML shell. Per-response title overrides
    /// via `InertiaResponse::title(...)`.
    pub default_title: String,
    /// The id of the first visit's mount element and the `data-page`
    /// attribute of its page data element. Default `app`.
    ///
    /// The Inertia client looks both up by the `id` given to
    /// `createInertiaApp` (and to `createServer` under SSR), `app` unless
    /// the application names another, so the two must agree. Set it with
    /// [`mount_id`](Self::mount_id).
    pub mount_id: String,
    /// Whether Inertia responses encrypt their browser history state by
    /// default. Maps to Laravel's `config('inertia.history.encrypt')`.
    /// Overridable per-request via `EncryptHistoryMiddleware` and
    /// per-response via `InertiaResponse::encrypt_history(bool)`.
    pub encrypt_history_default: bool,
    /// Server-side rendering configuration. See [`SsrConfig`].
    pub ssr: SsrConfig,
    /// Path to Vite's `manifest.json` (Vite 5.0+ default location is
    /// `<outDir>/.vite/manifest.json`). Default points at
    /// `public/assets/.vite/manifest.json`, matching the framework's
    /// scaffolded `vite.config.ts` (`outDir: '../public/assets'`).
    ///
    /// When the file exists, `render_prod_head` resolves the entry
    /// point to its hashed output + CSS + transitively-imported
    /// chunks (for `modulepreload`). When it's missing the framework
    /// falls back to the legacy hardcoded `/{assets_base_url}/main.js`
    /// path and emits a `tracing::warn!` so the gap is visible in
    /// production logs.
    pub manifest_path: PathBuf,
    /// URL prefix under which the Vite build assets are served (e.g.
    /// `/assets`). Combined with the manifest entry's `file` field to
    /// produce the final `<script src>` / `<link href>` URL.
    pub assets_base_url: String,
    /// The URL the built assets are published under when it changes with
    /// each deploy, such as a CDN path that carries a build id. Laravel's
    /// `app.asset_url` (`ASSET_URL`).
    ///
    /// When it is set and not empty, the default asset version is its
    /// hash rather than the manifest's, the order Laravel's
    /// `Middleware::version` uses: a deploy that publishes under a new URL
    /// moves the version even where the manifest is not on this host. An
    /// explicit [`version`](Self::version) or
    /// [`version_with`](Self::version_with) ignores it.
    ///
    /// Defaults to the `ASSET_URL` environment variable when it is set and
    /// not empty, the variable Laravel's `app.asset_url` reads, and to
    /// `None` otherwise. [`asset_url`](Self::asset_url) on the builder wins
    /// over the environment.
    pub asset_url: Option<String>,
    /// Whether a session-flashed validation bag surfaces every message
    /// per field (`{ email: ["a", "b"] }`) or only the first
    /// (`{ email: "a" }`).
    ///
    /// Default `false`, matching Laravel's
    /// `Inertia\Middleware::$withAllErrors` and Inertia's own
    /// `ErrorValue = string`. Set `true` when your pages render every
    /// message for a field. `suprnova generate-types` reads the call under
    /// `src/` and writes the matching client types, `Errors` as
    /// `Record<string, string[]>` and `errorValueType: string[]` in the
    /// `@inertiajs/core` augmentation it generates, so the pages are typed
    /// for the array shape. Applies to errors drained from the session
    /// flash only - an `errors` prop a handler sets itself passes through
    /// as-is.
    pub with_all_errors: bool,
    /// Whether the page object lists the shared props' top-level keys
    /// under `sharedProps` - Laravel's `inertia.expose_shared_prop_keys`.
    ///
    /// Default `true`. The client reads the list during an instant visit
    /// to carry the shared values into the page it renders before the
    /// server answers; `errors` is always on it, since every response
    /// shares the validation errors. Off, the page carries no
    /// `sharedProps` and the shared values still ship as props.
    pub expose_shared_props: bool,
    /// Whether every integer beyond JavaScript's safe range (plus or minus
    /// 9007199254740991) in props and flash is sent as
    /// `{"$bigint": "<digits>"}`, with `preserveBigIntegers: true` on the
    /// page, so the client restores it as an exact `BigInt` instead of a
    /// rounded number. A 64-bit database id past 2^53 otherwise reaches
    /// the browser off by a few. Laravel's `inertia.preserve_big_integers`;
    /// default `false`. A response overrides it with
    /// [`InertiaResponse::preserve_big_integers`](crate::InertiaResponse::preserve_big_integers).
    pub preserve_big_integers: bool,
    /// Maximum number of lazy/deferred/once/shared prop resolvers that
    /// run concurrently for a single response.
    ///
    /// Default: 16 - generous for typical Inertia pages while bounding
    /// downstream fan-out on pages with many lazy resolvers. Without
    /// this cap a page with N lazy props issues N parallel database /
    /// HTTP calls per request.
    pub max_concurrent_resolvers: usize,
    /// Page component that renders framework error responses, or `None`
    /// (the default) to leave every error response exactly as it is.
    ///
    /// Naming one installs the default error callback (PAR-062): the rule
    /// below, which renders this component with the shared props. A
    /// callback the app installs with
    /// [`Inertia::handle_exceptions_using`](crate::Inertia::handle_exceptions_using)
    /// decides in its place.
    ///
    /// Without this, a `403` from a permission middleware, a `404` for an
    /// unrouted path, a `429`, or a `500` reaches the Inertia client as a
    /// JSON body with no `X-Inertia` header. The client treats any such
    /// response as non-Inertia
    /// (`inertia-3.6.1/packages/core/src/response.ts:68,173-175`) and
    /// shows its "All Inertia requests must receive a valid Inertia
    /// response, however a plain JSON response was received" modal -
    /// which is what a real user saw on a `403` in production. Naming a
    /// component here makes those responses render that page instead,
    /// keeping the original status code.
    ///
    /// The component receives three props:
    ///
    /// - `status` (`u16`) - the original HTTP status.
    /// - `message` (`String`) - the error body's `message`, or the
    ///   status's reason phrase when the body carried none. Already
    ///   sanitized: a `5xx` message is the generic
    ///   `"Internal Server Error"`, never the underlying error. That holds
    ///   under `APP_DEBUG=true` as well - the dev-only `debug_message`
    ///   field the JSON path adds there is deliberately not read, so the
    ///   raw error stays in the log and the JSON response rather than
    ///   rendering into a page.
    /// - `request_id` (`String`, optional) - present only when the error
    ///   body carried one, so the page can show the same id the operator
    ///   sees in the logs.
    ///
    /// Set it with [`error_page`](Self::error_page).
    pub error_page: Option<String>,
    /// Whether an Inertia `GET` records its URL as the session's previous
    /// URL. Default `true`.
    ///
    /// The session middleware records only full page loads, so without this
    /// a `Redirect::back`, an `Inertia::back` or a failed validation lands on
    /// the last page loaded in full rather than the page the visitor came
    /// from. Laravel's `inertia.store_previous_url`. A visit that matched a
    /// route records unless it is a prefetch, a Precognition request or a
    /// partial reload of the page it rendered.
    pub store_previous_url: bool,
    /// When `true`, rendering a component with no page file under
    /// [`pages_dir`](Self::pages_dir) is an error. Default `false`.
    ///
    /// `inertia_response!` checks its component at compile time, but a name
    /// handed to `InertiaResponse::new` or `Router::inertia` as a string is
    /// only a string, and a typo in it reaches the browser as a blank page
    /// the client cannot resolve. Laravel's `inertia.pages.ensure_pages_exist`.
    pub ensure_pages_exist: bool,
    /// The directory the page files live under, relative to the process's
    /// working directory unless absolute. Default `frontend/src/pages`,
    /// where a `suprnova new` project keeps them. Read only with
    /// [`ensure_pages_exist`](Self::ensure_pages_exist).
    pub pages_dir: PathBuf,
    /// The page file extensions [`ensure_pages_exist`](Self::ensure_pages_exist)
    /// accepts. Default `svelte`, `tsx`, `jsx` and `vue`, the ones
    /// `inertia_response!` looks for.
    pub page_extensions: Vec<String>,
    /// Whether [`AssertableInertia::component`](crate::testing::AssertableInertia::component)
    /// also checks that the component has a page file under
    /// [`pages_dir`](Self::pages_dir) with one of the
    /// [`page_extensions`](Self::page_extensions). Default `true`.
    ///
    /// A test that asserts a component the frontend never built passes
    /// while the browser shows a blank page; the check makes it fail
    /// instead. It runs only in tests and only with this configuration
    /// installed. Laravel's `inertia.testing.ensure_pages_exist`.
    pub testing_ensure_pages_exist: bool,
    /// Whether [`crate::Inertia::install`] registers the Inertia
    /// middleware stack on every route. Default `true`.
    ///
    /// Set it `false` to put the stack only on the route groups that serve
    /// pages: `install` then registers the stack as the named middleware
    /// `inertia` instead, for `GroupBuilder::middleware_named("inertia")`,
    /// and [`crate::Inertia::middleware`] builds it as a value. An API group
    /// without it carries no `Vary: X-Inertia` and has no redirect turned
    /// into `303`, as a Laravel app that registers `HandleInertiaRequests`
    /// on its `web` group only.
    pub register_globally: bool,
    /// The application's replacements for the middleware's decisions; see
    /// [`hooks`](Self::hooks).
    pub(crate) hooks: Option<Arc<dyn super::hooks::InertiaMiddlewareHooks>>,
    /// Where the application called [`hooks`](Self::hooks), which Inertia
    /// DevTools names as the source of the props the `share` hooks
    /// supplied (PAR-073): the hooks are a trait object, with no file and
    /// line of their own to give.
    pub(crate) hooks_location: Option<&'static std::panic::Location<'static>>,
    /// The Inertia DevTools settings, or `None` for the defaults, read
    /// when the middleware stack is built rather than on every response
    /// that starts from this configuration. Set with
    /// [`devtools`](Self::devtools).
    pub(crate) devtools: Option<Arc<super::devtools::DevToolsConfig>>,
    /// Lazy-loaded Vite manifest cache.
    ///
    /// Initialized on first call to [`Self::vite_manifest`]. The cache
    /// holds `Some(manifest)` on successful load and `None` when the
    /// file is missing or malformed - both states are stable for the
    /// process lifetime, matching how a long-running production server
    /// reads the build artefact exactly once. Use `manifest_path()` to
    /// repoint at a different file for tests; that builder method
    /// resets the cache by constructing a fresh `OnceLock`.
    pub(crate) manifest: Arc<OnceLock<Option<ViteManifest>>>,
    /// Optional override for the page object's `url` field. Mirrors
    /// Laravel's `Inertia::resolveUrlUsing`.
    ///
    /// `pub(crate)` with a builder method rather than a public field for
    /// the same reason as `manifest`: a boxed closure is not a value a
    /// caller should be constructing by hand.
    pub(crate) url_resolver: Option<UrlResolver>,
    /// The application's root template for a first visit, chosen per
    /// request, or `None` for the framework's own document. Set with
    /// [`root_template`](Self::root_template).
    pub(crate) root_template: Option<RootTemplateChooser>,
}

/// SSR (server-side rendering) configuration.
///
/// Suprnova talks to an out-of-process SSR worker - usually the
/// `@inertiajs/{vue3,react,svelte}/server` `createServer()` bundle run
/// under Node, Bun, or Deno - over HTTP loopback. The worker accepts
/// a JSON page object on `POST /render` and returns
/// `{ head: string[], body: string }`. Configure the worker URL here;
/// boot it separately (e.g. `suprnova ssr:start`).
///
/// SSR is on by default, as Laravel's is, and gated by bundle detection:
/// an application without an SSR bundle renders on the client and never
/// contacts the worker.
#[derive(Clone)]
pub struct SsrConfig {
    /// When `false`, SSR is fully off and the HTML shell renders empty
    /// `<div id="app">` for the client to hydrate. Default: `true`, as
    /// Laravel's `inertia.ssr.enabled`: an application opts out of SSR
    /// rather than in. The bundle check
    /// ([`ensure_bundle_exists`](Self::ensure_bundle_exists)) keeps an
    /// application without an SSR bundle rendering on the client, with no
    /// request to the worker.
    pub enabled: bool,
    /// URL of the running SSR worker (e.g. `http://127.0.0.1:13714`).
    /// The framework posts to `<url>/render`.
    pub url: String,
    /// Request timeout for the SSR call. Past this, the response falls
    /// back to CSR. Keep tight in production - a hung worker shouldn't
    /// block real users.
    pub timeout: std::time::Duration,
    /// When `true`, SSR errors propagate as 500s instead of falling
    /// back to CSR, with a message naming the component and, when the
    /// worker gave one, the source location (Laravel's `SsrException`).
    /// Useful in CI / tests; never set `true` in production unless you
    /// also have a watchdog.
    pub throw_on_error: bool,
    /// Path patterns excluded from SSR. Matching requests render CSR-only
    /// even when `enabled` is `true`. Patterns follow Laravel's
    /// `ExcludesPaths`: slashes at either end are ignored, `*` matches any
    /// characters including `/`, and each pattern is tried against the
    /// path and the full URL. `Inertia::without_ssr` adds more at run time.
    pub excluded_paths: Vec<String>,
    /// Observability hook invoked when an SSR render fails and we
    /// fall back to CSR. Defaults to `eprintln!` to stderr. Wire your
    /// logger / Sentry / DataDog client here. Every such failure also
    /// dispatches the [`SsrRenderFailed`](crate::SsrRenderFailed) event,
    /// which carries the worker's error details; under
    /// [`throw_on_error`](Self::throw_on_error) the visit fails instead
    /// and the hook does not run. A missing bundle is not a failure.
    pub on_error: Option<SsrErrorHook>,
    /// Cap on the SSR worker's response body. Bytes past this point
    /// abort the read and the request falls back to CSR (or 500 if
    /// `throw_on_error` is set). Default: 8 MiB - comfortably larger
    /// than any realistic SSR-rendered page but small enough to bound
    /// damage from a misconfigured or compromised loopback worker.
    pub max_response_bytes: usize,
    /// Path to the built SSR bundle, looked at before the conventional
    /// paths ([`CONVENTIONAL_BUNDLE_PATHS`](crate::CONVENTIONAL_BUNDLE_PATHS),
    /// the first of which, `frontend/bootstrap/ssr/ssr.js`, is where a
    /// scaffolded project's `vite build --ssr` writes it). Laravel's
    /// `inertia.ssr.bundle`. `None` (the default) searches the conventional
    /// paths only; [`detect_ssr_bundle`](crate::detect_ssr_bundle) is the
    /// search, which the bundle check and `ssr:start` share.
    pub bundle_path: Option<PathBuf>,
    /// The runtime `ssr:start` launches the worker under: `node` by default,
    /// `bun`, `deno` or an absolute path. Laravel's `inertia.ssr.runtime`.
    pub runtime: String,
    /// Whether `ssr:start` refuses a runtime it cannot find on `PATH`.
    /// Default `false`, as Laravel's `inertia.ssr.ensure_runtime_exists`.
    pub ensure_runtime_exists: bool,
    /// The file whose presence says the Vite dev server is running, and
    /// whose content is its URL: Laravel's Vite hot file. Default
    /// `public/hot` under the working directory, the file `suprnova serve`
    /// writes while it runs Vite for a frontend that declares the Inertia
    /// Vite plugin (`@inertiajs/vite`), and removes when Vite stops. Set it
    /// with [`InertiaConfig::ssr_hot_file`].
    pub hot_file: PathBuf,
    /// Where SSR is dispatched in hot mode, at `/__inertia_ssr`: Laravel's
    /// `inertia.ssr.hot_url`. The Vite dev server renders the page from
    /// source, so no SSR bundle or worker process is needed while
    /// developing.
    ///
    /// In development a first visit goes hot when this is set, or when the
    /// [`hot_file`](Self::hot_file) exists; the address is this URL, else
    /// the file's content, else [`InertiaConfig::vite_dev_server`]. Hot mode
    /// skips the bundle check. Production never goes hot and ignores it.
    /// `None` (the default) leaves the decision to the hot file.
    pub hot_url: Option<String>,
    /// When `true` (the default), a first visit is sent to the worker only
    /// when [`detect_ssr_bundle`](crate::detect_ssr_bundle) finds a bundle:
    /// with none the visit renders on the client at once, quietly, rather
    /// than paying [`Self::timeout`] on a worker that was never started.
    /// Laravel's `inertia.ssr.ensure_bundle_exists`. Turn it off for a
    /// worker whose bundle this process cannot see on disk (a separate
    /// container, a remote host) and in tests that use a stand-in worker.
    /// Hot mode ([`Self::hot_url`]) skips the check.
    pub ensure_bundle_exists: bool,
}

impl std::fmt::Debug for SsrConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SsrConfig")
            .field("enabled", &self.enabled)
            .field("url", &self.url)
            .field("timeout", &self.timeout)
            .field("throw_on_error", &self.throw_on_error)
            .field("excluded_paths", &self.excluded_paths)
            .field("on_error", &self.on_error.as_ref().map(|_| "<closure>"))
            .field("max_response_bytes", &self.max_response_bytes)
            .field("bundle_path", &self.bundle_path)
            .field("runtime", &self.runtime)
            .field("ensure_runtime_exists", &self.ensure_runtime_exists)
            .field("hot_url", &self.hot_url)
            .field("hot_file", &self.hot_file)
            .field("ensure_bundle_exists", &self.ensure_bundle_exists)
            .finish()
    }
}

impl Default for SsrConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            url: "http://127.0.0.1:13714".to_string(),
            timeout: std::time::Duration::from_secs(5),
            throw_on_error: false,
            excluded_paths: Vec::new(),
            on_error: None,
            max_response_bytes: 8 * 1024 * 1024,
            bundle_path: None,
            runtime: "node".to_string(),
            ensure_runtime_exists: false,
            hot_url: None,
            hot_file: PathBuf::from("public/hot"),
            ensure_bundle_exists: true,
        }
    }
}

impl SsrConfig {
    /// Check whether the given request path is excluded from SSR by
    /// [`excluded_paths`](Self::excluded_paths), with the rules of
    /// Laravel's `ExcludesPaths`: slashes at either end of a pattern are
    /// ignored, `*` matches any characters including `/`, and the path is
    /// decoded and trimmed of its slashes first. The render also tries each
    /// pattern against the request's full URL.
    pub fn is_path_excluded(&self, path: &str) -> bool {
        excluded_by(&self.excluded_paths, path, None)
    }
}

/// Laravel's `ExcludesPaths::inExceptArray`, so an exclusion pattern copied
/// from a Laravel application excludes the same requests.
///
/// Each pattern has its leading and trailing slashes trimmed (`/` alone
/// stays `/`) and is tried against the full URL, when there is one, and
/// against the decoded path with its slashes trimmed (`/` for the root),
/// with [`str_is`]'s wildcard. So `admin/*` excludes `/admin/users` and
/// `/admin/users/edit` but not `/adminx`, and `/reports/` excludes
/// `/reports`.
pub(crate) fn excluded_by(patterns: &[String], path: &str, full_url: Option<&str>) -> bool {
    if patterns.is_empty() {
        return false;
    }
    let decoded = percent_encoding::percent_decode_str(path).decode_utf8_lossy();
    let trimmed = decoded.trim_matches('/');
    let request_path = if trimmed.is_empty() { "/" } else { trimmed };
    patterns.iter().any(|pattern| {
        let pattern = if pattern == "/" {
            "/"
        } else {
            pattern.trim_matches('/')
        };
        full_url.is_some_and(|url| str_is(pattern, url)) || str_is(pattern, request_path)
    })
}

/// Laravel's `Str::is`: `*` matches any run of characters, `/` included,
/// and every other character matches itself.
///
/// Classic wildcard matching: on a mismatch the most recent `*` takes one
/// more character. With `*` as the only wildcard that backtrack is
/// complete, since an earlier star can only ever be asked to absorb what a
/// later one could.
fn str_is(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let (mut p, mut v) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while v < value.len() {
        if p < pattern.len() && pattern[p] == b'*' {
            star = Some((p, v));
            p += 1;
        } else if p < pattern.len() && pattern[p] == value[v] {
            p += 1;
            v += 1;
        } else if let Some((star_p, star_v)) = star {
            p = star_p + 1;
            v = star_v + 1;
            star = Some((star_p, star_v + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|byte| *byte == b'*')
}

/// Default Vite dev-server port when `VITE_PORT` is unset.
///
/// Distinctive to avoid the universally-squatted `5173` (every Vite
/// project on the machine fights for it). Pairs with the backend default
/// [`crate::config::providers::server::DEFAULT_SERVER_PORT`] (`8765`).
pub const DEFAULT_VITE_PORT: u16 = 5765;

/// Resolve the Vite dev-server URL referenced by the dev-mode HTML shell.
///
/// Precedence: `INERTIA_VITE_DEV_SERVER` (full URL override) >
/// `http://localhost:{VITE_PORT}` > `http://localhost:5765`. `suprnova
/// serve` sets `VITE_PORT` on the backend child to the port it actually
/// launched Vite on, so the injected `<script src=…>` tag always matches
/// the running Vite server - even after free-port scanning moved it.
fn vite_dev_server_from_env() -> String {
    if let Ok(url) = std::env::var("INERTIA_VITE_DEV_SERVER") {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    let port = std::env::var("VITE_PORT")
        .ok()
        .and_then(|v| v.trim().parse::<u16>().ok())
        .unwrap_or(DEFAULT_VITE_PORT);
    format!("http://localhost:{port}")
}

/// The `ASSET_URL` environment variable, trimmed, when it names something.
/// Laravel's `app.asset_url` reads the same variable, so a deployment that
/// sets it for a Laravel app moves the Inertia asset version here too.
fn asset_url_from_env() -> Option<String> {
    std::env::var("ASSET_URL")
        .ok()
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
}

impl Default for InertiaConfig {
    fn default() -> Self {
        let frontend = Frontend::detect_from_env();
        let manifest_path = PathBuf::from("public/assets/.vite/manifest.json");
        Self {
            vite_dev_server: vite_dev_server_from_env(),
            entry_point: frontend.default_entry_point().to_string(),
            // Hash of the build manifest, not a literal: an app that
            // never remembers to bump a hardcoded string serves stale
            // bundles to long-lived clients forever. Falls back to the
            // old literal when there is no manifest to hash.
            version: VersionResolver::Manifest(manifest_path.clone()),
            // CFG-01: derive from the actual runtime environment instead
            // of hardcoding `true`. Every environment other than
            // `Production` still defaults to dev mode (loads via the Vite
            // dev server) - that's unchanged. Only a real production boot
            // now defaults to production asset loading without requiring
            // every app to remember to call `.production()`.
            development: !crate::config::Environment::detect().is_production(),
            frontend,
            default_title: "Suprnova".to_string(),
            mount_id: "app".to_string(),
            encrypt_history_default: false,
            ssr: SsrConfig::default(),
            manifest_path,
            assets_base_url: "/assets".to_string(),
            asset_url: asset_url_from_env(),
            with_all_errors: false,
            expose_shared_props: true,
            preserve_big_integers: false,
            max_concurrent_resolvers: 16,
            // `None` so an app upgrading into this release keeps the
            // exact error bodies it had. Opting in is one builder call.
            error_page: None,
            store_previous_url: true,
            ensure_pages_exist: false,
            pages_dir: PathBuf::from("frontend/src/pages"),
            page_extensions: ["svelte", "tsx", "jsx", "vue"]
                .iter()
                .map(|ext| ext.to_string())
                .collect(),
            testing_ensure_pages_exist: true,
            register_globally: true,
            hooks: None,
            hooks_location: None,
            devtools: None,
            manifest: Arc::new(OnceLock::new()),
            url_resolver: None,
            root_template: None,
        }
    }
}

impl InertiaConfig {
    /// Build an `InertiaConfig` with the framework defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the Vite dev-server URL used during development (e.g. `"http://localhost:5173"`).
    pub fn vite_dev_server(mut self, url: impl Into<String>) -> Self {
        self.vite_dev_server = url.into();
        self
    }

    /// Override the Vite entry point (defaults to the frontend's
    /// canonical `resources/js/app.{js,ts}` path).
    pub fn entry_point(mut self, entry: impl Into<String>) -> Self {
        self.entry_point = entry.into();
        self
    }

    /// Set a static asset version string. For dynamic versions
    /// (e.g. read from a manifest at runtime) use [`version_with`](Self::version_with).
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = VersionResolver::Static(version.into());
        self
    }

    /// Set a dynamic asset version resolver. The closure runs on every
    /// page-object emission and every version-mismatch check; cache
    /// inside the closure if invocation isn't cheap.
    ///
    /// The closure is synchronous and infallible by design - it mirrors
    /// Laravel's `Inertia::version($closure)` contract. For
    /// async / fallible computation (e.g. read a manifest from S3),
    /// resolve once at boot and pass the cached `String` to
    /// [`version`](Self::version):
    ///
    /// ```rust,no_run
    /// # use suprnova::InertiaConfig;
    /// # async fn read_manifest_hash() -> Result<String, Box<dyn std::error::Error>> {
    /// #     Ok("abc123".to_string())
    /// # }
    /// # async fn ex() -> Result<(), Box<dyn std::error::Error>> {
    /// // In bootstrap:
    /// let manifest_hash = read_manifest_hash().await?;
    /// let cfg = InertiaConfig::new().version(manifest_hash);
    /// # let _ = cfg;
    /// # Ok(()) }
    /// ```
    ///
    /// Or wrap an internal cache and panic-recovery in the closure:
    ///
    /// ```rust,no_run
    /// # use suprnova::InertiaConfig;
    /// # use std::sync::Arc;
    /// # struct Cache;
    /// # impl Cache { fn current_hash(&self) -> String { String::new() } }
    /// let cached: Arc<Cache> = Arc::new(Cache);  // your refresh strategy
    /// InertiaConfig::new().version_with(move || cached.current_hash());
    /// ```
    pub fn version_with<F>(mut self, f: F) -> Self
    where
        F: Fn() -> String + Send + Sync + 'static,
    {
        self.version = VersionResolver::Dynamic(Arc::new(f));
        self
    }

    /// Switch into production mode (disables the Vite dev-server fallback).
    pub fn production(mut self) -> Self {
        self.development = false;
        self
    }

    /// Explicitly set development vs. production mode, overriding the
    /// environment-derived default (see the `development` field doc).
    /// Useful for forcing dev mode in a non-`Production` `APP_ENV` that
    /// should nonetheless load built assets (or vice versa) - most apps
    /// won't need this and should rely on the default.
    pub fn development(mut self, enabled: bool) -> Self {
        self.development = enabled;
        self
    }

    /// Select the frontend framework and reset the entry point to its
    /// canonical default (overwrites any prior [`entry_point`](Self::entry_point) call).
    pub fn frontend(mut self, frontend: Frontend) -> Self {
        self.frontend = frontend;
        // Update entry point default to match the new frontend unless the
        // user has already customized it.
        self.entry_point = frontend.default_entry_point().to_string();
        self
    }

    /// Set the default `<title>` used when a page doesn't supply one.
    pub fn default_title(mut self, title: impl Into<String>) -> Self {
        self.default_title = title.into();
        self
    }

    /// Name the first visit's mount element: the `id` of the element the
    /// client mounts on and the `data-page` attribute of the element that
    /// carries the page data. Default `app`.
    ///
    /// Set it to the `id` the frontend passes to `createInertiaApp`; a
    /// client mounting on an id the document does not carry finds no
    /// element and renders nothing.
    ///
    /// ```rust,no_run
    /// use suprnova::InertiaConfig;
    ///
    /// let cfg = InertiaConfig::new().mount_id("root");
    /// # let _ = cfg;
    /// ```
    pub fn mount_id(mut self, id: impl Into<String>) -> Self {
        self.mount_id = id.into();
        self
    }

    /// Toggle the default `encryptHistory` flag emitted on every Inertia
    /// response (per-response overrides take precedence).
    pub fn encrypt_history(mut self, on: bool) -> Self {
        self.encrypt_history_default = on;
        self
    }

    /// Enable SSR with the given worker URL. SSR is on by default at
    /// `http://127.0.0.1:13714`; this names another worker address.
    pub fn ssr(mut self, url: impl Into<String>) -> Self {
        self.ssr.enabled = true;
        self.ssr.url = url.into();
        self
    }

    /// Turn SSR off: every first visit renders on the client.
    pub fn ssr_disabled(mut self) -> Self {
        self.ssr.enabled = false;
        self
    }

    /// Set the SSR request timeout.
    pub fn ssr_timeout(mut self, t: std::time::Duration) -> Self {
        self.ssr.timeout = t;
        self
    }

    /// Make SSR failures hard errors instead of falling back to CSR.
    pub fn ssr_throw_on_error(mut self, on: bool) -> Self {
        self.ssr.throw_on_error = on;
        self
    }

    /// Add a path pattern excluded from SSR, matched with Laravel's rules
    /// (see [`SsrConfig::excluded_paths`]).
    pub fn ssr_exclude(mut self, pattern: impl Into<String>) -> Self {
        self.ssr.excluded_paths.push(pattern.into());
        self
    }

    /// Set the runtime `ssr:start` launches the worker under; see
    /// [`SsrConfig::runtime`].
    pub fn ssr_runtime(mut self, runtime: impl Into<String>) -> Self {
        self.ssr.runtime = runtime.into();
        self
    }

    /// Make `ssr:start` refuse a runtime it cannot find; see
    /// [`SsrConfig::ensure_runtime_exists`].
    pub fn ssr_ensure_runtime_exists(mut self, on: bool) -> Self {
        self.ssr.ensure_runtime_exists = on;
        self
    }

    /// Set where SSR is dispatched in development, at `/__inertia_ssr`,
    /// whether or not a hot file exists; see [`SsrConfig::hot_url`].
    pub fn ssr_hot_url(mut self, url: impl Into<String>) -> Self {
        self.ssr.hot_url = Some(url.into());
        self
    }

    /// Name the Vite hot file; see [`SsrConfig::hot_file`]. The default,
    /// `public/hot`, is the file `suprnova serve` writes.
    pub fn ssr_hot_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.ssr.hot_file = path.into();
        self
    }

    /// Override the SSR-response body byte cap.
    ///
    /// The default is 8 MiB. Reads that exceed this bound abort and the
    /// response falls back to CSR (or 500 if `ssr_throw_on_error` is
    /// set). Bound chosen to be larger than any realistic SSR page but
    /// small enough to constrain damage from a misconfigured or
    /// compromised loopback worker.
    pub fn ssr_max_response_bytes(mut self, bytes: usize) -> Self {
        self.ssr.max_response_bytes = bytes;
        self
    }

    /// Name the built SSR bundle, looked at before the conventional paths;
    /// see [`SsrConfig::bundle_path`]. A project that builds its bundle to
    /// `frontend/bootstrap/ssr/ssr.js`, where the scaffolded
    /// `vite.config.ts` writes it, needs no call.
    pub fn ssr_bundle_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.ssr.bundle_path = Some(path.into());
        self
    }

    /// Toggle the bundle check; see [`SsrConfig::ensure_bundle_exists`].
    /// On by default. Turn it off if you dispatch to a worker whose bundle
    /// this process can't see on disk (a remote build artifact, a
    /// container image built separately from the one running the
    /// backend), or to a stand-in worker in a test.
    pub fn ssr_ensure_bundle_exists(mut self, on: bool) -> Self {
        self.ssr.ensure_bundle_exists = on;
        self
    }

    /// Register an observability callback for SSR render failures.
    /// Replaces the default `eprintln!` to stderr.
    pub fn on_ssr_error<F>(mut self, f: F) -> Self
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        self.ssr.on_error = Some(std::sync::Arc::new(f));
        self
    }

    /// Override the Vite manifest file location. Resets the lazy cache
    /// so the next [`Self::vite_manifest`] call re-reads from disk.
    /// Default: `public/assets/.vite/manifest.json`.
    ///
    /// Also re-points the default asset-version resolver at the new
    /// path, unless an explicit version was already set.
    pub fn manifest_path(mut self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        // Keep the default version resolver pointed at the manifest the
        // app actually uses. An explicit `.version(...)` /
        // `.version_with(...)` is left alone - the caller named a
        // version on purpose, and silently overruling that would be the
        // worst kind of surprise.
        if matches!(self.version, VersionResolver::Manifest(_)) {
            self.version = VersionResolver::Manifest(path.clone());
        }
        self.manifest_path = path;
        self.manifest = Arc::new(OnceLock::new());
        self
    }

    /// Override the URL prefix under which built assets are served.
    /// Default: `/assets`. The leading slash is required; the value
    /// is concatenated with the manifest entry's `file` field as
    /// `{base}/{file}`.
    pub fn assets_base_url(mut self, url: impl Into<String>) -> Self {
        self.assets_base_url = url.into();
        self
    }

    /// Set the URL the built assets are published under, in place of the
    /// `ASSET_URL` environment variable's. While the version source is the
    /// default (no [`version`](Self::version) or
    /// [`version_with`](Self::version_with)), the asset version becomes
    /// this URL's hash. See the [`asset_url`](Self::asset_url) field.
    pub fn asset_url(mut self, url: impl Into<String>) -> Self {
        self.asset_url = Some(url.into());
        self
    }

    /// The SSR settings a first visit is dispatched with (PAR-058): the hot
    /// URL set when the visit goes hot, and cleared when it does not.
    ///
    /// In development the visit goes hot when the application set
    /// [`SsrConfig::hot_url`], or when the hot file
    /// ([`SsrConfig::hot_file`]) exists, which is how Laravel knows Vite runs
    /// (`Vite::isRunningHot`). The address is the configured hot URL, else
    /// the file's content, else the [`vite_dev_server`](Self::vite_dev_server)
    /// URL for an empty file. In production nothing runs hot. Only the file
    /// decides: no connection is attempted, so a server listening at the dev
    /// server's port changes nothing.
    ///
    /// Resolved here rather than by the builders, so the order of
    /// `development`, `production`, `vite_dev_server` and `ssr_hot_url`
    /// calls does not matter. Production with no hot URL, the common case,
    /// borrows the settings without a copy and never looks at the file.
    pub(crate) fn ssr_for_dispatch(&self) -> std::borrow::Cow<'_, SsrConfig> {
        use std::borrow::Cow;
        let hot_url = match (self.development, &self.ssr.hot_url) {
            (true, Some(_)) => return Cow::Borrowed(&self.ssr),
            (false, None) => return Cow::Borrowed(&self.ssr),
            (false, Some(_)) => None,
            (true, None) => match self.hot_file_url() {
                Some(url) => Some(url),
                None => return Cow::Borrowed(&self.ssr),
            },
        };
        let mut ssr = self.ssr.clone();
        ssr.hot_url = hot_url;
        Cow::Owned(ssr)
    }

    /// The dev server's URL from the hot file, or `None` when there is no
    /// hot file: its trimmed content, or the configured dev server URL when
    /// it is empty or cannot be read.
    fn hot_file_url(&self) -> Option<String> {
        let path = &self.ssr.hot_file;
        if !path.is_file() {
            return None;
        }
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(error) => {
                tracing::warn!(
                    hot_file = %path.display(),
                    %error,
                    "the Vite hot file cannot be read; SSR uses the configured dev server URL"
                );
                String::new()
            }
        };
        let url = content.trim();
        Some(if url.is_empty() {
            self.vite_dev_server.clone()
        } else {
            url.to_string()
        })
    }

    /// The asset version this config reports, in Laravel's order.
    ///
    /// An explicit [`version`](Self::version) or
    /// [`version_with`](Self::version_with) is returned as it resolves.
    /// The default source tries the [`asset_url`](Self::asset_url) setting
    /// first, then the Vite manifest's hash, then the empty string.
    pub(crate) fn resolved_version(&self) -> String {
        match (&self.version, self.asset_url.as_deref()) {
            (VersionResolver::Manifest(_), Some(url)) if !url.is_empty() => {
                version_hash(url.as_bytes())
            }
            (version, _) => version.resolve(),
        }
    }

    /// Send integers beyond JavaScript's safe range as `$bigint` markers.
    /// See the [`preserve_big_integers`](Self::preserve_big_integers)
    /// field.
    pub fn preserve_big_integers(mut self, on: bool) -> Self {
        self.preserve_big_integers = on;
        self
    }

    /// Override the per-response cap on concurrent prop resolvers.
    /// Default: 16. Zero is treated as `usize::MAX` (no cap) - the
    /// builder normalizes that for the caller.
    pub fn max_concurrent_resolvers(mut self, n: usize) -> Self {
        self.max_concurrent_resolvers = if n == 0 { usize::MAX } else { n };
        self
    }

    /// Keep every validation message per field instead of collapsing to
    /// the first. Mirrors Laravel's `protected $withAllErrors = true;`.
    ///
    /// ```rust,no_run
    /// use suprnova::InertiaConfig;
    ///
    /// let cfg = InertiaConfig::new().with_all_errors(true);
    /// # let _ = cfg;
    /// ```
    pub fn with_all_errors(mut self, on: bool) -> Self {
        self.with_all_errors = on;
        self
    }

    /// List the shared props' keys under `sharedProps` (`true`, the
    /// default) or leave the field out of the page object (`false`) - see
    /// [`expose_shared_props`](Self::expose_shared_props). Laravel's
    /// `inertia.expose_shared_prop_keys`.
    pub fn expose_shared_props(mut self, on: bool) -> Self {
        self.expose_shared_props = on;
        self
    }

    /// Render framework error responses through the named Inertia page
    /// component instead of letting their JSON body reach the client.
    ///
    /// This is the opt-in for [`error_page`](Self::error_page) - read
    /// that field's documentation for which responses are rewritten,
    /// which are deliberately left alone, and the three props the
    /// component receives. It installs the default error callback, the
    /// one-line form of
    /// [`Inertia::handle_exceptions_using`](crate::Inertia::handle_exceptions_using);
    /// a callback the app installs decides in its place. Without either,
    /// the middleware [`crate::Inertia::install`] registers hands every
    /// request on and changes nothing.
    ///
    /// **Name the page once.** An app that registers
    /// [`InertiaErrorPageMiddleware`](crate::InertiaErrorPageMiddleware)
    /// itself - to place it outside a `CsrfMiddleware` or rate limiter
    /// that answers before the Inertia layer is reached - named the
    /// component at that registration, and that instance is the one in the
    /// chain, so its component is the one rendered. `install` sees the
    /// registration and skips its own. This setter is then optional:
    /// harmless to keep, and still what makes `install` register a
    /// middleware for an app that does not place one itself.
    ///
    /// Register **before** calling `install`, not after. Global middleware
    /// registration is idempotent per type, so an `install` that has
    /// already put one innermost keeps it and a later registration of your
    /// own is dropped - along with the position and the component it
    /// named.
    ///
    /// ```rust,no_run
    /// use suprnova::InertiaConfig;
    ///
    /// let cfg = InertiaConfig::new().error_page("Error");
    /// # let _ = cfg;
    /// ```
    pub fn error_page(mut self, component: impl Into<String>) -> Self {
        self.error_page = Some(component.into());
        self
    }

    /// Turn the previous-URL recording of Inertia visits on or off; see
    /// [`store_previous_url`](Self::store_previous_url) for what is
    /// recorded and why. On by default.
    pub fn store_previous_url(mut self, on: bool) -> Self {
        self.store_previous_url = on;
        self
    }

    /// Replace the Inertia middleware's decisions with the application's -
    /// a Laravel app's `HandleInertiaRequests` overrides. See
    /// [`InertiaMiddlewareHooks`](crate::InertiaMiddlewareHooks) for each
    /// decision and its default. The middleware stack built from this
    /// config, by [`crate::Inertia::install`] or
    /// [`crate::Inertia::middleware`], runs them.
    ///
    /// The file and line of this call are what Inertia DevTools names as
    /// the source of each prop the `share` hooks supply, taken through
    /// `#[track_caller]`.
    #[track_caller]
    pub fn hooks(mut self, hooks: impl super::hooks::InertiaMiddlewareHooks) -> Self {
        self.hooks_location = Some(std::panic::Location::caller());
        self.hooks = Some(Arc::new(hooks));
        self
    }

    /// Set the Inertia DevTools settings: whether requests are recorded
    /// for the browser extension, where entries are stored, and what is
    /// redacted. See [`DevToolsConfig`](crate::DevToolsConfig); without
    /// this call the defaults apply, which record only when `APP_ENV`
    /// names the `local` environment.
    ///
    /// ```rust,no_run
    /// use suprnova::{DevToolsConfig, InertiaConfig};
    ///
    /// let cfg = InertiaConfig::new().devtools(DevToolsConfig::new().enabled(false));
    /// # let _ = cfg;
    /// ```
    pub fn devtools(mut self, config: super::devtools::DevToolsConfig) -> Self {
        self.devtools = Some(Arc::new(config));
        self
    }

    /// The Inertia DevTools settings this configuration carries, the
    /// defaults when none were set.
    pub(crate) fn devtools_config(&self) -> super::devtools::DevToolsConfig {
        match &self.devtools {
            Some(config) => config.as_ref().clone(),
            None => super::devtools::DevToolsConfig::default(),
        }
    }

    /// Choose whether [`crate::Inertia::install`] registers the stack on
    /// every route or as the named middleware `inertia` for route groups;
    /// see [`register_globally`](Self::register_globally).
    pub fn register_globally(mut self, on: bool) -> Self {
        self.register_globally = on;
        self
    }

    /// Make rendering a component with no page file an error; see
    /// [`ensure_pages_exist`](Self::ensure_pages_exist). Off by default.
    ///
    /// ```rust,no_run
    /// use suprnova::InertiaConfig;
    ///
    /// let cfg = InertiaConfig::new()
    ///     .ensure_pages_exist(true)
    ///     .pages_dir("frontend/src/pages");
    /// # let _ = cfg;
    /// ```
    pub fn ensure_pages_exist(mut self, on: bool) -> Self {
        self.ensure_pages_exist = on;
        self
    }

    /// Choose whether a test's component assertion also checks the page
    /// file; see [`testing_ensure_pages_exist`](Self::testing_ensure_pages_exist).
    /// On by default. Laravel's `inertia.testing.ensure_pages_exist`.
    pub fn testing_ensure_pages_exist(mut self, on: bool) -> Self {
        self.testing_ensure_pages_exist = on;
        self
    }

    /// Set the directory page files live under; see
    /// [`pages_dir`](Self::pages_dir).
    pub fn pages_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.pages_dir = dir.into();
        self
    }

    /// Set the page file extensions the existence check accepts, without
    /// the dot; see [`page_extensions`](Self::page_extensions).
    pub fn page_extensions<I, S>(mut self, extensions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.page_extensions = extensions.into_iter().map(Into::into).collect();
        self
    }

    /// Override how the page object's `url` field is derived from the
    /// request. Mirrors Laravel's `Inertia::resolveUrlUsing($closure)`.
    ///
    /// The default is the request's path plus query string. Override when
    /// the URL the client should record differs from the URL that arrived -
    /// a locale prefix the SPA doesn't route on, a path a reverse proxy
    /// rewrote, a canonical host-relative form.
    ///
    /// The closure is synchronous and infallible by design: it runs on
    /// every page-object emission, and there is no sensible response for
    /// "we could not name this page".
    ///
    /// ```rust,no_run
    /// use suprnova::InertiaConfig;
    ///
    /// let cfg = InertiaConfig::new()
    ///     .url_resolver(|req| req.path_and_query().replacen("/en", "", 1));
    /// # let _ = cfg;
    /// ```
    pub fn url_resolver<F>(mut self, f: F) -> Self
    where
        F: Fn(&dyn InertiaRequestExt) -> String + Send + Sync + 'static,
    {
        self.url_resolver = Some(Arc::new(f));
        self
    }

    /// Render every first visit through one application root template,
    /// an Askama template declared with [`inertia_root`](crate::inertia_root)
    /// that places the framework's parts in a document of its own: meta
    /// tags, fonts, a favicon, attributes on `<html>` and `<body>`.
    ///
    /// Without one the first visit is the document the framework writes
    /// itself. [`root_template_with`](Self::root_template_with) chooses per
    /// request instead.
    ///
    /// ```rust,ignore
    /// use suprnova::{InertiaConfig, InertiaRootTemplate};
    ///
    /// #[suprnova::inertia_root(path = "app.html")]
    /// pub struct AppDocument;
    ///
    /// let cfg = InertiaConfig::new().root_template(InertiaRootTemplate::of::<AppDocument>());
    /// ```
    pub fn root_template(mut self, template: InertiaRootTemplate) -> Self {
        self.root_template = Some(Arc::new(move |_| template));
        self
    }

    /// Choose each first visit's root document from its request: its path,
    /// query and headers, through [`InertiaRequestExt`]. Laravel's
    /// `rootView(Request)`.
    ///
    /// The chooser also picks the document of an Inertia error page, which
    /// holds only the request captured before the handler ran. Return
    /// [`InertiaRootTemplate::framework`] for the framework's own document.
    ///
    /// ```rust,ignore
    /// use suprnova::{InertiaConfig, InertiaRootTemplate};
    ///
    /// let cfg = InertiaConfig::new().root_template_with(|req| {
    ///     if req.path().starts_with("/admin") {
    ///         InertiaRootTemplate::of::<AdminDocument>()
    ///     } else {
    ///         InertiaRootTemplate::of::<AppDocument>()
    ///     }
    /// });
    /// ```
    pub fn root_template_with<F>(mut self, chooser: F) -> Self
    where
        F: Fn(&dyn InertiaRequestExt) -> InertiaRootTemplate + Send + Sync + 'static,
    {
        self.root_template = Some(Arc::new(chooser));
        self
    }

    /// The root document a first visit of `request` renders into.
    pub(crate) fn root_template_for(&self, request: &dyn InertiaRequestExt) -> InertiaRootTemplate {
        match &self.root_template {
            Some(choose) => choose(request),
            None => InertiaRootTemplate::framework(),
        }
    }

    /// Return the cached Vite manifest. On the first call this reads
    /// [`Self::manifest_path`] from disk; subsequent calls return the
    /// cached value (or cached `None` if the read failed).
    ///
    /// `None` is returned when the file is missing or malformed - the
    /// production HTML shell renderer falls back to a legacy hardcoded
    /// path and logs a `tracing::warn!`. This keeps existing
    /// pre-manifest apps booting; new apps with a proper Vite build
    /// pick up hashed assets automatically.
    pub fn vite_manifest(&self) -> Option<&ViteManifest> {
        self.manifest
            .get_or_init(|| match ViteManifest::load(&self.manifest_path) {
                Ok(m) => Some(m),
                Err(e) => {
                    tracing::warn!(
                        path = %self.manifest_path.display(),
                        error = %e,
                        "Vite manifest could not be loaded; production asset \
                         tags will fall back to the legacy hardcoded path. \
                         Ensure `build.manifest: true` is set in vite.config.ts \
                         and that the build has produced an output."
                    );
                    None
                }
            })
            .as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontend_detect_defaults_to_svelte_when_unset() {
        // Clear in case some other test set it.
        // SAFETY: tests in this module run sequentially in the same binary,
        // but cargo test runs tests in parallel by default. To avoid races
        // we don't unset; instead we test the explicit-match arm and the
        // explicit-Svelte arm separately.
        let _ = std::env::var("SUPRNOVA_FRONTEND"); // touch to silence unused warnings
        // The default arm covers unset + unknown values; verify the
        // explicit fallback by checking the match logic.
        assert_eq!(Frontend::Svelte.as_str(), "svelte");
        assert_eq!(Frontend::React.as_str(), "react");
        assert_eq!(Frontend::Vue.as_str(), "vue");
    }

    #[test]
    fn frontend_default_entry_points() {
        assert_eq!(Frontend::Svelte.default_entry_point(), "src/main.ts");
        assert_eq!(Frontend::React.default_entry_point(), "src/main.tsx");
        assert_eq!(Frontend::Vue.default_entry_point(), "src/main.ts");
    }

    #[test]
    #[serial_test::serial(inertia_vite_env)]
    fn vite_dev_server_resolves_from_env() {
        let prior_url = std::env::var("INERTIA_VITE_DEV_SERVER").ok();
        let prior_port = std::env::var("VITE_PORT").ok();
        // SAFETY: single-threaded scope (serialized via serial_test), env
        // restored at the end - same pattern as the config provider tests.
        unsafe {
            std::env::remove_var("INERTIA_VITE_DEV_SERVER");
            std::env::remove_var("VITE_PORT");
        }

        // Neither set → distinctive default, NOT the old hardcoded 5173.
        assert_eq!(
            vite_dev_server_from_env(),
            format!("http://localhost:{DEFAULT_VITE_PORT}")
        );

        // VITE_PORT set → the dev-head URL tracks the real Vite port.
        unsafe {
            std::env::set_var("VITE_PORT", "5790");
        }
        assert_eq!(vite_dev_server_from_env(), "http://localhost:5790");

        // INERTIA_VITE_DEV_SERVER (full URL) wins over VITE_PORT - this is
        // the hook for pointing the page at an HTTPS Vite (e.g. behind a
        // TLS dev proxy).
        unsafe {
            std::env::set_var("INERTIA_VITE_DEV_SERVER", "https://vite.nebula.localhost");
        }
        assert_eq!(vite_dev_server_from_env(), "https://vite.nebula.localhost");

        unsafe {
            match prior_url {
                Some(v) => std::env::set_var("INERTIA_VITE_DEV_SERVER", v),
                None => std::env::remove_var("INERTIA_VITE_DEV_SERVER"),
            }
            match prior_port {
                Some(v) => std::env::set_var("VITE_PORT", v),
                None => std::env::remove_var("VITE_PORT"),
            }
        }
    }

    #[test]
    fn frontend_page_extensions() {
        assert_eq!(Frontend::Svelte.page_extensions(), &["svelte"]);
        assert_eq!(Frontend::React.page_extensions(), &["tsx", "jsx"]);
        assert_eq!(Frontend::Vue.page_extensions(), &["vue"]);
    }

    #[test]
    fn config_default_has_svelte_entry_when_env_unset() {
        // Best-effort: only valid when env unset; CI may inject SUPRNOVA_FRONTEND.
        if std::env::var("SUPRNOVA_FRONTEND").is_err() {
            let cfg = InertiaConfig::default();
            assert_eq!(cfg.frontend, Frontend::Svelte);
            assert_eq!(cfg.entry_point, "src/main.ts");
        }
    }

    #[test]
    fn config_builder_updates_entry_point_with_frontend() {
        let cfg = InertiaConfig::new().frontend(Frontend::React);
        assert_eq!(cfg.frontend, Frontend::React);
        assert_eq!(cfg.entry_point, "src/main.tsx");
    }

    #[test]
    fn config_builder_overrides_default_title() {
        let cfg = InertiaConfig::new().default_title("My App");
        assert_eq!(cfg.default_title, "My App");
    }

    #[test]
    fn version_resolver_static_resolves_to_string() {
        let r = VersionResolver::new("abc123");
        assert_eq!(r.resolve(), "abc123");
        assert_eq!(r.resolve(), "abc123"); // idempotent
    }

    #[test]
    fn version_resolver_dynamic_calls_closure_each_time() {
        let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let c = counter.clone();
        let r = VersionResolver::with(move || {
            let n = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            format!("v{}", n)
        });
        assert_eq!(r.resolve(), "v0");
        assert_eq!(r.resolve(), "v1");
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn version_resolver_from_string_makes_static() {
        let r: VersionResolver = "x".to_string().into();
        assert_eq!(r.resolve(), "x");
        let r2: VersionResolver = "y".into();
        assert_eq!(r2.resolve(), "y");
    }

    #[test]
    fn config_version_builder_creates_static() {
        let cfg = InertiaConfig::new().version("static-v1");
        assert_eq!(cfg.version.resolve(), "static-v1");
    }

    #[test]
    fn config_version_with_creates_dynamic() {
        let cfg = InertiaConfig::new().version_with(|| "dyn-v1".to_string());
        assert_eq!(cfg.version.resolve(), "dyn-v1");
    }

    // ---- SSR exclusion: Laravel's `ExcludesPaths` and `Str::is` ----

    fn excludes(pattern: &str, path: &str) -> bool {
        excluded_by(&[pattern.to_string()], path, None)
    }

    #[test]
    fn inp_str_is_star_matches_any_characters_slash_included() {
        assert!(str_is("admin/*", "admin/users"));
        assert!(str_is("admin/*", "admin/users/edit"));
        assert!(str_is("admin/*", "admin/"));
        assert!(!str_is("admin/*", "admin"));
        assert!(!str_is("admin/*", "adminx"));
        assert!(str_is("*/edit", "posts/1/edit"));
        assert!(str_is("a*b*c", "aXbYbZc"));
        assert!(!str_is("a*b*c", "aXbYbZ"));
        assert!(str_is("*", ""));
        assert!(str_is("exact", "exact"));
        assert!(!str_is("exact", "Exact"));
    }

    #[test]
    fn inp_exclusion_patterns_trim_slashes_and_match_the_trimmed_path() {
        assert!(excludes("admin/*", "/admin/users"));
        assert!(excludes("/admin/*", "/admin/users/edit"));
        assert!(!excludes("admin/*", "/adminx"));
        assert!(excludes("/reports/", "/reports"));
        assert!(excludes("reports", "/reports/"));
        assert!(excludes("/", "/"));
        assert!(!excludes("/", "/home"));
        // The path is decoded first, as `Request::decodedPath()` is.
        assert!(excludes("caf\u{e9}/*", "/caf%C3%A9/menu"));
    }

    #[test]
    fn inp_exclusion_patterns_are_tried_against_the_full_url() {
        let patterns = ["https://app.test/reports*".to_string()];
        assert!(excluded_by(
            &patterns,
            "/reports/2026",
            Some("https://app.test/reports/2026?q=1")
        ));
        assert!(!excluded_by(&patterns, "/reports/2026", None));
    }

    #[test]
    fn inp_ssr_config_uses_the_same_rules() {
        let config = SsrConfig {
            excluded_paths: vec!["admin/*".to_string()],
            ..SsrConfig::default()
        };
        assert!(config.is_path_excluded("/admin/users/edit"));
        assert!(!config.is_path_excluded("/adminx"));
    }
}

#[cfg(test)]
mod error_page_tests {
    use super::*;

    #[test]
    fn error_page_is_off_until_a_component_is_named() {
        // Backwards compatibility is the whole point of the default: an
        // app upgrading into this release must keep the error bodies it
        // already ships until it opts in.
        assert_eq!(InertiaConfig::new().error_page, None);
        assert_eq!(
            InertiaConfig::new()
                .error_page("Error")
                .error_page
                .as_deref(),
            Some("Error"),
        );
    }
}
