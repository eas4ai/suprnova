//! Rate limiting — two complementary surfaces.
//!
//! ## Sliding-window driver SPI
//!
//! [`RateLimiterDriver`] is the storage SPI for a sliding-window
//! algorithm: each key tracks a deque of hit timestamps. On every
//! `try_acquire`, evict entries older than `now - window`, then if the
//! remaining count is below `max_requests`, append `now` and accept;
//! otherwise reject.
//!
//! The in-memory driver uses `tokio::time::Instant` so `start_paused`
//! tests can use `tokio::time::advance` to drive the clock. The Redis
//! driver uses `chrono::Utc::now().timestamp_millis()` with a Lua
//! script for atomic check-and-record. [`RateLimitMiddleware`] is the
//! HTTP wrapper around the driver and is what most application code
//! reaches for to throttle a route.
//!
//! ## Laravel-shape facade
//!
//! [`RateLimiter`] (the struct, not the driver trait) mirrors
//! `Illuminate\Cache\RateLimiter` — a Cache-backed fixed-window counter
//! API. Use it for the `Cache::add(timer)` + `Cache::increment(counter)`
//! workflow when you want named limiters, `attempt()` callbacks, and
//! `X-RateLimit-*` response headers. [`ThrottleRequestsMiddleware`] is
//! the HTTP wrapper for named limiters and is the closest analogue of
//! Laravel's `throttle:api` route middleware.
//!
//! The two surfaces coexist deliberately: the driver SPI is what
//! Suprnova natively shipped and is the right shape for "one slot per
//! request" sliding-window enforcement against arbitrary storage; the
//! Cache-backed facade is what Laravel apps expect and what the named
//! limiter / response-callback pattern needs.

pub mod algorithm;
pub mod laravel;
pub mod limit;
pub mod memory;
pub mod redis;
pub mod throttle;

pub use laravel::{NamedLimiterRegistry, RateLimiter};
pub use limit::{GlobalLimit, Limit, LimitResult, Unlimited};
pub use throttle::ThrottleRequestsMiddleware;

use crate::error::FrameworkError;
use async_trait::async_trait;
use std::time::Duration;

/// Configuration for the sliding-window rate-limit algorithm.
#[derive(Debug, Clone)]
pub struct SlidingWindowConfig {
    /// Maximum number of requests allowed within the window.
    pub max_requests: u32,
    /// Length of the sliding window.
    pub window: Duration,
}

/// Storage SPI for the sliding-window rate-limiter algorithm.
///
/// Suprnova's native surface, separate from the Laravel-shape
/// [`RateLimiter`] facade (Cache-backed fixed-window counter).
/// Implementations: [`memory::InMemoryRateLimiter`] and
/// [`redis::RedisRateLimiter`]. [`RateLimitMiddleware`] is the HTTP
/// wrapper that drives this trait.
#[async_trait]
pub trait RateLimiterDriver: Send + Sync {
    /// Try to acquire one slot for `key` under `config`. Returns `Ok(true)`
    /// if accepted (slot consumed); `Ok(false)` if rejected.
    async fn try_acquire(
        &self,
        key: &str,
        config: &SlidingWindowConfig,
    ) -> Result<bool, FrameworkError>;

    /// Compute how long to wait before another `try_acquire` is likely to succeed.
    /// Returns `None` if the bucket has free slots right now.
    async fn retry_after(
        &self,
        key: &str,
        config: &SlidingWindowConfig,
    ) -> Result<Option<Duration>, FrameworkError>;
}

// ============================================================================
// Middleware integration
// ============================================================================

use crate::container::App;

/// Default sweep interval for the in-memory bucket map. The map drops
/// any bucket whose last hit aged out past
/// [`DEFAULT_INACTIVITY_WINDOW`], so a request burst followed by
/// silence frees the map within one sweep cycle.
const DEFAULT_SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// Default inactivity window for the in-memory bucket map. Buckets
/// whose most-recent hit is this old or older are dropped on the next
/// sweep. Set to 15 minutes to comfortably outlive every Laravel
/// default window (1-minute / 5-minute throttles) while still
/// reclaiming attacker-spammed keys within a short cycle.
const DEFAULT_INACTIVITY_WINDOW: Duration = Duration::from_secs(900);

