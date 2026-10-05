# Rate Limiting

Suprnova ships two complementary rate-limit surfaces:

| Surface | Use when... | Backend |
|---------|-------------|---------|
| `RateLimiterDriver` + `RateLimitMiddleware` | You want strict sliding-window enforcement against arbitrary storage (Redis ZSET, in-memory deque) | `dyn RateLimiterDriver` |
| `RateLimiter` + `ThrottleRequestsMiddleware` | You want Laravel-shape named limiters, `attempt()` workflow callbacks, or `X-RateLimit-*` response headers | `Cache` store (memory or Redis) |

The sliding-window driver is Suprnova's native shape - one slot per request, no separate timer key, atomic Lua eval on Redis. The Laravel facade is what migrated apps reach for and what the named-limiter / response-callback pattern requires. The two coexist by design, and a route can layer both.

## Sliding-window driver SPI

`RateLimiterDriver` is the storage SPI for the sliding-window algorithm. Each key tracks a deque of hit timestamps. On every `try_acquire`, entries older than `now - window` are evicted; if the remaining count is below `max_requests`, `now` is appended and the call accepts. Otherwise it rejects.

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::rate_limit::{RateLimiterDriver, SlidingWindowConfig};

let limiter: Arc<dyn RateLimiterDriver> = Arc::new(InMemoryRateLimiter::new());
let cfg = SlidingWindowConfig {
    max_requests: 60,
    window: Duration::from_secs(60),
};
let ok = limiter.try_acquire("user:42", &cfg).await?;
if !ok {
    let wait = limiter.retry_after("user:42", &cfg).await?;
    // wait is the Option<Duration> until the oldest slot in the bucket
    // ages out.
}
```

### Built-in drivers

| Driver | Storage | Selected via |
|--------|---------|--------------|
| `InMemoryRateLimiter` | Per-process `HashMap<String, Bucket>` with `tokio::time::Instant` so `start_paused` tests can drive the clock (it does not follow `TestClock`) | `RATE_LIMIT_DRIVER=memory` (default) |
| `RedisRateLimiter` | Redis ZSET + Lua atomic check-and-record; it reads `suprnova::clock::now()`, so `TestClock` moves its window ([Moving the clock](testing.md#moving-the-clock)) | `RATE_LIMIT_DRIVER=redis` + `RATE_LIMIT_REDIS_URL` |

`bootstrap_from_env()` wires the matching driver into the container. Outside production an unknown driver value falls back to memory with a `warn!` log.

### Production fails closed on the in-memory driver

In production, resolving to the in-memory limiter is a boot failure:

```
refusing to boot in production: RATE_LIMIT_DRIVER is unset, which defaults
to the in-memory limiter. Per-process buckets mean every configured quota
is multiplied by your replica count and reset by every deploy...
```

The in-memory driver keeps its buckets in one process's heap. Behind N
replicas each keeps its own count, so a "5 attempts per 15 minutes"
password-reset throttle is really 5N, and every deploy resets all of them
to zero. The limit you configured is not the limit you get - and nothing
says so, because the requests succeed, which is what a working throttle
looks like from the outside. It surfaces as a credential-stuffing or
account-enumeration incident, not as an error.

An **unrecognised** driver value fails for the same reason: it falls back
to memory. `RATE_LIMIT_DRIVER=Redis` - capitalised - would otherwise warn
once at boot and quietly leave a multi-replica deployment throttling
per-process. That is the case most likely to reach production, because it
looks configured.

Either point it at Redis:

```env
RATE_LIMIT_DRIVER=redis
RATE_LIMIT_REDIS_URL=redis://cache.internal:6379
```

or, if you genuinely run a single process, say so:

```env
RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION=true
```

Development, testing and **staging** are untouched. Staging is
deliberately not gated, on the same reasoning as the mail guard: hard
failing it pushes teams to set the override globally, which disarms the
check exactly where it matters.

### `RateLimitMiddleware`

The HTTP wrapper around the driver. Construct with a `key_fn` closure to drive bucket selection per-request:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::container::App;
use suprnova::rate_limit::{
    BackendErrorPolicy, RateLimitMiddleware, RateLimiterDriver, SlidingWindowConfig,
};

let limiter: Arc<dyn RateLimiterDriver> =
    App::resolve_make::<dyn RateLimiterDriver>().unwrap();

let mw = RateLimitMiddleware::new(
    limiter,
    SlidingWindowConfig {
        max_requests: 100,
        window: Duration::from_secs(60),
    },
    |req| format!("route:{}", req.path()),
)
.on_backend_error(BackendErrorPolicy::FailClosed);
```

