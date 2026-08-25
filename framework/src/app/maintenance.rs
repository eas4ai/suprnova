//! Maintenance mode - Suprnova's analogue of Laravel's `down` / `up`.
//!
//! While the application is "down", [`MaintenanceMiddleware`] short-circuits
//! every request with a `503 Service Unavailable` (configurable status code),
//! optional `Retry-After` / `Refresh` headers, an optional redirect, and an
//! optional bypass: visiting the secret URL sets an encrypted cookie that lets
//! that browser through.
//!
//! State lives behind the [`MaintenanceMode`] trait with two drivers:
//! [`FileMaintenanceMode`] (a JSON file at `storage_path("framework/down")`,
//! the default) and [`CacheMaintenanceMode`] (a key in the shared [`Cache`],
//! for multi-node deployments without a shared filesystem). The driver is
//! chosen by the `MAINTENANCE_DRIVER` environment variable (`file` | `cache`).
//!
//! ```rust,no_run
//! use suprnova::{global_middleware, MaintenanceMiddleware};
//! # fn ex() {
//! // In bootstrap.rs - health checks stay reachable while down.
//! global_middleware!(MaintenanceMiddleware::new().except(["api/health"]));
//! # }
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::cache::Cache;
use crate::error::FrameworkError;
use crate::http::{Cookie, HttpResponse, Request, Response, SameSite};
use crate::middleware::{Middleware, Next};
use subtle::ConstantTimeEq;

/// Bypass cookie name (Laravel uses `laravel_maintenance`).
const BYPASS_COOKIE: &str = "suprnova_maintenance";

/// Cache key used by [`CacheMaintenanceMode`].
const CACHE_KEY: &str = "suprnova:maintenance";

/// How long a bypass cookie stays valid after visiting the secret URL, in
/// seconds. Both halves of the expiry read it: the `expires_at` the server
/// stamps into the sealed payload, and the `max-age` the browser sees.
/// One literal, because two would drift.
const BYPASS_TTL_SECS: i64 = 12 * 60 * 60;

/// [`BYPASS_TTL_SECS`] as a `Duration`, for the cookie's `max-age`.
const BYPASS_TTL: Duration = Duration::from_secs(BYPASS_TTL_SECS as u64);

/// How far past [`BYPASS_TTL_SECS`] a deadline may sit before
/// [`has_valid_bypass_cookie`] refuses it as one it could not have issued.
///
/// This is an allowance for clock differences between hosts, not a grace
/// period: it does not extend how long a cookie lives. Multi-pod is the
/// default topology, every pod mints from its own clock, and a pod running
/// a second or two ahead would otherwise have every cookie it issues
/// refused by a correctly-clocked peer - killing a legitimate bypass in the
/// middle of the incident it was needed for. A minute comfortably covers
/// NTP-disciplined drift while still capping a cookie at one TTL.
const BYPASS_SKEW_SECS: i64 = 60;

/// The data recorded when the application is taken down. Mirrors the fields
/// Laravel writes to its "down" file.
///
/// The `Debug` impl is hand-written rather than derived so a stray
/// `dbg!()` or `tracing::info!(?payload)` does not leak the bypass
/// `secret` (anyone who possesses it can issue themselves the bypass
/// cookie). Pattern mirrors [`crate::EncryptionKey`]'s redacting
/// `Debug`.
#[derive(Clone, Serialize, Deserialize)]
pub struct MaintenancePayload {
    /// Request paths that stay reachable while down - exact match or a
    /// trailing-`*` prefix (e.g. `"api/health"`, `"webhooks/*"`).
    #[serde(default)]
    pub except: Vec<String>,
    /// If set, requests are redirected here (`302`) instead of served the
    /// maintenance response.
    #[serde(default)]
    pub redirect: Option<String>,
    /// Seconds for the `Retry-After` header.
    #[serde(default)]
    pub retry: Option<u64>,
    /// Seconds for the `Refresh` header (browser auto-refresh).
    #[serde(default)]
    pub refresh: Option<u64>,
    /// Secret URL segment that, when visited, installs the bypass cookie.
    #[serde(default)]
    pub secret: Option<String>,
    /// Status code for the maintenance response (default `503`).
    #[serde(default = "default_status")]
    pub status: u16,
    /// Pre-rendered HTML body served instead of the plain text response.
    #[serde(default)]
    pub template: Option<String>,
}