/// Wire the in-memory rate limiter as the default. Idempotent.
///
/// The driver is registered with a periodic sweep task — the bucket
/// map drops any bucket whose last hit aged out past 15 minutes,
/// preventing unbounded growth when keying by an attacker-controlled
/// signature. The sweep self-terminates when the driver `Arc` count
/// drops to zero (see [`memory::InMemoryRateLimiter::with_periodic_sweep`]).
pub async fn bootstrap_default() {
    if App::has_binding::<dyn RateLimiterDriver>() {
        return;
    }
    let driver = memory::InMemoryRateLimiter::with_periodic_sweep(
        DEFAULT_SWEEP_INTERVAL,
        DEFAULT_INACTIVITY_WINDOW,
    );
    App::bind::<dyn RateLimiterDriver>(driver);
}

/// Operator opt-in that lets a production deployment boot on the
/// in-memory rate limiter. Truthiness rules match
/// `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION` — deliberately, so an
/// operator learns one escape-hatch pattern rather than three.
///
/// The legitimate use is a genuinely single-process deployment, where a
/// per-process quota *is* the global quota.
const ALLOW_MEMORY_LIMITER_ENV: &str = "RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION";

/// Which backend `RATE_LIMIT_DRIVER` selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RateLimitDriverKind {
    /// Per-process buckets in a `HashMap`.
    Memory,
    /// Buckets in Redis, shared by every process that points at it.
    Redis,
}

impl RateLimitDriverKind {
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "memory" => Some(Self::Memory),
            "redis" => Some(Self::Redis),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Redis => "redis",
        }
    }

    /// Whether a quota configured here means the same thing to every
    /// replica.
    fn is_shared(self) -> bool {
        matches!(self, Self::Redis)
    }
}

/// The outcome of resolving `RATE_LIMIT_DRIVER`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LimiterSelection {
    driver: RateLimitDriverKind,
    /// `Some(raw)` when the value named a driver this build does not know
    /// and selection fell back to memory. Carried out so the warning can
    /// quote the operator's literal value — usually the typo itself.
    unknown_value: Option<String>,
}

/// Decide which limiter backend to bind, refusing in production the
/// choices that silently stop limiting anything meaningful.
///
/// Explicit arguments rather than reading env directly, for the same
/// reason [`crate::mail::boot`]'s `select_driver` does: this crate's
/// tests run massively parallel in one binary, where an env write races
/// every other test in flight.
///
/// **Why the in-memory driver is a production hazard.** Its buckets live
/// in one process's heap. Behind N replicas each keeps its own count, so
/// a "5 attempts per 15 minutes" password-reset throttle is really 5N,
/// and every deploy resets all of them to zero. The limit an operator
/// configured is not the limit they get, and nothing says so — the
/// requests succeed, which is exactly what a working throttle looks like
/// from the outside. That is worth failing a boot over on the same
/// reasoning as SEC-03: a security control that silently does much less
/// than it claims is worse than one that is visibly absent.
///
/// An unrecognised value collapses into the same failure, because it
/// falls back to memory. `RATE_LIMIT_DRIVER=Redis` — capitalised — would
/// otherwise warn once at boot and quietly leave a multi-replica
/// deployment throttling per-process.
fn select_limiter_driver(
    raw: Option<&str>,
    is_production: bool,
    allow_memory: bool,
) -> Result<LimiterSelection, FrameworkError> {
    let selection = match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(value) => match RateLimitDriverKind::parse(value) {
            Some(driver) => LimiterSelection {
                driver,
                unknown_value: None,
            },
            None => LimiterSelection {
                driver: RateLimitDriverKind::Memory,
                unknown_value: Some(value.to_string()),
            },
        },
        None => LimiterSelection {
            driver: RateLimitDriverKind::Memory,
            unknown_value: None,
        },
    };

    if is_production && !allow_memory && !selection.driver.is_shared() {
        let cause = match (&selection.unknown_value, raw) {
            (Some(bad), _) => format!(
                "RATE_LIMIT_DRIVER=`{bad}` is not a driver this build knows, so it \
                 would fall back to the in-memory limiter"
            ),
            (None, Some(_)) => {
                "RATE_LIMIT_DRIVER=`memory` keeps its buckets in this process's heap".to_string()
            }
            (None, None) => {
                "RATE_LIMIT_DRIVER is unset, which defaults to the in-memory limiter".to_string()
            }
        };
        return Err(FrameworkError::internal(format!(
            "refusing to boot in production: {cause}. Per-process buckets mean \
             every configured quota is multiplied by your replica count and reset \
             by every deploy, so login and password-reset throttles do not limit \
             what they claim to. Set RATE_LIMIT_DRIVER=redis with \
             RATE_LIMIT_REDIS_URL, or set {ALLOW_MEMORY_LIMITER_ENV}=true to \
             acknowledge per-process limits — which is only accurate if you run \
             exactly one process."
        )));
    }

    Ok(selection)
}

