//! [`ThrottleRequestsMiddleware`] - HTTP wrapper around the
//! Cache-backed [`RateLimiter`] facade. Mirrors
//! `Illuminate\Routing\Middleware\ThrottleRequests`.
//!
//! Construct one of four ways:
//!
//! - [`ThrottleRequestsMiddleware::default`] - 60 requests a minute for
//!   each signed-in user, and for each client IP when nobody is signed in.
//!   The limit of the plain `throttle` alias.
//! - [`ThrottleRequestsMiddleware::by_name`] - resolve a named limiter
//!   registered via [`RateLimiter::define`]. The named callback receives
//!   the `&Request` and returns a [`LimitResult`] (single limit, list of
//!   limits, or a short-circuit response).
//! - [`ThrottleRequestsMiddleware::with`] - provide a max-attempts /
//!   decay-minutes / prefix tuple directly. The literal Laravel
//!   `throttle:60,1` shape.
//! - [`ThrottleRequestsMiddleware::with_limits`] - build the
//!   [`Limit`]s in Rust and pass them through. Useful when the limits
//!   are computed at boot time and don't need to be named.
//!
//! Every wrapped response carries one `X-RateLimit-Limit` and
//! `X-RateLimit-Remaining` pair, the one of the limit with the fewest
//! attempts left, unless the handler's response already carries an equal
//! or lower `X-RateLimit-Remaining`. 429 responses additionally carry
//! `Retry-After` and `X-RateLimit-Reset`. This matches Laravel's
//! `ThrottleRequests::getHeaders($maxAttempts, $remainingAttempts,
//! $retryAfter, $response)` shape.
//!
//! Each limit counts under the key it carries, prefixed with the limiter's
//! name for a named limiter, so a direct caller of
//! [`RateLimiter::attempts`] reads the bucket the middleware counts in.
//! Limits whose keys clean alike share that bucket, and a request counts
//! in it once for each of them, as Laravel's `ThrottleRequests` hits a key
//! once for each limit.
//!
//! The middleware checks every limit before it counts a request, as
//! Laravel does, so a request refused there leaves every count where it
//! was. The admission itself stays on the atomic post-increment count (see
//! [`RateLimiter::hit_and_check`]), and the middleware never takes a count
//! back, so no window admits more requests than its limit (see
//! `count_request` for why there is no give-back).

use async_trait::async_trait;

use crate::Middleware;
use crate::Next;
use crate::Request;
use crate::http::{HttpResponse, Response};

use super::laravel::{NamedLimiterFn, RateLimiter, give_shared_keys_fallback_keys};
use super::limit::{Limit, LimitResult};

use std::sync::Arc;

/// HTTP throttling middleware backed by the Cache-shape
/// [`RateLimiter`].
///
/// This is the Laravel-shape companion to
/// [`RateLimitMiddleware`](super::RateLimitMiddleware) (which wraps the
/// sliding-window [`RateLimiterDriver`](super::RateLimiterDriver) SPI).
/// Use [`ThrottleRequestsMiddleware`] when you want named limiters,
/// `X-RateLimit-*` headers, or `Limit::response(...)` short-circuits;
/// use the driver middleware when you want exact sliding-window
/// semantics against a non-Cache backend.
pub struct ThrottleRequestsMiddleware {
    mode: Mode,
    prefix: String,
}

enum Mode {
    Named(String),
    /// The `throttle:60,1` shape: one bucket for each signed-in user and
    /// one for each client IP, each per path. `throttle:<guest>|<user>,1`
    /// gives the two their own limits; [`ThrottleRequestsMiddleware::with`]
    /// gives both the same.
    Inline {
        guest_max_attempts: i64,
        user_max_attempts: i64,
        decay_seconds: u64,
    },
    Limits(Vec<Limit>),
    /// The [`Default`] limit: one bucket for each signed-in user, and one
    /// for each client IP when nobody is signed in.
    PerUserOrIp {
        max_attempts: i64,
        decay_seconds: u64,
    },
}

impl ThrottleRequestsMiddleware {
    /// Requests the [`Default`] limit allows in one
    /// [`DEFAULT_DECAY_SECONDS`](Self::DEFAULT_DECAY_SECONDS) window.
    pub const DEFAULT_MAX_ATTEMPTS: i64 = 60;

