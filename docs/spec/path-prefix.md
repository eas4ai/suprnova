# Serving under a path prefix

Status: Agreed 2026-10-05
Prefix: PFX

Drafted 2026-10-05 from issue #142 (an application served under a path
behind a reverse proxy that strips the prefix) and the developer's
acceptance of the recommendations made from the code that day ("I am going
to accept your recommendations"). The plan was posted on the issue. An
audit against main `c34ee7b15` the same day found browser URLs the draft
missed; the rulings of 2026-10-05 that settled its questions are written
into the requirements as design. The Observed section describes
`c34ee7b15`; only the requirements are contract once Agreed. Laravel
references cite `reference/framework-13.27.0/src/Illuminate/`.

The `path-prefix` mechanism runs Live's browser unit suite for PFX-006's
browser clauses, and rebuilds the tracked `dist/` and checks it against
the sources, as `mem-footprint` does, on the `node_modules` already
installed under `crates/suprnova-live/browser/`.

## Observed at c34ee7b15

Status: Observed

- Proxy headers are trusted only when the TCP peer is in the
  `APP_TRUSTED_PROXIES` allowlist, which is empty by default, so every
  `X-Forwarded-*` header is ignored until an operator lists a proxy
  (`framework/src/http/trusted_proxies.rs`, module docs). The request reads
  `X-Forwarded-For`, `-Host`, `-Port`, `-Proto`, `-Ssl` and `X-Real-IP`
  (`framework/src/http/request.rs:697-705,720-818,848-858,910-920,961-985`),
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
  `Path=/` (`framework/src/app/maintenance.rs:503,554,558`), and
  `InertiaResponse::location` and `location_for`
  (`framework/src/inertia/response.rs:951-955,969-980`).
  `InertiaResponse::redirect` writes `X-Inertia-Redirect`
  (`inertia/response.rs:993-997`), and `version_conflict` writes
  `X-Inertia-Location` (`inertia/response.rs:1313-1317`).
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
  time. The scaffold's Inertia pages post to and link literal paths, such
  as `form.post('/login')` and `router.post('/logout')`
  (`suprnova-cli/src/templates/files/frontend/vue/src/pages/auth/Login.vue.tpl:15`,
  `frontend/vue/src/pages/Dashboard.vue.tpl:8`, and their React and Svelte
  siblings).
- `sign_url` signs whatever URL it is given; `sign_route` signs the
  relative path `route()` returns (`framework/src/routing/signed.rs:147-164,236-260,428-440`);
  verification uses the path the application receives (`url.rs:218-222`).
- Live's endpoints are fixed at the root: the action endpoint and assets
  (`framework/src/live/routes.rs:20-21`; `framework/src/live/assets.rs:31,279,628-629`),
  the upload and async endpoints in the browser bundle
  (`crates/suprnova-live/browser/src/uploads/feature.ts:31`,
  `crates/suprnova-live/browser/src/async-updates/browser-host.ts:37-38`,
  `crates/suprnova-live/browser/src/async-updates/connections.ts:24-25`;
  LIVE-002 lists all six). The component assets are served under
  `/suprnova-ui/` (`framework/src/live/ui_assets.rs:24-25`) and, through
  `try_live_ui_assets_for`, under a library's `/<namespace>-ui/`
  (`framework/src/live/routes.rs:140`; `ui_assets.rs:116-118`). No
  framework code writes a link to those assets: application views do
  (`app/templates/live/forms.html:7-25`), and the shipped `header-bar`
  view defaults its `brand_href` to `/`
  (`crates/suprnova-live/components/header-bar/header-bar.html:8`). Live
  redirect targets build from the route pattern
  (`framework/src/live/ports/response.rs:147-157`; `router.rs:206-211`),
  and reflected URLs build from `request.path()`
  (`framework/src/live/document.rs:711`; `framework/src/render_cache/stitch.rs:212`);
  the browser refuses a reflected URL whose path differs from the
  document's (`crates/suprnova-live/browser/src/application/url.ts:19`).
- Pagination links use a path the caller supplies
  (`framework/src/pagination/mod.rs:290-291`); auth-flow mail links take a
  base URL from the caller (`framework/src/auth_flows/email_verify.rs:106-120`,
  `framework/src/auth_flows/password_reset.rs:115`); `Storage::url` with a
  relative base is root-relative (`framework/src/filesystem/mod.rs:1250`).
  The `lang` prop `LocaleShare` shares carries the catalog URL
  `/_suprnova/lang/<locale>.ftl?v=<hash>`
  (`framework/src/localization/mod.rs:672`), which the scaffold frontends
  fetch as given
  (`suprnova-cli/src/templates/files/frontend/vue/src/lib/lang.ts.tpl:71`
  and its siblings).
- The session, XSRF and remember-me cookies default to `Path=/`
  (`framework/src/session/config.rs:105,222`; `framework/src/csrf/middleware.rs:237`;
  `session/middleware.rs:1014,1045`); `SessionConfig` is built once at boot;
  a `__Host-` cookie name requires `Path=/` and is checked at boot
  (`session/config.rs:59-65`; `framework/src/config/mod.rs:100-118`).
  `SessionConfig.cookie_path` is a `String` defaulting to `/`, so an unset
  path cannot be told from `/` (`session/config.rs:42,105,222`), and the
  CSRF middleware copies it at construction (`csrf/middleware.rs:430-431`).
  `parse_cookies` keeps the last of two cookies with one name
  (`framework/src/http/cookie.rs:579-603`).
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
Apart from the value `/`, the value MUST be ignored unless every character
is an ASCII letter or digit, `-`, `.`, `_`, `~` or `/`, it starts with
`/`, does not end with `/`, has no empty, `.` or `..` segment, and the
request carries the header once. A header sent on two or more lines MUST
be ignored whole. An ignored value leaves the root to `APP_URL` (PFX-002).
A trusted value of exactly `/` MUST make the root `/`, the host root, in
place of the `APP_URL` path; an empty value MUST be ignored. The
manual's prefix-deployment section (PFX-009) MUST say that the proxy has
to set or clear the header, as the manual says for `X-Forwarded-For`.
Falsifier: a prefix from an untrusted peer, a value breaking the rule (`%2e%2e`, a non-ASCII letter, `/a/../b`, `/a/./b`, `/a//b`, `/billing/` or `billing`) or a value sent on two header lines reaches any URL the framework builds; a valid prefix from a trusted proxy does not; a trusted `/` leaves the `APP_URL` path in a URL; a trusted empty value removes it; or the prefix-deployment section does not say the proxy must set or clear `X-Forwarded-Prefix`.
Mechanism: `path-prefix`.
Rationale: Laravel's `TrustProxies` trusts the header from trusted proxies (`Http/Middleware/TrustProxies.php:22-27`). The developer accepted joining the existing gate rather than a separate switch, because Suprnova's trust is already opt-in. Ruled 2026-10-05: a repeated header is ignored, a trusted `/` means the root is `/`, and an empty value is ignored.
Status: Agreed 2026-10-05

[PFX-002] The framework MUST expose one public root for the URLs it builds
for the browser: the trusted prefix when the request carries one,
otherwise the path in `APP_URL`, which is also the root outside a request
(console commands, jobs, mail). A trusted prefix MUST replace the
`APP_URL` path, never add to it. The root is `/` at the host root. A URL
carries the root when its path is the root followed by the application
path, which at the host root is the application path unchanged; a path is
under the root when the root is `/`, or when it starts with the root, byte
for byte, followed by `/`, `?`, `#` or its end. `url::to`, `secure`,
`full`, `current`, `Request::url` and `full_url` MUST carry it. The root
MUST travel with the request, so a RenderCache background rebuild and the
error-page middleware use the request's root, captured before the handler
runs. The manual's prefix-deployment section (PFX-009) MUST say that a
deployment whose mail and job links must work behind a header-only prefix
sets the prefix in `APP_URL`.
Falsifier: with `APP_URL=https://example.org/billing` and no header, or with a trusted prefix `/billing`, a URL this spec covers lacks `/billing` or carries it twice (the `APP_URL` path and the header both `/billing`); a console command's URL ignores the `APP_URL` path; a RenderCache background rebuild, an error page or `url::current` renders without the root; or the prefix-deployment section does not say that mail and job links behind a header-only prefix need the prefix in `APP_URL`.
Mechanism: `path-prefix`.
Rationale: Today only `url::to` and the helpers built on it honour the `APP_URL` path. Laravel's `Request::root()` includes the prefix (`Http/Request.php:120-122`). Ruled 2026-10-05: `url::current` carries the root.
Status: Agreed 2026-10-05

## Where the root applies

[PFX-003] `route()`, `try_route()`, `route_with_params` and
`try_route_with_params` MUST return the root followed by the route's path,
still relative. Signed URLs MUST be signed and verified over the root and
the path, so a link signed behind a prefix verifies behind it.
Falsifier: `route("invoices.show", ...)` behind `/billing` returns `/invoices/7`; an application at `/` gets a different `route()` output than today; a URL signed behind the prefix fails verification behind it; or a URL signed under one root verifies under another.
Mechanism: `path-prefix`.
Rationale: Relative output keeps redirects and Inertia unchanged for applications at `/`; the issue's open question, answered.
Status: Agreed 2026-10-05

[PFX-004] Every `Location`, `X-Inertia-Location` and `X-Inertia-Redirect`
header the framework emits whose target is a root-relative path (PFX-010)
MUST carry the root exactly once: every `Redirect` constructor (`back`,
`refresh`, `refresh_for`, `guest`, `intended`, `signed_route` and
`temporary_signed_route` included), the
`redirect` and `redirect_to` helpers and the `redirect!` macro, the
previous and intended URLs the session records, the auth, email-verified,
two-factor and RBAC middleware, router redirects, maintenance mode,
`InertiaResponse::location`, `location_for`, `redirect` and
`version_conflict`, and Live's redirect targets. A path that already
carries the root, such as `route()`'s output, MUST NOT get it again.
Falsifier: any redirect to a root-relative path behind a prefix sends a `Location`, `X-Inertia-Location` or `X-Inertia-Redirect` without it, or `Redirect::to(route(...))`, `signed_route`, `back()` or `InertiaResponse::location(route(...))` sends it twice.
Mechanism: `path-prefix`.
Rationale: A rooted path is told apart from an application path by PFX-010. Ruled 2026-10-05: `X-Inertia-Redirect` is one of the headers that carry the root.
Status: Agreed 2026-10-05

[PFX-005] The Inertia page `url` and the version 409 `X-Inertia-Location`
MUST carry the root. The validation redirect MUST compare `Referer` with
the host the trust rule gives (`X-Forwarded-Host` from a trusted proxy,
else `Host`), and MUST treat a `Referer` on that host whose path is not
under the root (PFX-002) as foreign, falling back as it does for another
host. It MUST NOT add the root to a `Referer` that has it, and MUST add
it on its fallback paths. The Vite tags MUST be the root followed by a
root-relative `assets_base_url` (one starting with a single `/`),
attribute-escaped; an absolute or network-path (CDN) `assets_base_url`
MUST NOT get the root. The scaffold's three `vite.config.ts.tpl` files
MUST set `base` to `'./'`, and the scaffold's frontend modules, the entry
included, MUST import stylesheets and modules by relative or package
specifiers, never by a path that starts with `/`, so code-split chunks
and assets resolve against the entry script's URL, which the Vite tags
put under the root, and one build runs at `/` and under a prefix. Both
are checked from the templates, without a Vite build.
Falsifier: Inertia pushes an unprefixed URL into history; a 409 location lacks the prefix; a validation redirect doubles it, drops it, or follows a same-host `Referer` whose path is not under the root; behind a trusted proxy that sends `X-Forwarded-Host`, a validation redirect follows a `Referer` whose authority matches the raw `Host` but not the forwarded host, or treats one that matches the forwarded host as foreign; an asset tag lacks it, carries it on a CDN URL or carries an unescaped value; or a scaffold `vite.config.ts.tpl` sets `base` to anything but `'./'`, or a scaffold frontend template imports by a path that starts with `/`.
Mechanism: `path-prefix`.
Rationale: Ruled 2026-10-05: a `Referer` on the forwarded host whose path is outside the root is foreign, so the redirect falls back as it does for another host.
Status: Agreed 2026-10-05

[PFX-006] All six of Live's endpoints (LIVE-002), the upload endpoint
included, Live's assets under `/__live/assets` and the endpoint its boot
configuration names, Live's reflected URLs, the localization catalog URL
`/_suprnova/lang/<locale>.ftl` that the `lang` prop carries, the
framework's pagination links built from a path, and a `Storage::url` with
a root-relative base MUST carry the root. A query-only pagination link
MUST stay relative. This requirement covers the URLs the framework
builds; module specifiers inside a script it serves are not among them.
Falsifier: a Live island behind a prefix calls any of the six endpoints at the root, loads a Live asset from it, or has its URL reflection refused; the `lang` prop's catalog URL, a pagination link built from a path or a storage URL with a root-relative base lacks the prefix; or a query-only pagination link gains it.
Mechanism: `path-prefix`.
Rationale: Ruled 2026-10-05: the catalog URL is one of these URLs, and Live's upload endpoint sits under the root, which Live spec 08 is amended to say; module specifiers inside a component script are REG-032's, and this spec does not change them.
Status: Agreed 2026-10-05

[PFX-011] The framework MUST provide `suprnova::url::root()`, a public
function that carries no capability, so application views and a
component's view (REG-031) can write links under the root. It MUST return
the root of the request being handled, or the root PFX-002 gives outside a
request, with no trailing slash: `/billing` under a prefix `/billing`, the
empty string at the host root, so its output followed by
`/suprnova-ui/button/button.css` is a root-relative link at every root.
A component's view writes such a link in the one form REG-031 admits for
it: `url::root()` followed directly by constant text that starts with
exactly one `/` whose next character is neither `/` nor `\`, such as
`{{ suprnova::url::root() }}/suprnova-ui/button/button.css`. The manual
MUST write every component asset link under `/suprnova-ui/` or a
library's `/<namespace>-ui/` (REG-017) with it, and MUST name it as the
way a template writes a root-relative link that `route()` does not build.
A default link a shipped component's view writes, such as `header-bar`'s
`brand_href`, MUST carry the root. This requirement covers the URLs that
load a component's stylesheets and scripts, not the module specifiers
inside a script.
Falsifier: `suprnova::url::root()` does not exist, carries a capability in the REG allowlist, or returns anything but `/billing` behind a trusted prefix `/billing`, the empty string at the host root, or the `APP_URL` path outside a request; a manual chapter writes a `/suprnova-ui/` or `/<namespace>-ui/` link without it; or `header-bar` rendered without a `brand_href` behind `/billing` links to `/`.
Mechanism: `path-prefix`.
Rationale: Ruled 2026-10-05: views and application markup get the root from a capability-free `url::root()`. The framework writes none of these links itself (Observed), so the root reaches them only through a function a view can call. Laravel's `Request::root()` drops a trailing slash the same way (`Http/Request.php:120-122`).
Status: Agreed 2026-10-05

[PFX-012] The framework MUST provide an Inertia shared-data provider that
gives every page the root, as `url::root()` returns it, as the shared prop
`root`, and the scaffold MUST register it. Every page the scaffold writes
for its three frontends, the auth pages, the dashboard and the error page,
MUST build each URL it posts to, visits or links from that prop, so the
same frontend build runs at `/` and under `/billing`.
Falsifier: behind a trusted prefix `/billing`, a scaffolded application's page props carry no `root` or one other than `/billing`; or a scaffold page template posts to, visits or links a literal application path such as `/login`, `/logout`, `/register` or `/` instead of one built from the prop.
Mechanism: `path-prefix`.
Rationale: Ruled 2026-10-05: the scaffold's own pages work under a prefix, the same build at `/` and under `/billing`, through a shared root prop. The scaffold's server-side redirects get the root through PFX-004.
Status: Agreed 2026-10-05

## Cookies, cache and routing

[PFX-007] When the application sets no cookie path (no `SESSION_PATH`, no
`cookie_path` on the `SessionConfig` it builds and no
`CsrfMiddleware::xsrf_cookie_path`), the session, XSRF, remember-me and
maintenance-bypass cookies MUST default their `Path` to the root, set on
each response from that request's root. A `__Host-` cookie keeps
`Path=/`, as the prefix requires; the manual's prefix-deployment section
(PFX-009) MUST say that applications sharing a host with such cookies
need distinct cookie names.
Falsifier: with no cookie path set, the session, XSRF, remember-me or bypass cookie is set with a `Path` other than the request's root (`/` at the host root); a `__Host-` cookie is set with a `Path` other than `/`; or two applications under `/a` and `/b` on one host, with no `__Host-` name, overwrite each other's session cookie.
Mechanism: `path-prefix`.
Rationale: A cookie left at `Path=/` from before a deployment moved under a prefix is handled by PFX-013.
Status: Agreed 2026-10-05

[PFX-013] When a request carries two or more cookies of one name, the
framework MUST read the one sent first, which RFC 6265 section 5.4 orders
as the cookie with the longest path, so a stale `Path=/` cookie cannot
shadow the one scoped to the root. `parse_cookies` and every cookie the
framework reads, the session, XSRF, remember-me and maintenance-bypass
cookies included, MUST follow this rule.
Falsifier: for the header `Cookie: suprnova_session=scoped; suprnova_session=stale`, `parse_cookies` returns `stale` or the session middleware loads the session that `stale` names; or the same order for the XSRF, remember-me or bypass cookie yields the second value.
Mechanism: `path-prefix`.
Rationale: Ruled 2026-10-05: the first of two cookies of one name wins. `parse_cookies` keeps the last today (Observed), so after an upgrade a stale `Path=/` session cookie would shadow the root-scoped one until it expired.
Status: Agreed 2026-10-05

[PFX-008] The RenderCache key, for documents and their stitched and
nested entries, MUST include the root as a dimension no policy can omit,
so a representation rendered under one root is never served under
another.
Falsifier: a page cached under `/a`, a stale entry refreshed in the background included, is served or rebuilt for a request under `/b`.
Mechanism: `path-prefix`.
Status: Agreed 2026-10-05

[PFX-009] Route matching MUST stay on the path the application receives.
The manual MUST document prefix deployment in a section of
`manual/deployment.md`: a runnable proxy configuration, the trust rule,
the root, `url::root()`, and a `### Why Suprnova diverges` callout naming
the trailing slash that is ignored rather than trimmed, the relative
`route()` that carries the root, and the `APP_URL` path as the root when
no prefix header arrives. The paragraph on reverse proxies in
`manual/urls.md` MUST link to that section and MUST NOT describe a
trusted-proxy middleware that rewrites the request URL, which does not
exist.
Falsifier: a route stops matching because a prefix is present; `manual/deployment.md` has no prefix-deployment section naming `X-Forwarded-Prefix`, `APP_TRUSTED_PROXIES` and `url::root()` with that callout; or `manual/urls.md` still says a trusted-proxy middleware updates the request URL.
Mechanism: `path-prefix`.
Rationale: Laravel's relative `route($name, $parameters, false)` strips the base URL (`Routing/RouteUrlGenerator.php:103-108`), so keeping the root is a divergence the manual has to state.
Status: Agreed 2026-10-05

## Rooted paths

[PFX-010] A redirect target or a path given to a URL builder that starts
with exactly one `/` and is already under the root (PFX-002) MUST be left
as it is; any other target starting with exactly one `/` MUST get the
root. An absolute URL, a network-path reference (`//host/x`) and a
relative reference (`?q`, `#f`, `x`) MUST be left as they are. The
manual's prefix-deployment section (PFX-009) MUST say that an application
whose own routes begin with its deployment prefix cannot be told apart
and must not do so.
Falsifier: behind `/billing`, `Redirect::to(route(...))`, `signed_route`, `back()`, `Redirect::to("/billing?tab=2")` or `Redirect::to("/billing#x")` sends the root twice; `Redirect::to("?page=2")`, an absolute URL or a `//host` reference gains it; or a plain root-relative application path is sent without it.
Mechanism: `path-prefix`.
Rationale: Laravel avoids the question because its `route()` is absolute by default (`Routing/UrlGenerator.php:527`). Of the three options offered (leave a rooted path alone; a rooted-path type; absolute `route()`), the developer ruled "a" on 2026-10-05. Ruled 2026-10-05: a rooted path followed by `?` or `#`, and a query-only target, are left as they are.
Status: Agreed 2026-10-05