impl std::fmt::Debug for MaintenancePayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaintenancePayload")
            .field("except", &self.except)
            .field("redirect", &self.redirect)
            .field("retry", &self.retry)
            .field("refresh", &self.refresh)
            .field("secret", &self.secret.as_ref().map(|_| "[REDACTED]"))
            .field("status", &self.status)
            .field("template", &self.template)
            .finish()
    }
}

fn default_status() -> u16 {
    503
}

impl Default for MaintenancePayload {
    fn default() -> Self {
        Self {
            except: Vec::new(),
            redirect: None,
            retry: None,
            refresh: None,
            secret: None,
            status: 503,
            template: None,
        }
    }
}

impl MaintenancePayload {
    /// A fresh payload: status `503`, no options set.
    pub fn new() -> Self {
        Self::default()
    }
}

/// What the encrypted bypass cookie carries.
///
/// The cookie used to carry the bare secret, which left its 12-hour TTL a
/// `max-age` the *client* enforces: a captured cookie stayed valid until
/// somebody rotated the secret. Stamping `expires_at` inside the
/// AEAD-sealed payload moves the deadline to the server, where an attacker
/// cannot move it.
///
/// `deny_unknown_fields` plus two required fields is also what rejects a
/// pre-upgrade cookie: a bare secret is not a JSON object with these keys,
/// so it fails to deserialize and the request is treated as carrying no
/// bypass at all.
///
/// No `Debug` impl, deliberately - the same concern that made
/// [`MaintenancePayload`]'s `Debug` redact its secret by hand. Do not
/// derive one.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BypassCookie {
    /// The maintenance secret this cookie was issued for.
    secret: String,
    /// Unix timestamp, in seconds, after which this cookie is refused.
    expires_at: i64,
}

/// Storage backend for maintenance-mode state.
#[async_trait]
pub trait MaintenanceMode: Send + Sync {
    /// Record `payload` and put the application into maintenance mode.
    async fn activate(&self, payload: &MaintenancePayload) -> Result<(), FrameworkError>;
    /// Bring the application back up.
    async fn deactivate(&self) -> Result<(), FrameworkError>;
    /// Whether the application is currently down.
    async fn active(&self) -> Result<bool, FrameworkError>;
    /// The payload recorded by [`activate`](Self::activate).
    async fn data(&self) -> Result<MaintenancePayload, FrameworkError>;
}

/// File-backed maintenance state: a JSON file (default
/// `storage_path("framework/down")`). The default driver.
pub struct FileMaintenanceMode {
    path: PathBuf,
}

impl FileMaintenanceMode {
    /// Use the default path, `storage_path("framework/down")`.
    pub fn new() -> Self {
        Self {
            path: super::paths::storage_path("framework/down"),
        }
    }