    /// Length of the [`Default`] limit's window, in seconds.
    pub const DEFAULT_DECAY_SECONDS: u64 = 60;
}

impl Default for ThrottleRequestsMiddleware {
    /// The limit of the plain `throttle` alias: 60 requests a minute
    /// ([`DEFAULT_MAX_ATTEMPTS`](Self::DEFAULT_MAX_ATTEMPTS) in
    /// [`DEFAULT_DECAY_SECONDS`](Self::DEFAULT_DECAY_SECONDS)), counted for
    /// each signed-in user, and for each client IP when nobody is signed
    /// in. It is the shape of Laravel's default `api` limiter,
    /// `Limit::perMinute(60)->by($request->user()?->id ?: $request->ip())`.
    ///
    /// ```rust,no_run
    /// use suprnova::middleware::register_middleware_alias;
    /// use suprnova::rate_limit::ThrottleRequestsMiddleware;
    ///
    /// register_middleware_alias("throttle", ThrottleRequestsMiddleware::default);
    /// ```
    ///
    /// # What the bucket is
    ///
    /// The user's bucket follows the user across routes and across
    /// addresses, and the address's bucket is shared by every route, as
    /// they are in Laravel. [`ThrottleRequestsMiddleware::with`] differs:
    /// it counts per address and per path. Use [`Self::prefix`] to give a
    /// group of routes a budget of its own.
    ///
    /// The user is the one the default guard signed in, so the session
    /// middleware has to run before this one for a signed-in user to be
    /// counted as a user. The address is [`Request::ip`], which resolves
    /// through the trusted proxies.
    ///
    /// The limit reads who is asking, as `Auth::id()` does. Where the
    /// render cache stores the route, that read counts as a read of the
    /// principal.
    fn default() -> Self {
        Self {
            mode: Mode::PerUserOrIp {
                max_attempts: Self::DEFAULT_MAX_ATTEMPTS,
                decay_seconds: Self::DEFAULT_DECAY_SECONDS,
            },
            prefix: String::new(),
        }
    }
}

impl ThrottleRequestsMiddleware {
    /// Build a throttle middleware that resolves the named limiter at
    /// request time. Mirrors `ThrottleRequests::using('api')` /
    /// `throttle:api`.
    ///
    /// The limiter must have been registered via
    /// [`RateLimiter::define`]; otherwise every request returns
    /// `503 Service Unavailable`, and the log and the response's
    /// [`ErrorReport`](crate::ErrorReport), never the body, name the
    /// missing limiter (matching the `MissingRateLimiterException` that Laravel
    /// throws - Suprnova surfaces it as an HTTP response rather than
    /// panicking the worker thread).
    pub fn by_name(name: impl Into<String>) -> Self {
        Self {
            mode: Mode::Named(name.into()),
            prefix: String::new(),
        }
    }

    /// Build a throttle middleware with literal Laravel-shape
    /// `max,decay,prefix` arguments. Mirrors
    /// `ThrottleRequests::with($maxAttempts, $decayMinutes, $prefix)`.
    ///
    /// The bucket is the one Laravel's `resolveRequestSignature` picks: the
    /// signed-in user, by the identifier [`Auth::id`](crate::Auth::id)
    /// reads, and the client IP ([`Request::ip`]) when nobody is signed in.
    /// The user's bucket follows the user across addresses. Unlike Laravel,
    /// the key also holds the request path, `user:<id>:path:<path>` or
    /// `ip:<address>:path:<path>`, so two routes behind the same limit
    /// count apart.
    ///
    /// The user is the one the default guard signed in, so the session
    /// middleware has to run before this one. Where the render cache stores
    /// the route, the read of the user counts as a read of the principal,
    /// as it does for [`Self::default`].
    pub fn with(max_attempts: i64, decay_minutes: u64, prefix: impl Into<String>) -> Self {
        Self::with_guest_and_user(max_attempts, max_attempts, decay_minutes, prefix)
    }

    /// The `throttle:<guest>|<user>,<minutes>,<prefix>` shape: `guest`
    /// attempts for a request nobody is signed in for, `user` attempts for
    /// a signed-in user, keyed as [`Self::with`] keys them.
    fn with_guest_and_user(
        guest_max_attempts: i64,
        user_max_attempts: i64,
        decay_minutes: u64,
        prefix: impl Into<String>,
    ) -> Self {
        Self {
            mode: Mode::Inline {
                guest_max_attempts,
                user_max_attempts,
                decay_seconds: 60 * decay_minutes,
            },
            prefix: prefix.into(),
        }
    }

