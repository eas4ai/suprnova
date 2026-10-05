# Security headers

Status: Draft
Prefix: SEC

Drafted 2026-10-05 from the developer's direction "We should add full
support for security headers https://github.com/sadco-io/http-security-headers-tower",
his ruling "Yes I will accept your recommendation to build it into the
framework" (the crate is the checklist, not a dependency), and his
direction for caching pages served under a nonce: "we can cache the data
if the query is the same and the data has not changed". The Observed
section describes framework `2bd4bd53d` (v3.2.1), checked against the code
by an independent reader the same day.

## Observed at 2bd4bd53d

Status: Observed

- No middleware sets security headers for an application. The debug page
  sends its own CSP, which allows no script (`framework/src/error/debug_page.rs:65-66,218-219`),
  and is built outside the middleware chain (`framework/src/server.rs:735-738`).
  Live's asset and transport responses send their own headers: `nosniff`
  (`framework/src/live/ui_assets.rs:78,90,117`; `framework/src/live/assets.rs:385,396,420`),
  and a CSP and `Referrer-Policy` on the async transport and uploads
  (`framework/src/live/async_transport.rs:575-586`; `framework/src/live/upload.rs:1347-1386`).
- No nonce is generated for an application; Live accepts one from the
  caller (`framework/src/live/assets.rs:140-145`).
- `RenderCache::install` registers RenderCache as the last global
  middleware (`framework/src/render_cache/mod.rs:629`; the scaffold's
  `live/mod.rs.tpl:67-74`). It declines to store a Complete response whose
  CSP header carries a nonce (CACHE-004; `framework/src/render_cache/middleware.rs:2789-2798,3920-3925`;
  `framework/src/render_cache/decline.rs:86-89`), and stitched Live
  documents get a fresh nonce per request (`framework/src/render_cache/stitch.rs:242-246`).
  Stored entries replay only the headers on a fixed list, which leaves out
  HSTS, `Content-Security-Policy-Report-Only` and `Reporting-Endpoints`
  (`crates/suprnova-live/src/render_cache/entry.rs:74-89`; `middleware.rs:3695-3697`).
- The Inertia error-page middleware keeps CSP headers across its swap
  (`framework/src/inertia/error_page_middleware.rs:436-448`).
- The framework's middleware is its own `Middleware` trait
  (`framework/src/middleware/mod.rs:103`), not tower layers.
- RenderCache records dependencies per render, in one task-local collector
  that exists only inside a RenderCache scope (`framework/src/render_cache/collector.rs:1-3,509-511`).
  It records tables, and rows only for primary-key point reads
  (`framework/src/eloquent/model.rs:633,739`); no framework read records a
  query class (`collector.rs:48-53`); table reads ignore the connection
  (`collector.rs:76-80`); counters move only where RenderCache is enabled
  and migrated, and are read in the render's snapshot transaction
  (`middleware.rs:3305-3319`). In authority mode a hit costs one ledger
  statement (`middleware.rs:1784-1789`).

## Requirements

[SEC-001] The framework MUST ship a `SecurityHeaders` middleware on its own
`Middleware` trait, configured from the environment or a builder, with
strict, balanced and relaxed presets; the scaffold MUST register the
balanced preset with nonces. It MUST be the outermost global middleware,
outside RenderCache and the error-page middleware, so every response,
cache hits and error pages included, carries the headers.
Falsifier: an application cannot turn the headers on without writing its own middleware; a strict-preset response can be framed, runs a script without the nonce, or omits HSTS on a secure request; a cache hit or an error page lacks the headers; or a new scaffold sends none.
Mechanism: `security-headers`.
Rationale: The developer chose a built-in implementation; sadco-io/http-security-headers-tower is a tower layer for Axum and Actix and exposes its nonce through their request extensions. Outermost, because stored entries replay only listed headers and RenderCache would never see an inner CSP.
Status: Draft

[SEC-002] The middleware MUST support Content-Security-Policy, with a
report-only mode and violation reporting; Strict-Transport-Security, sent
only on a secure request as the trusted-proxy rules decide, never in the
local environment, with preload opt-in; X-Frame-Options with the CSP
`frame-ancestors` directive; `X-Content-Type-Options: nosniff`;
Referrer-Policy; Permissions-Policy; and Cross-Origin-Opener-Policy,
-Embedder-Policy and -Resource-Policy. A nonce policy MUST apply only to
HTML responses.
Falsifier: one of these headers cannot be configured; HSTS is sent on a plain HTTP request or in the local environment; report-only mode enforces; or a JSON response carries a nonce policy.
Mechanism: `security-headers`.
Status: Draft

[SEC-003] With a nonce policy, the middleware MUST generate a fresh nonce
for every HTML request, readable by application code and views, record it
for RenderCache as Live's bootstrap nonce is recorded, and the framework
MUST stamp it on every executable script and script or style link it
emits: the Inertia document (RDOC-005), the development preamble, and
Live's bootstrap.
Falsifier: two requests share a nonce, application code cannot read it, or a framework-emitted executable script or link lacks it.
Mechanism: `security-headers`.
Status: Draft

[SEC-004] A route or group MUST be able to change or turn off any header,
and a header the response already carries MUST win over the middleware's,
so Live's own headers stand.
Falsifier: a route cannot override or disable a header, or the middleware replaces a header the response already carries.
Mechanism: `security-headers`.
Rationale: The checklist crate's `if_not_present`.
Status: Draft

[SEC-005] A Complete page served under a nonce CSP MUST render on every
request, and its queries MUST be served by the data cache (QCACHE-001 to
QCACHE-004): results keyed by query hash, deleted by the writes that touch
their tables and fields, triggered by the route. Stitched Live documents
keep CACHE-004's fresh nonce per hit, taken from the request.
Falsifier: a Complete nonce page is served from a stored rendering, or its queries bypass the data cache.
Mechanism: `security-headers`, `data-cache`.
Rationale: The developer's direction, "we can cache the data if the query is the same and the data has not changed", which is his original cache design (data-cache.md) and replaces stamping nonces into stored HTML.
Status: Draft

[SEC-006] The manual MUST document the middleware, its presets, every
header, nonces and the caching of nonce pages, with a "Why Suprnova
diverges" section: Laravel ships `FrameGuard` and `Vite::useCspNonce` but
no CSP, HSTS or policy middleware, and its applications use packages such
as `spatie/laravel-csp`.
Falsifier: the manual omits a header, the presets, nonces or the divergence.
Mechanism: `manual-check`.
Rationale: `Http/Middleware/FrameGuard.php:20`, `Foundation/Vite.php:161-163`.
Status: Draft
