use std::sync::Arc;
use std::time::Duration;
use suprnova::broadcasting::{
    BroadcastHub, BroadcastingWsHandler, ChannelRegistry, InMemoryBroadcastHub,
};
use suprnova::{
    AuthMiddleware as SessionAuthMiddleware, RateLimitMiddleware, RateLimiterDriver,
    SlidingWindowConfig,
    container::App,
    delete, get, group, identity_key, names_identity, post,
    rate_limit::{BackendErrorPolicy, memory::InMemoryRateLimiter},
    routes, ws,
};

/// Resolve the shared limiter, or a private in-memory one when bootstrap
/// has not run (tests that assemble the router by hand).
fn limiter() -> Arc<dyn RateLimiterDriver> {
    App::resolve_make::<dyn RateLimiterDriver>()
        .unwrap_or_else(|_| Arc::new(InMemoryRateLimiter::new()))
}

/// The caller's address as the framework resolves it.
///
/// `Request::ip()` honours the configured trusted-proxy allowlist and
/// only returns a forwarded hop when the peer is actually a trusted
/// proxy, normalising it to a parsed `IpAddr` on the way out.
///
/// The `/ping` demo used to read `x-forwarded-for` directly, which any
/// client can set. A bucket keyed on a header the caller controls is not
/// a rate limit — a fresh value per request means a fresh bucket per
/// request. `Request::ip()`'s own doc comment names this hazard, and the
/// demo bypassed it; since `app/` is the worked example people copy, the
/// bypass was the part most likely to spread.
fn client_ip_key(req: &suprnova::Request) -> String {
    req.ip().unwrap_or_else(|| "anon".into())
}

use crate::broadcasting::{ChatChannel, UserRegisteredChannel};
use crate::controllers;
use crate::middleware::AuthMiddleware;
use crate::ws as app_ws;