    /// Build a throttle middleware from a list of explicit
    /// [`Limit`]s. The first limit to trip wins. This is the most
    /// Rust-idiomatic constructor and doesn't require a named-limiter
    /// registration.
    ///
    /// Limits that share a key count under their
    /// [`fallback_key`](Limit::fallback_key)s, as the limits of a named
    /// limiter do (see [`RateLimiter::limiter`]), so a per-minute and a
    /// per-hour limit on one key keep a count each.
    pub fn with_limits(limits: Vec<Limit>) -> Self {
        Self {
            mode: Mode::Limits(limits),
            prefix: String::new(),
        }
    }

    /// Set a prefix that is prepended to every limit's key. Mirrors
    /// the third positional argument of Laravel's
    /// `ThrottleRequests::with`.
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// Build the middleware from the arguments of a `throttle` alias, the
    /// way Laravel reads `throttle:60,1`. It is the factory to register the
    /// alias with:
    ///
    /// ```rust,no_run
    /// use suprnova::middleware::register_middleware_alias_with_args;
    /// use suprnova::rate_limit::ThrottleRequestsMiddleware;
    ///
    /// register_middleware_alias_with_args("throttle", ThrottleRequestsMiddleware::from_alias_args);
    /// ```
    ///
    /// | A route writes | It gets |
    /// |---|---|
    /// | `throttle` | [`Self::default`] |
    /// | `throttle:60` | 60 requests a minute, [`Self::with`] |
    /// | `throttle:60,5` | 60 requests in 5 minutes |
    /// | `throttle:60,5,uploads` | the same, with the key prefix `uploads` |
    /// | `throttle:10\|60,1` | 10 requests a minute for a guest, 60 for a signed-in user |
    /// | `throttle:api` | the limiter named `api`, [`Self::by_name`] |
    ///
    /// A first argument of two whole numbers joined by `|` is the guest's
    /// limit and the signed-in user's, as Laravel's `resolveMaxAttempts`
    /// reads it. Any other first argument that is no number names a
    /// limiter.
    ///
    /// # Errors
    ///
    /// A number that does not parse where one is expected, a limit or a
    /// window of zero, and more than three arguments.
    pub fn from_alias_args(arguments: &[&str]) -> Result<Self, crate::FrameworkError> {
        let refused = |what: String| crate::FrameworkError::internal(what);
        let Some(first) = arguments.first() else {
            return Ok(Self::default());
        };
        if arguments.len() > 3 {
            return Err(refused(format!(
                "throttle takes a limit, a window in minutes and a key prefix, \
                 and was given {} arguments",
                arguments.len()
            )));
        }
        let (guest_max_attempts, user_max_attempts) = match first.parse::<i64>() {
            Ok(max_attempts) => (max_attempts, max_attempts),
            Err(_) => match guest_and_user_limits(first) {
                Some(limits) => limits,
                // A first argument that is no number names a limiter.
                None if arguments.len() > 1 => {
                    return Err(refused(format!(
                        "throttle:{first} names a limiter, which takes no further arguments"
                    )));
                }
                None => return Ok(Self::by_name(*first)),
            },
        };
        for max_attempts in [guest_max_attempts, user_max_attempts] {
            if max_attempts < 1 {
                return Err(refused(format!(
                    "a throttle limit of {max_attempts} would refuse every request"
                )));
            }
        }
        let decay_minutes = match arguments.get(1) {
            None => 1,
            Some(minutes) => minutes
                .parse::<u64>()
                .ok()
                .filter(|m| *m > 0)
                .ok_or_else(|| {
                    refused(format!(
                        "`{minutes}` is not a throttle window: a whole number of minutes, 1 or more"
                    ))
                })?,
        };
        let prefix = arguments.get(2).copied().unwrap_or_default();
        Ok(Self::with_guest_and_user(
            guest_max_attempts,
            user_max_attempts,
            decay_minutes,
            prefix,
        ))
    }
}

