//! Session middleware for suprnova framework

use crate::Request;
use crate::error::FrameworkError;
use crate::http::Response;
use crate::http::cookie::{Cookie, SameSite};
use crate::middleware::{Middleware, Next};
use async_trait::async_trait;
use rand::RngExt;
use secrecy::{ExposeSecret, SecretString};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use super::config::SessionConfig;
use super::driver::DatabaseSessionDriver;
use super::store::{SessionData, SessionStore};

pub(crate) type PendingRememberRevocation = (String, String, String);

// Per-request session slot. `tokio::task_local!` (not `thread_local!`)
// so the binding survives `.await` points that resume on a different
// worker thread - the same fix applied to `InertiaContext` in Tier 0.
//
// The slot is `Arc<Mutex<Option<SessionData>>>` rather than a bare
// `RefCell` because (a) the future inside `SESSION_CONTEXT.scope` may
// move between threads (so we need `Send + Sync`), and (b) the
// middleware needs to read the saved session back out *after* the
// scope returns. Closures passed to `session_mut` do not await, so a
// synchronous `std::sync::Mutex` is sound - guards drop before `.await`.
tokio::task_local! {
    pub(crate) static SESSION_CONTEXT: Arc<Mutex<Option<SessionData>>>;
    /// Active request session configuration. Auth flows use this to build
    /// remember-me cookies under the same prefix and attributes as the
    /// middleware handling the request; outside a request it falls back to
    /// `SessionConfig::from_env()`.
    pub(crate) static SESSION_CONFIG_CONTEXT: SessionConfig;
    /// Per-request slot for cookies that handlers want to attach to the
    /// outgoing response. `Auth::login_remember` and
    /// `Auth::revoke_remember_tokens` push into here; `SessionMiddleware`
    /// drains the slot when assembling the response, applying each cookie
    /// next to the session cookie.
    ///
    /// We can't have handlers mutate the `Response` directly - they
    /// return one synchronously, and the cookie machinery is in the
    /// middleware layer that already owns the response. A task-local
    /// slot is the same shape we use for the session itself.
    pub(crate) static PENDING_COOKIES: Arc<Mutex<Vec<Cookie>>>;
    /// Exact remember credentials invalidated by synchronous identity
    /// transitions. `SessionMiddleware` drains these after the handler and
    /// before persisting the replacement session.
    pub(crate) static PENDING_REMEMBER_REVOCATIONS: Arc<Mutex<Vec<PendingRememberRevocation>>>;
}

/// Queue one exact remember credential for end-of-request revocation.
#[must_use = "a synchronous identity transition must not silently drop exact remember cleanup"]
pub(crate) fn push_pending_remember_revocation(
    guard_name: &str,
    user_id: String,
    selector: String,
) -> bool {
    PENDING_REMEMBER_REVOCATIONS
        .try_with(|slot| {
            slot.lock()
                .unwrap()
                .push((guard_name.to_owned(), user_id, selector));
        })
        .is_ok()
}

/// Remove every exact remember credential queued in the active request.
pub(crate) fn take_pending_remember_revocations() -> Option<Vec<PendingRememberRevocation>> {
    PENDING_REMEMBER_REVOCATIONS
        .try_with(|slot| std::mem::take(&mut *slot.lock().unwrap()))
        .ok()
}

/// Restore revocations that could not yet be completed so middleware can retry.
#[must_use = "failed remember cleanup must remain queued for the middleware fail-closed gate"]
pub(crate) fn restore_pending_remember_revocations(
    mut revocations: Vec<PendingRememberRevocation>,
) -> bool {
    PENDING_REMEMBER_REVOCATIONS
        .try_with(|slot| {
            let mut queued = slot.lock().unwrap();
            revocations.append(&mut queued);
            *queued = revocations;
        })
        .is_ok()
}

/// Whether synchronous identity transitions can queue exact remember cleanup.
pub(crate) fn pending_remember_revocations_scope_installed() -> bool {
    PENDING_REMEMBER_REVOCATIONS.try_with(|_| ()).is_ok()
}

/// Push a cookie into the per-request pending-cookies slot.
///
/// Internal helper used by `Auth::login_remember` /
/// `Auth::revoke_remember_tokens`. The session middleware drains the
/// slot after the handler returns and attaches every cookie to the
/// response.
///
/// Returns `true` when the cookie was queued and `false` when the
/// `PENDING_COOKIES` task-local was not installed (e.g. a unit test
/// running without the session middleware). Callers that have
/// already done side-effecting work - issuing a DB row, rotating a
/// token - must check the result and either roll back or fail loud:
/// dropping the cookie silently leaves the client without the durable
/// half of the credential.
#[must_use = "callers that already committed side effects (DB rows, token rotations) must check whether the cookie was actually queued"]
pub(crate) fn push_pending_cookie(cookie: Cookie) -> bool {
    PENDING_COOKIES
        .try_with(|slot| {
            slot.lock().unwrap().push(cookie);
        })
        .is_ok()
}

/// Queue a cookie for the outgoing response, replacing any cookie
/// already queued under the same name. Backs the public
/// `Cookie::queue` facade (`http::cookie::Cookie::queue`) - unlike
/// `push_pending_cookie` above, which always appends (nothing in this
/// file or `Auth` ever queues the same name twice in one request), a
/// second `Cookie::queue` call for the same name is expected to
/// *replace* what's queued rather than add a duplicate `Set-Cookie`
/// line for it. Both write into the same jar, so a cookie queued by
/// either path is visible to `queued_cookie` / `unqueue_cookie` and
/// drains onto the response the same way.
///
/// Silently does nothing outside a request scope - the same posture
/// `inertia::flash::push` takes outside a flash scope.
pub(crate) fn queue_cookie(cookie: Cookie) {
    let _ = replace_pending_cookie(cookie);
}

/// Replace any queued cookie with the same name and report whether the
/// request-scoped pending-cookie jar was available.
#[must_use = "callers replacing a fail-closed cookie must verify the replacement reached the response jar"]
pub(crate) fn replace_pending_cookie(cookie: Cookie) -> bool {
    PENDING_COOKIES
        .try_with(|slot| {
            let mut guard = slot.lock().unwrap();
            guard.retain(|c| c.name() != cookie.name());
            guard.push(cookie);
        })
        .is_ok()
}

/// Look up a cookie queued under `name`, whether by `queue_cookie` or
/// by `push_pending_cookie` (e.g. a remember-me cookie `Auth` already
/// queued this request). `None` when nothing is queued under that
/// name, including outside a request scope.
pub(crate) fn queued_cookie(name: &str) -> Option<Cookie> {
    PENDING_COOKIES
        .try_with(|slot| {
            slot.lock()
                .unwrap()
                .iter()
                .find(|c| c.name() == name)
                .cloned()
        })
        .ok()
        .flatten()
}

/// Remove a cookie queued under `name`, if any. No-op when nothing is
/// queued under that name, or outside a request scope.
pub(crate) fn unqueue_cookie(name: &str) {
    let _ = PENDING_COOKIES.try_with(|slot| {
        slot.lock().unwrap().retain(|c| c.name() != name);
    });
}

/// Attach every queued pending cookie to `response`, in queue order.
///
/// Shared by the normal end-of-`handle` drain point and by
/// `SessionMiddleware::handle`'s own internal fail-closed 500 paths
/// (existing-session read failure with a dirtied fallback session,
/// session write failure for a dirty session, session-cookie
/// encryption failure). A queued cookie can already represent a side
/// effect committed elsewhere - e.g. `Auth::login_remember` has
/// already written the fresh remember-me token row by the time it
/// calls `push_pending_cookie` - so dropping it only because a 500
/// short-circuited the normal control-flow path before the drain loop
/// would strand that side effect on the server with no cookie ever
/// reaching the client to redeem it. That is the exact hazard
/// `push_pending_cookie`'s doc comment warns callers about, so this
/// middleware attaches pending cookies the same way regardless of
/// whether `response` ends up being the handler's own response or one
/// of this middleware's internally-synthesized error responses.
fn attach_pending_cookies(response: Response, pending_cookies: Vec<Cookie>) -> Response {
    let mut response = response;
    for cookie in pending_cookies {
        response = match response {
            Ok(res) => Ok(res.cookie(cookie)),
            Err(res) => Err(res.cookie(cookie)),
        };
    }
    response
}

/// Whether the per-request pending-cookies slot is installed.
///
/// Lets callers pre-check **before** doing irreversible work (DB inserts,
/// token rotation) that the cookie they're about to queue will actually
/// reach the response. Pairs with [`push_pending_cookie`] for the
/// "atomic credential issue" path: bail before the side effect if the
/// scope is absent rather than after, so a dropped cookie can never
/// leave an orphan DB row behind.
pub(crate) fn pending_cookies_scope_installed() -> bool {
    PENDING_COOKIES.try_with(|_| ()).is_ok()
}