/// Read `RATE_LIMIT_DRIVER` env and configure the matching driver.
///
/// Outside production, an unset or unrecognised value falls back to the
/// in-memory limiter with a warning. In production both cases are a hard
/// boot failure unless the operator opts in — see
/// `select_limiter_driver` in this module. (Deliberately not an
/// intra-doc link: the target is private, and this doc is public.)
pub async fn bootstrap_from_env() -> Result<(), FrameworkError> {
    let raw = std::env::var("RATE_LIMIT_DRIVER").ok();
    let selection = select_limiter_driver(
        raw.as_deref(),
        crate::config::Environment::detect().is_production(),
        crate::config::env::env_flag_enabled(ALLOW_MEMORY_LIMITER_ENV),
    )?;

    if let Some(bad) = &selection.unknown_value {
        tracing::warn!(
            driver = %bad,
            "unknown RATE_LIMIT_DRIVER, falling back to memory"
        );
    }

    // Worth one line at boot: "which limiter is actually active" is the
    // first question during any throttling incident, and the answer is
    // otherwise invisible.
    tracing::debug!(
        driver = selection.driver.as_str(),
        shared = selection.driver.is_shared(),
        "rate limiter driver selected"
    );

    match selection.driver {
        RateLimitDriverKind::Memory => bootstrap_default().await,
        RateLimitDriverKind::Redis => {
            let url = std::env::var("RATE_LIMIT_REDIS_URL")
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
            let prefix = std::env::var("RATE_LIMIT_PREFIX").unwrap_or_else(|_| "suprnova:".into());
            let d = redis::RedisRateLimiter::connect(&url, &prefix).await?;
            App::bind::<dyn RateLimiterDriver>(std::sync::Arc::new(d));
        }
    }
    Ok(())
}

use crate::Request;
use crate::http::{HttpResponse, Response};
use std::sync::Arc;

/// Build a rate-limit key that identifies the **account being acted on**,
/// not the caller acting on it.
///
/// # Why per-IP throttling is not enough
///
/// Address-keyed limits answer "is one client making too many requests".
/// They do not answer "is one mailbox being flooded". An attacker with a
/// botnet, a rotating proxy pool, or simply an IPv6 /64 stays under every
/// per-IP budget while sending a victim thousands of password-reset
/// emails — the victim's inbox is the resource being exhausted, and the
/// victim's address is the only thing the requests have in common. The
/// reverse is also true: behind carrier-grade NAT or an office gateway,
/// per-IP limits punish a crowd for one member's behaviour.
///
/// Stack this alongside a per-IP limiter rather than replacing it. Each
/// catches what the other cannot: per-IP stops one host enumerating many
/// addresses, per-identity stops many hosts targeting one address.
///
/// # Where the identity is read from
///
/// `field` is looked up in the query string first, then in a buffered
/// form body — so one key function serves `POST /resend?email=…` and a
/// form-encoded `POST /password/request` alike. Reading the body
/// requires
/// [`RateLimitMiddleware::key_reads_body`]; without it the body half is
/// simply skipped.
///
/// # Normalisation and hashing
///
/// The value is trimmed and lowercased, because `Alice@Example.com` and
/// `alice@example.com` reach the same mailbox and must therefore share a
/// bucket — otherwise the limit is bypassed by changing capitalisation.
///
/// It is then hashed. The key is an opaque bucket identifier, and a
/// rate-limit backend is frequently a shared Redis with weaker access
/// control than the primary database; storing raw addresses there would
/// turn a key dump into a list of who is resetting their password, and
/// would let key length grow with attacker-controlled input.
///
/// # Fallback
///
/// A request with no such field falls back to the caller's IP, never to
/// one shared constant — a single `no-identity` bucket would let one
/// caller exhaust it and lock out everyone else's fieldless requests.
///
/// That fallback key is deliberately *not* spelled the way a plain
/// per-IP key would be. This limiter is meant to be stacked alongside an
/// address-keyed one, and the two carry different windows and quotas; if
/// both produced `{prefix}:ip:{addr}` they would share one bucket in the
/// backend and each would be evaluated under the other's config. The
/// suffix keeps them apart.
///
/// ```rust,no_run
/// # use std::sync::Arc;
/// # use std::time::Duration;
/// # use suprnova::rate_limit::{RateLimitMiddleware, SlidingWindowConfig, identity_key};
/// # use suprnova::rate_limit::memory::InMemoryRateLimiter;
/// # fn ex() {
/// let mw = RateLimitMiddleware::new(
///     Arc::new(InMemoryRateLimiter::new()),
///     SlidingWindowConfig { max_requests: 3, window: Duration::from_secs(900) },
///     |req| identity_key(req, "email", "auth-issuance"),
/// )
/// .key_reads_body(4096);
/// # }
/// ```
pub fn identity_key(request: &Request, field: &str, prefix: &str) -> String {
    match read_identity(request, field) {
        Some(value) if !value.trim().is_empty() => {
            let normalised = value.trim().to_lowercase();
            format!("{prefix}:{field}:{}", hashed_identity(&normalised))
        }
        // No identity to key on — fall back to the caller, so the request
        // is still throttled by *something*. The `-absent` marker keeps
        // this out of any co-mounted per-IP limiter's bucket.
        _ => format!(
            "{prefix}:{field}-absent:ip:{}",
            request.ip().unwrap_or_else(|| "anon".into())
        ),
    }
}