/// Read `<guest>|<user>`, two whole numbers joined by `|`, as Laravel's
/// `resolveMaxAttempts` splits the first `throttle` argument. `None` when
/// the argument is not that shape, so it names a limiter instead.
fn guest_and_user_limits(argument: &str) -> Option<(i64, i64)> {
    let (guest, user) = argument.split_once('|')?;
    Some((guest.parse().ok()?, user.parse().ok()?))
}

#[async_trait]
impl Middleware for ThrottleRequestsMiddleware {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        let limits = match resolve_limits(&self.mode, &request) {
            ResolvedLimits::Ok(limits) => limits,
            ResolvedLimits::ShortCircuit(resp) => return Ok(resp),
            ResolvedLimits::MissingLimiter(name) => {
                // A boot misconfiguration, not a client error - but the body must
                // not name the internal limiter or echo framework API instructions
                // to the public. The operator gets the actionable detail in the log.
                tracing::error!(
                    name = %name,
                    "throttle middleware: named limiter [{name}] not registered - \
                     register it with RateLimiter::define(\"{name}\", |req| ...) at boot",
                );
                // No error value exists here, only the missing name; the
                // report carries it, as the log does. Logged just above, so
                // the report skips its own line (PAR-111).
                return Err(HttpResponse::text("Service Unavailable")
                    .status(503)
                    .with_reported_logged_error_from(&crate::FrameworkError::internal(format!(
                        "throttle middleware: named limiter [{name}] not registered - \
                         register it with RateLimiter::define(\"{name}\", |req| ...) at boot"
                    ))));
            }
        };

        // The keys the limits carry, prefixed, never rewritten: a direct
        // caller reads the buckets the middleware counts in. Computed once so
        // the check, the count, the deferred hits and the headers agree.
        let keys: Vec<Option<String>> = limits
            .iter()
            .map(|limit| prefixed_key(limit, &self.mode, &self.prefix))
            .collect();
        let mut buckets = buckets_of(&limits, &keys);

        // Check every limit before the request is counted, as Laravel's
        // `handleRequest` does. The first limit whose bucket already holds
        // it refuses the request, and nothing has been counted. A limit with
        // an `after` callback is only counted once the response is known, so
        // the check is all it gets here.
        for (index, (limit, key)) in limits.iter().zip(&keys).enumerate() {
            let Some(key) = key else { continue }; // Unlimited never trips.
            let full = if limit.after_callback.is_some() {
                RateLimiter::too_many_attempts(key, limit.max_attempts).await?
            } else {
                holds_limit(&mut buckets, &limits, index).await?
            };
            if full {
                return Err(build_too_many_attempts_response(&request, limit, key).await?);
            }
        }

        if let Some((limit, key)) = count_request(&limits, &buckets).await? {
            return Err(build_too_many_attempts_response(&request, limit, key).await?);
        }

        request
            .record_live_security_check(crate::live::attestation::SecurityCheck::RateLimit, None);
        let response = next(request).await;

        // Apply after-callback gated hits, and add the X-RateLimit headers
        // to the outgoing response.
        match response {
            Ok(r) => Ok(settle(r, &limits, &keys).await?),
            Err(r) => Err(settle(r, &limits, &keys).await?),
        }
    }
}

/// One bucket a request is checked and counted in. Limits whose keys clean
/// to one stored key share it, as the [`RateLimiter`] facade stores them.
struct Bucket<'a> {
    /// The key of the first limit in the bucket, as written; the facade
    /// cleans it, so it names the bucket every other key of it names.
    key: &'a str,
    /// The limits counted when the request is admitted, those without an
    /// `after` callback, by index, in the order they were given.
    counted: Vec<usize>,
    /// The count the bucket held when the request was checked, read once.
    before: Option<i64>,
    /// Whether the bucket's window was open then, read once when a count
    /// reaches a limit.
    open: Option<bool>,
}