/// Return the request's active session configuration, or the environment
/// configuration when called outside `SessionMiddleware::handle`.
pub(crate) fn current_session_config() -> SessionConfig {
    SESSION_CONFIG_CONTEXT
        .try_with(Clone::clone)
        .unwrap_or_else(|_| SessionConfig::from_env())
}

/// Whether the per-request session slot is installed.
///
/// Pre-check for sync session-mutating primitives (`Auth::login_id`)
/// that need to refuse a silent no-op when called outside a request
/// scope. Mirrors [`pending_cookies_scope_installed`].
pub(crate) fn session_scope_installed() -> bool {
    SESSION_CONTEXT.try_with(|_| ()).is_ok()
}

/// Get the current session (read-only)
///
/// Returns a clone of the current session data if available.
///
/// # Example
///
/// ```rust,no_run
/// use suprnova::session::session;
///
/// if let Some(session) = session() {
///     let name: Option<String> = session.get("name");
/// }
/// ```
pub fn session() -> Option<SessionData> {
    SESSION_CONTEXT
        .try_with(|slot| slot.lock().unwrap().clone())
        .ok()
        .flatten()
}

/// Get the current session and modify it
///
/// # Example
///
/// ```rust,no_run
/// use suprnova::session::session_mut;
///
/// session_mut(|session| {
///     session.put("name", "John");
/// });
/// ```
pub fn session_mut<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut SessionData) -> R,
{
    SESSION_CONTEXT
        .try_with(|slot| slot.lock().unwrap().as_mut().map(f))
        .ok()
        .flatten()
}

/// Generate a cryptographically secure session ID
///
/// Generates a 40-character alphanumeric string.
pub fn generate_session_id() -> String {
    let mut rng = rand::rng();
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

    (0..40)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

/// Generate a CSRF token
///
/// Same format as session ID for consistency.
pub fn generate_csrf_token() -> String {
    generate_session_id()
}

const SESSION_COOKIE_PAYLOAD_SEPARATOR: char = '.';

fn parse_session_cookie_payload(payload: &str) -> Option<(String, Option<u64>)> {
    if super::store::is_valid_session_id(payload) {
        return Some((payload.to_string(), None));
    }

    let (session_id, touched_at) = payload.split_once(SESSION_COOKIE_PAYLOAD_SEPARATOR)?;
    if !super::store::is_valid_session_id(session_id) {
        return None;
    }

    let touched_at = touched_at.parse::<u64>().ok()?;
    Some((session_id.to_string(), Some(touched_at)))
}

fn unix_timestamp_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn session_touch_is_due(last_touch: Option<u64>, now: u64, interval: std::time::Duration) -> bool {
    match last_touch {
        None => true,
        Some(last_touch) if last_touch > now => true,
        Some(last_touch) => now.saturating_sub(last_touch) >= interval.as_secs().max(1),
    }
}

fn effective_session_touch_interval(config: &SessionConfig) -> std::time::Duration {
    let half_lifetime = std::time::Duration::from_secs((config.lifetime.as_secs() / 2).max(1));
    config
        .touch_interval
        .max(std::time::Duration::from_secs(1))
        .min(half_lifetime)
}

static SESSION_GC_RUNS: AtomicU64 = AtomicU64::new(0);
static SESSION_GC_SUCCESSES: AtomicU64 = AtomicU64::new(0);
static SESSION_GC_FAILURES: AtomicU64 = AtomicU64::new(0);
static SESSION_GC_REMOVED_ROWS: AtomicU64 = AtomicU64::new(0);
static SESSION_GC_LAST_SUCCESS: AtomicU64 = AtomicU64::new(0);
static SESSION_GC_LAST_FAILURE: AtomicU64 = AtomicU64::new(0);

/// Process-local observability snapshot for the supervised session collector.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionGcMetrics {
    /// Total collector attempts in this process.
    pub runs: u64,
    /// Collector attempts that completed successfully.
    pub successes: u64,
    /// Collector attempts that returned an error.
    pub failures: u64,
    /// Cumulative expired rows removed by successful runs.
    pub removed_rows: u64,
    /// Unix timestamp of the most recent successful run, or zero before one.
    pub last_success_unix_seconds: u64,
    /// Unix timestamp of the most recent failed run, or zero before one.
    pub last_failure_unix_seconds: u64,
}

/// Return process-local session collector counters and last-run timestamps.
pub fn session_gc_metrics() -> SessionGcMetrics {
    SessionGcMetrics {
        runs: SESSION_GC_RUNS.load(Ordering::Relaxed),
        successes: SESSION_GC_SUCCESSES.load(Ordering::Relaxed),
        failures: SESSION_GC_FAILURES.load(Ordering::Relaxed),
        removed_rows: SESSION_GC_REMOVED_ROWS.load(Ordering::Relaxed),
        last_success_unix_seconds: SESSION_GC_LAST_SUCCESS.load(Ordering::Relaxed),
        last_failure_unix_seconds: SESSION_GC_LAST_FAILURE.load(Ordering::Relaxed),
    }
}

/// Session middleware
///
/// Handles session lifecycle:
/// 1. Reads session ID from cookie
/// 2. Loads session data from storage
/// 3. Makes session available during request
/// 4. Saves session after request
/// 5. Sets session cookie on response
pub struct SessionMiddleware {
    config: SessionConfig,
    store: Arc<dyn SessionStore>,
}

/// Publish `store` into the application container as `dyn SessionStore`,
/// but only if nothing is registered there yet.
///
/// # Security - SEC-02(b)
///
/// [`crate::session::destroy_all_for_user`] (the primitive
/// [`crate::auth_flows::PasswordReset::complete`] and friends call to
/// revoke every session belonging to a user) used to construct a fresh
/// [`DatabaseSessionDriver`] unconditionally, regardless of what store
/// this middleware was actually configured with. An app running a
/// custom store (Redis, per the `with_store` worked example in
/// `manual/session.md`) would have its revocation calls silently
/// operate against the wrong backend and report success while revoking
/// nothing - the manual's own claim that "a security-team forced reset
/// also kicks out an active attacker" did not hold.
///
/// `SessionMiddleware` is normally constructed exactly once per process
/// (`new` / `with_store` / `install` / `install_with_gc` are all called
/// from `bootstrap::register()`), so registering the configured store
/// here means every revocation call transparently uses it with zero
/// extra application wiring.
///
/// `bind_if_absent` (not `bind`) deliberately: this is implicit,
/// framework-driven registration, so it must not clobber a binding the
/// application installed itself, and - just as importantly - it must
/// not let a *second* `SessionMiddleware` constructed later in the same
/// process (a pattern this framework's own test suite uses heavily,
/// constructing many short-lived middleware instances with distinct
/// in-memory stores in one test binary) silently steal the slot away
/// from whichever store a still-running test expects revocation calls
/// to reach. Tests that need to observe revocation against a specific
/// store should override the binding hermetically via
/// `crate::container::testing::TestContainer::fake` / `scope` rather
/// than relying on this global registration.
fn register_configured_store(store: Arc<dyn SessionStore>) {
    crate::container::App::bind_if_absent::<dyn SessionStore>(store);
}

impl SessionMiddleware {
    /// Create a new session middleware with the given configuration
    pub fn new(config: SessionConfig) -> Self {
        let store = Arc::new(DatabaseSessionDriver::new(config.lifetime));
        register_configured_store(store.clone());
        Self { config, store }
    }

    /// Create session middleware with a custom store
    pub fn with_store(config: SessionConfig, store: Arc<dyn SessionStore>) -> Self {
        register_configured_store(store.clone());
        Self { config, store }
    }

    /// Construct the middleware AND register a [`SessionGcSupervisor`]
    /// that calls [`SessionStore::gc`] once per `interval`. The Tokio
    /// equivalent of Laravel's `StartSession::collectGarbage` lottery -
    /// a real supervised task instead of a 2/100 chance per request.
    ///
    /// The gc loop is spawned through
    /// [`crate::supervisor::SupervisorRegistry::spawn`] so it (a) gets
    /// a proper restart loop with exponential backoff on panic, and
    /// (b) participates in the framework's shutdown drain - when
    /// `Server::run` fires its supervisor cancellation token, the gc
    /// loop exits cleanly within the 5-second grace window instead of
    /// being silently force-aborted.
    ///
    /// Errors from `gc()` are logged at `warn!` and do not kill the
    /// loop. Apps that want explicit scheduling control should keep
    /// using `new` / `with_store` and register their own
    /// [`crate::Schedule`] entry.
    pub async fn install_with_gc(config: SessionConfig, interval: std::time::Duration) -> Self {
        let me = Self::new(config);
        let supervisor: Arc<dyn crate::supervisor::Supervisor> = Arc::new(SessionGcSupervisor {
            store: me.store.clone(),
            interval,
        });
        crate::supervisor::SupervisorRegistry::spawn(supervisor).await;
        me
    }

