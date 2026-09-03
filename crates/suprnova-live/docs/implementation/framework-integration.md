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

`AuthMiddleware::new()` records the principal only on its authenticated branch
and answers `401` for an anonymous request before any engine or application
work. `AuthMiddleware::optional()` records the principal when one exists and
lets an anonymous request continue; the action boundary then closes only the
identity absences the request's mount kind permits, recorded by kind at
registration, so a public seed promotes for the anonymous visitor's own
session while an identity-bound island still fails with a missing principal.
The engine validates the closed facts against the catalog's own requirements,
so a closure the catalog does not permit still fails. An identity-bound island
binds the session fact its render request carried; the session middleware
rotates the session id when a request signs the user in, so a document that
renders identity-bound islands in the very request that logs the user in binds
the pre-rotation id and its first action refreshes. Ordinary login flows
render on the following request and never meet this. `LiveTenantMiddleware`
records the resolved tenant or, when the resolver answers `None`, the
tenantless disposition; a resolver that cannot determine the tenant returns an
error so the request fails instead of mounting untenanted. The rate-limit
middleware records the rate fact on its allowed branch. The asynchronous
control routes and the WebSocket upgrade require the same complete,
unexpired check set as a mount-bound request before any fact is trusted, and
the transport table is bounded per scope and process-wide. A document route
registered through `try_live_mount` or `try_live_document` carries the
document policy instead: a public seed waives the identity facts, and an
identity-bound mount requires the session and principal facts from the route's
own middleware. The tenant is optional for identity-bound mounts: a resolver
that names a tenant binds it into the island's scope, a single-tenant
deployment leaves it absent, and a request whose tenant differs from the bound
one fails the scope comparison either way.

A component that declares exactly one stream renders its island root with
the island-owned `live:stream` directive for it. The framework enables the
engine's opt-in island-stream policy on its mount and execution services; the
engine emits the root itself and rejects a template that carries the
directive, so component templates never declare island-owned directives, and
the browser runtime reads the directive from the emitted root and opens the
asynchronous transport. A component with several streams gets no directive
and subscribes each through the runtime's registered calls.

The asynchronous feature of the browser runtime stays inert until a host
supplies clocks, timers, randomness, transports, and an authority that issues
subscriptions. The runtime's asynchronous artifacts now ship that host:
`browserAsyncOptions()` issues and renews through `/__live/v1/async/subscriptions`
with the browser's same-origin credentials, drives SSE membership control
through `/__live/v1/async/memberships` with the issued bearer credential, and
opens the native transports. A document whose islands declare streams boots
through `suprnova-live.boot.async.esm.js`, which configures that host before
`boot()`; the classic boot configures `window.SuprnovaLiveAsync` when the
classic asynchronous artifact is present. The bearer SSE reader sends
same-origin credentials so the events route can re-resolve the session
identity, and the framework follows each productive SSE batch with a delayed
comment trailer because WebKit releases a fetch stream's buffered tail only
when more bytes arrive (see the asynchronous-updates record).

An action on an island whose component declares an upload field additionally
resolves the registered upload mount authority by route, slot, component, and
contract. The protocol version takes no part in that match: the request's own
selection already proved its version is one the component supports, and the
shipped runtime negotiates the newest one.

## CSRF and origin verification

The shipped browser runtime sends the Live media type, the request body, and
the browser's own `Sec-Fetch-Site` header; it carries no session CSRF token.
`CsrfMiddleware` therefore treats a verified origin as the configured CSRF
proof for every Live request on its own, whatever origin policy the
application configured: a same-origin Live request records the origin fact
and the stateless CSRF disposition, and the token path is not consulted. When
the header is absent or cross-site, token validation runs and the Live request
is refused with the ordinary `419`. A Live read such as the event stream
changes no state: with the origin proof it records the origin fact and a
not-required CSRF check, and without the proof it records neither, so the
asynchronous boundary's complete-check requirement refuses it. Ordinary
routes keep the configured policy, so using Live never relaxes token
validation elsewhere; the default `CsrfMiddleware::new()` is enough. The custom media type additionally forces
a cross-origin preflight that no Live route answers.

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

## Dogfood surface

The root `app/` crate hosts the durable dogfood surface in `app/src/live/`:
`app.counter` (a plain island), `app.avatar-uploader` (an upload field with a
PNG policy finalized by `save_avatar`), and `app.activity-feed` (a
stream-backed island refreshed over SSE or WebSocket with polling as the
fallback). `app::live::registry()` builds the registry, `bootstrap::register`
binds it with `LiveUploadHost` and the stream and upload gates, and
`app::live::routes` installs the guarded reserved routes, the authenticated
reacquisition route `/account/uploads/{handle}/reacquire`, the identity-bound
dashboard at `/live`, and the public page at `/live/public`. The application
entry point installs those routes through `Application::try_routes`. Templates
live under `app/templates/live/`, the Askama default for a crate without an
`askama.toml`, which is also where `suprnova live:make` writes.

## Evidence

`framework/tests/live_dogfood.rs` drives a public island through the real
session, CSRF, guard, tenant, and rate-limit middleware in process, and
`framework/tests/live_dogfood_server.rs` boots `Server::run` on a socket and
performs the same document and action round trip. `app/tests/live_dogfood.rs`,
`live_upload_reacquire.rs`, and `live_async_dogfood.rs` exercise the
application surface through its own global middleware stack with seeded
sessions: ordinary SSR, identity-bound and public islands, actions, CSRF and
principal enforcement, polling, a closed 409 for a tampered snapshot,
immutable assets, the complete upload lifecycle with reacquisition and
finalization, an SSE-delivered published event, a cookie-authorized WebSocket
membership, and the ordinary fresh render as the poll. The browser scenario in
`crates/suprnova-live/browser/e2e/app-dogfood.spec.ts` drives `app/examples/live_dogfood_host.rs`,
a real `Server::run` of the application routes behind the production stack:
the public page for an anonymous visitor, sign-in and three actions through
the shipped runtime, the anonymous action refused with `401`, and the feed's
asynchronous subscription with a refresh after a published event.