/// Group the limits by the bucket they count in: the bucket of each limit,
/// by index, and the buckets in the order of their first limit. An
/// unlimited limit has no key and no bucket.
fn buckets_of<'a>(limits: &[Limit], keys: &'a [Option<String>]) -> Buckets<'a> {
    let mut stored: Vec<String> = Vec::new();
    let mut buckets: Vec<Bucket<'a>> = Vec::new();
    let mut of_limit = Vec::with_capacity(limits.len());
    for (index, (limit, key)) in limits.iter().zip(keys).enumerate() {
        let Some(key) = key else {
            of_limit.push(None);
            continue;
        };
        let cleaned = RateLimiter::clean_rate_limiter_key(key);
        let position = match stored.iter().position(|known| *known == cleaned) {
            Some(position) => position,
            None => {
                stored.push(cleaned);
                buckets.push(Bucket {
                    key,
                    counted: Vec::new(),
                    before: None,
                    open: None,
                });
                buckets.len() - 1
            }
        };
        if limit.after_callback.is_none()
            && let Some(bucket) = buckets.get_mut(position)
        {
            bucket.counted.push(index);
        }
        of_limit.push(Some(position));
    }
    Buckets { buckets, of_limit }
}

/// The buckets of one request, and which bucket each limit counts in.
struct Buckets<'a> {
    buckets: Vec<Bucket<'a>>,
    of_limit: Vec<Option<usize>>,
}

/// Whether the bucket of the limit at `index` already holds that limit in
/// an open window, so the request is refused before it is counted. It is
/// Laravel's `tooManyAttempts` check, without its reset of a count whose
/// window has ended: the count is then left to age out, because a reset
/// here could erase counts a new window has already taken.
async fn holds_limit(
    buckets: &mut Buckets<'_>,
    limits: &[Limit],
    index: usize,
) -> Result<bool, crate::FrameworkError> {
    let (Some(Some(position)), Some(limit)) = (buckets.of_limit.get(index), limits.get(index))
    else {
        return Ok(false);
    };
    let Some(bucket) = buckets.buckets.get_mut(*position) else {
        return Ok(false);
    };
    let before = match bucket.before {
        Some(before) => before,
        None => {
            let before = RateLimiter::attempts(bucket.key).await?;
            bucket.before = Some(before);
            before
        }
    };
    if before < limit.max_attempts {
        return Ok(false);
    }
    match bucket.open {
        Some(open) => Ok(open),
        None => {
            let open = RateLimiter::window_is_open(bucket.key).await?;
            bucket.open = Some(open);
            Ok(open)
        }
    }
}

/// Count the request in each of its buckets, and answer the limit that
/// refuses it and that limit's key, if one does.
///
/// Each bucket takes one atomic increment, by the number of limits counted
/// in it, as Laravel hits a key once for each limit. The request is
/// admitted only when the first of the counts it got in a bucket is within
/// every limit counted there, which is Laravel's check-before-hit decided
/// on the atomic post-increment count.
///
/// # Why a count is never taken back
///
/// A decrement that gives a refused request's count back carries no mark of
/// the window it came from, and the cache store has no step that decrements
/// a count only while a given window lasts. If the window ended between the
/// count and the give-back, the decrement would land in the next window, as
/// `-1` on a new counter or as one count off requests that window already
/// admitted, and that window would admit one request more than its limit.
/// So the middleware takes nothing back, and that is why it cannot
/// over-admit: an increment hands each request in a window its own counts,
/// counts in a window only grow, and a request is admitted only on a count
/// within the limit, so no more requests than the limit get one, however
/// late any step of a refused request runs.
///
/// A request refused at the check was never counted. The one refused
/// request that keeps a count is one that passed the check beside
/// concurrent requests and found the bucket full when it counted. The
/// bucket that refused it already holds its limit, so that count admits or
/// refuses nothing else in the window, and it ends with the window. A
/// bucket of that request counted before the one that refused it keeps a
/// count too, which can refuse a request early but never admits one; the
/// buckets are counted fewest places left first, so the bucket that runs
/// out is usually counted first.
async fn count_request<'b>(
    limits: &'b [Limit],
    buckets: &Buckets<'b>,
) -> Result<Option<(&'b Limit, &'b str)>, crate::FrameworkError> {
    let mut order: Vec<&Bucket<'b>> = buckets
        .buckets
        .iter()
        .filter(|bucket| !bucket.counted.is_empty())
        .collect();
    order.sort_by_key(|bucket| places_left(bucket, limits));
    for bucket in order {
        let Some(decay_seconds) = bucket
            .counted
            .first()
            .and_then(|index| limits.get(*index))
            .map(Limit::decay_seconds)
        else {
            continue;
        };
        let hits = i64::try_from(bucket.counted.len()).unwrap_or(i64::MAX);
        let total = RateLimiter::increment(bucket.key, decay_seconds, hits).await?;
        let first = total.saturating_sub(hits).saturating_add(1);
        let refusing = bucket
            .counted
            .iter()
            .filter_map(|index| limits.get(*index))
            .find(|limit| first > limit.max_attempts);
        if let Some(limit) = refusing {
            return Ok(Some((limit, bucket.key)));
        }
    }
    Ok(None)
}