On rejection (over quota) it returns HTTP 429 with a `Retry-After` header.

### Per-address limit with `ip_based`

`RateLimitMiddleware::ip_based(max_requests, window)` allows each client address `max_requests` requests in `window`. It is the limit a login form or a public API needs, and it takes no key function and no driver:

```rust
use std::time::Duration;
use suprnova::rate_limit::RateLimitMiddleware;

let twenty_a_minute = RateLimitMiddleware::ip_based(20, Duration::from_secs(60));
```

The arguments are a `u32` request count and a `Duration`. Three details matter:

- **The driver is the installed one.** The middleware uses the rate limiter your application installed, which `RATE_LIMIT_DRIVER` selects. It looks the limiter up when a request arrives, so you can build the middleware where you register routes, before the drivers boot. When no limiter is installed the lookup fails as a backend error, and [the backend-error policy](#backend-error-policy) decides what the request gets.
- **The key names the address and the limit.** The address is [`Request::ip()`](requests.md#host-scheme-ip), which resolves through the trusted proxies. The key also carries the limit's numbers, so two limits with different numbers never share a bucket. Two limits with the same numbers share one, so the budget belongs to the client, whichever route spends it.
- **A request with no address gets a bucket of its own.** One shared bucket for all such requests would let one caller use it up and lock the others out. Every request the server accepts has a peer address, so only an in-process request has none.

`.on_backend_error(...)`, `.only_when(...)` and `.key_reads_body(...)` chain onto it as they do onto `RateLimitMiddleware::new`. Set up [`APP_TRUSTED_PROXIES`](#the-client-address-behind-a-proxy) before you deploy behind a proxy, or every client shares one bucket.

### Capping open connections with `connections_per_ip`

`ip_based` counts how often a client asks. It does not count what a client holds. A WebSocket is asked for once and then stays open, so a client that opens sockets and keeps them can use all the connections the server accepts. `RateLimitMiddleware::connections_per_ip(max)` gives each client address `max` open connections and refuses the next one with `429 Too Many Requests`. Put it on a WebSocket route:

```rust
use suprnova::rate_limit::RateLimitMiddleware;
use suprnova::ws;

ws!("/ws/broadcast", Broadcast).middleware(RateLimitMiddleware::connections_per_ip(100));
```

It returns a `ConnectionsPerIp`, which is a `Middleware`. What it counts and how it answers:

- **What is counted on a WebSocket route.** A socket counts from the moment the middleware lets the upgrade pass until the session of the socket ends, which is when the handler has returned and the close handshake is done. If a later middleware refuses the upgrade, the place is given back at once. A peer that vanishes without closing holds its place until the handler notices, which is when a read or a write on the socket fails.
- **What is counted on any other route.** A request, while it is handled. The place is given back when the handler drops or consumes the request, and reading the body consumes it. A connection that is kept alive and idle is not counted, and a streamed response, such as server-sent events, outlives its place. Use this cap for WebSocket routes.
- **The answer.** A request over the cap gets status `429` with the body `429 Too Many Requests`. It carries no `Retry-After` header, because nothing says when a socket will close.
- **The counts are per process.** They are kept in the memory of one process, which is the right place: the sockets an address holds in a process are what that process runs out of. Behind several replicas each replica counts for itself, so the cap for one address is `max` for each replica.
- **IPv6 addresses count by /64.** An IPv4 address counts by itself. An IPv6 address counts with its /64 network, because a subscriber line gets a /64 or more and a count for each of its addresses would be no cap. An IPv4 address written in IPv6 form, `::ffff:203.0.113.1`, counts as the IPv4 address.
- **The address is `Request::ip()`.** It reads the forwarded headers of a trusted proxy and of nobody else. A request with no address to resolve is not counted. If you run your own accept loop, pass the peer address with `suprnova::server::handle_request_with_peer`. Through `handle_request` no request has an address, and nothing is capped.

A clone of a `ConnectionsPerIp` shares its counts, so one cap can guard several routes. `open_for(address)` returns the connections an address holds through the cap right now, counted by /64 for IPv6, which is useful in a test or a diagnostic.

The middleware counts through `Request::hold_for_connection(hold)`. That method takes a guard value whose `Drop` gives the place back, and keeps it on the request. For a WebSocket upgrade the server moves the guards into the task that runs the socket and drops them when the task ends. For any other request they drop with the request. Use it in your own middleware when you take something that a connection has to give back:

```rust
use suprnova::{async_trait, Middleware, Next, Request, Response};

struct Slot(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

struct CountOpen(std::sync::Arc<std::sync::atomic::AtomicUsize>);

#[async_trait]
impl Middleware for CountOpen {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        request.hold_for_connection(Slot(self.0.clone()));
        next(request).await
    }
}
```

`hold_for_connection` needs `&mut Request` and a value that is `Send + Sync + 'static`.

### Limiting per recipient, not just per caller

An address-keyed limit answers *is one client making too many requests*. It cannot answer *is one mailbox being flooded*. An attacker spread across a botnet, a proxy pool, or a single IPv6 `/64` stays under every per-IP budget while sending one victim thousands of password-reset emails - the inbox is the resource being exhausted, and the victim's address is the only thing those requests share. The reverse hurts too: behind carrier-grade NAT or an office gateway, per-IP limits punish a crowd for one member's behaviour.

`identity_key` keys a bucket on the account being *acted on*:

```rust
use suprnova::rate_limit::{identity_key, names_identity};

let per_recipient = RateLimitMiddleware::new(
    limiter.clone(),
    SlidingWindowConfig { max_requests: 3, window: Duration::from_secs(900) },
    |req| identity_key(req, "email", "auth-issuance"),
)
.key_reads_body(4096)
.only_when(|req| names_identity(req, "email"))
.on_backend_error(BackendErrorPolicy::FailClosed);
```

Stack it *alongside* a per-IP limiter rather than replacing one with the other. Each catches what the other cannot: per-IP stops one host enumerating many addresses; per-recipient stops many hosts targeting one address.

Four details carry the security:

- **`key_reads_body`** buffers the body (to the given cap) before the key is computed, so the field can be read out of a form-encoded POST, or a top-level string field of a JSON body, as well as a query string. It is opt-in because buffering is work an unauthenticated caller gets to make you do; the cap bounds it. A body over the cap is rejected with 413 rather than passed through unkeyed - otherwise padding the body would be a way out of the limit.
- **Both places are read, and they must agree.** A blank `?email=` does not hide the address in the body. A request whose query string and body name two different addresses is ambiguous - the handler reads one of them, and the key cannot know which - so every such request shares one bucket. It is also refused by, and counted against, the bucket of each address it names. A decoy in the place the handler ignores therefore cannot buy a fresh quota, and an address that has spent its own bucket gets no extra attempt through the shared one. No legitimate client is affected, because none sends two different addresses.
- **`only_when`** skips the limiter for requests that name nobody. Without it those fall into `identity_key`'s address fallback and are counted against *this* limiter's quota - and since a per-recipient budget is normally the tighter of the pair, it would silently become the binding limit for every route that names no one.
- **The value is normalised and hashed.** `Alice@Example.com` and `alice@example.com` reach the same mailbox and must share a bucket, or the limit is bypassed by changing capitalisation. The result is hashed because a rate-limit backend is frequently a shared Redis with weaker access control than the primary database, and a key dump should not read as a list of who is resetting their password.

### Backend-error policy

`BackendErrorPolicy` governs what happens when the limiter *backend* itself errors - e.g. Redis is unreachable - as distinct from a request legitimately exceeding its quota. The backend cannot make a decision, so the middleware must choose between availability and the limit's guarantee.

| Policy | Behaviour | When to use |
|--------|-----------|-------------|
| `FailOpen` (default) | Pass the request through; log at `warn` | Most public APIs - a limiter outage should not take down traffic |
| `FailClosed` | Reject with HTTP 503 + `Retry-After: 1`; log at `error` | Sensitive routes (login, password reset, payments) where unbounded traffic during a backend outage is worse than briefly rejecting |

Choose with `.on_backend_error(BackendErrorPolicy::FailClosed)` on the middleware. Quota-exhausted requests are always 429 regardless of the policy - the policy only affects backend-error fallthrough.

## Cache-backed Laravel-shape facade

`RateLimiter` (the struct) mirrors `Illuminate\Cache\RateLimiter`. It's a fixed-window counter built on top of the Suprnova [`Cache`](cache.md) facade. Use it for named limiters, `attempt()` workflows, or any time you want the `X-RateLimit-*` headers Laravel apps expect.

### Storage layout

For an attempt counter key `K` with decay of `D` seconds:

- `K` - i64 counter incremented by every `hit`. Initial seed is 0 (via `Cache::add`).
- `K:timer` - i64 unix-seconds-since-epoch when the window ends, set via `Cache::add` so only the first caller in a window pins the deadline.

Both keys carry the same TTL so the cache cleans them up automatically when the window ends. When the counter has reached `max_attempts` but the `:timer` is gone, `too_many_attempts` resets the counter - this is what makes the window slide forward after a quota-exhausted period.

### Counter API

```rust
use suprnova::RateLimiter;

// Burn one attempt; seeds the window if missing.
let n = RateLimiter::hit("login:1.2.3.4", 60).await?;

// Burn one attempt AND test the limit in a single atomic round-trip.
// Returns `true` when this hit pushed the bucket over `max` (refuse the
// request), `false` when it was admitted. Use this instead of a separate
// `too_many_attempts` + `hit` pair: checking and then hitting as two calls
// lets concurrent requests slip past the limit (a check-then-act race).
// `i64::MAX` as the max means "unlimited" - always admits, still counts.
let over_limit = RateLimiter::hit_and_check("login:1.2.3.4", 5, 60).await?;
if over_limit { /* return 429 */ }

// Increment by N; useful for "cost-weighted" limits (each request burns
// more than one attempt).
let n = RateLimiter::increment("api:user:1", 60, 5).await?;

// Read the current count (0 when never hit or expired).
let attempts = RateLimiter::attempts("login:1.2.3.4").await?;

// Number of seconds until the window reopens (0 when no window open).
let secs = RateLimiter::available_in("login:1.2.3.4").await?;

// Retries left before tripping.
let remaining = RateLimiter::remaining("login:1.2.3.4", 5).await?;
// retries_left is the Laravel-spelt alias of remaining.
let remaining = RateLimiter::retries_left("login:1.2.3.4", 5).await?;

// Is the bucket over its limit RIGHT NOW (with window still open)?
let over = RateLimiter::too_many_attempts("login:1.2.3.4", 5).await?;

// Drop only the counter (timer stays - the window is still pinned).
RateLimiter::reset_attempts("login:1.2.3.4").await?;

// Drop both counter and timer.
RateLimiter::clear("login:1.2.3.4").await?;
```

### `attempt()` workflow

Run a callback only when the bucket is under quota; the hit is only burned when the callback runs:

```rust
let result = RateLimiter::attempt(
    "login:1.2.3.4",
    5,
    || async { do_login_work().await },
    60,
).await?;
match result {
    Some(value) => { /* callback ran, attempt counted */ }
    None => { /* over limit, callback was NOT run */ }
}
```

This is the right shape for login forms - you don't burn an attempt unless the work actually reached the callback.

### Named limiters

Register at boot, resolve at request time. The Laravel-side name `for` is a Rust reserved keyword, so the primary Rust-side name is `define`; the literal Laravel alias is exposed via `r#for`.

```rust
use suprnova::{Limit, RateLimiter};

// At boot - `define` is the primary Rust-side name.
RateLimiter::define("api", |req| {
    // `req.ip()`, not the raw `X-Forwarded-For` header - see below.
    let key = req.ip().unwrap_or_else(|| "anon".into());
    Limit::per_minute(60).by(format!("ip:{key}")).into()
});

// Laravel-side alias - same thing under the keyword-escape spelling.
RateLimiter::r#for("uploads", |_req| Limit::per_hour(100).into());

// Resolve.
let cb = RateLimiter::limiter("api").unwrap();
let limit_result = cb(&request);
```

A named-limiter callback returns a [`LimitResult`], constructible from:

- A single `Limit` - apply this limit.
- A `Vec<Limit>` - apply every limit; first to trip wins.
- An `HttpResponse` - short-circuit immediately with this response (used for "admin gets unlimited access" via `Limit::none()`, or to refuse the request outright).

### Sanitising keys

`RateLimiter::clean_rate_limiter_key(key)` strips `&abc;` HTML-entity markers from a key - Laravel uses this for user-supplied strings that round-trip through `htmlentities`. Suprnova reproduces the strip stage exactly but does NOT prepend the `htmlentities` encoding (which only matters for non-UTF-8 inputs, irrelevant for Rust `String`). The function is deterministic and idempotent inside Suprnova; consumers who need byte-identical hashing with a PHP service should run their own `htmlentities` pre-step on the input.

```rust
assert_eq!(RateLimiter::clean_rate_limiter_key("a&amp;b"), "aab");
```

## `Limit` builder

The data type returned by named-limiter callbacks. Shorthand constructors mirror Laravel's `Limit::per*`:

```rust
use suprnova::Limit;
use std::time::Duration;

Limit::per_second(10, 1);           // 10 per 1 second (max_attempts, decay_seconds)
Limit::per_minute(60);              // 60 per minute
Limit::per_minutes(5, 100);         // 100 per 5 minutes (decay-first, Laravel signature)
Limit::per_hour(1_000);             // 1000/hr
Limit::per_hours(6, 5_000);         // 5000 per 6 hours
Limit::per_day(10_000);             // 10000/day
Limit::per_days(7, 50_000);         // 50000 per 7 days
Limit::new(123, Duration::from_secs(45));  // bare ctor

// Builder chain.
let l = Limit::per_minute(5)
    .by("user:42")
    .response(|req| {
        suprnova::HttpResponse::text("blocked").status(429)
    })
    .after(|response| response.status_code() >= 400);
```

- `.by(key)` - set the bucket key. Empty key is "global" (every caller shares one bucket).
- `.response(callback)` - generate a custom response when the limit trips; the default is plain 429 "Too Many Attempts.".
- `.after(callback)` - only burn the attempt when `callback(response)` returns true. Canonical use: only count failed logins (`after(|r| r.status_code() >= 400)`).

`Limit::none()` returns an `Unlimited` (a `GlobalLimit` with `max_attempts = i64::MAX`). Returning it from a named limiter is the Laravel pattern for bypass. `GlobalLimit` itself is a thin wrapper around `Limit` with an empty key, kept for parity with `Illuminate\Cache\RateLimiting\GlobalLimit`.

## `ThrottleRequestsMiddleware`

HTTP wrapper around the Cache-backed facade. Mirrors `Illuminate\Routing\Middleware\ThrottleRequests`. Four constructors:

```rust
use suprnova::{Limit, ThrottleRequestsMiddleware};

// The default limit - 60 requests a minute; see below.
ThrottleRequestsMiddleware::default();

// Named limiter - resolves at request time via RateLimiter::limiter(name).
ThrottleRequestsMiddleware::by_name("api");

// Inline max/decay/prefix - the literal Laravel `throttle:60,1` shape.
ThrottleRequestsMiddleware::with(60, 1, "myroute");

// Explicit list of Limits - first-to-trip wins; most Rust-idiomatic.
ThrottleRequestsMiddleware::with_limits(vec![
    Limit::per_hour(5_000).by("user:1"),
    Limit::per_minute(60).by("user:1"),
]);
```

`.prefix(...)` sets a key prefix on any of them.

### The default limit

`ThrottleRequestsMiddleware::default()` allows 60 requests a minute. The constants `DEFAULT_MAX_ATTEMPTS` (`60`) and `DEFAULT_DECAY_SECONDS` (`60`) hold the numbers. It is the shape of Laravel's default `api` limiter: `Limit::perMinute(60)->by($request->user()?->id ?: $request->ip())`.

The bucket is one for each signed-in user, and one for each client address when nobody is signed in. The two are spelled apart, as `user:<id>` and `ip:<address>`, so a user whose id reads like an address shares no bucket with it. The user's bucket follows the user across routes and across addresses, and the address's bucket is shared by every route, as in Laravel. `with(...)` differs: it counts per address and per path. Use `.prefix(...)` to give a group of routes a budget of its own.

The user is the one the default guard signed in. Run the session middleware before this one, or a signed-in user counts as an address. The address is `Request::ip()`, so [the trusted proxies](#the-client-address-behind-a-proxy) apply.

### Named aliases

A route can name a throttle instead of building one. `ThrottleRequestsMiddleware::from_alias_args` reads the arguments of the alias the way Laravel reads `throttle:60,1`. Register it once at boot, then name it on routes with `.middleware_named(...)`:

```rust
use suprnova::middleware::register_middleware_alias_with_args;
use suprnova::{Router, ThrottleRequestsMiddleware};

register_middleware_alias_with_args("throttle", ThrottleRequestsMiddleware::from_alias_args);

let router = Router::new()
    .post("/login", login)
    .middleware_named("throttle:5,1");
```

| A route writes | It gets |
|---|---|
| `throttle` | `ThrottleRequestsMiddleware::default()` |
| `throttle:60` | 60 requests a minute, as `with(60, 1, "")` |
| `throttle:60,5` | 60 requests in 5 minutes |
| `throttle:60,5,uploads` | the same, with the key prefix `uploads` |
| `throttle:api` | the limiter named `api`, as `by_name("api")` |

A first argument that is not a number names a limiter, and a limiter takes no further arguments. The alias refuses a number that does not parse, a limit or a window of zero, and more than three arguments. The route that names it then fails to register. See [Middleware](middleware.md#named-aliases-and-groups) for how names resolve.

Wire it into a route group:

```rust
use suprnova::{Limit, RateLimiter, Router, ThrottleRequestsMiddleware};

RateLimiter::define("api", |req| {
    Limit::per_minute(60)
        .by(req.ip().unwrap_or_else(|| "anon".into()))
        .into()
});

let router = Router::new()
    .get("/api/items", list_items)
    .post("/api/items", create_item)
    .middleware(ThrottleRequestsMiddleware::by_name("api"));
```

### The client address behind a proxy

Key a limit on `req.ip()`, never on the header. `X-Forwarded-For` is caller-supplied. A limiter keyed on the raw header is defeated by sending a different value on each request - the attacker picks their own bucket, so the quota is per-request rather than per-client.

`Request::ip()` is the safe read. It returns the TCP peer address unless the peer is listed in `APP_TRUSTED_PROXIES`. When the peer is a trusted proxy, `req.ip()` reads the client address from the **right** of `X-Forwarded-For`:

1. It starts at the rightmost entry, which the peer wrote.
2. It skips each entry that is a trusted proxy.
3. It returns the first entry that is not a trusted proxy. That entry is the client. Nothing to its left was written by a proxy you trust.

Every proxy hop must be in the list. A proxy adds the address it saw to the right end of the header, and the client's own value stays at the left end. If a hop is missing from the list, `req.ip()` stops at that hop and takes it for the client, so every client behind it shares one address and one bucket. Behind a content delivery network in front of a proxy of your own, list the network's edge too.

`APP_TRUSTED_PROXIES` takes addresses and ranges in CIDR form, separated by commas:

```env
APP_TRUSTED_PROXIES=10.0.0.5,173.245.48.0/20,2400:cb00::/32
```

Four more rules apply:

- **A range must hold proxies and nothing else.** A client that connects from a listed range is believed like a proxy, and it writes the address `req.ip()` returns. The network of your cluster's pods and the range of a VPN have clients in them.
- **`/0` is refused.** A range of every address, such as `0.0.0.0/0`, makes every client a trusted proxy. Boot fails on it, as it does on an entry that is neither an address nor a range.
- **`X-Real-IP` is read only when there is no `X-Forwarded-For`.** With both headers present, `X-Real-IP` is ignored. A proxy that sets `X-Real-IP` and nothing else has to remove the client's `X-Forwarded-For` from the request.
- **The proxy has to write the header.** A proxy that adds to `X-Forwarded-For` or replaces it is a proxy to list. A proxy that passes the client's header on unchanged is not, because the client then writes all of it. The `Forwarded` header of RFC 7239 is not read.

An entry that is not an address, such as `unknown`, ends the walk, and `req.ip()` returns the proxy that wrote that entry. The clients of such a proxy share its address, and one limit.

The corollary matters as much: with `APP_TRUSTED_PROXIES` unset - the default - `req.ip()` behind a terminating proxy returns *the proxy's* address on every request, and every per-IP limit in the app collapses into a single shared bucket. `ThrottleRequestsMiddleware::with(20, 1, "login")` then means 20 attempts a minute across all users combined, which any one caller can spend to lock everybody out. Deploying behind nginx, Traefik, an ALB or Cloudflare means setting [`APP_TRUSTED_PROXIES`](env-vars.md#behind-a-reverse-proxy-set-app_trusted_proxies). See [Requests](requests.md#host-scheme-ip) for the full order in which `ip()` reads the request.

### Response headers

Every wrapped response carries:

- `X-RateLimit-Limit` - the configured `max_attempts`.
- `X-RateLimit-Remaining` - retries left for this bucket.

429 responses additionally carry:

- `Retry-After` - seconds until the window reopens.
- `X-RateLimit-Reset` - unix-seconds-since-epoch when the bucket reopens.

This matches Laravel's `ThrottleRequests::getHeaders` shape exactly.

### Missing named limiter

When a route is wired to `by_name("X")` but no limiter under `X` has been registered, the middleware returns HTTP 503 with a body that names the missing limiter. Laravel throws `MissingRateLimiterException`; we surface it as an HTTP response so a misconfigured boot does not panic the worker thread.

### Driver-vs-facade composition

The two middlewares can coexist on a single router. Layer the sliding-window driver for low-level fairness, then the Cache-backed throttle for per-endpoint named limits:

```rust
let router = Router::new()
    .get("/api/items", list_items)
    .middleware(RateLimitMiddleware::new(limiter_driver, cfg, key_fn))
    .middleware(ThrottleRequestsMiddleware::by_name("api"));
```

## Configuration

The driver SPI is configured via environment variables; the Cache-backed facade is configured wherever your [`Cache`](cache.md) store is configured (memory or Redis).

| Variable | Used by | Default |
|----------|---------|---------|
| `RATE_LIMIT_DRIVER` | Driver SPI bootstrap | `memory` (refused in production - see above) |
| `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION` | Production fail-closed override | unset |
| `RATE_LIMIT_REDIS_URL` | Redis driver | `redis://127.0.0.1:6379` |
| `RATE_LIMIT_PREFIX` | Redis key prefix | `suprnova:` |
| `CACHE_DRIVER` / `REDIS_URL` / `CACHE_DEFAULT_TTL` / `REDIS_PREFIX` | Cache-backed `RateLimiter` facade (see [`Cache`](cache.md)) | various |

## Migration from Laravel

| Laravel | Suprnova |
|---------|----------|
| `RateLimiter::for('api', fn ($req) => Limit::perMinute(60))` | `RateLimiter::define("api", \|req\| Limit::per_minute(60).into())` or `RateLimiter::r#for(...)` |
| `RateLimiter::hit($key, $decay)` | `RateLimiter::hit(key, decay).await?` |
| `RateLimiter::tooManyAttempts($key, $max)` | `RateLimiter::too_many_attempts(key, max).await?` |
| `RateLimiter::availableIn($key)` | `RateLimiter::available_in(key).await?` |
| `RateLimiter::attempt($key, $max, $cb, $decay)` | `RateLimiter::attempt(key, max, \|\| async { ... }, decay).await?` |
| `RateLimiter::retriesLeft($key, $max)` | `RateLimiter::retries_left(key, max).await?` |
| `RateLimiter::cleanRateLimiterKey($key)` | `RateLimiter::clean_rate_limiter_key(key)` |
| `Limit::perMinute(60)->by($ip)->response(fn () => abort(429))` | `Limit::per_minute(60).by(ip).response(\|_\| HttpResponse::text("...").status(429))` |
| `Limit::perMinutes(3, 100)` | `Limit::per_minutes(3, 100)` |
| `Limit::none()` | `Limit::none()` |
| `throttle:api` middleware | `.middleware_named("throttle:api")` or `ThrottleRequestsMiddleware::by_name("api")` |
| `throttle:60,1` middleware | `.middleware_named("throttle:60,1")` with `from_alias_args` registered, or `ThrottleRequestsMiddleware::with(60, 1, "")` |
| `throttle` middleware (the default limit) | `ThrottleRequestsMiddleware::default()` |
| `X-RateLimit-Limit/Remaining/Reset` + `Retry-After` headers | Same headers, same shape |

### Why Suprnova diverges

Laravel ships one shape: `Illuminate\Cache\RateLimiter` (Cache-backed fixed-window counter) with `Illuminate\Routing\Middleware\ThrottleRequests` as its HTTP wrapper. Suprnova ships both that shape *and* a native sliding-window driver SPI because two real questions need two real answers.

A Cache-backed counter is the right answer to "I have named limiters, response callbacks, after-callbacks for failed-login-only counting, and I want to be source-compatible with Laravel migrations." It's the wrong answer to "I need exact one-slot-per-request sliding-window enforcement against a Redis ZSET with atomic Lua eval and no separate timer key." That second question is what most Rust services hitting Tokio's concurrency limits actually have, so `RateLimiterDriver` + `RateLimitMiddleware` exist alongside, not behind a feature flag.

The backend-error policy is also a Suprnova addition. Laravel's middleware never surfaces a "the limiter is broken" decision because PHP's per-request lifecycle hides it - the next request gets a fresh process. A long-lived Tokio worker that loses Redis for ten seconds must decide what to do with the requests arriving during that window; `BackendErrorPolicy::FailOpen` (default) vs `FailClosed` is that decision exposed explicitly.

## Next

- [Middleware](middleware.md) - how middleware composes, runs, and short-circuits in the request chain
- [Cache](cache.md) - the store the Laravel-shape `RateLimiter` facade is built on
- [Configuration](configuration.md) - typed config for the cache and Redis backends
- [Auth Flows](auth-flows.md) - `LoginThrottleMiddleware` and the brute-force lockout pattern build on this surface
- [Error Model](error-model.md) - why `Result<HttpResponse, HttpResponse>` lets the middleware short-circuit cleanly