    /// Use an explicit path (primarily for tests).
    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl Default for FileMaintenanceMode {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MaintenanceMode for FileMaintenanceMode {
    async fn activate(&self, payload: &MaintenancePayload) -> Result<(), FrameworkError> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                FrameworkError::internal(format!("maintenance: create {}: {e}", parent.display()))
            })?;
        }
        let json = serde_json::to_string_pretty(payload)
            .map_err(|e| FrameworkError::internal(format!("maintenance: serialize: {e}")))?;
        // Write to a sibling temp file then rename into place. `rename` is
        // atomic on the same filesystem, so a request reading the down file
        // concurrently with `down` never observes a half-written (and thus
        // unparseable) file - which would otherwise surface as a 500.
        let tmp = self.path.with_extension("tmp");
        tokio::fs::write(&tmp, json).await.map_err(|e| {
            FrameworkError::internal(format!("maintenance: write {}: {e}", tmp.display()))
        })?;
        tokio::fs::rename(&tmp, &self.path).await.map_err(|e| {
            FrameworkError::internal(format!(
                "maintenance: rename into {}: {e}",
                self.path.display()
            ))
        })
    }

    async fn deactivate(&self) -> Result<(), FrameworkError> {
        match tokio::fs::remove_file(&self.path).await {
            Ok(()) => Ok(()),
            // Already up - idempotent.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(FrameworkError::internal(format!(
                "maintenance: remove {}: {e}",
                self.path.display()
            ))),
        }
    }

    async fn active(&self) -> Result<bool, FrameworkError> {
        // tokio::fs::try_exists returns Ok(false) on NotFound (mirroring the
        // std::path::Path::exists semantics we used before) and Err on other
        // IO errors - surface those so a flaky FS shows up rather than being
        // silently treated as "up". Callers that want the prior fail-open
        // behaviour wrap with `.unwrap_or(false)` (see MaintenanceMiddleware).
        tokio::fs::try_exists(&self.path).await.map_err(|e| {
            FrameworkError::internal(format!("maintenance: probe {}: {e}", self.path.display()))
        })
    }

    async fn data(&self) -> Result<MaintenancePayload, FrameworkError> {
        let raw = tokio::fs::read_to_string(&self.path).await.map_err(|e| {
            FrameworkError::internal(format!("maintenance: read {}: {e}", self.path.display()))
        })?;
        serde_json::from_str(&raw).map_err(|e| {
            FrameworkError::internal(format!("maintenance: parse {}: {e}", self.path.display()))
        })
    }
}

/// Cache-backed maintenance state: a single key in the shared [`Cache`]. Use
/// this when multiple nodes must observe `down` / `up` without a shared
/// filesystem.
pub struct CacheMaintenanceMode {
    key: String,
}

impl CacheMaintenanceMode {
    /// Use the default cache key.
    pub fn new() -> Self {
        Self {
            key: CACHE_KEY.to_string(),
        }
    }

    /// Use an explicit cache key (primarily for tests).
    pub fn with_key(key: impl Into<String>) -> Self {
        Self { key: key.into() }
    }
}

impl Default for CacheMaintenanceMode {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MaintenanceMode for CacheMaintenanceMode {
    async fn activate(&self, payload: &MaintenancePayload) -> Result<(), FrameworkError> {
        Cache::put(&self.key, payload, None).await
    }

    async fn deactivate(&self) -> Result<(), FrameworkError> {
        // `forget` reports whether the key existed; deactivation is
        // idempotent and doesn't care, so discard it.
        Cache::forget(&self.key).await.map(|_| ())
    }

    async fn active(&self) -> Result<bool, FrameworkError> {
        Cache::has(&self.key).await
    }

    async fn data(&self) -> Result<MaintenancePayload, FrameworkError> {
        Cache::get::<MaintenancePayload>(&self.key)
            .await?
            .ok_or_else(|| FrameworkError::internal("maintenance: cache key absent"))
    }
}

/// The configured maintenance driver, chosen by `MAINTENANCE_DRIVER`
/// (`file` - default - or `cache`).
pub fn maintenance_mode() -> Arc<dyn MaintenanceMode> {
    match std::env::var("MAINTENANCE_DRIVER").as_deref() {
        Ok("cache") => Arc::new(CacheMaintenanceMode::new()),
        _ => Arc::new(FileMaintenanceMode::new()),
    }
}

/// Generate a random hex bypass secret (16 bytes → 32 hex chars), used by
/// `down --with-secret`. Hex keeps it safe as a URL path segment.
pub(crate) fn random_secret() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("OS RNG must be available to mint a maintenance secret");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Outcome of evaluating a request against maintenance state.
///
/// Split out of [`MaintenanceMiddleware::handle`] as a pure function so the
/// full decision matrix is unit-testable without a live server or an
/// encryption key.
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    /// Let the request reach the handler.
    Pass,
    /// The request hit the secret URL: install the bypass cookie, redirect home.
    GrantBypass,
    /// Redirect to the configured maintenance path.
    Redirect(String),
    /// Serve the maintenance response.
    Unavailable,
}