/// The places a bucket had left at the check, for the tightest limit
/// counted in it. A bucket the check did not read sorts last.
fn places_left(bucket: &Bucket<'_>, limits: &[Limit]) -> i64 {
    let Some(before) = bucket.before else {
        return i64::MAX;
    };
    bucket
        .counted
        .iter()
        .filter_map(|index| limits.get(*index))
        .map(|limit| limit.max_attempts.saturating_sub(before))
        .min()
        .unwrap_or(i64::MAX)
}

/// Count the hits the limits' `after` callbacks ask for, then add one
/// `X-RateLimit-Limit` and `X-RateLimit-Remaining` pair: the one of the
/// limit with the fewest attempts left, the first such limit on a tie.
///
/// Laravel's `getHeaders` writes a limit's pair only when the response
/// carries no `X-RateLimit-Remaining` at or below that limit's remaining
/// count, and its header bag replaces rather than appends. So the lowest
/// count wins, and a handler that set an equal or lower count of its own
/// keeps its headers. A handler's value that is no whole number reads as
/// `0`, as PHP's `(int)` cast reads it, and keeps the handler's headers.
async fn settle(
    response: HttpResponse,
    limits: &[Limit],
    keys: &[Option<String>],
) -> Result<HttpResponse, crate::FrameworkError> {
    let mut lowest: Option<(i64, i64)> = None;
    for (limit, key) in limits.iter().zip(keys) {
        let Some(key) = key else { continue };
        if let Some(after) = &limit.after_callback
            && after(&response)
            && limit.max_attempts != i64::MAX
        {
            RateLimiter::hit(key, limit.decay_seconds()).await?;
        }
        let remaining = RateLimiter::remaining(key, limit.max_attempts).await?;
        if lowest.is_none_or(|(_, fewest)| remaining < fewest) {
            lowest = Some((limit.max_attempts, remaining));
        }
    }
    let Some((max_attempts, remaining)) = lowest else {
        return Ok(response);
    };
    let handler_remaining = response
        .header_value("X-RateLimit-Remaining")
        .map(|value| value.trim().parse::<i64>().unwrap_or(0));
    if handler_remaining.is_some_and(|handler| handler <= remaining) {
        return Ok(response);
    }
    Ok(response
        .replace_header("X-RateLimit-Limit", max_attempts.to_string())
        .replace_header("X-RateLimit-Remaining", remaining.to_string()))
}

enum ResolvedLimits {
    Ok(Vec<Limit>),
    ShortCircuit(HttpResponse),
    MissingLimiter(String),
}

fn resolve_limits(mode: &Mode, request: &Request) -> ResolvedLimits {
    match mode {
        Mode::Named(name) => {
            let Some(callback) = RateLimiter::limiter(name) else {
                return ResolvedLimits::MissingLimiter(name.clone());
            };
            match invoke(&callback, request) {
                LimitResult::Single(l) => ResolvedLimits::Ok(vec![l]),
                LimitResult::Many(v) => ResolvedLimits::Ok(v),
                LimitResult::Response(r) => ResolvedLimits::ShortCircuit(r),
            }
        }
        Mode::Inline {
            guest_max_attempts,
            user_max_attempts,
            decay_seconds,
        } => {
            // Mirrors Laravel's `resolveRequestSignature` (the user, else
            // the IP) and `resolveMaxAttempts` (the guest's or the user's
            // limit), but the path is added so two throttled routes with
            // the same scope don't share a bucket inadvertently. The user
            // is read once, so the limit and the key agree. The
            // middleware's `prefix` is prepended later inside
            // `prefixed_key`; baking it in here would land it twice.
            let (max_attempts, key) = match crate::session::auth_user_id() {
                Some(user) => (
                    *user_max_attempts,
                    format!("user:{user}:path:{}", request.path()),
                ),
                None => (*guest_max_attempts, default_request_key(request)),
            };
            ResolvedLimits::Ok(vec![
                Limit::new(max_attempts, std::time::Duration::from_secs(*decay_seconds)).by(key),
            ])
        }
        Mode::Limits(limits) => {
            let mut limits = limits.clone();
            give_shared_keys_fallback_keys(&mut limits);
            ResolvedLimits::Ok(limits)
        }
        Mode::PerUserOrIp {
            max_attempts,
            decay_seconds,
        } => ResolvedLimits::Ok(vec![
            Limit::new(
                *max_attempts,
                std::time::Duration::from_secs(*decay_seconds),
            )
            .by(user_or_ip_key(request)),
        ]),
    }
}