    /// Register the configured gc supervisor and return the middleware.
    /// The default cadence is once per hour. Drop-in replacement for
    /// `new(config)` in production bootstrap code.
    pub async fn install(config: SessionConfig) -> Self {
        let interval = config.gc_interval;
        Self::install_with_gc(config, interval).await
    }

    /// Read access to the bound session store. Lets callers feed the
    /// same store into a `Schedule` entry without rebuilding it.
    pub fn store(&self) -> Arc<dyn SessionStore> {
        self.store.clone()
    }

    /// Build the outbound session cookie. Returns `Err` if `Crypt`
    /// failed to encrypt the session id - which by design only happens
    /// when `Crypt` is not initialized.
    ///
    /// `Server::from_config` guarantees `Crypt` is installed before
    /// any middleware runs (it fails boot otherwise outside dev
    /// environments, and generates a transient dev key otherwise), so
    /// the error path is purely defensive. If it ever does fire, the
    /// middleware fails the request closed rather than emit a
    /// plaintext session id.
    fn create_session_cookie(
        &self,
        session_id: &str,
        touched_at: u64,
    ) -> Result<Cookie, crate::FrameworkError> {
        let payload = format!("{session_id}{SESSION_COOKIE_PAYLOAD_SEPARATOR}{touched_at}");
        let base = Cookie::encrypted(&self.config.cookie_name, &payload)?;
        let mut cookie = base
            .http_only(self.config.cookie_http_only)
            .secure(self.config.cookie_secure)
            .path(&self.config.cookie_path)
            .partitioned(self.config.cookie_partitioned);

        // `expire_on_close = true` → omit `Max-Age` so the browser
        // forgets the cookie when the window closes. Mirrors
        // Laravel's `session.expire_on_close`.
        if !self.config.expire_on_close {
            cookie = cookie.max_age(self.config.lifetime);
        }

        if let Some(ref domain) = self.config.cookie_domain {
            cookie = cookie.domain(domain);
        }

        cookie = match self.config.cookie_same_site.to_lowercase().as_str() {
            "strict" => cookie.same_site(SameSite::Strict),
            "none" => cookie.same_site(SameSite::None),
            _ => cookie.same_site(SameSite::Lax),
        };

        Ok(cookie.prefixed(self.config.cookie_prefix))
    }

    fn create_forget_session_cookie(&self) -> Cookie {
        let mut cookie = Cookie::forget(self.config.cookie_prefix.apply(&self.config.cookie_name))
            .http_only(self.config.cookie_http_only)
            .secure(self.config.cookie_secure)
            .path(&self.config.cookie_path)
            .partitioned(self.config.cookie_partitioned);

        if let Some(ref domain) = self.config.cookie_domain {
            cookie = cookie.domain(domain);
        }

        match self.config.cookie_same_site.to_lowercase().as_str() {
            "strict" => cookie.same_site(SameSite::Strict),
            "none" => cookie.same_site(SameSite::None),
            _ => cookie.same_site(SameSite::Lax),
        }
    }
}

const REMEMBER_GUARD_CARRIER_PREFIX: &str = "suprnova.remember.v1:";
const REMEMBER_GUARD_CARRIER_VERSION_PREFIX: &str = "suprnova.remember.v";

#[derive(serde::Serialize, serde::Deserialize)]
struct RememberGuardCarrier {
    guard: String,
    credential: String,
}

enum DecodedRememberCarrier {
    Supported(String, String),
    UnknownVersion,
    Malformed,
}

pub(crate) fn remember_selector(credential: &str) -> Result<String, FrameworkError> {
    let (selector, verifier) = credential.split_once('.').ok_or_else(|| {
        FrameworkError::internal("issued remember credential has no selector separator")
    })?;
    if selector.is_empty() || verifier.is_empty() {
        return Err(FrameworkError::internal(
            "issued remember credential has an empty selector or verifier",
        ));
    }
    Ok(selector.to_owned())
}

pub(crate) fn encode_remember_carrier(
    guard: &str,
    credential: &str,
) -> Result<String, FrameworkError> {
    let encoded = serde_json::to_string(&RememberGuardCarrier {
        guard: guard.to_owned(),
        credential: credential.to_owned(),
    })
    .map_err(|error| FrameworkError::internal(format!("encode remember carrier: {error}")))?;
    Ok(format!("{REMEMBER_GUARD_CARRIER_PREFIX}{encoded}"))
}

async fn retire_committed_selector(
    engine: &dyn crate::magnetar_integration::engine::MagnetarPasswordAuthEngine,
    user_id: &str,
    selector: &str,
) -> Result<(), FrameworkError> {
    let exact_error = match engine.revoke_remember_selector(user_id, selector).await {
        Ok(true) => return Ok(()),
        Ok(false) => "exact replacement was not found".to_owned(),
        Err(error) => format!("exact retirement failed: {error}"),
    };
    match engine.revoke_remember(user_id).await {
        Ok(_) => Ok(()),
        Err(error) => Err(FrameworkError::internal(format!(
            "retire committed remember replacement ({exact_error}); owner-wide fallback failed: {error}"
        ))),
    }
}

async fn retire_committed_replacement(
    engine: &dyn crate::magnetar_integration::engine::MagnetarPasswordAuthEngine,
    user_id: &str,
    replacement: &str,
) -> Result<(), FrameworkError> {
    match remember_selector(replacement) {
        Ok(selector) => retire_committed_selector(engine, user_id, &selector).await,
        Err(selector_error) => engine.revoke_remember(user_id).await.map(|_| ()).map_err(|error| {
            FrameworkError::internal(format!(
                "retire malformed committed remember replacement ({selector_error}); owner-wide fallback failed: {error}"
            ))
        }),
    }
}

type PendingRememberedOpaqueSession = (String, String, magnetar::sessions::WebSessionBinding);

async fn retire_unpersisted_opaque_session(
    engine: Option<&Arc<dyn crate::magnetar_integration::engine::MagnetarPasswordAuthEngine>>,
    pending: &mut Option<PendingRememberedOpaqueSession>,
    reason: &'static str,
) {
    let Some((_, session_id, _)) = pending.take() else {
        return;
    };
    let Some(engine) = engine else {
        return;
    };
    match engine.revoke_session(&session_id).await {
        Ok(true) => {}
        Ok(false) => tracing::warn!(
            session_id = %session_id,
            reason,
            "Magnetar remembered session was not found during cleanup"
        ),
        Err(error) => tracing::warn!(
            %error,
            session_id = %session_id,
            reason,
            "Magnetar remembered session cleanup did not complete"
        ),
    }
}

fn decode_remember_carrier(plaintext: &str) -> DecodedRememberCarrier {
    let Some(encoded) = plaintext.strip_prefix(REMEMBER_GUARD_CARRIER_PREFIX) else {
        return if plaintext.starts_with(REMEMBER_GUARD_CARRIER_VERSION_PREFIX) {
            DecodedRememberCarrier::UnknownVersion
        } else {
            DecodedRememberCarrier::Supported(
                crate::auth::Auth::default_guard_name(),
                plaintext.to_owned(),
            )
        };
    };
    let Ok(carrier) = serde_json::from_str::<RememberGuardCarrier>(encoded) else {
        return DecodedRememberCarrier::Malformed;
    };
    if carrier.guard.is_empty() || carrier.credential.is_empty() {
        return DecodedRememberCarrier::Malformed;
    }
    DecodedRememberCarrier::Supported(carrier.guard, carrier.credential)
}

/// Build an outbound remember-me cookie carrying the encrypted plaintext.
///
/// Framework-internal helper. `Auth::login_remember` and the
/// middleware rotation path both call it so cookie attribute defaults
/// live in one place. Mirrors the security profile of the session
/// cookie (HttpOnly, optional Secure, SameSite=Lax).
///
/// `max_age` is set explicitly to match the TTL of the underlying
/// `remember_tokens` row - codex review demanded "expires-at matches
/// token expiration." Callers (login_remember + middleware rotation)
/// pass the same `ttl_minutes` they used to issue the row.
///
/// Exposed as `pub` (rather than `pub(crate)`) because integration
/// tests in `framework/tests/remember_me.rs` need to verify the cookie
/// attributes a real handler would emit. `#[doc(hidden)]` keeps it out
/// of the public rustdoc surface.
#[doc(hidden)]
pub fn create_remember_cookie(
    config: &SessionConfig,
    plaintext: &str,
    max_age: std::time::Duration,
) -> Result<Cookie, crate::FrameworkError> {
    let base = Cookie::encrypted(super::super::auth::remember::COOKIE_NAME, plaintext)?;
    let mut cookie = base
        .http_only(true)
        .secure(config.cookie_secure)
        .path(&config.cookie_path)
        .partitioned(config.cookie_partitioned)
        .max_age(max_age);

    if let Some(ref domain) = config.cookie_domain {
        cookie = cookie.domain(domain);
    }

    cookie = match config.cookie_same_site.to_lowercase().as_str() {
        "strict" => cookie.same_site(SameSite::Strict),
        "none" => cookie.same_site(SameSite::None),
        _ => cookie.same_site(SameSite::Lax),
    };

    Ok(cookie.prefixed(config.cookie_prefix))
}

