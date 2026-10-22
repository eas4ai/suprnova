# Security headers

Status: Agreed 2026-10-15
Prefix: SEC

Drafted 2026-10-05 from the developer's direction "We should add full
support for security headers https://github.com/sadco-io/http-security-headers-tower",
his ruling "Yes I will accept your recommendation to build it into the
framework" (the crate is the checklist, not a dependency), and his
direction for caching pages served under a nonce: "we can cache the data
if the query is the same and the data has not changed", which the data
cache (QCACHE) carries with the search work since 2026-10-15. It closes the
last item of issue #143. The Observed section describes framework `16d607aad`;
the first draft described `2bd4bd53d` (v3.2.1).

## Observed at 16d607aad

Status: Observed

Re-checked against the code on 2026-10-15, when the developer put this work
ahead of search ("143 should have already been done before search").

- No middleware sets security headers for an application. No
  `SecurityHeaders` type and no HSTS, `X-Frame-Options`, Permissions-Policy
  or cross-origin policy setter exists in the framework; the only mentions
  are RenderCache's replay list and one test string
  (`crates/suprnova-live/src/render_cache/entry.rs:84-88`;
  `framework/src/inertia/error_page_middleware.rs:1092`).
- The debug page and the production error page send their own CSP, which
  allows no script (`framework/src/error/debug_page.rs:76-77,204-207,317-320`).
  For a browser's server error the server replaces the response outside the
  middleware chain (`framework/src/server.rs:971-1010`, `977-985`); both
  pages drop every `Content-*` header and keep the others
  (`debug_page.rs:957-971`), so headers an outer middleware set survive
  there except its CSP.
- Live's asset and transport responses send their own headers: `nosniff`
  (`framework/src/live/ui_assets.rs:177,189,267`;
  `framework/src/live/assets.rs:393,404,428`), and a CSP and
  `Referrer-Policy` on the async transport and uploads
  (`framework/src/live/async_transport.rs:571-590`;
  `framework/src/live/upload.rs:1340-1346,1390-1400`).
- No nonce is generated for an application request. Live accepts one from
  the caller (`framework/src/live/assets.rs:141-146`), and no scaffold or
  dogfood Live page passes one (`app/src/live/pages.rs:109`;
  `suprnova-cli/src/templates/files/live/src/views/mod.rs.tpl:101`).
  RenderCache mints its own: `fresh_nonce`
  (`crates/suprnova-live/src/render_cache/composite.rs:641-647`) fills a
  stitched hit's nonce holes and the stored headers that carry the nonce
  (`framework/src/render_cache/stitch.rs:242-246,886-903`;
  `composite.rs:278-307,1108-1113`). Live's bootstrap nonce is recorded for
  that hole-cutting (`framework/src/render_cache/live.rs:130-137`;
  `collector.rs:941-960`), and a second, different nonce in one render
  invalidates the capture (`collector.rs:934-938`).
- `RenderCache::install` registers RenderCache as the last global
  middleware (`framework/src/render_cache/mod.rs:684`; the scaffold's
  `live/mod.rs.tpl:66-74`). It declines to store a Complete response whose
  CSP carries a nonce (CACHE-004; `framework/src/render_cache/middleware.rs:2807-2817,3942-3951`;
  `framework/src/render_cache/decline.rs:86-89`). Stored entries replay
  only the 14 headers on a fixed list, which leaves out HSTS,
  `Content-Security-Policy-Report-Only` and `Reporting-Endpoints`
  (`entry.rs:74-89`; `middleware.rs:3720-3740`).
- The Inertia error-page middleware keeps CSP headers across its swap
  (`framework/src/inertia/error_page_middleware.rs:465-473,689-712`).
- The framework's middleware is its own `Middleware` trait
  (`framework/src/middleware/mod.rs:107`), not tower layers. The
  first-registered global middleware is the outermost
  (`framework/src/middleware/chain.rs:116`); the scaffold registers
  `LoggingMiddleware` first (`suprnova-cli/src/templates/files/backend/bootstrap.rs.tpl:115`),
  and `InertiaHeadersMiddleware` asks to be outermost so it sees every
  response (`framework/src/inertia/headers_middleware.rs:40-46`;
  registered at `framework/src/inertia/facade.rs:834`).
