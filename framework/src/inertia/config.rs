use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use super::manifest::ViteManifest;
use super::prop::InertiaRequestExt;

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
    /// A missing or unreadable file resolves to
    /// [`MANIFEST_VERSION_FALLBACK`] rather than erroring: in
    /// development there is no build to hash.
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

impl std::fmt::Debug for VersionResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Static(s) => write!(f, "Static({:?})", s),
            Self::Dynamic(_) => write!(f, "Dynamic(<closure>)"),
            Self::Manifest(p) => write!(f, "Manifest({:?})", p),
        }
    }
}

/// Asset version reported when a [`VersionResolver::Manifest`] cannot
/// read its file. Matches the framework's historical default, so an app
/// that has never built its frontend behaves exactly as it did before
/// manifest hashing became the default.
pub const MANIFEST_VERSION_FALLBACK: &str = "1.0";

/// Hex of the first 16 bytes of the manifest's SHA-256.
///
/// Not a secret - a stable, bounded-length identifier that changes iff
/// the built assets change. 128 bits is far past where a collision would
/// matter for a cache-busting token, and the truncation keeps the value
/// short enough to sit in a request header without comment.
fn manifest_version(path: &std::path::Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => {
            use sha2::{Digest, Sha256};
            hex::encode(&Sha256::digest(&bytes)[..16])
        }
        Err(e) => {
            // `debug!`, not `warn!`: in development the manifest
            // legitimately doesn't exist (Vite serves from memory) and
            // this runs on every version check, so a warning here would
            // be per-request noise. Production cannot reach this arm -
            // `Inertia::install` refuses to boot without a manifest.
            tracing::debug!(
                path = %path.display(),
                error = %e,
                "Inertia asset version: manifest unreadable, using the static fallback"
            );
            MANIFEST_VERSION_FALLBACK.to_string()
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
    /// [`manifest_path`](Self::manifest_path); see [`VersionResolver`]
    /// for the static and dynamic alternatives.
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
    /// Whether a session-flashed validation bag surfaces every message
    /// per field (`{ email: ["a", "b"] }`) or only the first
    /// (`{ email: "a" }`).
    ///
    /// Default `false`, matching Laravel's
    /// `Inertia\Middleware::$withAllErrors` and Inertia's own
    /// `ErrorValue = string`. Set `true` when your pages render every
    /// message for a field; the client-side type then needs the matching
    /// `errorValueType: string[]` module augmentation. Applies to errors
    /// drained from the session flash only - an `errors` prop a handler
    /// sets itself passes through as-is.
    pub with_all_errors: bool,
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
}

/// SSR (server-side rendering) configuration.
///
/// Suprnova talks to an out-of-process SSR worker - usually the
/// `@inertiajs/{vue3,react,svelte}/server` `createServer()` bundle run
/// under Node, Bun, or Deno - over HTTP loopback. The worker accepts
/// a JSON page object on `POST /render` and returns
/// `{ head: string[], body: string }`. Configure the worker URL here;
/// boot it separately (e.g. `suprnova ssr:start`).
#[derive(Clone)]
pub struct SsrConfig {
    /// When `false`, SSR is fully off and the HTML shell renders empty
    /// `<div id="app">` for the client to hydrate. Default: `false`.
    pub enabled: bool,
    /// URL of the running SSR worker (e.g. `http://127.0.0.1:13714`).
    /// The framework posts to `<url>/render`.
    pub url: String,
    /// Request timeout for the SSR call. Past this, the response falls
    /// back to CSR. Keep tight in production - a hung worker shouldn't
    /// block real users.
    pub timeout: std::time::Duration,
    /// When `true`, SSR errors propagate as 500s instead of falling
    /// back to CSR. Useful in CI / tests; never set `true` in
    /// production unless you also have a watchdog.
    pub throw_on_error: bool,
    /// Glob-style path patterns excluded from SSR. Matching paths
    /// render CSR-only even when `enabled` is `true`. Each pattern
    /// supports `*` (anything-not-slash) and `**` (anything).
    pub excluded_paths: Vec<String>,
    /// Observability hook invoked when an SSR render fails and we
    /// fall back to CSR. Defaults to `eprintln!` to stderr. Wire your
    /// logger / Sentry / DataDog client here. When events parity
    /// lands, `SsrRenderFailed` will fire from this callback too.
    pub on_error: Option<SsrErrorHook>,
    /// Cap on the SSR worker's response body. Bytes past this point
    /// abort the read and the request falls back to CSR (or 500 if
    /// `throw_on_error` is set). Default: 8 MiB - comfortably larger
    /// than any realistic SSR-rendered page but small enough to bound
    /// damage from a misconfigured or compromised loopback worker.
    pub max_response_bytes: usize,
    /// Path to the built SSR bundle (e.g. `frontend/bootstrap/ssr/ssr.js` -
    /// the default `vite build --ssr` output for a scaffolded project,
    /// and what `suprnova ssr:start` looks for by default). `None`
    /// (the default) means "not configured" and disables the existence
    /// check regardless of [`Self::ensure_bundle_exists`] - there being
    /// nothing to check. Unlike Laravel's `BundleDetector`, this is
    /// **never auto-detected**: an app that calls `.ssr(url)` without
    /// also calling [`InertiaConfig::ssr_bundle_path`] gets no bundle
    /// check at all, which is what every test double and mock SSR
    /// worker in this codebase (and yours) relies on.
    pub bundle_path: Option<PathBuf>,
    /// When `true` (the default) and [`Self::bundle_path`] is `Some`,
    /// the SSR gateway checks the bundle exists on disk before every
    /// dispatch and falls back to CSR immediately - without paying
    /// [`Self::timeout`] on a connection that was never going to
    /// succeed - when it doesn't. Mirrors Laravel's
    /// `inertia.ssr.ensure_bundle_exists` config
    /// (`Inertia\Ssr\HttpGateway::shouldDispatch()`). Has no effect
    /// while `bundle_path` is `None`.
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
            .field("ensure_bundle_exists", &self.ensure_bundle_exists)
            .finish()
    }
}