/// The bucket of the [`Default`] limit: the signed-in user, else the
/// client IP. The two are spelled apart, so a user whose id reads like an
/// address shares no bucket with that address.
fn user_or_ip_key(request: &Request) -> String {
    match crate::session::auth_user_id() {
        Some(user) => format!("user:{user}"),
        // `unknown` is the key of a request with no peer, which is an
        // in-process request. See `default_request_key`.
        None => format!("ip:{}", request.ip().unwrap_or_else(|| "unknown".into())),
    }
}

fn invoke(cb: &Arc<NamedLimiterFn>, request: &Request) -> LimitResult {
    cb(request)
}

/// The raw key a limit counts under: `<prefix>:<name>:<key>`, each part
/// present when set, or `None` for an unlimited limit. It stays raw because
/// the facade cleans each key once (see
/// [`RateLimiter::clean_rate_limiter_key`]), so the middleware and a direct
/// caller of `RateLimiter::attempts("<name>:<key>")` count the same bucket.
fn prefixed_key(limit: &Limit, mode: &Mode, prefix: &str) -> Option<String> {
    if limit.max_attempts == i64::MAX {
        return None;
    }
    let base = if limit.key.is_empty() {
        limit.fallback_key()
    } else {
        limit.key.clone()
    };
    let mut key = String::new();
    if !prefix.is_empty() {
        key.push_str(prefix);
        key.push(':');
    }
    if let Mode::Named(name) = mode {
        key.push_str(name);
        key.push(':');
    }
    key.push_str(&base);
    Some(key)
}

fn default_request_key(request: &Request) -> String {
    // Use `request.ip()` so the resolution goes through the
    // trusted-proxy gating in `Request::ip`: `X-Forwarded-For` /
    // `X-Real-IP` are honoured only when the TCP peer is in the
    // configured allowlist, and otherwise the TCP peer wins. Falls
    // back to the literal `"unknown"` only when no peer was threaded
    // into the request - that path is reserved for in-process tests
    // and the WS upgrade replay; production traffic always has a
    // peer.
    //
    // The middleware's `prefix` is prepended later inside
    // `prefixed_key`; do not bake it in here or it lands twice.
    //
    // # Security note
    //
    // Without a `TrustedProxiesConfig` opt-in, the bucket key is
    // grounded in the TCP peer - there is no XFF spoofing path. With
    // an opt-in, the operator has already attested that the listed
    // proxy hops can be trusted. Either way, the historical "every
    // anonymous caller shares one `anon` bucket" failure mode is
    // gone.
    let ip = request.ip().unwrap_or_else(|| "unknown".into());
    format!("ip:{ip}:path:{}", request.path())
}

async fn build_too_many_attempts_response(
    request: &Request,
    limit: &Limit,
    key: &str,
) -> Result<HttpResponse, HttpResponse> {
    let retry_after = RateLimiter::available_in(key).await.map_err(|e| {
        // The backend error (Redis address, Lua script detail, key names) must
        // not reach the wire - every other 5xx in the framework is sanitised to
        // a fixed body, this direct HttpResponse path was the exception. Detail
        // stays in the structured log.
        tracing::error!(error = %e, "rate limiter backend error computing retry-after");
        HttpResponse::text("Internal Server Error")
            .status(500)
            .with_reported_logged_error_from(&e)
    })?;
    let remaining = 0_i64;
    if let Some(cb) = &limit.response_callback {
        let resp = cb(request);
        let resp = inject_headers(resp, limit.max_attempts, remaining, Some(retry_after));
        return Ok(resp);
    }
    let resp = HttpResponse::text("Too Many Attempts.").status(429);
    Ok(inject_headers(
        resp,
        limit.max_attempts,
        remaining,
        Some(retry_after),
    ))
}