/// Build the `BroadcastingWsHandler` for `/ws/broadcast` by resolving
/// the hub and channel registry from the App container.
///
/// Falls back to a fresh in-process hub + registry when the container
/// hasn't been bootstrapped (e.g. in unit tests that assemble the
/// router without running `bootstrap::register()`). This mirrors the
/// pattern used by the rate-limit middleware.
fn broadcasting_handler() -> BroadcastingWsHandler {
    let hub: Arc<dyn BroadcastHub> =
        App::make::<dyn BroadcastHub>().unwrap_or_else(|| Arc::new(InMemoryBroadcastHub::new()));
    let registry: Arc<ChannelRegistry> = App::get::<Arc<ChannelRegistry>>().unwrap_or_else(|| {
        let mut r = ChannelRegistry::new();
        r.register(UserRegisteredChannel);
        r.register(ChatChannel);
        Arc::new(r)
    });
    BroadcastingWsHandler::new(hub, registry)
}

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/redirect-example", controllers::user::redirect_example),
    get!(
        "/preserve-fragment-example",
        controllers::user::preserve_fragment_example
    ),
    get!(
        "/ssr-opt-out-example",
        controllers::user::ssr_opt_out_example
    ),
    get!("/config", controllers::config_example::show).name("config.show"),

    // Task 11 — localization dogfood. GET renders a translated greeting
    // (`__!("welcome", app: ...)`); POST demonstrates a translated
    // `validation-required` failure on a missing `name` field. Kept
    // stateless like `/api/ping` and `/api/welcome` — excepted from CSRF
    // below for the same reason.
    get!("/lang-demo", controllers::lang_demo::show).name("lang-demo.show"),
    post!("/lang-demo", controllers::lang_demo::submit).name("lang-demo.submit"),

    // User routes group
    group!("/users", {
        get!("/", controllers::user::index).name("users.index"),
        get!("/{id}", controllers::user::show).name("users.show"),
        post!("/", controllers::user::store).name("users.store"),
    }),

    // Authenticated user routes — session-gated via the framework's
    // `AuthMiddleware`. The avatar upload exercises the full multipart
    // + storage + Auth stack end-to-end.
    group!("/users", {
        post!("/avatar", controllers::avatar_upload::upload).name("users.avatar.store"),
    }).middleware(SessionAuthMiddleware::new()),

    // Protected routes - requires Authorization header
    group!("/protected", {
        get!("/", controllers::home::index).name("protected.home"),
    }).middleware(AuthMiddleware),

    // Todo routes group
    group!("/todos", {
        get!("/", controllers::todo::list).name("todos.index"),
        post!("/random", controllers::todo::create_random).name("todos.create_random"),
    }),

    // SSE dogfood — streams UserRegistered broadcast events
    get!("/events/stream", controllers::sse_example::stream).name("events.stream"),

    // Phase 7A WebSocket dogfood — echo handler at /ws/echo.
    // Round-trips text messages with an "echo: " prefix; exits on peer close.
    ws!("/ws/echo", app_ws::echo::EchoHandler),

    // Phase 7B WebSocket broadcasting — JSON-envelope subscribe/publish.
    // Clients send {"type":"subscribe","channel":"user_registered"} to
    // receive UserRegistered events; ChatChannel requires a token in data.
    ws!("/ws/broadcast", broadcasting_handler()),

    // Phase 2 dogfood — cursor pagination over a 100-user fixture
    get!("/api/users", controllers::paginated_users::index).name("api.users.index"),

    // Phase 3 dogfood — JSON:API resources + Gate-authorized deletion
    // GET  /api/users/{id}  → JSON:API single resource (sparse fieldsets via ?fields[users]=...)
    // GET  /api/v3/users    → JSON:API collection
    // DELETE /api/posts/{id} → Gate::authorize("delete-post", ...) demo
    // Session-gated: `UserResource` serialises `email`, so these two
    // endpoints hand out every user's address to whoever asks unless
    // something stops them. Nothing did — they sat at the top level with
    // no middleware, which is the same defect Group 0 fixed in the `--api`
    // scaffold (`api_user_routes_are_behind_an_auth_gate`). The scaffold
    // got the fix; the dogfood, which is the other thing people copy,
    // did not.
    group!("/api", {
        get!("/users/{id}", controllers::admin::show_user).name("api.users.show"),
        get!("/v3/users", controllers::admin::list_users).name("api.v3.users.index"),
    }).middleware(SessionAuthMiddleware::new()),
    delete!("/api/posts/{id}", controllers::admin::delete_post).name("api.posts.destroy"),

    // Codex finding #17 — real Post model. Public GET listing remains
    // open; create/show require a session (the controllers also enforce
    // Gate::authorize through PostPolicy for show). The framework's
    // middleware map is keyed by `(method, path)` so the public GET
    // and the auth-gated POST can share the `/api/posts` path string
    // without leaking middleware across methods.
    get!("/api/posts", controllers::posts::index).name("api.posts.index"),
    group!("/api/posts", {
        get!("/{id}", controllers::posts::show).name("api.posts.show"),
        post!("/", controllers::posts::store).name("api.posts.store"),
    }).middleware(SessionAuthMiddleware::new()),

    // Phase 5B Task 20 — mail dogfood. `POST /api/welcome?email=...&name=...`
    // queues a WelcomeEmail Mailable onto the mail queue via Mail::queue.
    // The Mailable + SendMailJob are registered in bootstrap::register so
    // the worker can re-hydrate and dispatch.
    post!("/api/welcome", controllers::welcome::queue).name("api.welcome"),

    // Phase 11 — auth-flows dogfood.
    //
    // Public endpoints (no session middleware — they consume tokens
    // minted out-of-band or implement anti-enumeration responses for
    // arbitrary input):
    //   POST /auth/verify/resend?email=...  → 200, anti-enumeration
    //   GET  /auth/verify?token=...         → 302 / on success
    //   POST /auth/password/request         → 200, anti-enumeration
    //   POST /auth/password/reset           → 302 /?reset=ok on success
    //
    // Session-gated endpoints (require an authenticated session via
    // `SessionAuthMiddleware`):
    //   POST /auth/2fa/enroll   → 200 JSON {otpauth_url, qr_code_svg, recovery_codes}
    //   POST /auth/2fa/confirm  → 200 JSON {status:"confirmed"}
    //   POST /auth/2fa/disable  → 200 JSON {status:"disabled"}
    // P2-02(a)/(c) — the issuance routes mint or consume single-use
    // credentials, so they are the ones worth throttling. They carried no
    // limiter at all; the only throttled route in the app was the `/ping`
    // demo.
    //
    // `FailClosed` rather than the framework default: on a general API,
    // letting traffic through when the limiter backend is unreachable is
    // the right availability trade. Here it is not. A limiter outage is
    // precisely when unbounded password-reset issuance is most
    // attractive, and 503 on a reset form for the length of a Redis blip
    // is a far smaller problem than unbounded token minting.
    group!("/auth", {
        post!("/verify/resend", controllers::auth_verify::resend)
            .name("auth.verify.resend"),
        get!("/verify", controllers::auth_verify::verify).name("auth.verify.confirm"),
        post!("/password/request", controllers::auth_reset::request_reset)
            .name("auth.password.request"),
        post!("/password/reset", controllers::auth_reset::complete_reset)
            .name("auth.password.complete"),
    }).middleware(
        RateLimitMiddleware::new(
            limiter(),
            SlidingWindowConfig {
                max_requests: 10,
                window: Duration::from_secs(300),
            },
            // Keyed per-address across the whole issuance surface rather
            // than per-route, so an attacker cannot get a fresh budget by
            // rotating between the four endpoints.
            |req| format!("auth-issuance:ip:{}", client_ip_key(req)),
        )
        .on_backend_error(BackendErrorPolicy::FailClosed),
    ).middleware(
        // Per-recipient, stacked on top of the per-IP limit above. The
        // address limit asks "is this client noisy"; this one asks "is
        // this mailbox being flooded", and neither answers the other's
        // question. An attacker spread across a botnet or an IPv6 /64
        // stays under every per-IP budget while filling one victim's
        // inbox with reset mail, and the victim's address is the only
        // thing those requests share.
        //
        // Tighter than the IP budget on purpose: ten issuance calls in
        // five minutes is plausible for one person fumbling a flow,
        // whereas three reset mails to the *same* address in fifteen
        // minutes is already more than anyone needs.
        //
        // Of the four routes here only two name a recipient —
        // `/verify/resend` in the query string, `/password/request` in
        // the form body. `identity_key` reads either. The other two
        // consume a token the caller must already hold, so they have no
        // mailbox to flood and fall through to the IP bucket.
        RateLimitMiddleware::new(
            limiter(),
            SlidingWindowConfig {
                max_requests: 3,
                window: Duration::from_secs(900),
            },
            |req| identity_key(req, "email", "auth-issuance"),
        )
        // These bodies are a single short form field; 4 KiB is well
        // above any legitimate one and bounds what an unauthenticated
        // caller can make us buffer before the quota check.
        .key_reads_body(4096)
        // Stand aside for the two token-consuming routes. They name no
        // recipient, so this limiter has nothing to say about them — and
        // without this, its tighter quota (3/15min) would quietly become
        // their binding limit instead of the 10/5min chosen above. Behind
        // one office NAT that is a lockout. The per-IP limiter still
        // counts every one of those requests.
        .only_when(|req| names_identity(req, "email"))
        .on_backend_error(BackendErrorPolicy::FailClosed),
    ),
    group!("/auth/2fa", {
        post!("/enroll", controllers::auth_2fa::enroll).name("auth.2fa.enroll"),
        post!("/confirm", controllers::auth_2fa::confirm).name("auth.2fa.confirm"),
        post!("/disable", controllers::auth_2fa::disable).name("auth.2fa.disable"),
    }).middleware(SessionAuthMiddleware::new()),

    // Phase 5A dogfood — rate-limited ping endpoint.
    // 5 requests per 60-second window, keyed by X-Forwarded-For header
    // (falls back to "anon"). The in-memory limiter is bootstrapped in
    // bootstrap::register() so it is available here at route-build time.
    group!("/api", {
        post!("/ping", controllers::ping::pong).name("api.ping"),
    }).middleware({
        // Use the container binding if bootstrap has already wired it
        // (production path); fall back to a fresh in-memory limiter so
        // tests that assemble the router by hand without running
        // bootstrap::register() keep working.
        RateLimitMiddleware::new(
            limiter(),
            SlidingWindowConfig {
                max_requests: 5,
                window: Duration::from_secs(60),
            },
            // See `client_ip_key`: this used to read `x-forwarded-for`
            // straight off the request, which any client can set to a
            // fresh value per request and thereby get a fresh bucket per
            // request.
            |req| format!("ip:{}", client_ip_key(req)),
        )
    }),
}