/// Pure maintenance decision. `has_valid_bypass_cookie` is computed by the
/// caller (it needs the encryption key) so this stays side-effect free.
///
/// The secret-URL comparison is constant-time. The secret is a bearer
/// credential that travels in the request path, so an early-exit compare
/// would publish, in response latency, how long a prefix the caller got
/// right - the same reason Laravel moved this compare to `hash_equals`
/// (`PreventRequestsDuringMaintenance.php:76`).
fn decide(
    path: &str,
    payload: &MaintenancePayload,
    middleware_except: &[String],
    has_valid_bypass_cookie: bool,
) -> Decision {
    if middleware_except.iter().any(|p| path_matches(path, p))
        || payload.except.iter().any(|p| path_matches(path, p))
    {
        return Decision::Pass;
    }

    if let Some(secret) = payload.secret.as_deref().filter(|s| !s.is_empty()) {
        let expected = secret.trim_start_matches('/');
        // Constant-time compare, not `==`. The secret is a bearer
        // credential carried in the request path: whoever can produce it
        // gets the bypass cookie. `==` on `str` returns at the first
        // differing byte, so response time tells an attacker how many
        // leading bytes were right and reduces guessing a 32-char secret
        // from one 16^32 search to 32 sequential 16-way searches.
        // `ct_eq` returns `Choice(0)` immediately on a length mismatch -
        // the same short-circuit PHP's `hash_equals` takes - and
        // otherwise compares every byte. Mirrors the cookie compare in
        // `has_valid_bypass_cookie`.
        if bool::from(path.as_bytes().ct_eq(expected.as_bytes())) {
            return Decision::GrantBypass;
        }
        if has_valid_bypass_cookie {
            return Decision::Pass;
        }
    }

    if let Some(redirect) = payload.redirect.as_deref()
        && path != redirect.trim_start_matches('/')
    {
        return Decision::Redirect(redirect.to_string());
    }

    Decision::Unavailable
}

/// Global middleware that short-circuits requests while the application is in
/// maintenance mode. Register it with `global_middleware!`.
pub struct MaintenanceMiddleware {
    driver: Arc<dyn MaintenanceMode>,
    except: Vec<String>,
}

impl MaintenanceMiddleware {
    /// Build using the env-configured driver ([`maintenance_mode`]).
    pub fn new() -> Self {
        Self {
            driver: maintenance_mode(),
            except: Vec::new(),
        }
    }

    /// Build with an explicit driver.
    pub fn with_driver(driver: Arc<dyn MaintenanceMode>) -> Self {
        Self {
            driver,
            except: Vec::new(),
        }
    }

    /// Paths that stay reachable while down - exact match or a trailing-`*`
    /// prefix. Merged with any `except` recorded at `down` time.
    pub fn except<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.except.extend(paths.into_iter().map(Into::into));
        self
    }
}

impl Default for MaintenanceMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware for MaintenanceMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let path = request.path().trim_start_matches('/').to_string();

        // Fast path: a middleware-level exception skips the backend probe.
        if self.except.iter().any(|p| path_matches(&path, p)) {
            return next(request).await;
        }

        // A backend error on the active() probe fails open: a flaky
        // maintenance store must not 503 an app that was never taken down.
        if !self.driver.active().await.unwrap_or(false) {
            return next(request).await;
        }

        let payload = match self.driver.data().await {
            Ok(p) => p,
            Err(e) => {
                // Race: state cleared between the active() probe and the
                // data() read. If we're up now, pass; else surface the error.
                if !self.driver.active().await.unwrap_or(false) {
                    return next(request).await;
                }
                return Err(HttpResponse::from(e));
            }
        };

        let has_cookie = payload
            .secret
            .as_deref()
            .filter(|s| !s.is_empty())
            .is_some_and(|secret| has_valid_bypass_cookie(&request, secret));

        match decide(&path, &payload, &self.except, has_cookie) {
            Decision::Pass => next(request).await,
            Decision::GrantBypass => bypass_response(payload.secret.as_deref().unwrap_or_default()),
            Decision::Redirect(location) => {
                Err(HttpResponse::new().status(302).header("Location", location))
            }
            Decision::Unavailable => Err(service_unavailable(&payload)),
        }
    }
}