/// Does this request name an identity to key on?
///
/// Pair with [`RateLimitMiddleware::only_when`] so a per-identity limiter
/// stands aside for requests that name nobody, rather than imposing its
/// quota on them through [`identity_key`]'s fallback:
///
/// ```rust,no_run
/// # use std::sync::Arc;
/// # use std::time::Duration;
/// # use suprnova::rate_limit::{RateLimitMiddleware, SlidingWindowConfig, identity_key, names_identity};
/// # use suprnova::rate_limit::memory::InMemoryRateLimiter;
/// # fn ex() {
/// let mw = RateLimitMiddleware::new(
///     Arc::new(InMemoryRateLimiter::new()),
///     SlidingWindowConfig { max_requests: 3, window: Duration::from_secs(900) },
///     |req| identity_key(req, "email", "auth-issuance"),
/// )
/// .key_reads_body(4096)
/// .only_when(|req| names_identity(req, "email"));
/// # }
/// ```
///
/// Shares [`identity_key`]'s lookup exactly, so the two cannot disagree
/// about whether a field is present.
pub fn names_identity(request: &Request, field: &str) -> bool {
    read_identity(request, field).is_some()
}

/// Query string first, then a buffered form body. `None` for absent or
/// blank — a blank value is not an identity, and treating it as one
/// would hand every caller who sends `field=` the same free bucket.
fn read_identity(request: &Request, field: &str) -> Option<String> {
    request
        .query_param(field)
        .or_else(|| request.cached_form_field(field))
        .filter(|value| !value.trim().is_empty())
}

/// Hex of the first 16 bytes of SHA-256. Not a secret — a stable,
/// bounded-length, non-reversible bucket label. 128 bits is far past
/// where collisions matter for a throttle bucket, and the truncation
/// keeps Redis keys short.
fn hashed_identity(normalised: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(normalised.as_bytes());
    digest[..16].iter().fold(String::new(), |mut acc, byte| {
        use std::fmt::Write as _;
        let _ = write!(acc, "{byte:02x}");
        acc
    })
}

/// How [`RateLimitMiddleware`] reacts when the rate-limiter *backend* itself
/// errors — e.g. Redis is unreachable — as opposed to a request legitimately
/// exceeding its quota.
///
/// This is distinct from the over-quota path (always HTTP 429). A backend
/// error means the limiter could not make a decision at all, so the
/// middleware must choose between availability and the limit's guarantee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendErrorPolicy {
    /// Pass the request through when the backend errors. Prioritizes
    /// availability: a limiter outage does not take down the API. This is
    /// the default, matching most public-API expectations. The error is
    /// logged at `warn` so the outage is still visible.
    #[default]
    FailOpen,
    /// Reject the request with HTTP 503 (`Retry-After: 1`) when the backend
    /// errors. Prioritizes the limit's guarantee: for sensitive routes
    /// (login, password reset, payments) letting unbounded traffic through
    /// during a limiter outage is worse than briefly returning 503. The
    /// error is logged at `error`.
    FailClosed,
}