- A request knows whether it arrived over TLS: `Request::secure`, which
  believes forwarded headers only from `APP_TRUSTED_PROXIES`
  (`framework/src/http/request.rs:986-996`).
- RenderCache records dependencies per render, in one task-local collector
  that exists only inside a RenderCache scope (`collector.rs:1-3,395-417,509-511`).
  It records tables, rows for primary-key reads and a per-table identity
  for unkeyed writes (`framework/src/eloquent/model.rs:986-998,1093-1109`);
  no framework read records a query class (`collector.rs:49-53`); table
  reads ignore the connection (`collector.rs:76-80`); counters move only
  where RenderCache is enabled and migrated
  (`framework/src/render_cache/write_side.rs:11-16`;
  `framework/src/render_cache/orm.rs:64-68`), and are read in the render's
  snapshot transaction (`middleware.rs:3321-3345`). In authority mode a hit
  costs one ledger statement (`middleware.rs:1776-1794,1800-1807`).

## Requirements

[SEC-001] The framework MUST ship a `SecurityHeaders` middleware on its own
`Middleware` trait, configured from the environment or a builder, with
strict, balanced and relaxed presets; the scaffold MUST register the
balanced preset with nonces. It MUST be the outermost global middleware,
outside RenderCache and the error-page middleware, so every response,
cache hits and error pages included, carries the headers. An error page
the server replaces outside the chain keeps them, except that its own CSP,
which allows no script, stands.
Falsifier: an application cannot turn the headers on without writing its own middleware; a strict-preset response can be framed, runs a script without the nonce, or omits HSTS on a secure request; a cache hit or an error page lacks the headers (a server-replaced error page's own CSP excepted); or a new scaffold sends none.
Mechanism: `security-headers`.
Rationale: The developer chose a built-in implementation; sadco-io/http-security-headers-tower is a tower layer for Axum and Actix and exposes its nonce through their request extensions. Outermost, because stored entries replay only listed headers and RenderCache would never see an inner CSP.
Status: Agreed 2026-10-15

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
Status: Agreed 2026-10-15

[SEC-003] With a nonce policy, the middleware MUST generate a fresh nonce
for every HTML request, readable by application code and views, record it
for RenderCache as Live's bootstrap nonce is recorded, and the framework
MUST stamp it on every executable script and script or style link it
emits: the Inertia document (RDOC-005), the development preamble, and
Live's bootstrap. Where RenderCache fills a stitched hit's nonce holes and
nonce-bearing headers, it MUST use the request's nonce, not one it mints.
Falsifier: two requests share a nonce, application code cannot read it, a framework-emitted executable script or link lacks it, or a stitched hit's body and CSP carry a nonce other than the request's.
Mechanism: `security-headers`.
Status: Agreed 2026-10-15

[SEC-004] A route or group MUST be able to change or turn off any header,
and a header the response already carries MUST win over the middleware's,
so Live's own headers stand.
Falsifier: a route cannot override or disable a header, or the middleware replaces a header the response already carries.
Mechanism: `security-headers`.
Rationale: The checklist crate's `if_not_present`.
Status: Agreed 2026-10-15

[SEC-005] A Complete page served under a nonce CSP MUST render on every
request and MUST NOT be served from a stored rendering. A stitched Live
document keeps CACHE-004's fresh nonce per hit, and that nonce is the
request's (SEC-003). Serving such a page's queries from the data cache is
QCACHE-006, built with the search work.
Falsifier: a Complete nonce page is served from a stored rendering, or a stitched hit carries a nonce other than the request's.
Mechanism: `security-headers`.
Rationale: The developer moved the data cache to the search work on 2026-10-15 ("ok let's move it"); his direction for caching these pages, "we can cache the data if the query is the same and the data has not changed", is QCACHE-006.
Status: Agreed 2026-10-15

[SEC-006] The manual MUST document the middleware, its presets, every
header, nonces and the caching of nonce pages, with a "Why Suprnova
diverges" section: Laravel ships `FrameGuard` and `Vite::useCspNonce` but
no CSP, HSTS or policy middleware, and its applications use packages such
as `spatie/laravel-csp`.
Falsifier: the manual omits a header, the presets, nonces or the divergence.
Mechanism: `security-headers`.
Rationale: `Http/Middleware/FrameGuard.php:20`, `Foundation/Vite.php:161-163`.
Status: Agreed 2026-10-15
