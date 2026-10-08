//! SSR client + per-request opt-out.
//!
//! Inertia v3 SSR runs as a separate process (Node/Bun/Deno) using
//! `@inertiajs/{vue3,react,svelte}/server` `createServer()`. The worker
//! listens on HTTP and accepts the page object as JSON; we POST it and
//! receive `{ head: string[], body: string }` back.
//!
//! Suprnova talks to that worker over loopback HTTP. We don't manage
//! the worker process from the framework - `suprnova-cli` ships
//! `ssr:start` for that, and operators are free to use their own
//! supervisor.

use serde::Deserialize;
use std::time::Duration;

use crate::error::FrameworkError;
use crate::events::{SsrErrorType, SsrRenderFailed};
use crate::inertia::config::SsrConfig;
use crate::inertia::prop::InertiaRequestExt;

// Note: we don't define a typed request struct - the `@inertiajs/*/server`
// `createServer()` workers accept the raw page object JSON envelope.
// We send `serde_json::Value` directly to avoid an extra serialize step.

/// Response from the SSR worker. Heads is a list of `<head>` snippets
/// (e.g. `<title>...</title>`, `<meta ...>`); body is the prerendered
/// app shell.
#[derive(Deserialize, Debug, Clone, Default)]
pub struct SsrResponse {
    /// `<head>` fragments to inject into the rendered HTML (titles, meta tags, link tags).
    #[serde(default)]
    pub head: Vec<String>,
    /// Prerendered application shell HTML to inject into the response body.
    #[serde(default)]
    pub body: String,
}

/// The request the framework sends to the SSR worker, as
/// [`Inertia::configure_ssr_request_using`](crate::Inertia::configure_ssr_request_using)
/// sees it - Laravel's `PendingRequest` for the SSR server.
///
/// A worker behind a proxy or on another host may want a token, a tenant
/// header or a longer timeout for one kind of page; the configurator adds
/// them here. The page object is the body and is not changed.
#[derive(Debug, Clone)]
pub struct SsrRequest {
    url: String,
    headers: Vec<(String, String)>,
    timeout: Duration,
}

impl SsrRequest {
    /// The worker's render URL the request is posted to.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The headers added so far, beyond the content type, length and host
    /// the framework sets itself.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// The time the whole call may take before the response falls back to
    /// CSR (or fails, under `ssr_throw_on_error`).
    pub fn timeout_duration(&self) -> Duration {
        self.timeout
    }

    /// Add a header to the request. A name or value that is not a valid
    /// HTTP header makes the call fail, which the SSR error handling then
    /// treats like any other worker failure.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Add `Authorization: Bearer <token>`.
    pub fn bearer_token(self, token: impl AsRef<str>) -> Self {
        let value = format!("Bearer {}", token.as_ref());
        self.header("Authorization", value)
    }

    /// Replace the timeout of the whole call.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// The paths an SSR bundle is looked for at when `SsrConfig::bundle_path`
/// is unset, under the working directory: Laravel's `BundleDetector` list,
/// with `frontend/bootstrap/ssr/` where a Suprnova project's Vite build
/// writes the bundle (PAR-057).
pub const CONVENTIONAL_BUNDLE_PATHS: [&str; 6] = [
    "frontend/bootstrap/ssr/ssr.js",
    "frontend/bootstrap/ssr/app.js",
    "frontend/bootstrap/ssr/ssr.mjs",
    "frontend/bootstrap/ssr/app.mjs",
    "public/js/ssr.js",
    "public/js/app.js",
];

/// The SSR bundle on disk: the configured `bundle_path` when it exists,
/// else the first of [`CONVENTIONAL_BUNDLE_PATHS`] that does, else `None`.
/// Laravel's `BundleDetector::detect`, read by the dispatch's bundle check
/// and by `ssr:start`.
pub fn detect_bundle(config: &SsrConfig) -> Option<std::path::PathBuf> {
    config
        .bundle_path
        .iter()
        .cloned()
        .chain(
            CONVENTIONAL_BUNDLE_PATHS
                .iter()
                .map(std::path::PathBuf::from),
        )
        .find(|path| path.is_file())
}

// Per-request opt-out for SSR. Mirrors Laravel's
// `Inertia::disable_ssr()`. The flag is an `Arc<AtomicBool>` so the
// scope is set once (by the server when wrapping each request) and
// the handler can flip it during execution without needing to
// re-enter a new scope.
tokio::task_local! {
    pub(crate) static DISABLE_SSR: std::sync::Arc<std::sync::atomic::AtomicBool>;
}

/// Disable SSR for the rest of this request. Idempotent. No-op when
/// called outside a request scope (e.g. unit tests that don't wire up
/// the server's task-local scope).
pub fn disable_ssr_for_request() {
    let _ = DISABLE_SSR.try_with(|flag| {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    });
}

/// Check whether SSR has been disabled for the current task. Returns
/// `false` outside any scope (the default - caller's config wins).
pub fn is_disabled_for_request() -> bool {
    DISABLE_SSR
        .try_with(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
        .unwrap_or(false)
}

/// Initial scope value used by the server. Public so `crate::server`
/// can wrap each request without having to touch the internals.
#[doc(hidden)]
pub fn new_disable_ssr_flag() -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))
}