/// HTTP middleware that enforces a sliding-window rate limit.
///
/// The bucket key is determined by a caller-supplied closure, making it
/// trivial to rate-limit per-route, per-IP, per-user, or any composite.
///
/// On rejection (the caller is over quota) the middleware short-circuits with
/// HTTP 429 and a `Retry-After` header (seconds until the oldest slot
/// expires).
///
/// When the *backend* errors (e.g. Redis is unreachable) the response is
/// governed by [`BackendErrorPolicy`], chosen via
/// [`RateLimitMiddleware::on_backend_error`]. The default is
/// [`BackendErrorPolicy::FailOpen`] (pass through, log a warning); sensitive
/// routes can opt into [`BackendErrorPolicy::FailClosed`] (HTTP 503).
///
/// # Example
///
/// ```rust,no_run
/// use std::sync::Arc;
/// use std::time::Duration;
/// use suprnova::rate_limit::{BackendErrorPolicy, RateLimitMiddleware, SlidingWindowConfig};
/// use suprnova::rate_limit::memory::InMemoryRateLimiter;
/// # fn ex() {
/// let limiter = Arc::new(InMemoryRateLimiter::new());
/// let cfg = SlidingWindowConfig { max_requests: 100, window: Duration::from_secs(60) };
/// let mw = RateLimitMiddleware::new(limiter, cfg, |req| {
///     format!("route:{}", req.path())
/// })
/// // Opt sensitive routes into fail-closed (HTTP 503 if the backend is down):
/// .on_backend_error(BackendErrorPolicy::FailClosed);
/// # }
/// ```
pub struct RateLimitMiddleware<F>
where
    F: Fn(&Request) -> String + Send + Sync + 'static,
{
    limiter: Arc<dyn RateLimiterDriver>,
    config: SlidingWindowConfig,
    key_fn: F,
    on_backend_error: BackendErrorPolicy,
    /// Buffer the body to this cap before keying, so `key_fn` can read it
    /// via [`Request::cached_form_field`](crate::Request::cached_form_field).
    /// `None` (the default) leaves the body streaming and untouched.
    key_body_cap: Option<usize>,
    /// When present and false for a request, the limiter is skipped
    /// entirely. See [`RateLimitMiddleware::only_when`].
    #[allow(clippy::type_complexity)]
    applies: Option<Box<dyn Fn(&Request) -> bool + Send + Sync>>,
}