/// Build the maintenance response: the configured status (default `503`), the
/// template or a plain body, and the `Retry-After` / `Refresh` headers.
fn service_unavailable(payload: &MaintenancePayload) -> HttpResponse {
    let status = if payload.status == 0 {
        503
    } else {
        payload.status
    };
    let mut resp = match &payload.template {
        Some(html) => HttpResponse::html(html.clone()),
        None => HttpResponse::text("503 Service Unavailable"),
    }
    .status(status);
    if let Some(retry) = payload.retry {
        resp = resp.header("Retry-After", retry.to_string());
    }
    if let Some(refresh) = payload.refresh {
        resp = resp.header("Refresh", refresh.to_string());
    }
    resp
}

/// Redirect to the intended destination with the encrypted bypass cookie set.
///
/// The payload carries the deadline as well as the secret, so
/// [`has_valid_bypass_cookie`] can refuse an expired cookie whatever the
/// browser did with `max-age`. `max-age` stays on the cookie regardless: it
/// still gets the browser to drop the cookie on its own, and dropping it
/// would make this a session-length cookie.
fn bypass_response(secret: &str) -> Response {
    let payload = BypassCookie {
        secret: secret.to_string(),
        expires_at: chrono::Utc::now()
            .timestamp()
            .saturating_add(BYPASS_TTL_SECS),
    };
    let plaintext = serde_json::to_string(&payload).map_err(|e| {
        HttpResponse::from(FrameworkError::internal(format!(
            "maintenance bypass cookie encode: {e}"
        )))
    })?;
    let cookie = Cookie::encrypted(BYPASS_COOKIE, plaintext)?
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(BYPASS_TTL);
    Err(HttpResponse::new()
        .status(302)
        .header("Location", "/")
        .cookie(cookie))
}

/// Whether the request carries a bypass cookie that is intact, issued for
/// the current secret, and still inside its deadline.
///
/// The value is read back through `crate::http::Cookie::read_encrypted_for`,
/// passing BYPASS_COOKIE as the logical name. The ciphertext is
/// AEAD-authenticated, so a successful decrypt means an attacker can't
/// forge the plaintext; we still compare the recovered secret in constant
/// time so a downstream change to the cookie envelope (or a hand-crafted
/// variant) can't accidentally turn the compare into a timing-side-channel
/// oracle for the bypass secret.
///
/// Five ways to fail, all closed: no cookie, a value that does not decrypt,
/// a plaintext that is not a [`BypassCookie`] (which is what a pre-upgrade
/// bare-secret cookie looks like), a deadline that has passed, or a deadline
/// further out than [`BYPASS_TTL_SECS`] plus [`BYPASS_SKEW_SECS`] from now.
/// That last one is a cap rather than a check on anything the client
/// controls: this build only ever stamps `now + BYPASS_TTL_SECS`, so a
/// longer deadline was issued by something else, and refusing it means no
/// cookie is ever worth more than one TTL.
///
/// The cap is widened by [`BYPASS_SKEW_SECS`] because the mint and the check
/// routinely happen on different hosts. In the multi-pod deployment this
/// framework treats as the default, a pod whose clock runs slightly ahead
/// stamps a deadline that a correctly-clocked peer would otherwise reject
/// outright, so a working bypass would die on the second request. The
/// allowance absorbs that; a clock that jumps backwards by more than a
/// minute can still retire a live cookie early, which fails closed - visit
/// the secret URL again.
///
/// Rotating the secret still invalidates every outstanding cookie, because
/// the secret comparison runs after the deadline check rather than instead
/// of it.
fn has_valid_bypass_cookie(request: &Request, secret: &str) -> bool {
    let Some(wire) = request.cookie(BYPASS_COOKIE) else {
        return false;
    };
    let Ok(plaintext) = Cookie::read_encrypted_for(BYPASS_COOKIE, &wire) else {
        return false;
    };
    let Ok(payload) = serde_json::from_str::<BypassCookie>(&plaintext) else {
        return false;
    };
    let now = chrono::Utc::now().timestamp();
    let latest_issuable = now
        .saturating_add(BYPASS_TTL_SECS)
        .saturating_add(BYPASS_SKEW_SECS);
    if payload.expires_at <= now || payload.expires_at > latest_issuable {
        return false;
    }
    payload.secret.as_bytes().ct_eq(secret.as_bytes()).into()
}