/// Build a Max-Age=0 cookie that tells the client to drop the
/// `remember_me` cookie. Used by `Auth::revoke_remember_tokens` and by
/// the middleware when a remember cookie fails verification.
///
/// `pub` + `#[doc(hidden)]` for the same reason as
/// `create_remember_cookie`: integration tests need to verify the
/// "clear cookie" shape, but consumers should not depend on it.
#[doc(hidden)]
pub fn create_forget_remember_cookie(config: &SessionConfig) -> Cookie {
    let mut cookie = Cookie::forget(
        config
            .cookie_prefix
            .apply(super::super::auth::remember::COOKIE_NAME),
    )
    .path(&config.cookie_path)
    .secure(config.cookie_secure)
    .partitioned(config.cookie_partitioned)
    .same_site(SameSite::Lax);
    if let Some(ref domain) = config.cookie_domain {
        cookie = cookie.domain(domain);
    }
    cookie
}

/// Framework-managed supervisor that runs `SessionStore::gc` on a fixed
/// interval. Registered by [`SessionMiddleware::install_with_gc`] /
/// [`SessionMiddleware::install`] and lives inside the same
/// `SUPERVISOR_TASKS` JoinSet the rest of the framework drains on
/// shutdown.
///
/// The loop honours the supervisor cancellation token via
/// `tokio::select!` so it exits cleanly within the 5-second drain
/// window instead of being aborted. Per-tick `gc()` errors are
/// `warn!`-logged and the loop keeps going - the call site treats
/// transient backend failure as something to ride out, not something
/// to kill the daemon over. Panics escape this `run()` and are caught
/// then restarted with exponential backoff by the supervisor restart
/// machinery.
pub struct SessionGcSupervisor {
    /// Session store on which to invoke `gc()` each tick.
    pub store: Arc<dyn SessionStore>,
    /// Interval between sweeps.
    pub interval: std::time::Duration,
}

#[async_trait]
impl crate::supervisor::Supervisor for SessionGcSupervisor {
    fn name(&self) -> &'static str {
        "session_gc"
    }

    async fn run(
        &self,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<(), crate::FrameworkError> {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                _ = tokio::time::sleep(self.interval) => {
                    SESSION_GC_RUNS.fetch_add(1, Ordering::Relaxed);
                    match self.store.gc().await {
                        Ok(removed) => {
                            SESSION_GC_SUCCESSES.fetch_add(1, Ordering::Relaxed);
                            SESSION_GC_REMOVED_ROWS.fetch_add(removed, Ordering::Relaxed);
                            SESSION_GC_LAST_SUCCESS.store(unix_timestamp_now(), Ordering::Relaxed);
                            if removed > 0 {
                                tracing::debug!(removed, "session gc removed expired rows");
                            }
                        }
                        Err(e) => {
                            SESSION_GC_FAILURES.fetch_add(1, Ordering::Relaxed);
                            SESSION_GC_LAST_FAILURE.store(unix_timestamp_now(), Ordering::Relaxed);
                            tracing::warn!(error = %e, "session gc failed");
                        }
                    }
                }
            }
        }
    }

    fn restart_policy(&self) -> crate::supervisor::RestartPolicy {
        // run() only returns Ok on cancellation (which we don't want to
        // restart after) and never returns Err. Panics are still routed
        // through the supervisor restart loop.
        crate::supervisor::RestartPolicy::OnError
    }
}