impl<F> RateLimitMiddleware<F>
where
    F: Fn(&Request) -> String + Send + Sync + 'static,
{
    /// Create a new `RateLimitMiddleware`.
    ///
    /// * `limiter` — the rate-limiter backend (in-memory or Redis)
    /// * `config`  — window duration and per-key request cap
    /// * `key_fn`  — closure that maps each incoming request to a bucket key string
    pub fn new(
        limiter: Arc<dyn RateLimiterDriver>,
        config: SlidingWindowConfig,
        key_fn: F,
    ) -> Self {
        Self {
            limiter,
            config,
            key_fn,
            on_backend_error: BackendErrorPolicy::default(),
            key_body_cap: None,
            applies: None,
        }
    }

    /// Apply this limiter only to requests the predicate accepts; skip it
    /// entirely for the rest.
    ///
    /// This exists for limiters that are *stacked* on a broader one. A
    /// per-recipient limit, for instance, has nothing to say about a
    /// request that names no recipient — and if it falls back to keying
    /// those by address, its own (usually tighter) quota silently becomes
    /// the binding limit for every such route, overriding the per-IP
    /// budget that was chosen for them. Skipping is the honest answer:
    /// the broader limiter is still mounted and still counts the request.
    ///
    /// The predicate runs *after* any [`key_reads_body`](Self::key_reads_body)
    /// buffering, so it can inspect form fields.
    ///
    /// Skipping is not a way out of being limited — it only makes sense
    /// when another limiter covers the same route. Do not use it as the
    /// sole limiter on a path.
    pub fn only_when<P>(mut self, predicate: P) -> Self
    where
        P: Fn(&Request) -> bool + Send + Sync + 'static,
    {
        self.applies = Some(Box::new(predicate));
        self
    }

    /// Buffer the request body before computing the key, so `key_fn` can
    /// read form fields out of it with
    /// [`Request::cached_form_field`](crate::Request::cached_form_field).
    ///
    /// Off by default, and deliberately opt-in: buffering pulls the whole
    /// body into memory before any quota has been checked, which is work
    /// an unauthenticated caller gets to make you do. `max_bytes` bounds
    /// that.
    ///
    /// **A body over `max_bytes` is rejected with 413**, before the
    /// handler sees it. The alternative — pass it through unkeyed — would
    /// let a caller opt out of per-identity throttling by padding the
    /// body, which defeats the point. So only enable this on routes whose
    /// bodies are genuinely small (login and reset forms are a few hundred
    /// bytes), and size `max_bytes` above the largest legitimate one.
    ///
    /// The handler still reads the same bytes — [`Request::body_bytes`]
    /// returns the buffered copy — so this is otherwise invisible
    /// downstream.
    ///
    /// Not needed for keys built from the path, headers, or query string;
    /// those are already readable without touching the body.
    pub fn key_reads_body(mut self, max_bytes: usize) -> Self {
        self.key_body_cap = Some(max_bytes);
        self
    }

    /// Choose how the middleware reacts to a rate-limiter *backend* error
    /// (e.g. Redis is unreachable), as distinct from a request being over its
    /// quota. Defaults to [`BackendErrorPolicy::FailOpen`].
    ///
    /// Use [`BackendErrorPolicy::FailClosed`] on sensitive routes where letting
    /// unbounded traffic through during a limiter outage is unacceptable.
    pub fn on_backend_error(mut self, policy: BackendErrorPolicy) -> Self {
        self.on_backend_error = policy;
        self
    }
}

#[async_trait]
impl<F> crate::Middleware for RateLimitMiddleware<F>
where
    F: Fn(&Request) -> String + Send + Sync + 'static,
{
    async fn handle(&self, request: Request, next: crate::Next) -> Response {
        // Buffer first when the key comes out of the body. `buffer_body`
        // consumes the request, so an over-cap body cannot be handed
        // onward — see `key_reads_body` for why 413 is the right answer
        // rather than passing it through unkeyed.
        let request = match self.key_body_cap {
            None => request,
            Some(cap) => match request.buffer_body(cap).await {
                Ok(buffered) => buffered,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "rate limiter could not buffer the body to build its key; rejecting"
                    );
                    return Err(HttpResponse::text("413 Payload Too Large").status(413));
                }
            },
        };

        if let Some(applies) = &self.applies
            && !applies(&request)
        {
            return next(request).await;
        }

        let key = (self.key_fn)(&request);
        match self.limiter.try_acquire(&key, &self.config).await {
            Ok(true) => next(request).await,
            Ok(false) => {
                // Compute how long the caller must wait before trying again.
                let secs = self
                    .limiter
                    .retry_after(&key, &self.config)
                    .await
                    .ok()
                    .flatten()
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                Err(HttpResponse::text("429 Too Many Requests")
                    .status(429)
                    .header("retry-after", secs.to_string()))
            }
            // The limiter backend itself errored (e.g. Redis unreachable) —
            // it could not make a decision. Behavior is governed by the
            // configured `BackendErrorPolicy`. Either way the error is now
            // logged (it was previously swallowed silently): `warn` when
            // failing open since it self-limits to backend outages, `error`
            // when failing closed since that path actively rejects live
            // traffic.
            Err(e) => match self.on_backend_error {
                BackendErrorPolicy::FailOpen => {
                    tracing::warn!(
                        error = %e,
                        key = %key,
                        "rate limiter backend error; failing open (request passed through)"
                    );
                    next(request).await
                }
                BackendErrorPolicy::FailClosed => {
                    tracing::error!(
                        error = %e,
                        key = %key,
                        "rate limiter backend error; failing closed with 503"
                    );
                    Err(HttpResponse::text("503 Service Unavailable")
                        .status(503)
                        .header("retry-after", "1"))
                }
            },
        }
    }
}

#[cfg(test)]
mod driver_selection_tests {
    //! P2-02 — the in-memory limiter as a production default.
    //!
    //! Same discipline as the SEC-03 matrix in `mail::boot`: every case
    //! drives [`select_limiter_driver`] with explicit arguments, so none
    //! of these touch process env and they are safe in the
    //! massively-parallel lib test binary.