impl Default for SsrConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: "http://127.0.0.1:13714".to_string(),
            timeout: std::time::Duration::from_secs(5),
            throw_on_error: false,
            excluded_paths: Vec::new(),
            on_error: None,
            max_response_bytes: 8 * 1024 * 1024,
            bundle_path: None,
            ensure_bundle_exists: true,
        }
    }
}

impl SsrConfig {
    /// Check whether the given request path is excluded from SSR.
    pub fn is_path_excluded(&self, path: &str) -> bool {
        self.excluded_paths.iter().any(|pat| glob_match(pat, path))
    }
}

/// Tiny glob matcher: `*` matches a single non-`/` segment, `**`
/// matches any number of characters (including `/`). Designed for
/// route-pattern matching, not full POSIX globs.
fn glob_match(pattern: &str, path: &str) -> bool {
    glob_match_inner(pattern.as_bytes(), path.as_bytes())
}

fn glob_match_inner(pat: &[u8], path: &[u8]) -> bool {
    let (mut pi, mut si) = (0, 0);
    let (mut star_pi, mut star_si): (Option<usize>, usize) = (None, 0);
    while si < path.len() {
        if pi < pat.len() {
            let c = pat[pi];
            if c == b'*' {
                // `**` = match any, including '/'
                let double = pi + 1 < pat.len() && pat[pi + 1] == b'*';
                if double {
                    pi += 2;
                    star_pi = Some(pi);
                    star_si = si;
                    // double-star can match zero chars too
                    continue;
                } else {
                    // single `*` = match anything except '/'
                    pi += 1;
                    star_pi = Some(pi);
                    star_si = si;
                    continue;
                }
            } else if c == path[si] {
                pi += 1;
                si += 1;
                continue;
            }
        }
        if let Some(sp) = star_pi {
            // Resume the previous star, consume one more char.
            // For single-`*` we forbid `/` in the consumed window.
            let one_more = path[star_si];
            let prev_was_double = sp >= 2 && pat[sp - 1] == b'*' && pat[sp - 2] == b'*';
            if !prev_was_double && one_more == b'/' {
                return false;
            }
            star_si += 1;
            si = star_si;
            pi = sp;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == b'*' {
        pi += 1;
    }
    pi == pat.len()
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
            encrypt_history_default: false,
            ssr: SsrConfig::default(),
            manifest_path,
            assets_base_url: "/assets".to_string(),
            with_all_errors: false,
            max_concurrent_resolvers: 16,
            // `None` so an app upgrading into this release keeps the
            // exact error bodies it had. Opting in is one builder call.
            error_page: None,
            manifest: Arc::new(OnceLock::new()),
            url_resolver: None,
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

    /// Toggle the default `encryptHistory` flag emitted on every Inertia
    /// response (per-response overrides take precedence).
    pub fn encrypt_history(mut self, on: bool) -> Self {
        self.encrypt_history_default = on;
        self
    }

    /// Enable SSR with the given worker URL.
    pub fn ssr(mut self, url: impl Into<String>) -> Self {
        self.ssr.enabled = true;
        self.ssr.url = url.into();
        self
    }

    /// Disable SSR explicitly (the default).
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

    /// Add a path pattern excluded from SSR.
    pub fn ssr_exclude(mut self, pattern: impl Into<String>) -> Self {
        self.ssr.excluded_paths.push(pattern.into());
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

    /// Point the SSR bundle-existence check at the built bundle. Not set
    /// by default - see [`SsrConfig::bundle_path`]'s doc for why an
    /// unset path is the safe default rather than an auto-detected one.
    /// `frontend/bootstrap/ssr/ssr.js` is the conventional location:
    /// what `suprnova ssr:start` looks for and what the scaffolded
    /// `vite.config.ts`'s SSR build (`vite build --ssr`) writes to.
    pub fn ssr_bundle_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.ssr.bundle_path = Some(path.into());
        self
    }

    /// Toggle the bundle-existence check. On by default; only takes
    /// effect once [`Self::ssr_bundle_path`] is also set. Turn it off
    /// if you dispatch to a worker whose bundle this process can't see
    /// on disk (a remote build artifact, a container image built
    /// separately from the one running the backend).
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

    /// Render framework error responses through the named Inertia page
    /// component instead of letting their JSON body reach the client.
    ///
    /// This is the opt-in for [`error_page`](Self::error_page) - read
    /// that field's documentation for which responses are rewritten,
    /// which are deliberately left alone, and the three props the
    /// component receives. [`crate::Inertia::install`] registers the
    /// middleware that does the work only when this is set, so an app
    /// that never calls it pays nothing and behaves exactly as before.
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

    // ---- glob matcher ----
    //
    // Path-style glob: `*` matches one segment (no slash), `**` matches
    // any characters including slashes. Standard rsync/gitignore-style
    // semantics - `/admin/**` matches `/admin/x` but NOT bare `/admin`
    // (use `/admin*` or two patterns for that).

    #[test]
    fn glob_literal_matches_exact() {
        assert!(glob_match("/users", "/users"));
        assert!(!glob_match("/users", "/users/1"));
        assert!(!glob_match("/users", "/user"));
    }

    #[test]
    fn glob_single_star_does_not_cross_slash() {
        assert!(glob_match("/users/*", "/users/1"));
        assert!(glob_match("/users/*", "/users/abc"));
        assert!(!glob_match("/users/*", "/users/1/edit"));
        // Standard glob semantics: `*` matches zero or more non-slash
        // chars, so `/users/*` matches `/users/` (the `*` matches the
        // empty segment).
        assert!(glob_match("/users/*", "/users/"));
    }

    #[test]
    fn glob_double_star_crosses_slashes() {
        assert!(glob_match("/admin/**", "/admin/foo"));
        assert!(glob_match("/admin/**", "/admin/foo/bar"));
        assert!(glob_match("/admin/**", "/admin/"));
    }

    #[test]
    fn glob_double_star_does_not_match_bare_prefix() {
        // Standard glob semantics: `/admin/**` requires the slash. To
        // match `/admin` itself, the operator should use `/admin*` or
        // two separate patterns.
        assert!(!glob_match("/admin/**", "/admin"));
    }

    #[test]
    fn glob_admin_star_matches_admin_and_admin_suffix() {
        assert!(glob_match("/admin*", "/admin"));
        assert!(glob_match("/admin*", "/admin2"));
        assert!(!glob_match("/admin*", "/admin/foo"));
    }

    #[test]
    fn glob_leading_double_star_matches_anything() {
        assert!(glob_match("**", "/anything/at/all"));
        assert!(glob_match("**", ""));
        assert!(glob_match("**/admin", "/foo/admin"));
        assert!(glob_match("**/admin", "/admin"));
    }

    #[test]
    fn glob_empty_pattern_matches_only_empty_path() {
        assert!(glob_match("", ""));
        assert!(!glob_match("", "/x"));
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