#[async_trait]
impl Middleware for SessionMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        // Defensive: refuse to run at all when `Crypt` isn't installed.
        // `Server::from_config` guarantees a key is in place before
        // middleware boots (failing closed in production, generating a
        // transient key in dev). If we somehow got here without one -
        // e.g. an embedder built a service loop without going through
        // `Server::from_config` - bail out closed rather than emit or
        // accept plaintext session ids.
        if !crate::crypto::Crypt::is_initialized() {
            return Err(crate::http::HttpResponse::text(
                "Internal Server Error: encryption key not installed",
            )
            .status(500));
        }

        // Read the session ID from the inbound cookie. The cookie
        // value is AES-256-GCM ciphertext; decrypt failure (tamper,
        // key rotation) silently mints a fresh session id rather than
        // logging per-request - same fail-quietly semantics as Laravel
        // when the SESSION cookie is unreadable.
        //
        // `original_session_id` carries the id we LOADED the session
        // with so the regeneration-aware persistence step at the
        // bottom of `handle` knows which store row to destroy when a
        // handler (login, 2FA promotion, remember-me hydration, manual
        // regenerate, logout_and_invalidate) rotated the id this
        // request. `None` when no cookie was present or when the
        // cookie was unreadable - neither case names a real row, so
        // there's nothing to migrate away from.
        //
        // Shape validation: even a successfully-decrypted id must
        // match the 40-char lowercase-alphanumeric shape minted by
        // `generate_session_id` before we let it reach the store.
        // The AES-256-GCM cookie is authenticated, so a foreign id
        // requires a key-compromise OR a rotated key whose ciphertext
        // we can no longer trust - either way, the right move is to
        // mint a fresh id rather than route an attacker-controlled
        // string into the session-store lookup. Mirrors Laravel's
        // `Store::isValidId` check in `Illuminate/Session/Store.php`
        // (the source of [`super::store::is_valid_session_id`]).
        let (original_session_id, last_touch_at): (Option<String>, Option<u64>) =
            match request.cookie(&self.config.cookie_prefix.apply(&self.config.cookie_name)) {
                Some(raw) => match Cookie::read_encrypted_for(&self.config.cookie_name, &raw) {
                    Ok(payload) => match parse_session_cookie_payload(&payload) {
                        Some((id, touched_at)) => (Some(id), touched_at),
                        None => {
                            tracing::debug!(
                                "session cookie decrypted to an invalid payload; minting a fresh id"
                            );
                            (None, None)
                        }
                    },
                    Err(_) => (None, None),
                },
                None => (None, None),
            };
        let session_id = original_session_id
            .clone()
            .unwrap_or_else(generate_session_id);

        // A request without a valid session cookie cannot name a stored
        // session, so do not issue a guaranteed database miss. Keep a clean
        // session in memory for handlers that need one; it is persisted only
        // if request handling actually mutates it.
        let (mut session, stale_session_cookie, session_read_failed) =
            if original_session_id.is_none() {
                (
                    SessionData::new(session_id.clone(), generate_csrf_token()),
                    false,
                    false,
                )
            } else {
                match self.store.read(&session_id).await {
                    Ok(Some(s)) => (s, false, false),
                    Ok(None) => (
                        SessionData::new(generate_session_id(), generate_csrf_token()),
                        true,
                        false,
                    ),
                    Err(e) => {
                        // Store read failed (outage, corruption). Degrade
                        // gracefully by minting a fresh session - same posture as
                        // Laravel when the session row is unreadable. `warn!`, not
                        // `error!`: this fires once per request, so during an
                        // outage an error-level line would spam at request rate.
                        tracing::warn!(error = %e, "session read failed; minting a fresh session");
                        (
                            SessionData::new(session_id.clone(), generate_csrf_token()),
                            false,
                            true,
                        )
                    }
                }
            };

        // Age flash data from previous request
        session.age_flash_data();

        // Per-request bag of cookies handlers want attached. Populated
        // by `push_pending_cookie` (called from `Auth::login_remember`
        // and `Auth::revoke_remember_tokens`) and drained below right
        // next to where the session cookie is attached.
        let pending: Arc<Mutex<Vec<Cookie>>> = Arc::new(Mutex::new(Vec::new()));
        let pending_remember_revocations: Arc<Mutex<Vec<PendingRememberRevocation>>> =
            Arc::new(Mutex::new(Vec::new()));

        let magnetar_engine = crate::magnetar_integration::optional_password_engine();
        let mut pending_remembered_opaque_session: Option<PendingRememberedOpaqueSession> = None;

        // An installed engine makes the default guard's digest-only binding
        // authoritative, as well as any named guard record that explicitly
        // carries such a binding. Binding-less named records remain valid for
        // provider-backed SessionGuard implementations.
        if let Some(engine) = magnetar_engine.as_ref() {
            let default_guard_name = crate::auth::Auth::default_guard_name();
            let binding = session.magnetar_web_binding();
            let valid_user_id = match (session.user_id.as_deref(), binding.as_ref()) {
                (Some(expected_user_id), Some(binding)) => {
                    match engine.resolve_web_binding(binding).await {
                        Ok(verified) if verified.user_id() == expected_user_id => {
                            Some(expected_user_id.to_owned())
                        }
                        Ok(_)
                        | Err(magnetar::Error::InvalidInput { .. })
                        | Err(magnetar::Error::NotFound { .. })
                        | Err(magnetar::Error::Conflict { .. }) => None,
                        Err(error) => {
                            tracing::warn!(
                                %error,
                                "Magnetar web-session validation failed closed"
                            );
                            None
                        }
                    }
                }
                (None, None) => None,
                _ => None,
            };
            if session.user_id.is_some() && valid_user_id.is_none() {
                session.user_id = None;
                session.remove_auth_guard(&default_guard_name);
                session.clear_magnetar_web_binding();
                session.dirty = true;
                crate::auth::request_state::clear_guard_user(&default_guard_name);
            } else if let Some(valid_user_id) = valid_user_id {
                session.set_auth_guard_id(&default_guard_name, valid_user_id);
            } else if session.user_id.is_none() && session.magnetar_web_binding().is_some() {
                session.remove_auth_guard(&default_guard_name);
                session.clear_magnetar_web_binding();
                crate::auth::request_state::clear_guard_user(&default_guard_name);
            } else if session.auth_guard_id(&default_guard_name).is_some() {
                session.remove_auth_guard(&default_guard_name);
                crate::auth::request_state::clear_guard_user(&default_guard_name);
            }

            for guard_name in session.auth_guard_names() {
                if guard_name == default_guard_name {
                    continue;
                }
                if !session.has_auth_guard_magnetar_binding(&guard_name) {
                    continue;
                }
                let expected_user_id = session.auth_guard_id(&guard_name);
                let binding = session.auth_guard_magnetar_binding(&guard_name);
                let valid = match (expected_user_id.as_deref(), binding.as_ref()) {
                    (Some(expected_user_id), Some(binding)) => {
                        match engine.resolve_web_binding(binding).await {
                            Ok(verified) => verified.user_id() == expected_user_id,
                            Err(
                                magnetar::Error::InvalidInput { .. }
                                | magnetar::Error::NotFound { .. }
                                | magnetar::Error::Conflict { .. },
                            ) => false,
                            Err(error) => {
                                tracing::warn!(
                                    %error,
                                    guard = %guard_name,
                                    "Magnetar named web-session validation failed closed"
                                );
                                false
                            }
                        }
                    }
                    _ => false,
                };
                if !valid {
                    session.remove_auth_guard(&guard_name);
                    crate::auth::request_state::clear_guard_user(&guard_name);
                }
            }
        }

        // Remember-me hydration uses Magnetar whenever its engine is installed.
        // The legacy auth::remember table is consulted only with no engine.
        if let Some(raw_cookie) = request.cookie(
            &self
                .config
                .cookie_prefix
                .apply(super::super::auth::remember::COOKIE_NAME),
        ) {
            let decoded = match Cookie::read_encrypted_for(
                super::super::auth::remember::COOKIE_NAME,
                &raw_cookie,
            ) {
                Ok(plaintext) => decode_remember_carrier(&plaintext),
                Err(_) => DecodedRememberCarrier::Malformed,
            };
            let decoded = match decoded {
                DecodedRememberCarrier::Supported(guard_name, credential) => {
                    Some((guard_name, credential))
                }
                DecodedRememberCarrier::UnknownVersion => None,
                DecodedRememberCarrier::Malformed => {
                    crate::auth::request_state::clear_active_remember_carrier();
                    pending
                        .lock()
                        .unwrap()
                        .push(create_forget_remember_cookie(&self.config));
                    None
                }
            };
            if let Some((guard_name, credential)) = decoded {
                let is_default = guard_name == crate::auth::Auth::default_guard_name();
                if let Ok(selector) = remember_selector(&credential) {
                    crate::auth::request_state::set_active_remember_carrier(&guard_name, &selector);
                }
                let already_authenticated = session.auth_guard_id(&guard_name).is_some()
                    || (is_default && session.user_id.is_some());
                if !already_authenticated {
                    if let Some(engine) = magnetar_engine.as_ref() {
                        let metadata = magnetar::sessions::SessionMetadata {
                            user_agent: request
                                .headers()
                                .get(hyper::header::USER_AGENT)
                                .and_then(|value| value.to_str().ok())
                                .map(ToOwned::to_owned),
                            ip_address: None,
                        };
                        let replacement_lifetime = chrono::Duration::from_std(
                            self.config.remember_lifetime,
                        )
                        .map_err(|_| {
                            FrameworkError::internal(
                                "configured remember lifetime exceeds Magnetar range",
                            )
                        })?;
                        match engine
                              .remember_sign_in_attempt(
                                magnetar::sessions::RememberCredential::from_host(
                                    SecretString::from(credential),
                                ),
                                metadata,
                                replacement_lifetime,
                            )
                            .await
                        {
                                Ok(crate::magnetar_integration::engine::MagnetarRememberSignInAttempt::Authenticated(outcome)) => {
                                    let user_id = outcome.session.session.user_id.to_string();
                                    let opaque_session_id = outcome.session.session_id.clone();
                                    let binding = outcome.session.web_binding.clone();
                                  let replacement = outcome.replacement.expose_once();
                                  let replacement = replacement.expose_secret();
                                  let selector = match remember_selector(replacement) {
                                        Ok(selector) => selector,
                                        Err(error) => {
                                            if let Err(cleanup_error) = retire_committed_replacement(
                                                engine.as_ref(),
                                                &user_id,
                                                replacement,
                                            )
                                            .await
                                            {
                                                tracing::warn!(
                                                    %cleanup_error,
                                                    "Magnetar remember replacement cleanup did not complete"
                                                );
                                            }
                                            if let Err(cleanup_error) =
                                                engine.revoke_session(&opaque_session_id).await
                                            {
                                                tracing::warn!(
                                                    %cleanup_error,
                                                    "Magnetar remembered session cleanup did not complete"
                                                );
                                            }
                                            return Err(error.into());
                                        }
                                  };
                                  let cookie = match encode_remember_carrier(&guard_name, replacement)
                                      .and_then(|carrier| {
                                          create_remember_cookie(
                                              &self.config,
                                              &carrier,
                                              self.config.remember_lifetime,
                                          )
                                      })
                                    {
                                        Ok(cookie) => cookie,
                                        Err(error) => {
                                            if let Err(cleanup_error) = retire_committed_selector(
                                                engine.as_ref(),
                                                &user_id,
                                                &selector,
                                            )
                                            .await
                                            {
                                                tracing::warn!(
                                                    %cleanup_error,
                                                    "Magnetar remember replacement cleanup did not complete"
                                                );
                                            }
                                            if let Err(cleanup_error) =
                                                engine.revoke_session(&opaque_session_id).await
                                            {
                                                tracing::warn!(
                                                    %cleanup_error,
                                                    "Magnetar remembered session cleanup did not complete"
                                                );
                                            }
                                            return Err(error.into());
                                        }
                                    };

                                    pending_remembered_opaque_session = Some((
                                        guard_name.clone(),
                                        opaque_session_id,
                                        binding.clone(),
                                    ));

                                  session.rotate_id(generate_session_id());
                                session.csrf_token = generate_csrf_token();
                                session.replace_auth_guard_id(&guard_name, user_id.clone());
                                crate::auth::request_state::set_active_remember_carrier(
                                    &guard_name,
                                    &selector,
                                );
                                session.set_auth_guard_remember_selector(&guard_name, selector);
                                session
                                    .set_auth_guard_magnetar_binding(&guard_name, binding.clone());
                                if is_default {
                                    session.user_id = Some(user_id.clone());
                                    session.set_magnetar_web_binding(binding);
                                }
                                crate::auth::request_state::set_guard_user_id(&guard_name, user_id);
                                crate::auth::request_state::set_guard_via_remember(
                                    &guard_name,
                                    true,
                                  );
                                  pending.lock().unwrap().push(cookie);
                              }
                              Ok(crate::magnetar_integration::engine::MagnetarRememberSignInAttempt::RotationCommitted {
                                  user_id,
                                  replacement,
                                  error,
                                  disposition,
                              }) => {
                                  let replacement = replacement.expose_once();
                                  let replacement = replacement.expose_secret();
                                  match disposition {
                                      magnetar::sessions::RememberPostRotationDisposition::Retryable => {
                                          let selector = match remember_selector(replacement) {
                                                Ok(selector) => selector,
                                                Err(host_error) => {
                                                    if let Err(cleanup_error) = retire_committed_replacement(
                                                        engine.as_ref(),
                                                        &user_id,
                                                        replacement,
                                                    )
                                                    .await
                                                    {
                                                        tracing::warn!(
                                                            %cleanup_error,
                                                            "Magnetar remember replacement cleanup did not complete"
                                                        );
                                                    }
                                                    return Err(host_error.into());
                                                }
                                          };
                                          let cookie = match encode_remember_carrier(
                                              &guard_name,
                                              replacement,
                                          )
                                          .and_then(|carrier| {
                                              create_remember_cookie(
                                                  &self.config,
                                                  &carrier,
                                                  self.config.remember_lifetime,
                                              )
                                          }) {
                                                Ok(cookie) => cookie,
                                                Err(host_error) => {
                                                    if let Err(cleanup_error) = retire_committed_selector(
                                                        engine.as_ref(),
                                                        &user_id,
                                                        &selector,
                                                    )
                                                    .await
                                                    {
                                                        tracing::warn!(
                                                            %cleanup_error,
                                                            "Magnetar remember replacement cleanup did not complete"
                                                        );
                                                    }
                                                    return Err(host_error.into());
                                                }
                                          };
                                          crate::auth::request_state::set_verified_active_remember_carrier(
                                              &guard_name,
                                              &user_id,
                                              &selector,
                                          );
                                          pending.lock().unwrap().push(cookie);
                                          tracing::warn!(
                                              %error,
                                              "Magnetar remember sign-in will retry with the rotated credential"
                                          );
                                        }
                                        magnetar::sessions::RememberPostRotationDisposition::Reject => {
                                            if let Err(revoke_error) = retire_committed_replacement(
                                                engine.as_ref(),
                                                &user_id,
                                                replacement,
                                            )
                                            .await
                                            {
                                                tracing::warn!(
                                                    %revoke_error,
                                                    "Magnetar rejected remember replacement could not be retired"
                                                );
                                            }
                                            crate::auth::request_state::clear_active_remember_carrier();
                                          pending
                                              .lock()
                                              .unwrap()
                                              .push(create_forget_remember_cookie(&self.config));
                                          tracing::warn!(
                                              %error,
                                              "Magnetar remember sign-in rejected the rotated credential"
                                          );
                                      }
                                    }
                                }
                                Ok(crate::magnetar_integration::engine::MagnetarRememberSignInAttempt::RotationOutcomeUnknown {
                                    error,
                                }) => tracing::warn!(
                                    %error,
                                    "Magnetar remember rotation outcome is unknown; continuing anonymously"
                                ),
                                Err(
                                magnetar::Error::InvalidInput { .. }
                                | magnetar::Error::NotFound { .. }
                                | magnetar::Error::Conflict { .. },
                            ) => {
                                crate::auth::request_state::clear_active_remember_carrier();
                                pending
                                    .lock()
                                    .unwrap()
                                    .push(create_forget_remember_cookie(&self.config));
                            }
                            Err(error) => tracing::warn!(
                                %error,
                                "Magnetar remember sign-in failed; continuing anonymously"
                            ),
                        }
                    } else {
                        let ttl_minutes =
                            i64::try_from(self.config.remember_lifetime.as_secs() / 60)
                                .unwrap_or(i64::MAX);
                        match crate::auth::remember::verify_and_rotate(&credential, ttl_minutes)
                            .await
                        {
                            Ok(Some((user_id, new_credential))) => {
                                let selector = remember_selector(&new_credential)?;
                                let carrier =
                                    encode_remember_carrier(&guard_name, &new_credential)?;
                                let cookie = create_remember_cookie(
                                    &self.config,
                                    &carrier,
                                    self.config.remember_lifetime,
                                )?;

                                session.rotate_id(generate_session_id());
                                session.csrf_token = generate_csrf_token();
                                session.replace_auth_guard_id(&guard_name, user_id.clone());
                                crate::auth::request_state::set_active_remember_carrier(
                                    &guard_name,
                                    &selector,
                                );
                                session.set_auth_guard_remember_selector(&guard_name, selector);
                                if is_default {
                                    session.user_id = Some(user_id.clone());
                                }
                                crate::auth::request_state::set_guard_user_id(&guard_name, user_id);
                                crate::auth::request_state::set_guard_via_remember(
                                    &guard_name,
                                    true,
                                );
                                pending.lock().unwrap().push(cookie);
                            }
                            Ok(None) => {
                                crate::auth::request_state::clear_active_remember_carrier();
                                pending
                                    .lock()
                                    .unwrap()
                                    .push(create_forget_remember_cookie(&self.config));
                            }
                            Err(error) => tracing::warn!(
                                %error,
                                "legacy remember-me verification failed; continuing without it"
                            ),
                        }
                    }
                }
            }
        }

        // Capture the current URL before `next()` consumes the
        // request. We write it to the session under `_previous.url`
        // AFTER the handler runs, but only when the response indicates
        // a normal GET HTML page (200/300-range, not an Inertia
        // partial, not an AJAX endpoint). This mirrors Laravel's
        // `StartSession::storeCurrentUrl` behaviour and is what
        // [`Redirect::back`] reads.
        let is_get = *request.method() == hyper::Method::GET;
        let is_inertia = request.is_inertia();
        let wants_json = request
            .headers()
            .get("accept")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("application/json") && !v.contains("text/html"))
            .unwrap_or(false);
        let current_url = {
            let path = request.path().to_string();
            match request.uri().query() {
                Some(q) if !q.is_empty() => format!("{path}?{q}"),
                _ => path,
            }
        };

        // Bind both the session and the pending-cookies slot to
        // `tokio::task_local!` so they survive `.await` points that
        // resume on a different worker thread. Handlers read/write
        // through `session()` / `session_mut()` / `push_pending_cookie`.
        let slot: Arc<Mutex<Option<SessionData>>> = Arc::new(Mutex::new(Some(session)));
        let response = SESSION_CONFIG_CONTEXT
            .scope(
                self.config.clone(),
                SESSION_CONTEXT.scope(
                    slot.clone(),
                    PENDING_COOKIES.scope(
                        pending.clone(),
                        PENDING_REMEMBER_REVOCATIONS
                            .scope(pending_remember_revocations.clone(), next(request)),
                    ),
                ),
            )
            .await;

        let remembered_binding_preserved = pending_remembered_opaque_session.as_ref().is_none_or(
            |(guard_name, _, expected_binding)| {
                slot.lock()
                    .unwrap()
                    .as_ref()
                    .and_then(|session| session.auth_guard_magnetar_binding(guard_name))
                    .as_ref()
                    == Some(expected_binding)
            },
        );
        if !remembered_binding_preserved {
            retire_unpersisted_opaque_session(
                magnetar_engine.as_ref(),
                &mut pending_remembered_opaque_session,
                "remembered binding changed before persistence",
            )
            .await;
        }

        let pending_revocations =
            std::mem::take(&mut *pending_remember_revocations.lock().unwrap());
        for (guard_name, user_id, selector) in pending_revocations {
            if let Err(error) =
                crate::auth::Auth::revoke_remember_selector(&guard_name, &user_id, &selector).await
            {
                retire_unpersisted_opaque_session(
                    magnetar_engine.as_ref(),
                    &mut pending_remembered_opaque_session,
                    "deferred remember cleanup failed",
                )
                .await;
                tracing::error!(
                    %error,
                    guard = %guard_name,
                    "deferred remember credential revocation failed; discarding identity transition"
                );
                let pending_cookies = std::mem::take(&mut *pending.lock().unwrap());
                let failure = Err(crate::http::HttpResponse::text(
                    "Internal Server Error: identity transition cleanup failed",
                )
                .status(500));
                return attach_pending_cookies(failure, pending_cookies);
            }
        }

        // Take the potentially-modified session back out of the slot.
        let mut session = slot.lock().unwrap().take();

        // Record the current URL as `_previous.url` if this turned out
        // to be a "real" HTML page navigation - successful, GET, not
        // Inertia partial, not JSON-API. Drives `Redirect::back`.
        //
        // We only write when the value would change. Same-URL navigations
        // (a GET that returns to the same page on retry, a duplicate
        // request) leave the session clean - that preserves the
        // "unmodified session never gets a fail-closed write" invariant
        // exercised by the session-persistence regression tests.
        let response_status = match &response {
            Ok(r) | Err(r) => r.status_code(),
        };
        let is_redirect = (300..400).contains(&response_status);
        let is_success = (200..300).contains(&response_status);
        // SEC: `current_url` is built straight from `request.path()` +
        // query, and an origin-form HTTP request-target is syntactically
        // free to start with `//` (httparse's URI_MAP permits it; this
        // isn't rejected at the HTTP-parse layer). `_previous.url` backs
        // `Redirect::back`, `Redirect::refresh`, and `url::previous`, and
        // none of those readers re-check the value before it lands in a
        // `Location` header - so an app whose `fallback!` route (the
        // standard Inertia/SPA app-shell pattern: any unmatched path
        // renders 200) answers `GET //evil.test/anything` with 200 would,
        // without this guard, persist `//evil.test/anything` verbatim and
        // hand every later `Redirect::back()` an off-origin target.
        //
        // Guarding here, at the one write site, closes it for all three
        // readers at once - fixing only a caller would leave the others
        // exposed. When the candidate fails the check, the write is
        // skipped entirely rather than replaced with a synthesized value
        // like `/`: every reader already treats "no previous URL
        // recorded" as an expected, handled case with its own fallback
        // default, so declining to record an untrustworthy value is
        // strictly more informative than inventing one, and it can never
        // clobber a genuinely good URL recorded by an earlier, legitimate
        // navigation still sitting in the session.
        if is_get
            && !is_inertia
            && !wants_json
            && (is_success || is_redirect)
            && let Some(ref mut s) = session
            && s.previous_url().as_deref() != Some(current_url.as_str())
            && let Some(safe_current_url) = crate::routing::url::root_relative_or_none(&current_url)
        {
            s.set_previous_url(safe_current_url);
        }

        // Drain pending cookies - both the ones queued from the
        // middleware (remember-me rotation / clear) and any queued by
        // handlers via `Auth::login_remember` etc.
        let mut response = response;
        let mut pending_cookies = std::mem::take(&mut *pending.lock().unwrap());

        let touched_at = unix_timestamp_now();
        let touch_due = original_session_id.is_some()
            && !stale_session_cookie
            && !session_read_failed
            && session_touch_is_due(
                last_touch_at,
                touched_at,
                effective_session_touch_interval(&self.config),
            );
        if stale_session_cookie && session.as_ref().is_some_and(|session| !session.is_dirty()) {
            pending_cookies.push(self.create_forget_session_cookie());
        }
        if session_read_failed && session.as_ref().is_some_and(SessionData::is_dirty) {
            retire_unpersisted_opaque_session(
                magnetar_engine.as_ref(),
                &mut pending_remembered_opaque_session,
                "framework session state was unavailable",
            )
            .await;
            tracing::error!(
                session_id = %session_id,
                "session mutated after existing state could not be loaded; failing closed"
            );
            let failure = Err(crate::http::HttpResponse::text(
                "Internal Server Error: session state unavailable",
            )
            .status(500));
            return attach_pending_cookies(failure, pending_cookies);
        }

        // Persist and emit a cookie only when request handling created or
        // changed state, or an existing session's bounded sliding-expiry
        // touch is due. Clean cookieless requests remain entirely
        // session-store-free.
        if let Some(session) = session
            && (session.is_dirty() || touch_due)
        {
            // Regeneration-aware migration: when the session id changed
            // during this request (login, 2FA promotion, remember-me
            // hydration, manual regenerate, logout_and_invalidate),
            // destroy the row keyed on the OLD id before writing the
            // new one. Mirrors Laravel's `Store::migrate(true)` which
            // calls `handler->destroy($oldId)` as part of regenerate.
            //
            // Without this, the old row keeps carrying its prior
            // `user_id` (or whatever state it had) until TTL - an
            // attacker holding the prior encrypted session cookie can
            // replay it and remain authenticated, which is the exact
            // inverse of what `logout_and_invalidate` documents. This
            // also closes the silent DB-row leak that login,
            // 2FA-complete, and remember-me hydration would otherwise
            // accumulate at TTL.
            //
            // The inequality check is load-bearing: a normal navigation
            // keeps the same id, and destroying-then-writing would race
            // with concurrent reads on the same row. Destroy is a
            // security boundary: if it fails, the old authenticated row
            // remains replayable. Fail closed, expire the browser's old
            // credential, and return before writing or issuing the new id.
            if let Some(ref old_id) = original_session_id
                && old_id != &session.id
            {
                match self.store.destroy(old_id).await {
                    Ok(()) => {
                        tracing::debug!(
                            old_session_id = %old_id,
                            new_session_id = %session.id,
                            "session id rotated; destroyed old store row"
                        );
                    }
                    Err(e) => {
                        retire_unpersisted_opaque_session(
                            magnetar_engine.as_ref(),
                            &mut pending_remembered_opaque_session,
                            "prior framework session row could not be destroyed",
                        )
                        .await;
                        tracing::error!(
                            error = %e,
                            "session id rotation cleanup failed; failing closed with 500"
                        );
                        pending_cookies.push(self.create_forget_session_cookie());
                        let failure = Err(crate::http::HttpResponse::text(
                            "Internal Server Error: session rotation failed",
                        )
                        .status(500));
                        return attach_pending_cookies(failure, pending_cookies);
                    }
                }
            }

            let write_succeeded = match self.store.write(&session).await {
                Ok(()) => true,
                Err(e) if !session.is_dirty() => {
                    tracing::warn!(
                        error = %e,
                        session_id = %session.id,
                        "session last-activity touch failed; continuing with existing cookie"
                    );
                    false
                }
                Err(e) => {
                    retire_unpersisted_opaque_session(
                        magnetar_engine.as_ref(),
                        &mut pending_remembered_opaque_session,
                        "framework session write failed",
                    )
                    .await;
                    // The session was mutated this request (login, logout,
                    // CSRF rotation, flash, remember-me hydration, ...) and
                    // we could not persist it. Returning the handler's
                    // success response now would lie: the client would get
                    // a session cookie for state the store never recorded,
                    // so the next request loads an empty session and the
                    // mutation silently vanishes - e.g. a "successful"
                    // login that didn't stick. Fail closed. We return
                    // BEFORE create_session_cookie below, so no cookie is
                    // attached: a cookie for an id the store never saw is
                    // worse than none.
                    tracing::error!(
                        error = %e,
                        session_id = %session.id,
                        "session write failed for a mutated session; failing closed with 500"
                    );
                    let failure = Err(crate::http::HttpResponse::text(
                        "Internal Server Error: session persistence failed",
                    )
                    .status(500));
                    return attach_pending_cookies(failure, pending_cookies);
                }
            };

            if write_succeeded {
                // Add session cookie to response. Encryption must succeed
                // here - we already verified Crypt is initialized at the
                // top of `handle`. If it doesn't, fail the request closed.
                let cookie = match self.create_session_cookie(&session.id, touched_at) {
                    Ok(c) => c,
                    Err(_) => {
                        retire_unpersisted_opaque_session(
                            magnetar_engine.as_ref(),
                            &mut pending_remembered_opaque_session,
                            "framework session cookie construction failed",
                        )
                        .await;
                        let failure = Err(crate::http::HttpResponse::text(
                            "Internal Server Error: session cookie encryption failed",
                        )
                        .status(500));
                        return attach_pending_cookies(failure, pending_cookies);
                    }
                };

                response = match response {
                    Ok(res) => Ok(res.cookie(cookie)),
                    Err(res) => Err(res.cookie(cookie)),
                };
                pending_remembered_opaque_session = None;
            }
        }

        retire_unpersisted_opaque_session(
            magnetar_engine.as_ref(),
            &mut pending_remembered_opaque_session,
            "remembered framework session was not persisted",
        )
        .await;

        // Attach every pending cookie. Done after the session cookie
        // so the relative ordering in the `Set-Cookie` header list is
        // stable (session first, then remember-me / clears).
        attach_pending_cookies(response, pending_cookies)
    }
}