    use super::*;

    fn choose(
        raw: Option<&str>,
        is_production: bool,
        allow_memory: bool,
    ) -> Result<LimiterSelection, FrameworkError> {
        select_limiter_driver(raw, is_production, allow_memory)
    }

    fn expect_ok(raw: Option<&str>, is_production: bool, allow_memory: bool) -> LimiterSelection {
        choose(raw, is_production, allow_memory)
            .unwrap_or_else(|e| panic!("expected a driver selection for {raw:?}, got: {e}"))
    }

    #[test]
    fn outside_production_everything_still_resolves() {
        assert_eq!(
            expect_ok(None, false, false).driver,
            RateLimitDriverKind::Memory,
            "unset still defaults to memory in development"
        );
        assert_eq!(
            expect_ok(Some("memory"), false, false).driver,
            RateLimitDriverKind::Memory
        );
        assert_eq!(
            expect_ok(Some("redis"), false, false).driver,
            RateLimitDriverKind::Redis
        );
    }

    #[test]
    fn outside_production_an_unknown_driver_falls_back_and_reports_the_value() {
        let s = expect_ok(Some("Redis"), false, false);
        assert_eq!(s.driver, RateLimitDriverKind::Memory);
        assert_eq!(
            s.unknown_value.as_deref(),
            Some("Redis"),
            "the raw value is carried out so the warning can quote the typo"
        );
    }

    /// The finding. A per-process limiter behind N replicas is an N×
    /// quota that resets on every deploy, and nothing about the running
    /// system says so — the requests succeed, which is what a working
    /// throttle looks like from outside.
    #[test]
    fn production_refuses_the_in_memory_limiter() {
        for raw in [None, Some("memory")] {
            let result = choose(raw, true, false);
            assert!(
                result.is_err(),
                "RATE_LIMIT_DRIVER={raw:?} in production must refuse to boot, \
                 but resolved to {result:?}"
            );
        }
    }

    /// A capitalised or misspelled driver name silently *became* the
    /// memory limiter, so it has to fail for the same reason. This is the
    /// case most likely to reach production, because it looks configured.
    #[test]
    fn production_refuses_an_unknown_driver_rather_than_falling_back() {
        for raw in ["Redis", "REDIS", "redis ", "rediss", "in-memory", "none"] {
            // `"redis "` is trimmed and IS valid — assert the rest fail.
            let result = choose(Some(raw), true, false);
            if raw.trim() == "redis" {
                assert!(result.is_ok(), "{raw:?} trims to a real driver");
                continue;
            }
            assert!(
                result.is_err(),
                "RATE_LIMIT_DRIVER={raw:?} falls back to memory, so production \
                 must refuse it rather than warn once and carry on"
            );
        }
    }

    #[test]
    fn the_production_refusal_names_the_override_and_the_alternative() {
        let err = choose(None, true, false).expect_err("unset in production must refuse");
        let msg = format!("{err}");

        assert!(
            msg.contains(ALLOW_MEMORY_LIMITER_ENV),
            "the refusal must name the override that unblocks it: {msg}"
        );
        assert!(
            msg.contains("RATE_LIMIT_DRIVER=redis"),
            "and the fix that is actually correct: {msg}"
        );
        assert!(
            msg.contains("replica"),
            "and say why per-process buckets are the problem: {msg}"
        );
    }

    #[test]
    fn the_override_permits_a_single_process_deployment() {
        let s = expect_ok(None, true, true);
        assert_eq!(
            s.driver,
            RateLimitDriverKind::Memory,
            "the override exists precisely for a one-process deployment"
        );
    }

    #[test]
    fn production_accepts_redis_without_any_override() {
        let s = expect_ok(Some("redis"), true, false);
        assert_eq!(s.driver, RateLimitDriverKind::Redis);
        assert!(
            s.driver.is_shared(),
            "redis is the shared-quota driver — that is the whole reason it \
             passes the guard"
        );
    }

    #[test]
    fn a_blank_value_is_treated_as_unset() {
        assert_eq!(
            expect_ok(Some("   "), false, false).driver,
            RateLimitDriverKind::Memory
        );
        assert!(
            choose(Some(""), true, false).is_err(),
            "blank is unset, and unset in production is refused"
        );
    }
}