fn inject_headers(
    response: HttpResponse,
    max_attempts: i64,
    remaining: i64,
    retry_after_secs: Option<u64>,
) -> HttpResponse {
    let mut resp = response
        .header("X-RateLimit-Limit", max_attempts.to_string())
        .header("X-RateLimit-Remaining", remaining.to_string());
    if let Some(retry) = retry_after_secs {
        resp = resp.header("Retry-After", retry.to_string());
        // Reset is the unix-seconds-since-epoch when the bucket reopens
        // (now + retry_after). Matches Laravel's `availableAt($retry)`.
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        resp = resp.header("X-RateLimit-Reset", (now + retry).to_string());
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys_of(limits: &[Limit], mode: &Mode, prefix: &str) -> Vec<Option<String>> {
        limits
            .iter()
            .map(|limit| prefixed_key(limit, mode, prefix))
            .collect()
    }

    #[test]
    fn buckets_join_keys_that_clean_alike_and_skip_unlimited() {
        let limits = vec![
            Limit::per_minute(2).by("café"),
            Limit::none().into(),
            Limit::per_hour(10).by("cafe"),
            Limit::per_minute(3).by("cafe").after(|_| true),
            Limit::per_minute(4).by("other"),
        ];
        let keys = keys_of(&limits, &Mode::Named("api".into()), "");
        assert_eq!(
            keys,
            vec![
                Some("api:café".into()),
                None,
                Some("api:cafe".into()),
                Some("api:cafe".into()),
                Some("api:other".into()),
            ],
            "each limit keeps the key it carries"
        );

        let buckets = buckets_of(&limits, &keys);
        assert_eq!(
            buckets.of_limit,
            vec![Some(0), None, Some(0), Some(0), Some(1)]
        );
        assert_eq!(buckets.buckets[0].key, "api:café");
        assert_eq!(
            buckets.buckets[0].counted,
            vec![0, 2],
            "a limit with an `after` callback is not counted with the request"
        );
        assert_eq!(buckets.buckets[1].counted, vec![4]);
    }

    #[test]
    fn places_left_is_the_tightest_limits_and_unread_buckets_sort_last() {
        let limits = vec![Limit::per_minute(5).by("a"), Limit::per_minute(3).by("a")];
        let keys = keys_of(&limits, &Mode::Limits(vec![]), "");
        let mut buckets = buckets_of(&limits, &keys);
        assert_eq!(places_left(&buckets.buckets[0], &limits), i64::MAX);
        buckets.buckets[0].before = Some(2);
        assert_eq!(places_left(&buckets.buckets[0], &limits), 1);
    }

    #[test]
    fn prefixed_key_includes_named_limiter_name() {
        let mode = Mode::Named("api".into());
        let limit = Limit::per_minute(5).by("user:1");
        let key = prefixed_key(&limit, &mode, "").unwrap();
        assert_eq!(key, "api:user:1");
    }

    #[test]
    fn prefixed_key_returns_none_for_unlimited() {
        let mode = Mode::Inline {
            guest_max_attempts: i64::MAX,
            user_max_attempts: i64::MAX,
            decay_seconds: 60,
        };
        let limit: Limit = Limit::none().into();
        assert!(prefixed_key(&limit, &mode, "").is_none());
    }

    #[test]
    fn prefixed_key_uses_fallback_when_limit_unkeyed() {
        let mode = Mode::Inline {
            guest_max_attempts: 10,
            user_max_attempts: 10,
            decay_seconds: 60,
        };
        let limit = Limit::per_minute(10);
        let key = prefixed_key(&limit, &mode, "p").unwrap();
        // Prefix + fallback_key with no name.
        assert!(key.starts_with("p:"));
        assert!(key.contains("attempts:10:decay:60"));
    }

    #[test]
    fn prefixed_key_prepends_user_prefix() {
        let mode = Mode::Limits(vec![]);
        let limit = Limit::per_minute(5).by("user:1");
        let key = prefixed_key(&limit, &mode, "shop").unwrap();
        assert_eq!(key, "shop:user:1");
    }
}