/// Whether SSR runs for `request`: Laravel's `HttpGateway::ssrIsEnabled`.
///
/// The condition `Inertia::disable_ssr` or `disable_ssr_if` set decides
/// when there is one, so it can turn SSR on as well as off; otherwise the
/// configuration does. The per-request opt-out
/// ([`disable_ssr_for_request`]) then turns it off, and so does an
/// exclusion pattern from the configuration or `Inertia::without_ssr`
/// matching the path or the full URL.
fn ssr_runs_for(config: &SsrConfig, request: &dyn InertiaRequestExt) -> bool {
    let registry = crate::App::inertia_registry();
    let runtime = registry.runtime();
    if !runtime.ssr_enabled_for(config.enabled, request) || is_disabled_for_request() {
        return false;
    }
    let added = runtime.ssr_exclusions();
    if config.excluded_paths.is_empty() && added.is_empty() {
        return true;
    }
    let full_url = request.full_url();
    let excluded = |patterns: &[String]| {
        crate::inertia::config::excluded_by(patterns, request.path(), Some(&full_url))
    };
    !(excluded(&config.excluded_paths) || excluded(&added))
}

/// Where a first visit is posted, and whether that is the hot endpoint.
struct Target {
    url: String,
    hot: bool,
}

/// Where a first visit is posted, Laravel's `HttpGateway::dispatch`
/// order: the hot URL's `/__inertia_ssr` in hot mode, with no bundle check
/// (PAR-058); else the worker's `/render` when a bundle is found or the
/// check is off (PAR-057); else `None`, which renders on the client.
///
/// The configuration decides hot mode before this runs
/// (`InertiaConfig::ssr_for_dispatch`, from the hot file): a hot URL here
/// means hot. The page is serialized only after this, so a visit that
/// renders on the client never pays for it.
fn dispatch_target(config: &SsrConfig) -> Option<Target> {
    if let Some(hot) = config.hot_url.as_deref() {
        return Some(Target {
            url: endpoint(hot, "/__inertia_ssr"),
            hot: true,
        });
    }
    if config.ensure_bundle_exists && detect_bundle(config).is_none() {
        return None;
    }
    Some(Target {
        url: endpoint(&config.url, "/render"),
        hot: false,
    })
}

/// `base` with `path` appended, the trailing slashes of `base` dropped, as
/// Laravel's `getProductionUrl` and `getHotUrl` join them.
fn endpoint(base: &str, path: &str) -> String {
    format!("{}{path}", base.trim().trim_end_matches('/'))
}

