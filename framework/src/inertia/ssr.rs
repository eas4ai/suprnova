//! SSR client + per-request opt-out.
//!
//! Inertia v3 SSR runs as a separate process (Node/Bun/Deno) using
//! `@inertiajs/{vue3,react,svelte}/server` `createServer()`. The worker
//! listens on HTTP and accepts the page object as JSON; we POST it and
//! receive `{ head: string[], body: string }` back.
//!
//! Suprnova talks to that worker over loopback HTTP. We don't manage
//! the worker process from the framework — `suprnova-cli` ships
//! `ssr:start` for that, and operators are free to use their own
//! supervisor.

use serde::Deserialize;
use std::time::Duration;

use crate::error::FrameworkError;
use crate::inertia::config::SsrConfig;

// Note: we don't define a typed request struct — the `@inertiajs/*/server`
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
/// `false` outside any scope (the default — caller's config wins).
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

/// Laravel's `HttpGateway::shouldDispatch()`: when
/// [`SsrConfig::ensure_bundle_exists`] is on and a
/// [`SsrConfig::bundle_path`] is configured, dispatch is gated on the
/// built bundle actually being on disk — so a worker that was never
/// started, or a bundle that was never built, fails fast, before paying
/// `config.timeout` on a connection that was never going to succeed.
///
/// Returns `Some(reason)` when dispatch should be skipped, `None` when
/// it's fine to proceed — bundle exists, the check is off, or (the
/// common case for every test in this codebase) no path is configured
/// at all, which is treated the same as "off": there's nothing to
/// check. This check runs unconditionally of `throw_on_error` — a
/// missing bundle is a deployment/build problem the caller should see
/// in logs, not a request-time failure mode to escalate to a 500, and
/// that matches `HttpGateway::shouldDispatch()`, which sits entirely
/// outside the HTTP-error branch `throw_on_error` guards in Laravel too.
fn missing_bundle_reason(config: &SsrConfig) -> Option<String> {
    if !config.ensure_bundle_exists {
        return None;
    }
    let bundle_path = config.bundle_path.as_ref()?;
    if bundle_path.exists() {
        return None;
    }
    Some(format!(
        "SSR bundle not found at {} (ensure_bundle_exists is on); falling back to CSR. \
         Run `vite build --ssr`, or turn the check off with \
         InertiaConfig::ssr_ensure_bundle_exists(false) if the worker's bundle lives \
         somewhere this process can't see.",
        bundle_path.display()
    ))
}

/// Render via the SSR worker. Returns `Ok(Some(_))` when SSR succeeded,
/// `Ok(None)` when SSR was disabled, the path was excluded, or the
/// configured bundle doesn't exist on disk (caller falls back to CSR),
/// and `Err` only when `throw_on_error` is true.
pub(crate) async fn render(
    config: &SsrConfig,
    path: &str,
    page: &serde_json::Value,
) -> Result<Option<SsrResponse>, FrameworkError> {
    if !config.enabled {
        return Ok(None);
    }
    if is_disabled_for_request() {
        return Ok(None);
    }
    if config.is_path_excluded(path) {
        return Ok(None);
    }
    if let Some(msg) = missing_bundle_reason(config) {
        if let Some(cb) = &config.on_error {
            cb(&msg);
        } else {
            eprintln!("[inertia] {}", msg);
        }
        return Ok(None);
    }

    let body = serde_json::to_vec(page)
        .map_err(|e| FrameworkError::internal(format!("SSR page serialization failed: {e}")))?;
    let url = format!("{}/render", config.url.trim_end_matches('/'));

    let result = post_json(&url, body, config.timeout, config.max_response_bytes).await;
    match result {
        Ok(resp) => Ok(Some(resp)),
        Err(e) => {
            if config.throw_on_error {
                Err(FrameworkError::internal(format!("SSR render failed: {e}")))
            } else {
                let msg = format!(
                    "SSR worker unreachable at {} ({}); falling back to CSR",
                    url, e
                );
                if let Some(cb) = &config.on_error {
                    cb(&msg);
                } else {
                    eprintln!("[inertia] {}", msg);
                }
                Ok(None)
            }
        }
    }
}

/// Process-global hyper client shared across all SSR calls.
///
/// Constructing a `Client` is expensive — it sets up a connection pool
/// and an HTTP/1.1 handshake state. A per-request `Client` resets the
/// pool every time, so we keep one for the lifetime of the process.
/// `hyper_util::client::legacy::Client` is `Clone`-cheap (`Arc` inside)
/// and `Send + Sync`, so a `OnceLock` works.
fn shared_client() -> &'static hyper_util::client::legacy::Client<
    hyper_util::client::legacy::connect::HttpConnector,
    http_body_util::Full<bytes::Bytes>,
> {
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;
    use std::sync::OnceLock;

    static SSR_CLIENT: OnceLock<
        Client<
            hyper_util::client::legacy::connect::HttpConnector,
            http_body_util::Full<bytes::Bytes>,
        >,
    > = OnceLock::new();
    SSR_CLIENT.get_or_init(|| Client::builder(TokioExecutor::new()).build_http())
}