/// Match a normalized request path (no leading `/`) against an `except`
/// pattern: exact, or a trailing-`*` prefix. `"*"` matches everything.
fn path_matches(path: &str, pattern: &str) -> bool {
    let pattern = pattern.trim_start_matches('/');
    match pattern.strip_suffix('*') {
        Some(prefix) => {
            let prefix = prefix.trim_end_matches('/');
            prefix.is_empty() || path == prefix || path.starts_with(&format!("{prefix}/"))
        }
        None => path == pattern,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_down_path() -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut p = std::env::temp_dir();
        p.push(format!("suprnova-maint-{}-{unique}", std::process::id()));
        p.push("framework/down");
        p
    }

    fn down(secret: Option<&str>) -> MaintenancePayload {
        MaintenancePayload {
            secret: secret.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn payload_deserializes_with_default_status_503() {
        let p: MaintenancePayload = serde_json::from_str(r#"{"secret":"abc"}"#).unwrap();
        assert_eq!(p.status, 503);
        assert_eq!(p.secret.as_deref(), Some("abc"));
        assert!(p.except.is_empty());
        assert_eq!(p.retry, None);
    }

    #[test]
    fn path_matching_exact_and_wildcard() {
        assert!(path_matches("api/health", "api/health"));
        assert!(!path_matches("api/health", "api/status"));
        assert!(path_matches("webhooks/stripe", "webhooks/*"));
        assert!(path_matches("webhooks", "webhooks/*")); // the prefix itself
        assert!(!path_matches("web", "webhooks/*"));
        assert!(path_matches("anything/here", "*"));
        assert!(path_matches("admin", "/admin")); // leading slash tolerated
    }

    #[test]
    fn decide_serves_503_for_a_plain_request_while_down() {
        assert_eq!(
            decide("dashboard", &down(None), &[], false),
            Decision::Unavailable
        );
    }

    #[test]
    fn decide_passes_payload_and_middleware_exceptions() {
        let mut p = down(None);
        p.except = vec!["api/health".into()];
        assert_eq!(decide("api/health", &p, &[], false), Decision::Pass);
        assert_eq!(
            decide("status", &down(None), &["status".to_string()], false),
            Decision::Pass
        );
    }

    #[test]
    fn decide_grants_bypass_only_on_the_secret_url() {
        let p = down(Some("let-me-in"));
        assert_eq!(decide("let-me-in", &p, &[], false), Decision::GrantBypass);
        assert_eq!(decide("elsewhere", &p, &[], false), Decision::Unavailable);
    }

    #[test]
    fn decide_rejects_a_same_length_secret_that_differs_in_one_byte() {
        // The bypass secret is a bearer credential carried in the URL
        // path. A short-circuiting compare leaks how many leading bytes
        // an attacker got right, which turns 32 hex chars into 32 cheap
        // guesses instead of one expensive one. Same length, one byte
        // different, so only a constant-time compare can tell them apart
        // without timing.
        let p = down(Some("0123456789abcdef"));
        assert_eq!(
            decide("0123456789abcdee", &p, &[], false),
            Decision::Unavailable,
            "a near-miss secret must not grant bypass"
        );
        // Differing in the FIRST byte must take the same code path as
        // differing in the last - both are plain rejections.
        assert_eq!(
            decide("1123456789abcdef", &p, &[], false),
            Decision::Unavailable
        );
        // A length mismatch short-circuits, exactly as `hash_equals` does.
        assert_eq!(
            decide("0123456789abcde", &p, &[], false),
            Decision::Unavailable
        );
        // The exact secret still grants bypass.
        assert_eq!(
            decide("0123456789abcdef", &p, &[], false),
            Decision::GrantBypass
        );
    }

    #[test]
    fn decide_passes_when_bypass_cookie_is_valid() {
        let p = down(Some("let-me-in"));
        assert_eq!(decide("dashboard", &p, &[], true), Decision::Pass);
    }

    #[test]
    fn decide_ignores_an_empty_secret() {
        let p = down(Some(""));
        assert_eq!(decide("", &p, &[], false), Decision::Unavailable);
    }

    #[test]
    fn decide_redirects_except_for_the_redirect_target_itself() {
        let mut p = down(None);
        p.redirect = Some("/maintenance".into());
        assert_eq!(
            decide("dashboard", &p, &[], false),
            Decision::Redirect("/maintenance".to_string())
        );
        // The redirect target serves the page rather than looping.
        assert_eq!(decide("maintenance", &p, &[], false), Decision::Unavailable);
    }

    #[tokio::test]
    async fn file_driver_full_lifecycle() {
        let driver = FileMaintenanceMode::with_path(temp_down_path());
        assert!(!driver.active().await.unwrap());

        let payload = MaintenancePayload {
            retry: Some(60),
            secret: Some("letmein".into()),
            except: vec!["api/health".into()],
            ..Default::default()
        };
        driver.activate(&payload).await.unwrap();
        assert!(driver.active().await.unwrap());

        let read = driver.data().await.unwrap();
        assert_eq!(read.retry, Some(60));
        assert_eq!(read.secret.as_deref(), Some("letmein"));
        assert_eq!(read.except, vec!["api/health".to_string()]);
        assert_eq!(read.status, 503);

        driver.deactivate().await.unwrap();
        assert!(!driver.active().await.unwrap());
        // Idempotent: deactivating when already up is fine.
        driver.deactivate().await.unwrap();
    }

    #[tokio::test]
    async fn cache_driver_full_lifecycle() {
        // Own cache, not the process-global one.
        //
        // This used to call `Cache::bootstrap()` and read through whatever
        // store the binary happened to have installed. That made it a
        // bystander to every other test's container mutations: a store
        // swapped between `activate()` and `active()` takes the key with
        // it, and the assertion below fails with no hint of who moved it.
        // It went red exactly once in a gate run and could not be
        // reproduced in nineteen attempts afterwards, which is the most
        // expensive kind of failure to own - too rare to debug, frequent
        // enough to teach people the gate lies.
        //
        // A scoped binding removes the dependency rather than the
        // symptom: nothing outside this test can reach this store.
        use crate::cache::{CacheStore, InMemoryCache};

        let _scope = crate::testing::TestContainer::fake();
        let store = std::sync::Arc::new(InMemoryCache::new());
        crate::testing::TestContainer::bind::<dyn CacheStore>(store.clone());

        let key = format!("test:maint:{}", temp_down_path().display());
        let driver = CacheMaintenanceMode::with_key(key.clone());
        assert!(!driver.active().await.unwrap());

        let payload = MaintenancePayload {
            refresh: Some(15),
            status: 418,
            ..Default::default()
        };
        driver.activate(&payload).await.unwrap();

        // Isolation, asserted rather than assumed: the write landed in the
        // store *this test* bound. Holding a handle to it is what makes the
        // scope verifiable - without this the test would pass identically
        // while reading through the global store it used to depend on.
        assert!(
            store.has(&key).await.unwrap(),
            "the driver must write into this test's own store, not whichever \
             one the binary happens to have installed"
        );

        assert!(driver.active().await.unwrap());
        let read = driver.data().await.unwrap();
        assert_eq!(read.refresh, Some(15));
        assert_eq!(read.status, 418);

        driver.deactivate().await.unwrap();
        assert!(!driver.active().await.unwrap());
    }

    #[test]
    fn service_unavailable_uses_configured_status_with_503_fallback() {
        // Explicit status is honored (Laravel's `down --status`).
        let mut payload = MaintenancePayload {
            status: 418,
            ..Default::default()
        };
        assert_eq!(service_unavailable(&payload).status_code(), 418);
        // An unset (0) status falls back to 503.
        payload.status = 0;
        assert_eq!(service_unavailable(&payload).status_code(), 503);
        payload.status = 503;
        assert_eq!(service_unavailable(&payload).status_code(), 503);
    }
}