/// Regenerate the session ID (for security after login)
///
/// This creates a new session ID while preserving session data,
/// which helps prevent session fixation attacks.
pub fn regenerate_session_id() {
    session_mut(|session| {
        session.rotate_id(generate_session_id());
    });
}

/// Invalidate the current session (clear all data)
///
/// Mirrors Laravel's `Store::invalidate`: flush every value, then
/// regenerate the session id (and CSRF token) so a fixed/stale id can't
/// be replayed against the now-empty session. Rotating the id here keeps
/// `invalidate_session` in lockstep with [`regenerate_session_id`] and
/// `Auth::logout_and_invalidate` - the regeneration-aware persistence
/// step in [`SessionMiddleware`] then destroys the old store row.
pub fn invalidate_session() {
    session_mut(|session| {
        session.flush();
        session.rotate_id(generate_session_id());
        session.csrf_token = generate_csrf_token();
    });
}

/// Helper to get the CSRF token from current session
pub fn get_csrf_token() -> Option<String> {
    session().map(|s| s.csrf_token)
}

/// Mint a new CSRF token for the current session without otherwise
/// touching session data. Mirrors Laravel's `Store::regenerateToken`
/// (`Illuminate/Session/Store.php:755-758`). Returns the new token
/// (or `None` when no session scope is installed).
pub fn regenerate_csrf_token() -> Option<String> {
    session_mut(|session| {
        let token = generate_csrf_token();
        session.csrf_token = token.clone();
        session.dirty = true;
        token
    })
}