/// Render via the SSR worker. Returns `Ok(Some(_))` when SSR succeeded,
/// `Ok(None)` when SSR was disabled, the request was excluded, or no
/// bundle was found while the check is on (caller falls back to CSR),
/// and `Err` only when `throw_on_error` is true.
///
/// A missing bundle is not reported: with SSR on by default, an
/// application that has no bundle would otherwise log it on every first
/// visit. Laravel's `HttpGateway::dispatch` returns `null` the same way.
/// A worker that fails is reported through `report_failure`.
pub(crate) async fn render(
    config: &SsrConfig,
    request: &dyn InertiaRequestExt,
    page: &serde_json::Value,
) -> Result<Option<SsrResponse>, FrameworkError> {
    if !ssr_runs_for(config, request) {
        return Ok(None);
    }
    let Some(target) = dispatch_target(config) else {
        return Ok(None);
    };

    let body = serde_json::to_vec(page)
        .map_err(|e| FrameworkError::internal(format!("SSR page serialization failed: {e}")))?;
    let request = crate::App::inertia_registry()
        .runtime()
        .configure_ssr_request(SsrRequest {
            url: target.url,
            headers: Vec::new(),
            timeout: config.timeout,
        });
    let failure = match exchange(&request, Some(body), config.max_response_bytes).await {
        // A dev server without the Inertia Vite plugin serves no SSR: its
        // 404 means "render on the client", not a failure to report on
        // every first visit while developing.
        Ok(answer) if target.hot && answer.status == reqwest::StatusCode::NOT_FOUND => {
            tracing::debug!(
                url = %request.url,
                "the Vite dev server serves no SSR; the visit renders on the client"
            );
            return Ok(None);
        }
        Ok(answer) if answer.status.is_success() => return Ok(rendered(&answer.body)),
        Ok(answer) => error_answer(page, answer.status, &answer.body),
        Err(transport) => failure(page, transport, SsrErrorType::Connection),
    };
    report_failure(config, &request.url, failure).await
}

/// A render failure of `page` with no details beyond the message.
fn failure(page: &serde_json::Value, error: String, error_type: SsrErrorType) -> SsrRenderFailed {
    let text = |key: &str, default: &str| {
        page.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or(default)
            .to_string()
    };
    SsrRenderFailed {
        component: text("component", "Unknown"),
        url: text("url", "/"),
        error,
        error_type,
        hint: None,
        browser_api: None,
        stack: None,
        source_location: None,
    }
}

/// The failure a non-2xx answer reports: the worker's error JSON (`error`,
/// `type`, `hint`, `browserApi`, `stack`, `sourceLocation`), as Laravel's
/// `handleSsrFailure` reads it, or the status alone when the body is not
/// such an object.
fn error_answer(
    page: &serde_json::Value,
    status: reqwest::StatusCode,
    body: &[u8],
) -> SsrRenderFailed {
    let details = match serde_json::from_slice(body) {
        Ok(serde_json::Value::Object(details)) => details,
        _ => serde_json::Map::new(),
    };
    let text = |key: &str| {
        details
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let error = text("error").unwrap_or_else(|| format!("the SSR worker answered {status}"));
    let error_type =
        text("type").map_or(SsrErrorType::Unknown, |name| SsrErrorType::from_name(&name));
    SsrRenderFailed {
        hint: text("hint"),
        browser_api: text("browserApi"),
        stack: text("stack"),
        source_location: text("sourceLocation"),
        ..failure(page, error, error_type)
    }
}

/// Report a failed render, Laravel's `handleSsrFailure`: dispatch
/// [`SsrRenderFailed`], then fail the visit under `throw_on_error` with
/// the component and source location in the message, or else fire the
/// `on_error` hook (stderr without one) and render on the client.
///
/// The event is dispatched inline, before the visit continues, so a
/// listener sees every failure; it is built and sent only when something
/// listens for it or a fake records it.
async fn report_failure(
    config: &SsrConfig,
    url: &str,
    failure: SsrRenderFailed,
) -> Result<Option<SsrResponse>, FrameworkError> {
    let message = failure.message();
    let hook_message = if failure.error_type == SsrErrorType::Connection {
        format!(
            "SSR worker unreachable at {url} for component [{}] ({}); falling back to CSR",
            failure.component, failure.error
        )
    } else {
        format!("{message} (worker at {url}); falling back to CSR")
    };
    if crate::events::EventFacade::is_observed::<SsrRenderFailed>()
        && let Err(error) = crate::events::EventFacade::dispatch(failure).await
    {
        tracing::warn!(error = %error, "an SsrRenderFailed listener failed");
    }
    if config.throw_on_error {
        return Err(FrameworkError::internal(message));
    }
    match &config.on_error {
        Some(hook) => hook(&hook_message),
        None => eprintln!("[inertia] {hook_message}"),
    }
    Ok(None)
}

/// Whether the worker answers `GET {url}/health` with a 2xx within the
/// timeout, Laravel's `HttpGateway::isHealthy`. The request configurator
/// applies, so a worker behind a token is checked with it; any failure to
/// get an answer is unhealthy. The configured worker URL is checked, never
/// the hot URL, as Laravel's `getProductionUrl` is.
pub(crate) async fn is_healthy(config: &SsrConfig) -> bool {
    let request = crate::App::inertia_registry()
        .runtime()
        .configure_ssr_request(SsrRequest {
            url: endpoint(&config.url, "/health"),
            headers: Vec::new(),
            timeout: config.timeout,
        });
    match exchange(&request, None, config.max_response_bytes).await {
        Ok(answer) => answer.status.is_success(),
        Err(error) => {
            tracing::debug!(url = %request.url, %error, "the SSR health check got no answer");
            false
        }
    }
}

/// The client every SSR call shares, built once for the process: one
/// connection pool, and rustls for a worker at an `https` URL (SS-14).
///
/// Like the plain HTTP client it replaced, it follows no redirects and
/// ignores the proxy variables of the environment: the worker's address is
/// the configuration's, and a proxy set for outbound traffic must not
/// capture a loopback worker.
fn shared_client() -> Result<&'static reqwest::Client, String> {
    static CLIENT: std::sync::OnceLock<Result<reqwest::Client, String>> =
        std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| {
                    format!(
                        "build the SSR client: {}",
                        crate::error::render_error_chain(&e)
                    )
                })
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// The status and body the worker answered one call with.
struct Exchange {
    status: reqwest::StatusCode,
    body: Vec<u8>,
}