/// POST JSON to the SSR worker and deserialize the response. Uses
/// `hyper` directly — we already depend on it, so no extra crate.
///
/// Domain 20 audit D20-D: response body is read through
/// [`http_body_util::Limited`] so a misconfigured or compromised
/// loopback worker can't return arbitrarily large data and exhaust
/// memory. The cap is propagated from `SsrConfig::max_response_bytes`
/// (default 8 MiB). When the body exceeds the cap the Limited wrapper
/// returns an error which is surfaced as `Err("read body: ...")`;
/// `render()` then either falls back to CSR or propagates depending on
/// `throw_on_error`.
///
/// Content-Length pre-check: if the worker is honest enough to set
/// the header but reports a value larger than the cap, the request is
/// rejected before any body bytes are read.
async fn post_json(
    url: &str,
    body: Vec<u8>,
    timeout: Duration,
    max_response_bytes: usize,
) -> Result<SsrResponse, String> {
    use http_body_util::{BodyExt, Full, Limited};
    use hyper::Request;
    use hyper::header::{CONTENT_LENGTH, CONTENT_TYPE};

    let parsed = hyper::Uri::try_from(url).map_err(|e| format!("invalid url: {e}"))?;

    // Pick the default port from the URI scheme — defaulting to 80
    // on every URL (including `https://...`) sent the wrong Host
    // header for TLS-backed SSR endpoints, which some reverse
    // proxies reject. When the URI carries an explicit port, use it;
    // otherwise pick 443 for https and 80 for everything else.
    let scheme_default_port = match parsed.scheme_str() {
        Some("https") => 443,
        _ => 80,
    };
    let host_port = format!(
        "{}:{}",
        parsed.host().ok_or("missing host")?,
        parsed.port_u16().unwrap_or(scheme_default_port)
    );

    let req = Request::builder()
        .method("POST")
        .uri(url)
        .header(CONTENT_TYPE, "application/json")
        .header(CONTENT_LENGTH, body.len())
        .header("Host", host_port)
        .body(Full::new(bytes::Bytes::from(body)))
        .map_err(|e| format!("request build: {e}"))?;

    let client = shared_client();
    let fut = client.request(req);
    let resp = tokio::time::timeout(timeout, fut)
        .await
        .map_err(|_| format!("timeout after {:?}", timeout))?
        .map_err(|e| format!("hyper: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("ssr worker returned {}", status));
    }

    if let Some(cl) = resp
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
        && cl > max_response_bytes
    {
        return Err(format!(
            "ssr response Content-Length {cl} exceeds cap of \
             {max_response_bytes} bytes (configure via \
             InertiaConfig::ssr_max_response_bytes)"
        ));
    }

    let limited = Limited::new(resp.into_body(), max_response_bytes);
    let collected = limited
        .collect()
        .await
        .map_err(|e| format!("read body: {e}"))?;
    let bytes = collected.to_bytes();
    serde_json::from_slice::<SsrResponse>(&bytes).map_err(|e| format!("deserialize response: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssr_disabled_when_config_disabled() {
        let cfg = SsrConfig::default();
        assert!(!cfg.enabled);
    }

    #[tokio::test]
    async fn render_returns_none_when_disabled() {
        let cfg = SsrConfig::default();
        let page = serde_json::json!({"component": "Home"});
        let result = render(&cfg, "/foo", &page).await.unwrap();
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
        let result = render(&cfg, "/admin/users", &page).await.unwrap();
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
        // short-circuit before any connection attempt — proven by this
        // resolving immediately rather than waiting out `config.timeout`.
        let started = std::time::Instant::now();
        let result = render(&cfg, "/", &page).await.unwrap();
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
        // timeout, so that assertion fails deterministically here — it's
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
        let result = render(&cfg, "/", &page).await.unwrap();
        assert!(result.is_none());
        let msg = captured
            .lock()
            .expect("lock captured message")
            .clone()
            .expect("on_error must fire — the check is off, so render must actually dispatch");
        assert!(
            msg.contains("unreachable"),
            "with the check off, render must attempt (and fail) the connection, not \
             short-circuit on the missing bundle path: {msg}"
        );
    }

    #[test]
    fn missing_bundle_reason_is_none_when_bundle_exists() {
        let path = std::env::temp_dir().join(format!(
            "suprnova-ssr-test-bundle-{}.js",
            std::process::id()
        ));
        std::fs::write(&path, b"").expect("write test bundle");
        let cfg = SsrConfig {
            enabled: true,
            bundle_path: Some(path.clone()),
            ensure_bundle_exists: true,
            ..SsrConfig::default()
        };
        let result = missing_bundle_reason(&cfg);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_none());
    }

    #[test]
    fn missing_bundle_reason_is_none_when_no_path_is_configured() {
        // The safe default: `.ssr(url)` alone (no `.ssr_bundle_path`)
        // must never gate dispatch — this is what keeps every SSR test
        // in `framework/tests/inertia.rs` behaving exactly as before.
        let cfg = SsrConfig {
            enabled: true,
            bundle_path: None,
            ensure_bundle_exists: true,
            ..SsrConfig::default()
        };
        assert!(missing_bundle_reason(&cfg).is_none());
    }

    #[test]
    fn missing_bundle_reason_names_the_path() {
        let cfg = SsrConfig {
            enabled: true,
            bundle_path: Some(std::path::PathBuf::from("/nonexistent/ssr.js")),
            ensure_bundle_exists: true,
            ..SsrConfig::default()
        };
        let reason = missing_bundle_reason(&cfg).expect("missing bundle must be reported");
        assert!(reason.contains("/nonexistent/ssr.js"));
    }
}
