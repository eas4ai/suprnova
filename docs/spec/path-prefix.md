# Serving under a path prefix

Status: Draft
Prefix: PFX

Drafted 2026-10-05 from issue #142 (an application served under a path
behind a reverse proxy that strips the prefix) and the developer's
acceptance of the recommendations made from the code that day ("I am going
to accept your recommendations"). The plan was posted on the issue. The
Observed section describes framework `2bd4bd53d` (v3.2.1), checked against
the code by an independent reader the same day; only the requirements are
contract once Agreed. Laravel references cite
`reference/framework-13.27.0/src/Illuminate/`.

## Observed at 2bd4bd53d

Status: Observed

- Proxy headers are trusted only when the TCP peer is in the
  `APP_TRUSTED_PROXIES` allowlist, which is empty by default, so every
  `X-Forwarded-*` header is ignored until an operator lists a proxy
  (`framework/src/http/trusted_proxies.rs`, module docs). The request reads
  `X-Forwarded-For`, `-Host`, `-Port`, `-Proto`, `-Ssl` and `X-Real-IP`
  (`framework/src/http/request.rs:697-705,720-818,848-856,910-920,961-985`),
  skips RFC 7239 `Forwarded` (`trusted_proxies.rs:83-85`), and has no prefix.
- `url::to`, `secure` and `full` join a path onto `APP_URL`, keeping the
  path `APP_URL` carries (`framework/src/routing/url.rs:65-71,101-103,295-320`);
  `Request::url` and `full_url` build from the received path
  (`request.rs:1023-1060`). `route()`, `route_with_params` and their `try_`
  forms return a root-relative path without it
  (`framework/src/routing/router.rs:534-542,548,605,624`).
- `Redirect::to` and `Redirect::route` send the path as given
  (`framework/src/http/response.rs:775-787,1263,1462-1507`); `refresh`
  reads the session's previous URL, `refresh_for` and `guest` build from
  `url::current`, and `intended` reads the session's intended URL
  (`response.rs:847-894`); `Redirect::signed_route` goes through `to`
  (`response.rs:917-919`). The session records the previous URL from
  `request.path()` (`framework/src/session/middleware.rs:1991-1997,2306-2316`)
  and `back()` reads it (`response.rs:819-824`).
- Other `Location` and `X-Inertia-Location` headers are written outside
  `Redirect`: the auth middleware (`framework/src/auth/middleware.rs:234-239,339`),
  the email-verified and two-factor middleware
  (`framework/src/auth_flows/email_verified_middleware.rs:99`,
  `framework/src/auth_flows/two_factor_challenge_middleware.rs:114`), the
  RBAC middleware (`framework/src/rbac/middleware.rs:41`), router
  redirects (`router.rs:2104`), maintenance mode, whose bypass cookie is
  `Path=/` (`framework/src/app/maintenance.rs:503,555,558`), and
  `InertiaResponse::location` (`framework/src/inertia/response.rs:970-979`).
- The Inertia page `url` is the request's path and query
  (`inertia/response.rs:1088-1091`); `InertiaConfig::url_resolver` can
  rewrite it (`framework/src/inertia/config.rs:823-829`), and the version
  409 ignores the resolver (`framework/src/inertia/version_middleware.rs:122-132`).
  The validation redirect takes its target from `Referer`, compares its
  authority with the raw `Host` header, and otherwise falls back to the
  previous URL or the current path
  (`framework/src/inertia/validation_redirect_middleware.rs:53,115-150`).
- The Vite tags prepend a static `assets_base_url`, unescaped
  (`inertia/response.rs:2526-2550`); the scaffold's Vite config sets no
  `base`, so code-split chunks import `/assets/...` paths fixed at build
  time.
- `sign_url` signs whatever URL it is given; `sign_route` signs the
  relative path `route()` returns (`framework/src/routing/signed.rs:147-164,236-260,428-440`);
  verification uses the path the application receives (`url.rs:218-222`).
- Live's endpoints are fixed at the root: the action endpoint and assets
  (`framework/src/live/routes.rs:20-21`; `framework/src/live/assets.rs:31,279,628-629`),
  the upload and async endpoints in the browser bundle
  (`crates/suprnova-live/browser/src/uploads/feature.ts:31`,
  `crates/suprnova-live/browser/src/async-updates/browser-host.ts:37-38`,
  `crates/suprnova-live/browser/src/async-updates/connections.ts:24-25`;
  LIVE-002 lists all six), and the component stylesheets under
  `/suprnova-ui/` (`framework/src/live/ui_assets.rs:21`). Live redirect
  targets and reflected URLs build from `request.path()`
  (`framework/src/live/ports/response.rs:147-157`;
  `framework/src/live/document.rs:711`; `framework/src/render_cache/stitch.rs:212`).
- Pagination links use a path the caller supplies
  (`framework/src/pagination/mod.rs:290-291`); auth-flow mail links take a
  base URL from the caller (`framework/src/auth_flows/email_verify.rs:106-120`,
  `framework/src/auth_flows/password_reset.rs:115`); `Storage::url` with a
  relative base is root-relative (`framework/src/filesystem/mod.rs:1250`).
- The session, XSRF and remember-me cookies default to `Path=/`
  (`framework/src/session/config.rs:105,222`; `framework/src/csrf/middleware.rs:237`;
  `session/middleware.rs:1014,1045`); `SessionConfig` is built once at boot;
  a `__Host-` cookie name requires `Path=/` and is checked at boot
  (`session/config.rs:59-65`; `framework/src/config/mod.rs:100-118`).
- The RenderCache key has no prefix dimension
  (`framework/src/render_cache/middleware.rs:1486-1517`), and its
  background rebuilds run in spawned tasks that do not inherit task-locals
  (`middleware.rs:1146-1161,1184-1212`).
- The error-page middleware captures the request without its peer address
  or trust configuration (`framework/src/inertia/error_page_middleware.rs:249-262`).
- CSRF checks `Sec-Fetch-Site` and compares no origin with `APP_URL`
  (`csrf/middleware.rs:463-484`), so it is unaffected by a prefix.

## The prefix and the root

[PFX-001] The request MUST read `X-Forwarded-Prefix` under the same rule
as the other forwarded headers: only when the TCP peer is a trusted proxy.
The value MUST be ignored unless every character is a letter, digit, `-`,
`.`, `_`, `~` or `/`, it starts with `/`, does not end with `/`, and has no
empty, `.` or `..` segment. The deployment guidance MUST say that the
proxy has to set or clear the header, as it says for `X-Forwarded-For`.
Falsifier: a prefix from an untrusted peer, or a value breaking the rule (`%2e%2e` included), reaches any URL the framework builds; or a valid prefix from a trusted proxy does not.
Mechanism: `path-prefix`.
Rationale: Laravel's `TrustProxies` trusts the header from trusted proxies (`Http/Middleware/TrustProxies.php:22-27`). The developer accepted joining the existing gate rather than a separate switch, because Suprnova's trust is already opt-in.
Status: Draft

[PFX-002] The framework MUST expose one public root for the URLs it builds
for the browser: the trusted prefix when the request carries one,
otherwise the path in `APP_URL`, which is also the root outside a request
(console commands, jobs, mail). `url::to`, `secure`, `full`, `Request::url`
and `full_url` MUST carry it. The root MUST travel with the request, so a
RenderCache background rebuild and the error-page middleware use the
request's root, captured before the handler runs. A deployment whose mail
and job links must work behind a header-only prefix MUST set the prefix
in `APP_URL`; the manual MUST say so.
Falsifier: with `APP_URL=https://example.org/billing` and no header, or with a trusted prefix `/billing`, any URL this spec covers lacks `/billing`; a console command's URL ignores the `APP_URL` path; or a background rebuild or error page renders without the root.
Mechanism: `path-prefix`.
Rationale: Today only `url::to` honours the `APP_URL` path. Laravel's `Request::root()` includes the prefix (`Http/Request.php:120-122`).
Status: Draft

## Where the root applies

[PFX-003] `route()`, `try_route()`, `route_with_params` and
`try_route_with_params` MUST return the root followed by the route's path,
still relative. Signed URLs MUST be signed and verified over the root and
the path, so a link signed behind a prefix verifies behind it.
Falsifier: `route("invoices.show", ...)` behind `/billing` returns `/invoices/7`; an application at `/` gets a different `route()` output than today; or a URL signed behind the prefix fails verification behind it.
Mechanism: `path-prefix`.
Rationale: Relative output keeps redirects and Inertia unchanged for applications at `/`; the issue's open question, answered.
Status: Draft

[PFX-004] Every `Location` and `X-Inertia-Location` header the framework
emits MUST carry the root exactly once: every `Redirect` constructor
(`refresh`, `refresh_for`, `guest`, `intended` and `signed_route`
included), the previous and intended URLs the session records, the auth,
email-verified, two-factor and RBAC middleware, router redirects,
maintenance mode, `InertiaResponse::location`, and Live's redirect
targets. A path that already carries the root, such as `route()`'s
output, MUST NOT get it again.
Falsifier: any redirect behind a prefix sends a `Location` without it, or `Redirect::to(route(...))`, `signed_route` or `back()` sends it twice.
Mechanism: `path-prefix`.
Rationale: A rooted path is told apart from an application path by PFX-010.
Status: Draft

[PFX-005] The Inertia page `url` and the version 409 `X-Inertia-Location`
MUST carry the root. The validation redirect MUST compare `Referer` with
the forwarded host, not the raw `Host` header, MUST NOT add the root to a
`Referer` that has it, and MUST add it on its fallback paths. The Vite
tags MUST be the root followed by a relative `assets_base_url`,
attribute-escaped, and an absolute (CDN) `assets_base_url` MUST NOT get
the root. The scaffold's Vite build MUST resolve code-split chunks from
the base the document gives them, so one build runs at `/` and under a
prefix.
Falsifier: Inertia pushes an unprefixed URL into history; a 409 location lacks the prefix; a validation redirect doubles it or drops it; an asset tag lacks it, carries it on a CDN URL or carries an unescaped value; or a lazily loaded chunk 404s under a prefix.
Mechanism: `path-prefix`.
Status: Draft

[PFX-006] All six of Live's endpoints (LIVE-002), its assets, its
component stylesheets under `/suprnova-ui/` and its reflected URLs, the
framework's pagination links and a relative `Storage::url` MUST carry the
root.
Falsifier: a Live island behind a prefix calls any endpoint at the root, a component stylesheet or pagination link lacks the prefix, or a relative storage URL does.
Mechanism: `path-prefix`.
Status: Draft

## Cookies, cache and routing

[PFX-007] When `SESSION_PATH` is not set, the session, XSRF, remember-me
and maintenance-bypass cookies MUST default their `Path` to the root, set
on each response. A `__Host-` cookie keeps `Path=/`, as the prefix
requires; the manual MUST say that applications sharing a host with such
cookies need distinct cookie names.
Falsifier: two applications under `/a` and `/b` on one host overwrite each other's session cookie with no `SESSION_PATH` set and no `__Host-` name.
Mechanism: `path-prefix`.
Status: Draft

[PFX-008] The RenderCache key MUST include the root, so a representation
rendered under one root is never served under another.
Falsifier: a page cached under `/a` is served for a request under `/b`.
Mechanism: `path-prefix`.
Status: Draft

[PFX-009] Route matching MUST stay on the path the application receives;
the manual MUST document prefix deployment, the trust rule and the root.
Falsifier: a route stops matching because a prefix is present, or the manual does not describe the setup.
Mechanism: `path-prefix`, `manual-check`.
Status: Draft

## Rooted paths

[PFX-010] A redirect target or a path given to a URL builder that already
starts with the root, followed by `/` or its end, MUST be left as it is;
any other application path gets the root. The manual MUST say that an
application whose own routes begin with its deployment prefix cannot be
told apart and must not do so.
Falsifier: `Redirect::to(route(...))`, `signed_route` or `back()` sends the root twice, or a plain application path is sent without it.
Mechanism: `path-prefix`.
Rationale: Laravel avoids the question because its `route()` is absolute by default (`Routing/UrlGenerator.php:527`). Of the three options offered (leave a rooted path alone; a rooted-path type; absolute `route()`), the developer ruled "a" on 2026-10-05.
Status: Draft