/// Helper to check if user is authenticated
pub fn is_authenticated() -> bool {
    auth_user_id().is_some()
}

/// Helper to get the authenticated user ID
///
/// Consults the request-scoped auth state first (so a `once` /
/// `set_user` authentication that was never written to the session is
/// still visible to `Auth::id()`), then falls back to the persisted
/// session user.
pub fn auth_user_id() -> Option<String> {
    crate::auth::request_state::current_user_id().or_else(|| session().and_then(|s| s.user_id))
}

/// Return one session guard's request or persisted identifier.
pub(crate) fn guard_auth_user_id(guard_name: &str) -> Option<String> {
    crate::auth::request_state::guard_user_id(guard_name)
        .or_else(|| persisted_guard_auth_user_id(guard_name))
}

/// Return one session guard's persisted identifier, ignoring request-only overrides.
pub(crate) fn persisted_guard_auth_user_id(guard_name: &str) -> Option<String> {
    session().and_then(|session| {
        session.auth_guard_id(guard_name).or_else(|| {
            (guard_name == crate::auth::Auth::default_guard_name())
                .then_some(session.user_id)
                .flatten()
        })
    })
}

/// Persist one session guard's identifier and mirror the default guard.
pub(crate) fn set_guard_auth_user(guard_name: &str, user_id: impl Into<String>) {
    let user_id = user_id.into();
    let is_default = guard_name == crate::auth::Auth::default_guard_name();
    session_mut(|session| {
        session.replace_auth_guard_id(guard_name, user_id.clone());
        if is_default {
            session.clear_magnetar_web_binding();
            session.user_id = Some(user_id.clone());
            session.dirty = true;
        }
    });
    crate::auth::request_state::set_guard_user_id(guard_name, user_id);
}

