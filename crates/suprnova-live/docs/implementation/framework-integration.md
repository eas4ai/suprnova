# Framework integration

This document records how Suprnova hosts the Live engine after iteration 005:
which crate owns what, how the reserved routes are installed and guarded, which
providers an application binds, and how the request boundary fails closed. The
engine remains host-neutral; everything below is the framework's adapter layer
in `framework/src/live/` and the application surface it exposes as
`suprnova::live`.

## Ownership and topology

- `crates/suprnova-live` is the engine: protocol, snapshots, ledger, checker,
  uploads, asynchronous updates, and the reviewed browser artifacts. It never
  depends on the framework.
- `framework/src/live/` adapts the engine to Suprnova's router, middleware,
  sessions, CSRF, authentication, tenants, rate limiting, storage, and
  response machinery. Applications name only `suprnova::live` and
  `suprnova::view`; the hidden `suprnova::live::__private` contract exists for
  generated code alone.
- `suprnova-macros` owns the production `#[derive(LiveComponent)]`, `#[live]`,
  and `#[suprnova::view]` macros.
- `suprnova-cli` scaffolds components and drives the application-side tooling
  helper described in [views and checker](views-and-checker.md); it keeps no
  framework dependency.
- The root `app/` crate is the durable dogfood surface; a freshly generated
  application inherits the same wiring from the scaffold templates.

## Reserved routes and the route guard

`Router::try_live()` installs the versioned reserved namespace exactly once
after a collision preflight: `/__live/v1/action`, `/__live/v1/upload`, the
three `/__live/v1/async/*` control routes, the `/__live/v1/async/socket`
WebSocket handshake, and the immutable `/__live/v1/assets/*` routes. Every
request route carries the strict Live policy, so session, origin, CSRF,
principal, tenant, proxy, rate-limit, and middleware facts must all be present
with a typed disposition before engine work starts.

The framework records the session fact in `SessionMiddleware`, the origin and
CSRF facts in `CsrfMiddleware`, and the proxy and middleware facts itself. The
principal, tenant, and rate-limit facts come from application middleware, and
`Router::try_live_with(|guard| guard.middleware(...))` attaches that middleware
to exactly the reserved request routes, in the given order, never to the asset
routes:

```rust
router.try_live_with(|guard| {
    guard
        .middleware(AuthMiddleware::new())
        .middleware(LiveTenantMiddleware::new(Arc::new(TenantResolver)))
        .middleware(RateLimitMiddleware::new(limiter, window, key))
})?
```

`AuthMiddleware` records the principal only on its authenticated branch, so an
anonymous visitor can render a public island but cannot invoke an action; the
guard answers `401` before any engine or application work. `LiveTenantMiddleware`
records the resolved tenant or the tenantless disposition, and the rate-limit
middleware records the rate fact on its allowed branch. A document route
registered through `try_live_mount` or `try_live_document` carries the public
document policy instead, which waives the identity facts for public seeds and
requires them for identity-bound mounts.

## CSRF and origin verification

The shipped browser runtime sends the Live media type, the request body, and
the browser's own `Sec-Fetch-Site` header; it carries no session CSRF token.
`CsrfMiddleware` therefore treats a verified origin as the configured CSRF
proof for Live requests exactly as it does for every other same-origin state
change: when the configured origin policy accepts the request, it records the
origin fact and the stateless CSRF disposition, and the token path is not
consulted. When the origin policy is disabled or the header is absent or
cross-site, token validation runs and the Live request is refused with the
ordinary `419`. An application that uses Live enables origin verification with
`CsrfMiddleware::new().with_origin_policy(OriginPolicy::SameOriginOnly)` or one
of the same-site variants. The custom media type additionally forces a
cross-origin preflight that no Live route answers.

## Providers and configuration

Bind these before the runtime assembles, normally in the application's
bootstrap so the server, workers, and the console tooling helper all see them:

- `LiveRegistry`: every component, built once with `LiveRegistry::builder()`.
- `LiveConfig`: request and response byte limits and the trusted context
  lifetime; `LiveConfig::standard()` when unset.
- `LiveUploadHost`: the finalizer, direct provider, scanner, and application
  validator; absent capabilities fail closed with typed upload errors.
- The tenant resolver behind `LiveTenantMiddleware` and the rate limiter the
  guard uses.

`Server::run` and `Application::try_routes` bind the runtime, register every
declared mount, and finalize the mount catalog before the first request; a
mount declared after that point is rejected at startup.

## Failure contracts

- A request missing any strict-policy fact is rejected by the trusted context
  validator before engine admission, with a closed body and no partial protocol
  bytes.
- A guard middleware rejection (`401`, `403`, `429`) answers before the engine
  parses the body.
- A CSRF or origin failure answers `419` or `403` from `CsrfMiddleware`.
- Document mounts after bootstrap, repeated bootstrap, and route mismatches
  fail the document handler with typed `LiveDocumentError` kinds and never emit
  a partial document.
- Adapter errors preserve typed recovery and redaction; production messages
  never carry snapshots, tokens, cookies, or rendered HTML.

## Evidence

`framework/tests/live_dogfood.rs` drives a public island through the real
session, CSRF, guard, tenant, and rate-limit middleware in process, and
`framework/tests/live_dogfood_server.rs` boots `Server::run` on a socket and
performs the same document and action round trip. The application-level
evidence lives in `app/tests/live_*.rs` and the browser scenario in
`browser/e2e/`.