/// Send `request` to the worker, a `POST` of `body` as JSON or a `GET`
/// without one, and read the whole answer.
///
/// One deadline, computed once, bounds the whole call: awaiting the
/// response headers and reading the body draw down the same timeout, so a
/// worker that sends headers and then stalls mid-body cannot hold the
/// visit past it (T31). The body is capped at `max_response_bytes`
/// (`SsrConfig::max_response_bytes`, 8 MiB by default), so a misconfigured
/// or compromised worker cannot exhaust memory: a `Content-Length` over
/// the cap is refused before any body byte is read, and a body that grows
/// past it while streaming is abandoned (D20-D).
async fn exchange(
    request: &SsrRequest,
    body: Option<Vec<u8>>,
    max_response_bytes: usize,
) -> Result<Exchange, String> {
    let deadline = tokio::time::Instant::now() + request.timeout;
    let client = shared_client()?;
    let mut builder = match body {
        Some(body) => client
            .post(&request.url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body),
        None => client.get(&request.url),
    };
    for (name, value) in &request.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }

    let mut response = tokio::time::timeout_at(deadline, builder.send())
        .await
        .map_err(|_| {
            format!(
                "timeout after {:?} awaiting response headers",
                request.timeout
            )
        })?
        .map_err(|e| crate::error::render_error_chain(&e))?;
    let status = response.status();

    if let Some(length) = response.content_length()
        && length > max_response_bytes as u64
    {
        return Err(format!(
            "ssr response Content-Length {length} exceeds cap of \
             {max_response_bytes} bytes (configure via \
             InertiaConfig::ssr_max_response_bytes)"
        ));
    }
    let mut bytes = Vec::new();
    loop {
        let chunk = tokio::time::timeout_at(deadline, response.chunk())
            .await
            .map_err(|_| format!("timeout after {:?} reading response body", request.timeout))?
            .map_err(|e| format!("read body: {}", crate::error::render_error_chain(&e)))?;
        let Some(chunk) = chunk else {
            break;
        };
        if bytes.len() + chunk.len() > max_response_bytes {
            return Err(format!(
                "ssr response exceeds cap of {max_response_bytes} bytes \
                 (configure via InertiaConfig::ssr_max_response_bytes)"
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Exchange {
        status,
        body: bytes,
    })
}

/// The page a worker's successful answer carries, or `None` when there is
/// nothing to inline, so the visit renders on the client (SS-02).
///
/// Laravel returns `null` when `$response->json()` is empty or falsy, and
/// that is also what it reads from bytes that are not JSON at all. An
/// object without a non-empty `body` string is the same case here: the
/// worker's body carries the page data element and the mount element, so
/// inlining an empty one left a document the client could not start from.
/// None of these is reported as a failure. Head entries that are not
/// strings are left out.
fn rendered(bytes: &[u8]) -> Option<SsrResponse> {
    let Ok(serde_json::Value::Object(mut answer)) = serde_json::from_slice(bytes) else {
        return None;
    };
    let body = match answer.remove("body") {
        Some(serde_json::Value::String(body)) if !body.is_empty() => body,
        _ => return None,
    };
    let head = match answer.remove("head") {
        Some(serde_json::Value::Array(entries)) => entries
            .into_iter()
            .filter_map(|entry| match entry {
                serde_json::Value::String(fragment) => Some(fragment),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Some(SsrResponse { head, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request at `path` for the render calls below.
    struct At(&'static str);

    impl InertiaRequestExt for At {
        fn path(&self) -> &str {
            self.0
        }
        fn header(&self, _name: &str) -> Option<&str> {
            None
        }
    }

    #[test]
    fn inssr_ssr_is_enabled_by_default() {
        let cfg = SsrConfig::default();
        assert!(cfg.enabled);
        assert!(cfg.ensure_bundle_exists);
    }

    #[tokio::test]
    async fn render_returns_none_when_disabled() {
        let cfg = SsrConfig {
            enabled: false,
            ensure_bundle_exists: false,
            ..SsrConfig::default()
        };
        let page = serde_json::json!({"component": "Home"});
        let result = render(&cfg, &At("/foo"), &page).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn render_returns_none_when_path_excluded() {
        let cfg = SsrConfig {
            enabled: true,
            excluded_paths: vec!["/admin/**".to_string()],
            ..SsrConfig::default()
        };
        let page = serde_json::json!({"component": "Admin"});
        let result = render(&cfg, &At("/admin/users"), &page).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn render_returns_none_when_bundle_missing_and_ensure_bundle_exists_is_on() {
        let cfg = SsrConfig {
            enabled: true,
            bundle_path: Some(std::path::PathBuf::from(
                "/nonexistent/definitely-not-here/ssr.js",
            )),
            ensure_bundle_exists: true,
            ..SsrConfig::default()
        };
        let page = serde_json::json!({"component": "Home"});
        // No SSR worker is listening either, but the bundle check must
        // short-circuit before any connection attempt - proven by this
        // resolving immediately rather than waiting out `config.timeout`.
        let started = std::time::Instant::now();
        let result = render(&cfg, &At("/"), &page).await.unwrap();
        assert!(result.is_none());
        assert!(
            started.elapsed() < cfg.timeout,
            "a missing bundle must short-circuit, not pay the connect timeout"
        );
    }

    #[tokio::test]
    async fn render_skips_the_bundle_check_when_ensure_bundle_exists_is_off() {
        // Deviation from the brief: the brief's version of this test
        // asserted `elapsed >= 40ms` to prove the connection was
        // actually attempted (vs. short-circuited by the bundle check).
        // On Linux, connecting to an unbound loopback port returns
        // ECONNREFUSED via an immediate RST rather than waiting out the
        // timeout, so that assertion fails deterministically here - it's
        // not a flake, the premise (a refused local connection is slow)
        // doesn't hold. Asserting on the distinguishing on_error message
        // instead proves the same thing without depending on timing:
        // "bundle not found" only fires from the short-circuit branch,
        // "unreachable" only fires from an actual connection attempt.
        use std::sync::{Arc, Mutex};
        let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let captured_for_hook = captured.clone();
        let cfg = SsrConfig {
            enabled: true,
            bundle_path: Some(std::path::PathBuf::from("/nonexistent/ssr.js")),
            ensure_bundle_exists: false,
            timeout: std::time::Duration::from_millis(500),
            on_error: Some(Arc::new(move |msg: &str| {
                *captured_for_hook.lock().expect("lock captured message") = Some(msg.to_string());
            })),
            ..SsrConfig::default()
        };
        let page = serde_json::json!({"component": "Home"});
        let result = render(&cfg, &At("/"), &page).await.unwrap();
        assert!(result.is_none());
        let msg = captured
            .lock()
            .expect("lock captured message")
            .clone()
            .expect("on_error must fire - the check is off, so render must actually dispatch");
        assert!(
            msg.contains("unreachable"),
            "with the check off, render must attempt (and fail) the connection, not \
             short-circuit on the missing bundle path: {msg}"
        );
    }

    #[test]
    fn inssr_detect_bundle_finds_the_configured_path() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("ssr.js");
        std::fs::write(&path, b"").expect("write test bundle");
        let cfg = SsrConfig {
            bundle_path: Some(path.clone()),
            ..SsrConfig::default()
        };
        assert_eq!(detect_bundle(&cfg), Some(path));
    }

    #[test]
    fn inssr_detect_bundle_is_none_without_a_bundle() {
        // `framework/` holds a bundle at none of the conventional paths,
        // and a configured path that does not exist is not a bundle.
        let cfg = SsrConfig {
            bundle_path: Some(std::path::PathBuf::from("/nonexistent/ssr.js")),
            ..SsrConfig::default()
        };
        assert_eq!(detect_bundle(&cfg), None);
        assert_eq!(detect_bundle(&SsrConfig::default()), None);
    }

    /// T31 fix round 1. `post_json`'s header-await was bounded by
    /// `config.timeout`, but the body read (`Limited::collect()`) had
    /// no timeout of its own: `Limited` only bounds body *size*, never
    /// time. A worker that accepted the connection, sent headers
    /// (with a `Content-Length` promising more), then stalled without
    /// sending the rest or closing the connection could hang `render()`
    /// forever.
    ///
    /// `ssr_response_body_cap_falls_back_to_csr_when_exceeded`
    /// (`framework/tests/inertia.rs`) doesn't cover this: it writes its
    /// oversized body in one `write_all` with no stall, so the pre-fix
    /// code raced the unbounded body read against nothing and always
    /// finished fast regardless of the missing timeout.
    ///
    /// The outer `tokio::time::timeout` here is a test-level safety net
    /// in case of a regression, not the behaviour under test - every
    /// `.await` in this suite needs a bound, and without it a
    /// regression here would hang the whole test binary rather than
    /// fail this one test.
    #[tokio::test]
    async fn render_does_not_hang_when_the_worker_stalls_mid_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                // Headers promise a body that never fully arrives:
                // send the status line + a Content-Length, a few body
                // bytes, then hold the connection open without sending
                // the rest or closing it - well past both `cfg.timeout`
                // below and this test's own outer timeout.
                let header = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                               Content-Length: 1000000\r\n\r\n";
                let _ = sock.write_all(header.as_bytes()).await;
                let _ = sock.write_all(b"{\"body\":\"stalled").await;
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        });

        let cfg = SsrConfig {
            enabled: true,
            url: format!("http://{local}"),
            timeout: std::time::Duration::from_millis(200),
            // No bundle on disk here: the check would keep the stalled
            // worker from being asked at all.
            ensure_bundle_exists: false,
            ..SsrConfig::default()
        };
        let page = serde_json::json!({"component": "Home"});

        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            render(&cfg, &At("/"), &page),
        )
        .await
        .expect(
            "render() must resolve within cfg.timeout for the body phase too, \
                 not hang past this test's own outer safety-net timeout",
        )
        .unwrap();

        assert!(result.is_none(), "a stalled body must fall back to CSR");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "render() must respect cfg.timeout (~200ms) for the body read, \
             not the 5s outer safety net: took {:?}",
            started.elapsed()
        );

        server.abort();
    }
}