/// Helper to set the authenticated user
pub fn set_auth_user(user_id: impl Into<String>) {
    set_guard_auth_user(&crate::auth::Auth::default_guard_name(), user_id);
}

/// Clear one session guard without touching sibling guards.
pub(crate) fn clear_guard_auth_user(guard_name: &str) {
    let is_default = guard_name == crate::auth::Auth::default_guard_name();
    session_mut(|session| {
        session.remove_auth_guard(guard_name);
        if is_default {
            session.user_id = None;
            session.clear_magnetar_web_binding();
            session.dirty = true;
        }
    });
    crate::auth::request_state::clear_guard_user(guard_name);
}

/// Helper to clear the authenticated user (logout)
pub fn clear_auth_user() {
    clear_guard_auth_user(&crate::auth::Auth::default_guard_name());
}

/// Reserved key under `SessionData::data` for the "password verified
/// but 2FA challenge not yet completed" user-id.
///
/// Kept in the generic data bag rather than as a typed field on
/// `SessionData` so adding 2FA-challenge support doesn't require
/// every session driver to learn about a new column - the bag is
/// already serialized end-to-end.
const TWO_FACTOR_PENDING_KEY: &str = "_two_factor_pending_user_id";

/// Read the user-id of a user who has authenticated their password
/// but has not yet completed the 2FA TOTP challenge. Returns `None`
/// outside the request scope or when no challenge is pending.
///
/// Backs [`crate::auth_flows::TwoFactor::pending_user_id`].
pub fn two_factor_pending_user_id() -> Option<String> {
    session().and_then(|s| {
        s.data
            .get(TWO_FACTOR_PENDING_KEY)
            .and_then(|v| v.as_str().map(String::from))
    })
}

/// Stash a "2FA challenge pending" user-id in the session. The caller
/// (typically [`crate::auth_flows::TwoFactor::start_challenge`]) is
/// responsible for clearing the fully-authenticated slot first -
/// pending and authed are mutually exclusive states.
pub fn set_two_factor_pending(user_id: impl Into<String>) {
    let user_id = user_id.into();
    session_mut(|session| {
        session.data.insert(
            TWO_FACTOR_PENDING_KEY.to_string(),
            serde_json::Value::String(user_id),
        );
        session.dirty = true;
    });
}

/// Clear the "2FA challenge pending" user-id. Called by a successful
/// challenge completion (which promotes pending → authed), an
/// explicit "cancel challenge" UI action, or a fresh logout.
pub fn clear_two_factor_pending() {
    session_mut(|session| {
        if session.data.remove(TWO_FACTOR_PENDING_KEY).is_some() {
            session.dirty = true;
        }
    });
}

/// Reserved key under `SessionData::data` for the "user asked to be
/// remembered" preference that was supplied to
/// [`crate::auth_flows::TwoFactor::start_challenge`] and needs to
/// survive until `crate::auth_flows::TwoFactor::complete_challenge`
/// can re-issue the remember-me cookie.
///
/// Lives in the generic data bag rather than as a typed field on
/// `SessionData` for the same reason as [`TWO_FACTOR_PENDING_KEY`] -
/// avoiding driver churn for a feature whose state is naturally
/// transient.
const TWO_FACTOR_PENDING_REMEMBER_KEY: &str = "_two_factor_pending_remember";

/// Read the "user asked to be remembered" preference stashed by
/// [`set_two_factor_pending_remember`]. Returns `false` outside a
/// request scope or when no preference was set.
///
/// Backs `crate::auth_flows::TwoFactor::complete_challenge`'s
/// remember-me re-issue path.
pub fn two_factor_pending_remember() -> bool {
    session()
        .and_then(|s| {
            s.data
                .get(TWO_FACTOR_PENDING_REMEMBER_KEY)
                .and_then(|v| v.as_bool())
        })
        .unwrap_or(false)
}

/// Stash the "user asked to be remembered" preference alongside the
/// pending user-id. The caller - typically
/// [`crate::auth_flows::TwoFactor::start_challenge`] - passes through
/// the `remember` argument it received from the login form.
///
/// Stored as a JSON boolean; clears the slot when `remember` is
/// `false` to keep the bag minimal.
pub fn set_two_factor_pending_remember(remember: bool) {
    if remember {
        session_mut(|session| {
            session.data.insert(
                TWO_FACTOR_PENDING_REMEMBER_KEY.to_string(),
                serde_json::Value::Bool(true),
            );
            session.dirty = true;
        });
    } else {
        clear_two_factor_pending_remember();
    }
}

/// Clear the "remember-me on completion" preference. Called by a
/// successful challenge completion (after consuming the value), by
/// [`clear_two_factor_pending`] callers that want a clean teardown, or
/// when the preference is explicitly being reset to `false`.
pub fn clear_two_factor_pending_remember() {
    session_mut(|session| {
        if session
            .data
            .remove(TWO_FACTOR_PENDING_REMEMBER_KEY)
            .is_some()
        {
            session.dirty = true;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot_with(session: SessionData) -> Arc<Mutex<Option<SessionData>>> {
        Arc::new(Mutex::new(Some(session)))
    }

    #[tokio::test]
    async fn invalidate_session_rotates_id_and_flushes() {
        let original_id = "a".repeat(40);
        let original_csrf = "b".repeat(40);
        let mut session = SessionData::new(original_id.clone(), original_csrf.clone());
        session.user_id = Some("7".to_string());
        session.put("color", "blue");

        let slot = slot_with(session);
        SESSION_CONTEXT
            .scope(slot.clone(), async {
                invalidate_session();
            })
            .await;

        let after = slot.lock().unwrap().take().expect("session present");

        // Session-fixation guard: the id must rotate, matching Laravel's
        // `invalidate()` (flush + regenerate).
        assert_ne!(
            after.id, original_id,
            "invalidate_session must rotate the session id"
        );
        assert!(
            crate::session::store::is_valid_session_id(&after.id),
            "rotated id must match the generated session-id shape"
        );

        // Data is flushed and the CSRF token rotated.
        assert!(after.user_id.is_none());
        assert_eq!(after.get::<String>("color"), None);
        assert_ne!(after.csrf_token, original_csrf);
        assert!(after.is_dirty());
    }

    #[tokio::test]
    async fn invalidate_session_is_noop_without_scope() {
        // Outside a request scope there is nothing to invalidate; the
        // helper must not panic.
        invalidate_session();
    }
}
